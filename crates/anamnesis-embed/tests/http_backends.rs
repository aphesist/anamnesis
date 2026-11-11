//! HTTP backends against a tiny in-process mock server (std only, no network).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use anamnesis_embed::{
    Embedder, EmbedderSpec, HttpOptions, OllamaEmbedder, OpenAiCompatibleEmbedder, Prompts, RetryPolicy, build,
    build_with_secret, dot,
};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct Recorded {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn inputs(&self) -> Vec<String> {
        self.body["input"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect()
    }
}

type Handler = dyn Fn(usize, &Recorded) -> (u16, String) + Send + Sync;

struct Mock {
    url: String,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl Mock {
    fn start(handler: impl Fn(usize, &Recorded) -> (u16, String) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let log: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let log2 = log.clone();
        let handler: Arc<Handler> = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut headers = Vec::new();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push((k.trim().to_owned(), v.trim().to_owned()));
                    }
                }
                let len: usize = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0);
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                let rec = Recorded { path, headers, body: serde_json::from_slice(&body).unwrap_or(Value::Null) };
                let n = {
                    let mut l = log2.lock().unwrap();
                    l.push(rec.clone());
                    l.len() - 1
                };
                let (status, resp) = handler(n, &rec);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp}",
                    resp.len()
                );
                let _ = stream.flush();
            }
        });
        Self { url, log }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

/// A fake "model": a 4-d vector derived from the text, deliberately not
/// normalised so the client's normalisation is exercised.
fn fake_vec(text: &str) -> Vec<f32> {
    let len = text.chars().count() as f32;
    let q = if text.contains("Query:") || text.starts_with("query: ") { 5.0 } else { 0.0 };
    vec![len, 2.0, q, 1.0]
}

fn fast_opts() -> HttpOptions {
    HttpOptions {
        batch_size: 2,
        max_input_tokens: 0,
        timeout_secs: 5,
        retry: RetryPolicy { max_retries: 2, base_delay_ms: 5, max_delay_ms: 20 },
    }
}

fn ollama_handler(_: usize, r: &Recorded) -> (u16, String) {
    assert_eq!(r.path, "/api/embed");
    let embs: Vec<Vec<f32>> = r.inputs().iter().map(|t| fake_vec(t)).collect();
    (200, json!({ "model": r.body["model"], "embeddings": embs }).to_string())
}

fn texts(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn ollama_batches_normalises_and_prefixes() {
    let mock = Mock::start(ollama_handler);
    let e = OllamaEmbedder::new(Some(&mock.url), "qwen3-embedding:0.6b", None, Some(4), fast_opts());
    assert!(e.is_local());
    assert_eq!(e.id(), "ollama:qwen3-embedding:0.6b@4");

    let docs = texts(&["a", "bb", "ccc", "dddd", "eeeee"]);
    let out = e.embed(&docs).unwrap();
    assert_eq!(out.len(), 5);
    for (v, t) in out.iter().zip(&docs) {
        assert!((dot(v, v) - 1.0).abs() < 1e-5);
        let mut expect = fake_vec(t);
        anamnesis_embed::l2_normalize(&mut expect);
        assert_eq!(v, &expect, "order must be preserved");
    }
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 3, "5 texts at batch size 2");
    assert_eq!(reqs[0].body["model"], "qwen3-embedding:0.6b");
    assert_eq!(reqs[0].body["truncate"], true);
    assert_eq!(reqs[0].body["dimensions"], 4);
    // Qwen3 documents carry no prefix.
    assert_eq!(reqs[0].inputs(), texts(&["a", "bb"]));

    let q = e.embed_query(&texts(&["what is memory?"])).unwrap();
    assert_eq!(q.len(), 1);
    let last = mock.requests().pop().unwrap();
    let sent = &last.inputs()[0];
    assert!(sent.starts_with("Instruct: "), "{sent}");
    assert!(sent.ends_with("\nQuery:what is memory?"), "{sent}");
    assert_eq!(e.dim(), 4);
}

