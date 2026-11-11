//! Shared http helpers: status → error mapping and bounded retry with backoff.

use std::time::Duration;

use chrono::Utc;
use reqwest::{RequestBuilder, Response, StatusCode};

use crate::error::{HarnessError, Result};
use crate::limits::{BackoffPolicy, retry_after_from_headers};

const BODY_LIMIT: usize = 600;

pub(crate) fn client(timeout: Option<Duration>) -> reqwest::Client {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("anamnesis/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10));
    if let Some(t) = timeout {
        b = b.timeout(t);
    }
    b.build().expect("reqwest client")
}

/// Best-effort error code from a json error body (`{"error":{"code"|"type": …}}`).
pub(crate) fn error_code(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let e = v.get("error")?;
    e.get("code")
        .and_then(|c| c.as_str())
        .or_else(|| e.get("type").and_then(|c| c.as_str()))
        .or_else(|| e.get("status").and_then(|c| c.as_str()))
        .map(str::to_string)
}

/// Map a non-success response to an error, consuming the body.
pub(crate) async fn error_from_response(resp: Response) -> HarnessError {
    let status = resp.status();
    let retry_after = retry_after_from_headers(resp.headers(), Utc::now());
    let mut body = resp.text().await.unwrap_or_default();
    if body.len() > BODY_LIMIT {
        let mut cut = BODY_LIMIT;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body.truncate(cut);
    }
    classify(status, retry_after, body)
}

pub(crate) fn classify(status: StatusCode, retry_after: Option<Duration>, body: String) -> HarnessError {
    let code = error_code(&body).unwrap_or_default();
    // chatgpt plan flow (https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery)
    if code == "subscription_sharing_usage_limit_exceeded" {
        return HarnessError::UsageLimit(code);
    }
    if code == "subscription_sharing_user_not_eligible" {
        return HarnessError::Auth(code);
    }
    match status.as_u16() {
        401 | 403 => HarnessError::Auth(if code.is_empty() { status.to_string() } else { code }),
        429 => HarnessError::RateLimited { retry_after },
        400 | 404 | 413 | 422 => HarnessError::InvalidRequest(body),
        500..=599 => HarnessError::Unavailable(format!("http {}", status.as_u16())),
        s => HarnessError::Http { status: s, body },
    }
}

/// Send a request, retrying 429 and 5xx a bounded number of times. Server-requested
/// waits longer than `max_inline_wait_ms` are not slept through: the error is returned
/// so the caller can put the backend into cooldown.
pub(crate) async fn send_with_retry<F>(policy: &BackoffPolicy, mut make: F) -> Result<Response>
where
    F: FnMut() -> RequestBuilder,
{
    let mut attempt = 0u32;
    loop {
        let resp = match make().send().await {
            Ok(r) => r,
            Err(e) if e.is_connect() => return Err(HarnessError::Unavailable(e.to_string())),
            Err(e) => return Err(e.into()),
        };
        if resp.status().is_success() {
            return Ok(resp);
        }
        let err = error_from_response(resp).await;
        attempt += 1;
        let wait = match &err {
            HarnessError::RateLimited { retry_after } => retry_after.unwrap_or_else(|| policy.delay(attempt)),
            HarnessError::Unavailable(_) => policy.delay(attempt),
            _ => return Err(err),
        };
        if attempt > policy.max_retries || wait.as_millis() as u64 > policy.max_inline_wait_ms {
            return Err(err);
        }
        tracing::debug!(attempt, wait_ms = wait.as_millis() as u64, "retrying after {err}");
        tokio::time::sleep(wait).await;
    }
}

/// Join a base url and a path without doubling slashes.
pub(crate) fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// True for loopback / private-lan hosts (used to decide whether a call is "local").
pub(crate) fn is_local_url(u: &str) -> bool {
    let Ok(url) = url::Url::parse(u) else { return false };
    match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost" || d.ends_with(".localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_statuses() {
        assert!(matches!(
            classify(StatusCode::TOO_MANY_REQUESTS, Some(Duration::from_secs(3)), "{}".into()),
            HarnessError::RateLimited { retry_after: Some(_) }
        ));
        assert!(matches!(classify(StatusCode::UNAUTHORIZED, None, "".into()), HarnessError::Auth(_)));
        let body = r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded"}}"#.to_string();
        assert!(matches!(classify(StatusCode::TOO_MANY_REQUESTS, None, body), HarnessError::UsageLimit(_)));
        assert!(matches!(classify(StatusCode::from_u16(529).unwrap(), None, "".into()), HarnessError::Unavailable(_)));
    }

    #[test]
    fn local_urls() {
        assert!(is_local_url("http://127.0.0.1:11434"));
        assert!(is_local_url("http://localhost:8080/v1"));
        assert!(is_local_url("http://[::1]:1234"));
        assert!(!is_local_url("https://openrouter.ai/api/v1"));
    }
}
