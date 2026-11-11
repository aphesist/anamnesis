//! Incremental server-sent-events parser (https://html.spec.whatwg.org/multipage/server-sent-events.html).
//!
//! Bytes arrive in arbitrary chunks; events are emitted once their terminating blank line is seen.

use bytes::Bytes;
use futures_util::{Stream, StreamExt};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// The `event:` field, if any.
    pub event: Option<String>,
    /// All `data:` lines joined with `\n`.
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    has_data: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk; returns all events completed by it.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            // find a line terminator: \n, \r\n or a lone \r (only if not at buffer end,
            // since a \n may follow in the next chunk).
            let mut idx = None;
            for (i, b) in self.buf.iter().enumerate() {
                if *b == b'\n' {
                    idx = Some((i, 1));
                    break;
                }
                if *b == b'\r' {
                    if i + 1 < self.buf.len() {
                        idx = Some((i, if self.buf[i + 1] == b'\n' { 2 } else { 1 }));
                        break;
                    } else {
                        break; // wait for more
                    }
                }
            }
            let Some((i, term)) = idx else { break };
            let line: Vec<u8> = self.buf.drain(..i + term).take(i).collect();
            let line = String::from_utf8_lossy(&line).into_owned();
            if let Some(ev) = self.line(&line) {
                out.push(ev);
            }
        }
        out
    }

    /// Flush at end of stream: a final event without a trailing blank line is still delivered.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let rest = std::mem::take(&mut self.buf);
            let line = String::from_utf8_lossy(&rest).into_owned();
            let line = line.trim_end_matches(['\r', '\n']).to_string();
            if let Some(ev) = self.line(&line) {
                return Some(ev);
            }
        }
        self.dispatch()
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None; // comment / keep-alive
        }
        let (field, value) = match line.find(':') {
            Some(p) => {
                let v = &line[p + 1..];
                (&line[..p], v.strip_prefix(' ').unwrap_or(v))
            }
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data.push(value.to_string());
                self.has_data = true;
            }
            _ => {} // id, retry: not needed here
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        if !self.has_data {
            self.event = None;
            return None;
        }
        let ev = SseEvent {
            event: self.event.take(),
            data: self.data.join("\n"),
        };
        self.data.clear();
        self.has_data = false;
        Some(ev)
    }
}

/// Turn a byte stream (e.g. `reqwest::Response::bytes_stream`) into a stream of SSE events.
/// A transport error ends the stream with `Err(message)`.
pub fn sse_events<S, E>(bytes: S) -> impl Stream<Item = Result<SseEvent, String>> + Send
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: std::fmt::Display + 'static,
{
    let state = (
        Box::pin(bytes),
        SseParser::new(),
        std::collections::VecDeque::new(),
        false,
    );
    futures_util::stream::unfold(state, |(mut s, mut p, mut q, mut done)| async move {
        loop {
            if let Some(ev) = q.pop_front() {
                return Some((Ok(ev), (s, p, q, done)));
            }
            if done {
                return None;
            }
            match s.next().await {
                Some(Ok(chunk)) => q.extend(p.push(&chunk)),
                Some(Err(e)) => {
                    done = true;
                    return Some((Err(e.to_string()), (s, p, q, done)));
                }
                None => {
                    done = true;
                    if let Some(ev) = p.finish() {
                        q.push_back(ev);
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_across_chunks_and_crlf() {
        let mut p = SseParser::new();
        assert!(p.push(b"event: a\r\nda").is_empty());
        assert!(p.push(b"ta: {\"x\":1}\r").is_empty());
        let evs = p.push(b"\n\r\n: keepalive\n\ndata: b\ndata: c\n\n");
        assert_eq!(
            evs,
            vec![
                SseEvent {
                    event: Some("a".into()),
                    data: "{\"x\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "b\nc".into()
                },
            ]
        );
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut p = SseParser::new();
        assert!(p.push(b"data: [DONE]").is_empty());
        assert_eq!(
            p.finish(),
            Some(SseEvent {
                event: None,
                data: "[DONE]".into()
            })
        );
    }

    #[tokio::test]
    async fn byte_stream_adapter() {
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(b"data: 1\n")),
            Ok(Bytes::from_static(b"\ndata: 2\n\n")),
        ];
        let evs: Vec<_> = sse_events(futures_util::stream::iter(chunks))
            .collect()
            .await;
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[1].as_ref().unwrap().data, "2");
    }
}