#[test]
fn ollama_learns_dim_and_probes() {
    let mock = Mock::start(ollama_handler);
    let e = OllamaEmbedder::new(Some(&mock.url), "bge-m3", None, None, fast_opts());
    assert_eq!(e.id(), "ollama:bge-m3");
    // Unknown dimension: dim() probes the server once.
    assert_eq!(e.dim(), 4);
    assert_eq!(mock.requests().len(), 1);
    assert_eq!(e.dim(), 4);
    assert_eq!(mock.requests().len(), 1, "dimension is cached");
    // Empty input makes no request.
    assert!(e.embed(&[]).unwrap().is_empty());
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn unreachable_server_dim_is_zero_and_embed_errors() {
    // Bind then drop to get a port nobody listens on.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}");
    let mut opts = fast_opts();
    opts.retry = RetryPolicy { max_retries: 1, base_delay_ms: 1, max_delay_ms: 2 };
    let e = OllamaEmbedder::new(Some(&url), "m", None, None, opts);
    assert_eq!(e.dim(), 0);
    let err = e.embed(&texts(&["x"])).unwrap_err();
    assert!(format!("{err:#}").contains("gave up after 2 attempts"), "{err:#}");
}

fn openai_handler(_: usize, r: &Recorded) -> (u16, String) {
    assert_eq!(r.path, "/v1/embeddings");
    // Return items in reverse order with explicit indices.
    let mut data: Vec<Value> = r
        .inputs()
        .iter()
        .enumerate()
        .map(|(i, t)| json!({ "object": "embedding", "index": i, "embedding": fake_vec(t) }))
        .collect();
    data.reverse();
    (200, json!({ "object": "list", "data": data, "model": r.body["model"] }).to_string())
}

#[test]
fn openai_compatible_auth_order_and_prompts() {
    let mock = Mock::start(openai_handler);
    let base = format!("{}/v1/", mock.url);
    let e = OpenAiCompatibleEmbedder::new(
        &base,
        "intfloat/multilingual-e5-small",
        Some("sk-secret".into()),
        None,
        None,
        fast_opts(),
    );
    assert!(e.is_local());
    assert!(!format!("{e:?}").contains("sk-secret"));
    let docs = texts(&["um", "dois", "tres"]);
    let out = e.embed(&docs).unwrap();
    for (v, t) in out.iter().zip(&docs) {
        let mut expect = fake_vec(&format!("passage: {t}"));
        anamnesis_embed::l2_normalize(&mut expect);
        assert_eq!(v, &expect);
    }
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].header("authorization"), Some("Bearer sk-secret"));
    assert_eq!(reqs[0].body["encoding_format"], "float");
    assert!(reqs[0].body.get("dimensions").is_none());
    assert_eq!(reqs[0].inputs(), texts(&["passage: um", "passage: dois"]));

    e.embed_query(&texts(&["onde?"])).unwrap();
    assert_eq!(mock.requests().pop().unwrap().inputs(), texts(&["query: onde?"]));
    assert_eq!(e.dim(), 4);
}

#[test]
fn openai_no_key_sends_no_auth_and_custom_prompts() {
    let mock = Mock::start(openai_handler);
    let e = OpenAiCompatibleEmbedder::new(
        &format!("{}/v1", mock.url),
        "m",
        None,
        Some(Prompts { query: "Q> ".into(), document: "D> ".into() }),
        Some(4),
        fast_opts(),
    );
    e.embed(&texts(&["x"])).unwrap();
    let r = &mock.requests()[0];
    assert!(r.header("authorization").is_none());
    assert_eq!(r.body["dimensions"], 4);
    assert_eq!(r.inputs(), texts(&["D> x"]));
}

