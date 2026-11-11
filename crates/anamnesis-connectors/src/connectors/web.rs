//! `web`: clip a single url.
//!
//! Fetches the page, extracts the main content with the heuristic in [`crate::html`], renders
//! it as markdown-ish text and records page metadata (author, site, published date,
//! description, canonical url) in meta. Origin `external`. Kind `article` when the page says it
//! is one (`og:type=article` or an `<article>` element), else `clip`; config can force either.
//!
//! The external id is the normalised canonical url (see [`crate::url_norm`]): the page's own
//! `<link rel=canonical>` when it is on the same host as the fetched url, else the fetched url.
//! It is fixed at the first clip and kept in the cursor.
//!
//! A clip is a snapshot: later `list` calls return nothing unless `refetch` is true, in which
//! case the page is fetched again and returned only if its extracted text changed.

use std::time::Duration;

use anamnesis_core::{Document, ItemKind, Origin, Privacy};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::connector::{Batch, CheckReport, Connector};
use crate::error::{ConnectorError, Result};
use crate::html::{self, html_to_text};
use crate::http;
use crate::manifest::{API_VERSION, Capability, Manifest, Runtime};
use crate::markdown::parse_date;
use crate::scan::sha256_hex;
use crate::url_norm::normalize_url;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WebConfig {
    pub url: String,
    #[serde(default)]
    pub kind: Option<ItemKind>,
    #[serde(default)]
    pub origin: Option<Origin>,
    /// CSS selector for the main content, overriding the heuristic.
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub refetch: bool,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct WebCursor {
    v: u32,
    external_id: String,
    hash: String,
}

pub struct WebConnector;

pub fn web_manifest() -> Manifest {
    Manifest {
        id: "web".into(),
        name: "web clip".into(),
        description: "the main text of a single web page".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION,
        runtime: Runtime::Builtin,
        entry: None,
        config_schema: json!({
            "type": "object",
            "required": ["url"],
            "additionalProperties": false,
            "properties": {
                "url": {"type": "string", "pattern": "^https?://"},
                "kind": {"enum": ["clip", "article", "note", "quote"]},
                "origin": {"enum": ["owner_written", "owner_spoken", "owner_annotation", "owner_in_chat", "ai_generated", "ai_edited", "external"]},
                "selector": {"type": "string"},
                "refetch": {"type": "boolean"},
                "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 600}
            }
        }),
        capabilities: vec![Capability::Check],
        network_hosts: vec!["<config.url host>".into()],
        fs_paths: vec![],
        default_origin: Origin::External,
        default_privacy: Privacy::CloudOk,
        default_kind: Some(ItemKind::Clip),
        poll_interval_secs: None,
    }
}

/// Turn fetched html into a clip document. Pure; `fetched_url` is the url after redirects.
pub fn clip_html(page: &str, fetched_url: &str, cfg: &WebConfig) -> Result<Document> {
    let base = Url::parse(fetched_url).map_err(|e| ConnectorError::Parse(format!("url: {e}")))?;
    let mut x = html::extract(page, &base);
    if let Some(s) = &cfg.selector {
        let selector = scraper::Selector::parse(s).map_err(|e| ConnectorError::InvalidConfig(vec![format!("selector: {e}")]))?;
        let doc = scraper::Html::parse_document(page);
        if let Some(el) = doc.select(&selector).next() {
            x.content_html = el.html();
            x.text = html_to_text(&x.content_html);
        }
    }
    let fetched_norm = normalize_url(fetched_url)?;
    let canonical = x
        .canonical
        .as_deref()
        .and_then(|c| Url::parse(c).ok())
        .filter(|c| c.host_str() == base.host_str())
        .and_then(|c| normalize_url(c.as_str()).ok())
        .unwrap_or_else(|| fetched_norm.clone());
    let kind = cfg.kind.unwrap_or(if x.is_article { ItemKind::Article } else { ItemKind::Clip });
    let title = x.title.clone().unwrap_or_else(|| canonical.clone());
    let meta = json!({
        "canonical_url": canonical,
        "fetched_url": fetched_url,
        "author": x.author,
        "site_name": x.site_name,
        "published": x.published,
        "modified": x.modified,
        "description": x.description,
        "lang": x.lang,
        "content_hash": sha256_hex(x.text.as_bytes()),
        "connector": "web",
    });
    Ok(Document {
        external_id: canonical.clone(),
        uri: Some(canonical),
        kind,
        title,
        body: x.text,
        origin: cfg.origin.unwrap_or(Origin::External),
        created_at: x.published.as_deref().and_then(parse_date),
        modified_at: x.modified.as_deref().and_then(parse_date),
        tags: vec![],
        refs: x.links,
        meta,
    })
}

impl WebConnector {
    fn cfg(config: &Value) -> Result<WebConfig> {
        serde_json::from_value(config.clone()).map_err(|e| ConnectorError::InvalidConfig(vec![e.to_string()]))
    }
}

#[async_trait]
impl Connector for WebConnector {
    fn manifest(&self) -> Manifest {
        web_manifest()
    }

    async fn check(&self, config: &Value) -> Result<CheckReport> {
        let cfg = Self::cfg(config)?;
        let f = match http::get(&cfg.url, &[], Duration::from_secs(cfg.timeout_secs.unwrap_or(30))).await {
            Ok(f) => f,
            Err(e) => return Ok(CheckReport::failed(format!("could not reach the page: {e}"))),
        };
        Ok(match http::ensure_ok(&f, false) {
            Ok(()) => CheckReport::ok(format!("page answered ({} bytes)", f.bytes.len())),
            Err(e) => CheckReport::failed(e.to_string()),
        })
    }

    async fn list(&self, config: &Value, cursor: Option<&str>) -> Result<Batch> {
        let cfg = Self::cfg(config)?;
        let prev: Option<WebCursor> = match cursor {
            Some(c) => Some(serde_json::from_str(c).map_err(|e| ConnectorError::BadCursor(e.to_string()))?),
            None => None,
        };
        if prev.is_some() && !cfg.refetch {
            return Ok(Batch { next_cursor: cursor.map(str::to_owned), ..Default::default() });
        }
        let f = http::get(&cfg.url, &[("accept".to_string(), "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5".to_string())], Duration::from_secs(cfg.timeout_secs.unwrap_or(30))).await?;
        http::ensure_ok(&f, false)?;
        let ct = f.header("content-type").unwrap_or_default();
        if !ct.is_empty() && !ct.contains("html") && !ct.contains("text/") {
            return Err(ConnectorError::Unsupported(format!("{} is {ct}, not a web page", cfg.url)));
        }
        let page = String::from_utf8_lossy(&f.bytes);
        let mut doc = clip_html(&page, &f.final_url, &cfg)?;
        if let Some(p) = &prev {
            doc.external_id = p.external_id.clone();
        }
        let hash = sha256_hex(doc.body.as_bytes());
        let next = WebCursor { v: 1, external_id: doc.external_id.clone(), hash: hash.clone() };
        let changed = prev.as_ref().is_none_or(|p| p.hash != hash);
        Ok(Batch {
            upserts: if changed { vec![doc] } else { vec![] },
            deletes: vec![],
            renames: vec![],
            next_cursor: Some(serde_json::to_string(&next).expect("cursor serialises")),
            has_more: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html><html lang="en"><head>
<title>On Recollection | Garden</title>
<meta property="og:title" content="On Recollection">
<meta property="og:type" content="article">
<meta property="og:site_name" content="Garden">
<meta name="author" content="Ana Mnesis">
<meta property="article:published_time" content="2024-03-05T10:00:00Z">
<meta name="description" content="Is learning recollection?">
<link rel="canonical" href="https://garden.example/essays/recollection/?utm_source=x">
<script>var tracking = "should not appear";</script>
<style>.x{color:red}</style>
</head><body>
<header class="site-header"><nav><a href="/">home</a> <a href="/about">about</a></nav></header>
<div class="layout">
 <aside class="sidebar"><p>Subscribe to the newsletter for more posts like this one, every week, forever.</p></aside>
 <article class="post">
  <h1>On Recollection</h1>
  <p class="byline">by Ana</p>
  <p>In the Meno, Socrates argues that learning is a kind of recollection, drawing out of the soul what it already knew, before birth.</p>
  <p>The slave boy, questioned carefully, arrives at the doubling of the square, and so, the argument goes, he must have known it already.</p>
  <p>Whether this is true, and how one would measure it, is the question this essay circles; see <a href="/essays/meno#part-2">the Meno essay</a>.</p>
  <div class="share-buttons"><a href="https://twitter.example/share">share this post on social media</a></div>
  <form><input name="q"><button>search</button></form>
 </article>
</div>
<footer class="footer"><p>Copyright Garden, all rights reserved, no part of this may be reproduced.</p></footer>
</body></html>"#;

    #[test]
    fn extracts_main_content() {
        let d = clip_html(PAGE, "https://garden.example/essays/recollection?fbclid=1", &WebConfig::default()).unwrap();
        assert_eq!(d.external_id, "https://garden.example/essays/recollection");
        assert_eq!(d.title, "On Recollection");
        assert_eq!(d.kind, ItemKind::Article);
        assert_eq!(d.origin, Origin::External);
        assert!(d.body.contains("learning is a kind of recollection"), "{}", d.body);
        assert!(d.body.contains("doubling of the square"));
        for junk in ["should not appear", "color:red", "newsletter", "Copyright", "share this post", "search", "about"] {
            assert!(!d.body.contains(junk), "body contains {junk:?}:\n{}", d.body);
        }
        assert_eq!(d.meta["author"], "Ana Mnesis");
        assert_eq!(d.meta["site_name"], "Garden");
        assert_eq!(d.created_at.unwrap().to_rfc3339(), "2024-03-05T10:00:00+00:00");
        assert_eq!(d.refs, vec!["https://garden.example/essays/meno"]);
    }

    #[test]
    fn foreign_canonical_is_ignored_and_selector_overrides() {
        let page = r#"<html><head><link rel="canonical" href="https://elsewhere.example/x"><title>T</title></head>
<body><div id="a"><p>alpha text that is long enough to be a paragraph, with, commas.</p></div><div id="b"><p>beta</p></div></body></html>"#;
        let d = clip_html(page, "https://site.example/page/", &WebConfig::default()).unwrap();
        assert_eq!(d.external_id, "https://site.example/page");
        assert_eq!(d.kind, ItemKind::Clip);
        assert!(d.body.contains("alpha"));
        let cfg = WebConfig { selector: Some("#b".into()), ..Default::default() };
        let d = clip_html(page, "https://site.example/page/", &cfg).unwrap();
        assert_eq!(d.body.trim(), "beta");
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_clip() {
        let b = WebConnector.list(&json!({"url": "https://www.rust-lang.org/"}), None).await.unwrap();
        assert_eq!(b.upserts.len(), 1);
    }
}
