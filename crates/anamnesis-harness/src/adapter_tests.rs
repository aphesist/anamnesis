//! Adapter tests: SSE fixtures per api format, mock http servers for streaming, 429 handling.

use bytes::Bytes;
use futures_util::stream;

use crate::adapters::anthropic::{Anthropic, AnthropicConfig, MessagesDecoder};
use crate::adapters::gemini::{Gemini, GeminiConfig, GeminiDecoder};
use crate::adapters::openai_compatible::{
    ChatCompletionsDecoder, OpenAiCompatible, OpenAiCompatibleConfig,
};
use crate::backend::Backend;
use crate::decode::{Decoder, UsageAcc, decode_sse};
use crate::error::HarnessError;
use crate::limits::BackoffPolicy;
use crate::testutil::{MockServer, Reply, collect};
use crate::types::*;

pub(crate) const OPENAI_SSE: &str = "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hel\"}}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo, wörld\"}}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":4,\"total_tokens\":16}}\n\n\
data: [DONE]\n\n";

pub(crate) const ANTHROPIC_SSE: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-haiku-5-5\",\"usage\":{\"input_tokens\":25,\"cache_read_input_tokens\":5,\"output_tokens\":1}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: ping\n\
data: {\"type\":\"ping\"}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"a quiet \"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"room\"}}\n\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

pub(crate) const ANTHROPIC_ERROR_SSE: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n\
event: error\n\
data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";

pub(crate) const GEMINI_SSE: &str = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"thinking\",\"thought\":true}],\"role\":\"model\"}}]}\r\n\r\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"stip\"}],\"role\":\"model\"}}],\"usageMetadata\":{\"promptTokenCount\":9}}\r\n\r\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ple\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":9,\"candidatesTokenCount\":2,\"thoughtsTokenCount\":3,\"totalTokenCount\":14}}\r\n\r\n";

/// Run fixture bytes through a decoder, split into `chunk`-byte pieces.
async fn run<D: Decoder>(fixture: &str, chunk: usize, dec: D) -> (String, Vec<ChatEvent>) {
    let chunks: Vec<Result<Bytes, std::io::Error>> = fixture
        .as_bytes()
        .chunks(chunk)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    collect(decode_sse(stream::iter(chunks), dec)).await
}

fn usage_of(evs: &[ChatEvent]) -> Usage {
    evs.iter()
        .find_map(|e| match e {
            ChatEvent::Usage(u) => Some(*u),
            _ => None,
        })
        .expect("usage event")
}

fn assert_ends_done(evs: &[ChatEvent]) {
    assert!(
        matches!(evs.last(), Some(ChatEvent::Done { .. })),
        "{evs:?}"
    );
    let n_done = evs
        .iter()
        .filter(|e| matches!(e, ChatEvent::Done { .. }))
        .count();
    assert_eq!(n_done, 1);
    // usage comes before done
    let u = evs
        .iter()
        .position(|e| matches!(e, ChatEvent::Usage(_)))
        .unwrap();
    assert!(u < evs.len() - 1);
}

#[tokio::test]
async fn openai_fixture_any_chunking() {
    for chunk in [1, 3, 7, 64, 4096] {
        let (text, evs) = run(
            OPENAI_SSE,
            chunk,
            ChatCompletionsDecoder::new(UsageAcc::new(99)),
        )
        .await;
        assert_eq!(text, "hello, wörld");
        assert_eq!(
            usage_of(&evs),
            Usage {
                input: 12,
                output: 4,
                estimated: false,
                reported_cost_usd: None
            }
        );
        assert_ends_done(&evs);
        assert!(
            matches!(evs.last(), Some(ChatEvent::Done { stop_reason: Some(r) }) if r == "stop")
        );
    }
}

#[tokio::test]
async fn openai_without_usage_is_estimated() {
    let fixture =
        "data: {\"choices\":[{\"delta\":{\"content\":\"abcdefgh\"}}]}\n\ndata: [DONE]\n\n";
    let (_, evs) = run(fixture, 5, ChatCompletionsDecoder::new(UsageAcc::new(10))).await;
    assert_eq!(
        usage_of(&evs),
        Usage {
            input: 10,
            output: 2,
            estimated: true,
            reported_cost_usd: None
        }
    );
}

#[tokio::test]
async fn anthropic_fixture() {
    for chunk in [1, 11, 4096] {
        let (text, evs) = run(ANTHROPIC_SSE, chunk, MessagesDecoder::new(UsageAcc::new(0))).await;
        assert_eq!(text, "a quiet room");
        assert_eq!(
            usage_of(&evs),
            Usage {
                input: 30,
                output: 7,
                estimated: false,
                reported_cost_usd: None
            }
        );
        assert!(
            matches!(evs.last(), Some(ChatEvent::Done { stop_reason: Some(r) }) if r == "end_turn")
        );
    }
}

