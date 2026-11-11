//! Shared HTTP client for the networked connectors (rustls, no native tls).

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use url::Url;

use crate::error::{ConnectorError, Result};

pub const USER_AGENT: &str = concat!(
    "anamnesis/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/aphesimos/anamnesis)"
);

/// Responses larger than this are cut off with an error.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

pub fn client_for(url: &str, timeout: Duration) -> Result<reqwest::Client> {
    let u = Url::parse(url)
        .map_err(|e| ConnectorError::InvalidConfig(vec![format!("url {url:?}: {e}")]))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(ConnectorError::InvalidConfig(vec![format!(
            "url {url:?} must be http(s)"
        )]));
    }
    let mut b = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(10));
    // Loopback (personal dev servers, tests) never goes through a proxy.
    let loopback = match u.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if loopback {
        b = b.no_proxy();
    }
    b.build().map_err(ConnectorError::from)
}

#[derive(Debug, Clone)]
pub struct Fetched {
    pub status: u16,
    /// After redirects.
    pub final_url: String,
    pub headers: HeaderMap,
    pub bytes: Vec<u8>,
}

impl Fetched {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    }
}

/// GET with extra headers, a size cap, and the body read fully.
pub async fn get(url: &str, headers: &[(String, String)], timeout: Duration) -> Result<Fetched> {
    let client = client_for(url, timeout)?;
    let mut req = client.get(url);
    for (k, v) in headers {
        let hv = HeaderValue::from_str(v)
            .map_err(|e| ConnectorError::Http(format!("header {k}: {e}")))?;
        req = req.header(k.as_str(), hv);
    }
    let mut resp = req.send().await?;
    let status = resp.status().as_u16();
    let final_url = resp.url().to_string();
    let headers = resp.headers().clone();
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if bytes.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(ConnectorError::Http(format!(
                "{url}: response larger than {MAX_BODY_BYTES} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Fetched {
        status,
        final_url,
        headers,
        bytes,
    })
}

/// Error unless 2xx (or 304 when `allow_not_modified`).
pub fn ensure_ok(f: &Fetched, allow_not_modified: bool) -> Result<()> {
    if (200..300).contains(&f.status) || (allow_not_modified && f.status == 304) {
        Ok(())
    } else {
        Err(ConnectorError::Http(format!(
            "{} answered {}",
            f.final_url, f.status
        )))
    }
}
