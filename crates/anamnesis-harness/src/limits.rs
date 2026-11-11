//! Rate-limit awareness: retry-after parsing, exponential backoff, and per-backend cooldowns
//! the ui can show ("exhausted until …").

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::FailureKind;

/// Exponential backoff: `base * 2^(n-1)`, capped.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BackoffPolicy {
    pub base_ms: u64,
    pub max_ms: u64,
    /// Retries the http layer performs before giving up on a single request.
    pub max_retries: u32,
    /// A server-requested wait longer than this is not slept through inline;
    /// the backend goes into cooldown instead.
    pub max_inline_wait_ms: u64,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base_ms: 1_000,
            max_ms: 15 * 60_000,
            max_retries: 2,
            max_inline_wait_ms: 20_000,
        }
    }
}

impl BackoffPolicy {
    /// Delay for the n-th consecutive failure (n ≥ 1).
    pub fn delay(&self, n: u32) -> Duration {
        let exp = n.saturating_sub(1).min(30);
        let ms = self.base_ms.saturating_mul(1u64 << exp).min(self.max_ms);
        Duration::from_millis(ms)
    }
}

/// Parse a `retry-after` header value: delta-seconds or an http-date.
pub fn parse_retry_after(value: &str, now: DateTime<Utc>) -> Option<Duration> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    if let Ok(secs) = v.parse::<f64>()
        && secs >= 0.0
    {
        return Some(Duration::from_millis((secs * 1000.0) as u64));
    }
    let at = DateTime::parse_from_rfc2822(v).ok()?.with_timezone(&Utc);
    Some((at - now).to_std().unwrap_or(Duration::ZERO))
}

/// Read retry hints from response headers: `retry-after`, then `retry-after-ms`,
/// then anthropic's `anthropic-ratelimit-*-reset` (rfc3339).
pub fn retry_after_from_headers(
    headers: &reqwest::header::HeaderMap,
    now: DateTime<Utc>,
) -> Option<Duration> {
    let get = |k: &str| headers.get(k).and_then(|v| v.to_str().ok());
    if let Some(ms) = get("retry-after-ms").and_then(|v| v.trim().parse::<u64>().ok()) {
        return Some(Duration::from_millis(ms));
    }
    if let Some(d) = get("retry-after").and_then(|v| parse_retry_after(v, now)) {
        return Some(d);
    }
    for k in [
        "anthropic-ratelimit-requests-reset",
        "anthropic-ratelimit-tokens-reset",
        "anthropic-ratelimit-input-tokens-reset",
        "anthropic-ratelimit-output-tokens-reset",
    ] {
        if let Some(at) = get(k).and_then(|v| DateTime::parse_from_rfc3339(v).ok()) {
            return Some(
                (at.with_timezone(&Utc) - now)
                    .to_std()
                    .unwrap_or(Duration::ZERO),
            );
        }
    }
    None
}

/// Per-backend state exposed to the ui.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CooldownState {
    pub backend: String,
    /// While set and in the future, the router does not use this backend.
    pub until: Option<DateTime<Utc>>,
    /// Lowercase reason, e.g. "rate limited", "plan usage limit reached".
    pub reason: Option<String>,
    pub consecutive_failures: u32,
    /// True when the reset time was stated by the provider (not our backoff guess).
    pub reset_known: bool,
}

#[derive(Debug, Default)]
pub struct Limits {
    policy: BackoffPolicy,
    state: Mutex<HashMap<String, CooldownState>>,
}

impl Limits {
    pub fn new(policy: BackoffPolicy) -> Self {
        Self {
            policy,
            state: Mutex::new(HashMap::new()),
        }
    }

    pub fn policy(&self) -> BackoffPolicy {
        self.policy
    }

    /// If the backend is cooling down at `now`, when it ends.
    pub fn cooling_until(&self, backend: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let st = self.state.lock().unwrap();
        st.get(backend).and_then(|s| s.until).filter(|u| *u > now)
    }

    pub fn record_success(&self, backend: &str) {
        let mut st = self.state.lock().unwrap();
        if let Some(s) = st.get_mut(backend) {
            s.consecutive_failures = 0;
            s.until = None;
            s.reason = None;
            s.reset_known = false;
        }
    }