#[tokio::test]
async fn anthropic_stream_error() {
    let (_, evs) = run(
        ANTHROPIC_ERROR_SSE,
        9,
        MessagesDecoder::new(UsageAcc::new(0)),
    )
    .await;
    assert!(matches!(
        evs.last(),
        Some(ChatEvent::Error {
            failure: FailureKind::Unavailable,
            ..
        })
    ));
}

#[tokio::test]
async fn gemini_fixture_skips_thoughts() {
    for chunk in [2, 13, 4096] {
        let (text, evs) = run(GEMINI_SSE, chunk, GeminiDecoder::new(UsageAcc::new(0))).await;
        assert_eq!(text, "stipple");
        assert_eq!(
            usage_of(&evs),
            Usage {
                input: 9,
                output: 5,
                estimated: false,
                reported_cost_usd: None
            }
        );
        assert!(
            matches!(evs.last(), Some(ChatEvent::Done { stop_reason: Some(r) }) if r == "STOP")
        );
    }
}

#[tokio::test]
async fn truncated_stream_still_terminates() {
    let fixture = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";
    let (text, evs) = run(fixture, 4, ChatCompletionsDecoder::new(UsageAcc::new(1))).await;
    assert_eq!(text, "hi");
    assert_ends_done(&evs);
}

fn req() -> ChatRequest {
    let mut r = ChatRequest::new(vec![
        Message::user("hello"),
        Message::assistant("hi"),
        Message::user("label this"),
    ])
    .with_system("be brief");
    r.temperature = Some(0.2);
    r.max_tokens = Some(100);
    r
}

fn fast_policy() -> BackoffPolicy {
    BackoffPolicy {
        base_ms: 5,
        max_ms: 20,
        max_retries: 2,
        max_inline_wait_ms: 2_000,
    }
}

#[tokio::test]
async fn openai_compatible_against_mock() {
    let srv = MockServer::start(|r| {
        (r.method == "POST" && r.path == "/v1/chat/completions").then(|| Reply::sse(OPENAI_SSE, 17))
    })
    .await;
    let mut cfg = OpenAiCompatibleConfig::ollama("llama3");
    cfg.base_url = format!("{}/v1", srv.base);
    cfg.api_key = Some("sk-test".into());
    let b = OpenAiCompatible::new(cfg, fast_policy());
    assert!(b.is_local());
    let (text, evs) = collect(b.chat(req()).await.unwrap()).await;
    assert_eq!(text, "hello, wörld");
    assert_ends_done(&evs);
    let rec = &srv.recorded()[0];
    assert_eq!(rec.headers["authorization"], "Bearer sk-test");
    let body: serde_json::Value = serde_json::from_str(&rec.body).unwrap();
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "llama3");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"].as_array().unwrap().len(), 4);
    assert_eq!(body["max_tokens"], 100);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[tokio::test]
async fn openai_compatible_models_and_availability() {
    let srv = MockServer::start(|r| {
        (r.path == "/v1/models").then(|| {
            Reply::json(
                200,
                r#"{"object":"list","data":[{"id":"qwen3"},{"id":"llama3"}]}"#,
            )
        })
    })
    .await;
    let mut cfg = OpenAiCompatibleConfig::ollama("llama3");
    cfg.base_url = format!("{}/v1", srv.base);
    let a = OpenAiCompatible::new(cfg, fast_policy()).available().await;
    assert!(a.available);
    assert_eq!(a.models, vec!["qwen3", "llama3"]);
}