#[test]
fn retries_transient_errors_then_succeeds() {
    let mock = Mock::start(|n, r| {
        if n < 2 { (503, r#"{"error":"loading model"}"#.into()) } else { openai_handler(n, r) }
    });
    let e = OpenAiCompatibleEmbedder::new(&format!("{}/v1", mock.url), "m", None, None, None, fast_opts());
    let out = e.embed(&texts(&["hello"])).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(mock.requests().len(), 3);
}

#[test]
fn gives_up_after_max_retries() {
    let mock = Mock::start(|_, _| (429, r#"{"error":"rate limited"}"#.into()));
    let e = OpenAiCompatibleEmbedder::new(&format!("{}/v1", mock.url), "m", None, None, None, fast_opts());
    let err = format!("{:#}", e.embed(&texts(&["hello"])).unwrap_err());
    assert!(err.contains("429") && err.contains("gave up after 3 attempts"), "{err}");
    assert_eq!(mock.requests().len(), 3);
}

#[test]
fn client_errors_are_not_retried_and_key_never_leaks() {
    let mock = Mock::start(|_, _| (401, r#"{"error":{"message":"invalid api key"}}"#.into()));
    let e = OpenAiCompatibleEmbedder::new(
        &format!("{}/v1", mock.url),
        "m",
        Some("sk-very-secret-key".into()),
        None,
        None,
        fast_opts(),
    );
    let err = format!("{:#} {:?}", e.embed(&texts(&["hello"])).unwrap_err(), e);
    assert!(err.contains("401"), "{err}");
    assert!(!err.contains("sk-very-secret-key"), "{err}");
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn wrong_count_or_dimension_is_an_error() {
    let mock = Mock::start(|_, _| (200, json!({ "data": [{ "index": 0, "embedding": [1.0, 0.0] }] }).to_string()));
    let e = OpenAiCompatibleEmbedder::new(&format!("{}/v1", mock.url), "m", None, None, None, fast_opts());
    assert!(e.embed(&texts(&["a", "b"])).is_err());

    let mock = Mock::start(|_, _| (200, json!({ "embeddings": [[1.0, 0.0], [1.0]] }).to_string()));
    let e = OllamaEmbedder::new(Some(&mock.url), "m", None, None, fast_opts());
    assert!(e.embed(&texts(&["a", "b"])).is_err());

    let mock = Mock::start(|_, _| (200, "not json".into()));
    let e = OllamaEmbedder::new(Some(&mock.url), "m", None, None, fast_opts());
    assert!(e.embed(&texts(&["a"])).is_err());
}

#[test]
fn long_inputs_are_truncated_before_sending() {
    let mock = Mock::start(ollama_handler);
    let mut opts = fast_opts();
    opts.max_input_tokens = 50;
    let e = OllamaEmbedder::new(Some(&mock.url), "nomic-embed-text", None, None, opts);
    let long = "palavra ".repeat(5_000);
    e.embed(&[long.clone()]).unwrap();
    let sent = &mock.requests()[0].inputs()[0];
    assert!(sent.starts_with("search_document: palavra"));
    assert!(sent.len() < 400, "sent {} bytes", sent.len());
    assert!(anamnesis_embed::approx_tokens(sent) <= 50);
}

#[test]
fn build_from_spec_talks_to_server() {
    let mock = Mock::start(openai_handler);
    let spec: EmbedderSpec = serde_json::from_value(json!({
        "backend": "openai_compatible",
        "base_url": format!("{}/v1", mock.url),
        "model": "text-embedding-3-small",
        "truncate_dim": 2,
        "batch_size": 10,
        "retry": { "max_retries": 0 }
    }))
    .unwrap();
    let e = build_with_secret(&spec, &std::env::temp_dir(), Some("k")).unwrap();
    assert!(e.is_local());
    assert_eq!(e.dim(), 2);
    let v = e.embed(&texts(&["a", "b", "c"])).unwrap();
    assert_eq!(v.len(), 3);
    assert!(v.iter().all(|x| x.len() == 2 && (dot(x, x) - 1.0).abs() < 1e-5));
    assert_eq!(mock.requests().len(), 1);
    assert_eq!(mock.requests()[0].header("authorization"), Some("Bearer k"));

    let o = build(&EmbedderSpec::Ollama {
        base_url: Some(mock.url.clone()),
        model: "m".into(),
        prompts: None,
        dimensions: None,
        truncate_dim: None,
        http: HttpOptions::default(),
    }, &std::env::temp_dir())
    .unwrap();
    assert!(o.is_local());
}