    /// Record a failure. Rate-limit and usage-limit failures start a cooldown:
    /// the provider's stated time if given, else exponential backoff. Other failures
    /// only count (and transient ones back off).
    pub fn record_failure(
        &self,
        backend: &str,
        failure: &FailureKind,
        now: DateTime<Utc>,
    ) -> CooldownState {
        let mut st = self.state.lock().unwrap();
        let s = st
            .entry(backend.to_string())
            .or_insert_with(|| CooldownState {
                backend: backend.to_string(),
                until: None,
                reason: None,
                consecutive_failures: 0,
                reset_known: false,
            });
        s.consecutive_failures += 1;
        let backoff = chrono::Duration::from_std(self.policy.delay(s.consecutive_failures))
            .unwrap_or_default();
        match failure {
            FailureKind::RateLimited { retry_after_secs } => {
                s.reason = Some("rate limited".into());
                s.reset_known = retry_after_secs.is_some();
                s.until = Some(match retry_after_secs {
                    Some(secs) => now + chrono::Duration::seconds(*secs as i64),
                    None => now + backoff,
                });
            }
            FailureKind::UsageLimit { resets_at } => {
                s.reason = Some("plan usage limit reached".into());
                s.reset_known = resets_at.is_some();
                s.until =
                    Some(resets_at.unwrap_or(now + backoff.max(chrono::Duration::minutes(5))));
            }
            FailureKind::Unavailable => {
                s.reason = Some("temporarily unavailable".into());
                s.reset_known = false;
                s.until = Some(now + backoff);
            }
            _ => {}
        }
        s.clone()
    }

    /// Clear a cooldown by hand (the ui's "try again" button).
    pub fn clear(&self, backend: &str) {
        self.state.lock().unwrap().remove(backend);
    }

    pub fn snapshot(&self) -> Vec<CooldownState> {
        let mut v: Vec<_> = self.state.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.backend.cmp(&b.backend));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let p = BackoffPolicy {
            base_ms: 100,
            max_ms: 1000,
            max_retries: 3,
            max_inline_wait_ms: 0,
        };
        assert_eq!(p.delay(1), Duration::from_millis(100));
        assert_eq!(p.delay(2), Duration::from_millis(200));
        assert_eq!(p.delay(4), Duration::from_millis(800));
        assert_eq!(p.delay(5), Duration::from_millis(1000));
        assert_eq!(p.delay(64), Duration::from_millis(1000));
    }

    #[test]
    fn retry_after_seconds_and_date() {
        let now = DateTime::parse_from_rfc3339("2026-11-11T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(parse_retry_after("30", now), Some(Duration::from_secs(30)));
        assert_eq!(
            parse_retry_after("Wed, 11 Nov 2026 12:01:00 GMT", now),
            Some(Duration::from_secs(60))
        );
        assert_eq!(parse_retry_after("soon", now), None);
    }

    #[test]
    fn cooldown_from_rate_limit_then_success_clears() {
        let l = Limits::new(BackoffPolicy::default());
        let now = Utc::now();
        l.record_failure(
            "x",
            &FailureKind::RateLimited {
                retry_after_secs: Some(10),
            },
            now,
        );
        let until = l.cooling_until("x", now).unwrap();
        assert_eq!((until - now).num_seconds(), 10);
        assert!(
            l.cooling_until("x", now + chrono::Duration::seconds(11))
                .is_none()
        );
        l.record_success("x");
        assert!(l.cooling_until("x", now).is_none());
        assert_eq!(l.snapshot()[0].consecutive_failures, 0);
    }

    #[test]
    fn backoff_grows_without_retry_after() {
        let l = Limits::new(BackoffPolicy {
            base_ms: 1000,
            max_ms: 60_000,
            max_retries: 0,
            max_inline_wait_ms: 0,
        });
        let now = Utc::now();
        let a = l.record_failure(
            "x",
            &FailureKind::RateLimited {
                retry_after_secs: None,
            },
            now,
        );
        let b = l.record_failure(
            "x",
            &FailureKind::RateLimited {
                retry_after_secs: None,
            },
            now,
        );
        assert_eq!((a.until.unwrap() - now).num_seconds(), 1);
        assert_eq!((b.until.unwrap() - now).num_seconds(), 2);
        assert!(!b.reset_known);
    }
}
