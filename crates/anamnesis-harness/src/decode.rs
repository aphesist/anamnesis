//! Turning a provider's SSE byte stream into `ChatEvent`s.
//!
//! Each api format has a small `Decoder` state machine; `decode_sse` drives it and
//! guarantees the stream ends exactly once, with `Usage` before `Done`.

use bytes::Bytes;
use futures_util::{Stream, StreamExt};

use crate::sse::{SseEvent, sse_events};
use crate::types::{ChatEvent, ChatStream, FailureKind, Usage};

pub(crate) trait Decoder: Send + 'static {
    /// Map one SSE event. Returning a `Done` or `Error` ends the stream.
    fn event(&mut self, ev: &SseEvent) -> Vec<ChatEvent>;
    /// Called at end of input if no `Done`/`Error` was produced yet.
    fn finish(&mut self) -> Vec<ChatEvent>;
}

/// Tracks usage across a stream, falling back to an estimate.
#[derive(Debug, Default)]
pub(crate) struct UsageAcc {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub est_input: u64,
    pub out_chars: u64,
    pub force_estimated: bool,
}

impl UsageAcc {
    pub fn new(est_input: u64) -> Self {
        Self {
            est_input,
            ..Default::default()
        }
    }
    pub fn text(&mut self, t: &str) {
        self.out_chars += t.chars().count() as u64;
    }
    pub fn usage(&self) -> Usage {
        let estimated = self.force_estimated || self.input.is_none() || self.output.is_none();
        Usage {
            input: self.input.unwrap_or(self.est_input),
            output: self.output.unwrap_or(self.out_chars.div_ceil(4)),
            estimated,
            reported_cost_usd: None,
        }
    }
}

pub(crate) fn is_terminal(ev: &ChatEvent) -> bool {
    matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. })
}

pub(crate) fn decode_sse<S, E, D>(bytes: S, decoder: D) -> ChatStream
where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: std::fmt::Display + 'static,
    D: Decoder,
{
    let events = Box::pin(sse_events(bytes));
    let state = (
        events,
        decoder,
        std::collections::VecDeque::<ChatEvent>::new(),
        false,
    );
    let s =
        futures_util::stream::unfold(state, |(mut evs, mut dec, mut q, mut ended)| async move {
            loop {
                if let Some(ev) = q.pop_front() {
                    if is_terminal(&ev) {
                        ended = true;
                        q.clear();
                    }
                    return Some((ev, (evs, dec, q, ended)));
                }
                if ended {
                    return None;
                }
                match evs.next().await {
                    Some(Ok(ev)) => q.extend(dec.event(&ev)),
                    Some(Err(e)) => q.push_back(ChatEvent::Error {
                        message: format!("stream interrupted: {e}"),
                        failure: FailureKind::Unavailable,
                    }),
                    None => {
                        q.extend(dec.finish());
                        ended = true;
                    }
                }
            }
        });
    Box::pin(s)
}
