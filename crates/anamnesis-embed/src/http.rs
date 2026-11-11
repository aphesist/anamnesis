//! Shared HTTP plumbing for the remote backends: a blocking client, retries
//! with exponential backoff, and loopback detection for the egress gate.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Proxy};

/// Retry policy for transient failures (connection errors, timeouts, HTTP
/// 408/429/5xx). Other 4xx responses fail immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// Delay before the first retry; doubles each time.
    pub base_delay_ms: u64,
    /// Upper bound for a single delay.
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 500,
            max_delay_ms: 8_000,
        }
    }
}

impl RetryPolicy {
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }

    fn delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        let exp = self.base_delay_ms.saturating_mul(1u64 << attempt.min(16));
        let d = Duration::from_millis(exp.min(self.max_delay_ms));
        match retry_after {
            Some(ra) => ra
                .min(Duration::from_millis(self.max_delay_ms.max(1)))
                .max(d),
            None => d,
        }
    }
}

/// Options common to the HTTP backends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpOptions {
    /// Texts per request.
    pub batch_size: usize,
    /// Approximate token budget per input; longer inputs are truncated
    /// client-side before sending. 0 disables truncation.
    pub max_input_tokens: usize,
    /// Whole-request timeout.
    pub timeout_secs: u64,
    pub retry: RetryPolicy,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            batch_size: 32,
            max_input_tokens: 8_000,
            timeout_secs: 120,
            retry: RetryPolicy::default(),
        }
    }
}

/// True when `url`'s host is a loopback address (`localhost`, `127.0.0.0/8`,
/// `::1`). Text sent to such a server does not leave this machine.
pub fn is_loopback_url(url: &str) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split_once(']').map(|(h, _)| h);
    }
    Some(authority.split(':').next().unwrap_or(authority)).filter(|h| !h.is_empty())
}

pub(crate) struct HttpClient {
    agent: Agent,
    retry: RetryPolicy,
}

#[derive(Debug)]
enum Attempt {
    Retryable(anyhow::Error, Option<Duration>),
    Fatal(anyhow::Error),
}

impl HttpClient {
    pub(crate) fn new(base_url: &str, opts: &HttpOptions) -> Self {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        // Loopback servers are always reached directly, whatever proxy the
        // environment configures; everything else honours HTTPS_PROXY etc.
        let proxy = if is_loopback_url(base_url) {
            None
        } else {
            Proxy::try_from_env()
        };
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(opts.timeout_secs.max(1))))
            .http_status_as_error(false)
            .tls_config(tls)
            .proxy(proxy)
            .user_agent("anamnesis-embed")
            .build()
            .into();
        Self {
            agent,
            retry: opts.retry,
        }
    }

    /// POST a JSON body and decode a JSON response, retrying transient
    /// failures. `bearer` is sent as `Authorization: Bearer ...` and never
    /// appears in errors or logs.
    pub(crate) fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
        bearer: Option<&str>,
    ) -> Result<R> {
        let mut attempt = 0u32;
        loop {
            match self.try_once(url, body, bearer) {
                Ok(text) => {
                    return serde_json::from_str(&text)
                        .with_context(|| format!("unexpected response from {}", redact(url)));
                }
                Err(Attempt::Fatal(e)) => return Err(e),
                Err(Attempt::Retryable(e, retry_after)) => {
                    if attempt >= self.retry.max_retries {
                        return Err(e.context(format!("gave up after {} attempts", attempt + 1)));
                    }
                    let d = self.retry.delay(attempt, retry_after);
                    tracing::debug!(url = %redact(url), attempt, delay_ms = d.as_millis() as u64, error = %e, "embedding request failed, retrying");
                    std::thread::sleep(d);
                    attempt += 1;
                }
            }
        }
    }

    fn try_once<B: Serialize>(
        &self,
        url: &str,
        body: &B,
        bearer: Option<&str>,
    ) -> Result<String, Attempt> {
        let mut req = self.agent.post(url).header("Accept", "application/json");
        if let Some(key) = bearer.filter(|k| !k.is_empty()) {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        let mut resp = match req.send_json(body) {
            Ok(r) => r,
            Err(e) => {
                let err = anyhow!("request to {} failed: {e}", redact(url));
                return Err(match e {
                    ureq::Error::Io(_)
                    | ureq::Error::Timeout(_)
                    | ureq::Error::ConnectionFailed
                    | ureq::Error::HostNotFound => Attempt::Retryable(err, None),
                    _ => Attempt::Fatal(err),
                });
            }
        };
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let text = resp
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| {
                Attempt::Retryable(anyhow!("reading response from {}: {e}", redact(url)), None)
            })?;
        if (200..300).contains(&status) {
            return Ok(text);
        }
        let err = anyhow!("{} returned http {status}: {}", redact(url), snippet(&text));
        if status == 408 || status == 429 || status >= 500 {
            Err(Attempt::Retryable(err, retry_after))
        } else {
            Err(Attempt::Fatal(err))
        }
    }
}

