//! A tiny in-process http/1.1 mock server for adapter tests (no network).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Sent as separate writes with a short pause, to exercise chunked parsing.
    pub chunks: Vec<Vec<u8>>,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            chunks: vec![body.as_bytes().to_vec()],
        }
    }
    pub fn sse(body: &str, chunk: usize) -> Self {
        let chunks = body
            .as_bytes()
            .chunks(chunk.max(1))
            .map(|c| c.to_vec())
            .collect();
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            chunks,
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
}

type Handler = Arc<dyn Fn(&Recorded) -> Option<Reply> + Send + Sync>;

pub struct MockServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockServer {
    /// `handler` returns None for 404.
    pub async fn start(
        handler: impl Fn(&Recorded) -> Option<Reply> + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let reqs = requests.clone();
        let handler: Handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let reqs = reqs.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let Some(rec) = read_request(&mut sock).await else {
                        return;
                    };
                    reqs.lock().unwrap().push(rec.clone());
                    let reply = handler(&rec).unwrap_or_else(|| Reply::json(404, "{}"));
                    let total: usize = reply.chunks.iter().map(|c| c.len()).sum();
                    let mut head = format!(
                        "HTTP/1.1 {} X\r\ncontent-length: {total}\r\nconnection: close\r\n",
                        reply.status
                    );
                    for (k, v) in &reply.headers {
                        head.push_str(&format!("{k}: {v}\r\n"));
                    }
                    head.push_str("\r\n");
                    let _ = sock.write_all(head.as_bytes()).await;
                    for c in &reply.chunks {
                        let _ = sock.write_all(c).await;
                        let _ = sock.flush().await;
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { base, requests }
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<Recorded> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..(head_end + len).min(buf.len())]).to_string();
    Some(Recorded {
        method,
        path,
        headers,
        body,
    })
}

/// Collect a chat stream into (text, events).
pub async fn collect(stream: crate::types::ChatStream) -> (String, Vec<crate::types::ChatEvent>) {
    use futures_util::StreamExt;
    let evs: Vec<_> = stream.collect().await;
    let text = evs
        .iter()
        .filter_map(|e| match e {
            crate::types::ChatEvent::Token { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    (text, evs)
}