#[tokio::test]
async fn anthropic_against_mock() {
    let srv = MockServer::start(|r| {
        (r.method == "POST" && r.path == "/v1/messages").then(|| Reply::sse(ANTHROPIC_SSE, 23))
    })
    .await;
    let cfg = AnthropicConfig {
        base_url: srv.base.clone(),
        api_key: Some("ak-test".into()),
        ..Default::default()
    };
    let b = Anthropic::new(cfg, fast_policy());
    let mut r = req();
    r.model = Some(b.model_for(Task::Label));
    r.json_schema = Some(serde_json::json!({"type":"object"}));
    let (text, _) = collect(b.chat(r).await.unwrap()).await;
    assert_eq!(text, "a quiet room");
    let rec = &srv.recorded()[0];
    assert_eq!(rec.headers["x-api-key"], "ak-test");
    assert_eq!(rec.headers["anthropic-version"], "2023-06-01");
    let body: serde_json::Value = serde_json::from_str(&rec.body).unwrap();
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(body["system"], "be brief");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 100);
    assert!(
        body.get("temperature").is_none(),
        "temperature is not sent to current models"
    );
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(body["messages"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn anthropic_without_key_is_not_configured() {
    let b = Anthropic::new(AnthropicConfig::default(), fast_policy());
    assert!(matches!(
        b.chat(req()).await,
        Err(HarnessError::NotConfigured(_))
    ));
    assert!(!b.available().await.available);
}

#[tokio::test]
async fn gemini_against_mock() {
    let srv = MockServer::start(|r| {
        (r.method == "POST" && r.path == "/v1beta/models/gemini-test:streamGenerateContent?alt=sse")
            .then(|| Reply::sse(GEMINI_SSE, 31))
    })
    .await;
    let cfg = GeminiConfig {
        base_url: srv.base.clone(),
        api_key: Some("gk".into()),
        model: "gemini-test".into(),
        label_model: None,
        ..Default::default()
    };
    let b = Gemini::new(cfg, fast_policy());
    let (text, evs) = collect(b.chat(req()).await.unwrap()).await;
    assert_eq!(text, "stipple");
    assert_ends_done(&evs);
    let rec = &srv.recorded()[0];
    assert_eq!(rec.headers["x-goog-api-key"], "gk");
    let body: serde_json::Value = serde_json::from_str(&rec.body).unwrap();
    assert_eq!(body["contents"][1]["role"], "model");
    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be brief");
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 100);
}

#[tokio::test]
async fn rate_limit_429_with_retry_after() {
    // long retry-after: not slept through, surfaced as RateLimited with the duration
    let srv = MockServer::start(|_| {
        Some(
            Reply::json(429, r#"{"error":{"type":"rate_limit_error"}}"#)
                .header("retry-after", "120"),
        )
    })
    .await;
    let cfg = AnthropicConfig {
        base_url: srv.base.clone(),
        api_key: Some("k".into()),
        ..Default::default()
    };
    let b = Anthropic::new(cfg, fast_policy());
    match b.chat(req()).await {
        Err(HarnessError::RateLimited {
            retry_after: Some(d),
        }) => assert_eq!(d.as_secs(), 120),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("expected an error"),
    }
    assert_eq!(srv.recorded().len(), 1);
}

#[tokio::test]
async fn rate_limit_short_retry_after_is_retried() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = calls.clone();
    let srv = MockServer::start(move |_| {
        if c.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            Some(Reply::json(429, "{}").header("retry-after", "0"))
        } else {
            Some(Reply::sse(OPENAI_SSE, 50))
        }
    })
    .await;
    let mut cfg = OpenAiCompatibleConfig::ollama("m");
    cfg.base_url = format!("{}/v1", srv.base);
    let b = OpenAiCompatible::new(cfg, fast_policy());
    let (text, _) = collect(b.chat(req()).await.unwrap()).await;
    assert_eq!(text, "hello, wörld");
    assert_eq!(srv.recorded().len(), 2);
}

#[tokio::test]
async fn server_errors_back_off_then_give_up() {
    let srv = MockServer::start(|_| Some(Reply::json(503, "{}"))).await;
    let mut cfg = OpenAiCompatibleConfig::ollama("m");
    cfg.base_url = format!("{}/v1", srv.base);
    let b = OpenAiCompatible::new(cfg, fast_policy());
    assert!(matches!(
        b.chat(req()).await,
        Err(HarnessError::Unavailable(_))
    ));
    assert_eq!(srv.recorded().len(), 3); // 1 + max_retries
}

#[tokio::test]
async fn detect_all_builds_and_checks_backends() {
    let srv = MockServer::start(|r| match r.path.as_str() {
        "/v1/models" => Some(Reply::json(200, r#"{"data":[{"id":"llama3"}]}"#)),
        "/api/tags" => Some(Reply::json(
            200,
            r#"{"models":[{"name":"llama3:8b"},{"name":"nomic-embed-text"}]}"#,
        )),
        _ => None,
    })
    .await;
    let mut ollama = OpenAiCompatibleConfig::ollama("llama3");
    ollama.base_url = format!("{}/v1", srv.base);
    let empty = tempfile::tempdir().unwrap();
    let cfg = crate::detect::HarnessConfig {
        openai_compatible: vec![ollama],
        anthropic: Some(AnthropicConfig::default()), // no key: configured but unavailable
        claude_cli: crate::adapters::claude_cli::ClaudeCliConfig {
            search_path: Some(empty.path().as_os_str().to_owned()),
            ..Default::default()
        },
        probe_local: false,
        ..Default::default()
    };
    let d = crate::detect::detect_all(&cfg).await;
    assert_eq!(
        d.report.ollama_models,
        vec!["llama3:8b", "nomic-embed-text"]
    );
    assert!(!d.report.claude_cli.found);
    let ids: Vec<_> = d
        .report
        .backends
        .iter()
        .map(|b| (b.id.as_str(), b.available))
        .collect();
    assert_eq!(ids, vec![("ollama", true), ("anthropic", false)]);
    let routing = d.router.effective_routing();
    assert_eq!(routing[&Task::Label].as_deref(), Some("ollama"));
    assert_eq!(routing[&Task::Chat].as_deref(), Some("ollama"));
    // the ui shape serialises
    let json = serde_json::to_value(&routing).unwrap();
    assert_eq!(json["label"], "ollama");
}