/// Strip userinfo and query from a URL before it goes into an error or log.
fn redact(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let rest = match rest.split_once('/') {
        Some((auth, path)) => format!("{}/{path}", auth.rsplit_once('@').map_or(auth, |(_, h)| h)),
        None => rest.rsplit_once('@').map_or(rest, |(_, h)| h).to_owned(),
    };
    if scheme.is_empty() {
        rest
    } else {
        format!("{scheme}://{rest}")
    }
}

/// The start of an error body, on one line, for error messages.
fn snippet(text: &str) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s: String = one_line.chars().take(300).collect();
    if one_line.chars().count() > 300 {
        s.push('…');
    }
    s
}

/// Join a base URL and a path without doubling or dropping slashes.
pub(crate) fn join(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Check a batch response: right count, consistent dimension; normalise.
pub(crate) fn finish(
    mut vecs: Vec<Vec<f32>>,
    expected: usize,
    dim: Option<usize>,
) -> Result<Vec<Vec<f32>>> {
    if vecs.len() != expected {
        bail!(
            "server returned {} embeddings for {expected} inputs",
            vecs.len()
        );
    }
    let d = dim.or_else(|| vecs.first().map(Vec::len)).unwrap_or(0);
    for v in &mut vecs {
        if v.len() != d || d == 0 {
            bail!(
                "server returned embeddings of inconsistent dimension ({} vs {d})",
                v.len()
            );
        }
        crate::l2_normalize(v);
    }
    Ok(vecs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_detection() {
        for u in [
            "http://localhost:11434",
            "http://127.0.0.1:8080/v1",
            "http://127.1.2.3",
            "http://[::1]:1234/v1",
            "localhost:11434",
            "http://user:pw@localhost/x",
            "http://foo.localhost",
        ] {
            assert!(is_loopback_url(u), "{u}");
        }
        for u in [
            "https://api.openai.com/v1",
            "http://192.168.1.10:11434",
            "https://openrouter.ai/api/v1",
            "http://localhost.evil.com",
            "",
        ] {
            assert!(!is_loopback_url(u), "{u}");
        }
    }

    #[test]
    fn redaction() {
        assert_eq!(
            redact("https://user:secret@host/v1/x?key=abc"),
            "https://host/v1/x"
        );
        assert_eq!(redact("http://host:1/a"), "http://host:1/a");
    }

    #[test]
    fn backoff_grows_and_caps() {
        let p = RetryPolicy {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 1000,
        };
        assert_eq!(p.delay(0, None), Duration::from_millis(100));
        assert_eq!(p.delay(2, None), Duration::from_millis(400));
        assert_eq!(p.delay(10, None), Duration::from_millis(1000));
        assert_eq!(
            p.delay(0, Some(Duration::from_secs(60))),
            Duration::from_millis(1000)
        );
    }

    #[test]
    fn join_paths() {
        assert_eq!(
            join("http://h/v1/", "/embeddings"),
            "http://h/v1/embeddings"
        );
        assert_eq!(join("http://h", "api/embed"), "http://h/api/embed");
    }
}
