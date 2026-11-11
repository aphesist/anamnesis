//! Canonical url normalisation, used as the external id of web clips and for deduplicating
//! references.
//!
//! Rules (conservative: only changes that never change what the server returns in practice):
//! - scheme and host lowercased, default ports dropped, IDNs punycoded (by `url`);
//! - fragment removed;
//! - tracking parameters removed: `utm_*`, `fbclid`, `gclid`, `dclid`, `gbraid`, `wbraid`,
//!   `msclkid`, `yclid`, `mc_cid`, `mc_eid`, `igshid`, `_hsenc`, `_hsmi`, `mkt_tok`, `oly_*`,
//!   `vero_*`, `spm`, `ref_src`; other parameters keep their order;
//! - an empty query (`?`) is removed;
//! - a trailing slash is removed from non-root paths (`/a/` → `/a`); the root keeps `/`;
//! - `www.` is kept (it can be a different site), and the scheme is not upgraded.

use url::Url;

use crate::error::{ConnectorError, Result};

fn is_tracking(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.starts_with("utm_")
        || k.starts_with("oly_")
        || k.starts_with("vero_")
        || matches!(
            k.as_str(),
            "fbclid"
                | "gclid"
                | "dclid"
                | "gbraid"
                | "wbraid"
                | "msclkid"
                | "yclid"
                | "mc_cid"
                | "mc_eid"
                | "igshid"
                | "_hsenc"
                | "_hsmi"
                | "mkt_tok"
                | "spm"
                | "ref_src"
        )
}

pub fn normalize_url(input: &str) -> Result<String> {
    let mut u = Url::parse(input.trim())
        .map_err(|e| ConnectorError::Parse(format!("url {input:?}: {e}")))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Ok(u.to_string());
    }
    u.set_fragment(None);
    if u.query().is_some() {
        let kept: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !is_tracking(k))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if kept.is_empty() {
            u.set_query(None);
        } else {
            // Rebuild only when something was removed, to keep the original encoding otherwise.
            let before = u.query_pairs().count();
            if kept.len() != before {
                u.query_pairs_mut().clear().extend_pairs(kept);
            }
        }
    }
    let path = u.path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        let trimmed = path.trim_end_matches('/');
        u.set_path(if trimmed.is_empty() { "/" } else { trimmed });
    }
    Ok(u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_table() {
        let table = [
            (
                "https://Example.COM/a/b/?utm_source=x&utm_medium=y#frag",
                "https://example.com/a/b",
            ),
            ("https://example.com", "https://example.com/"),
            ("https://example.com/", "https://example.com/"),
            ("https://example.com/?", "https://example.com/"),
            ("http://example.com:80/x", "http://example.com/x"),
            ("https://example.com:443/x/", "https://example.com/x"),
            ("https://example.com:8443/x", "https://example.com:8443/x"),
            (
                "https://example.com/p?id=3&utm_campaign=z&b=2",
                "https://example.com/p?id=3&b=2",
            ),
            ("https://example.com/p?fbclid=abc", "https://example.com/p"),
            (
                "https://example.com/p?GCLID=abc&q=a+b",
                "https://example.com/p?q=a+b",
            ),
            (
                "https://example.com/p?q=a%20b",
                "https://example.com/p?q=a%20b",
            ),
            ("https://www.example.com/a//", "https://www.example.com/a"),
            ("  https://example.com/a#top  ", "https://example.com/a"),
            ("https://bücher.de/x", "https://xn--bcher-kva.de/x"),
            ("mailto:someone@example.com", "mailto:someone@example.com"),
        ];
        for (input, want) in table {
            assert_eq!(normalize_url(input).unwrap(), want, "input {input}");
        }
        assert!(normalize_url("not a url").is_err());
    }
}
