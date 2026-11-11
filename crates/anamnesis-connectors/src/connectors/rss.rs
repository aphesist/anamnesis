//! `rss`: RSS 0.9x/1.0/2.0, Atom and JSON Feed via `feed-rs`, fetched over HTTP.
//!
//! One document per entry (kind `feed`, origin `external`). external id = the entry id
//! (guid / atom id; feed-rs derives a stable one when missing). HTML content is rendered to
//! markdown-ish text. Entries that drop out of the feed are not deleted: a feed is a window,
//! not a mirror.
//!
//! Cursor: `{ "v": 1, "etag", "last_modified", "seen": { entry id: content hash } }`. The
//! fetch is conditional (`If-None-Match` / `If-Modified-Since`); only new or changed entries
//! are returned.

use std::collections::BTreeMap;
use std::time::Duration;

use anamnesis_core::{Document, ItemKind, Origin, Privacy};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::connector::{Batch, CheckReport, Connector};
use crate::error::{ConnectorError, Result};
use crate::html::html_to_text;
use crate::http;
use crate::manifest::{API_VERSION, Capability, Manifest, Runtime};
use crate::scan::sha256_hex;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RssConfig {
    url: String,
    #[serde(default)]
    origin: Option<Origin>,
    #[serde(default)]
    kind: Option<ItemKind>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct RssCursor {
    v: u32,
    etag: Option<String>,
    last_modified: Option<String>,
    seen: BTreeMap<String, String>,
}

pub struct RssConnector;

pub fn rss_manifest() -> Manifest {
    Manifest {
        id: "rss".into(),
        name: "feed".into(),
        description: "rss, atom and json feeds".into(),
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
                "origin": {"enum": ["owner_written", "owner_spoken", "owner_annotation", "owner_in_chat", "ai_generated", "ai_edited", "external"]},
                "kind": {"enum": ["note", "journal", "quote", "article", "pdf", "book", "chat", "clip", "feed", "skill"]},
                "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 600}
            }
        }),
        capabilities: vec![Capability::Check, Capability::Incremental],
        network_hosts: vec!["<config.url host>".into()],
        fs_paths: vec![],
        default_origin: Origin::External,
        default_privacy: Privacy::CloudOk,
        default_kind: Some(ItemKind::Feed),
        poll_interval_secs: Some(3600),
    }
}

fn text_of(t: &feed_rs::model::Text) -> String {
    let ct = t.content_type.to_string();
    if ct.contains("html") {
        html_to_text(&t.content)
    } else {
        t.content.trim().to_string()
    }
}

/// Parse feed bytes into documents. `feed_url` is the url the feed was fetched from; it
/// resolves relative links and is recorded in meta.
pub fn parse_feed(
    bytes: &[u8],
    feed_url: &str,
    origin: Origin,
    kind: ItemKind,
) -> Result<(Vec<Document>, Value)> {
    let feed = feed_rs::parser::Builder::new()
        .base_uri(Some(feed_url))
        .build()
        .parse(bytes)
        .map_err(|e| ConnectorError::Parse(format!("feed: {e}")))?;
    let feed_title = feed.title.as_ref().map(text_of);
    let feed_meta = json!({
        "feed_title": feed_title,
        "feed_url": feed_url,
        "feed_id": feed.id,
        "feed_type": format!("{:?}", feed.feed_type).to_lowercase(),
        "site": feed.links.iter().find(|l| l.rel.as_deref() != Some("self")).map(|l| l.href.clone()),
    });
    let mut docs = Vec::new();
    for e in feed.entries {
        let link = e
            .links
            .iter()
            .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
            .or(e.links.first())
            .map(|l| l.href.clone());
        let body = match (&e.content, &e.summary) {
            (Some(c), _) if c.body.as_deref().is_some_and(|b| !b.trim().is_empty()) => {
                let b = c.body.clone().unwrap_or_default();
                if c.content_type.to_string().contains("html") {
                    html_to_text(&b)
                } else {
                    b.trim().to_string()
                }
            }
            (_, Some(s)) => text_of(s),
            _ => String::new(),
        };
        let title = e
            .title
            .as_ref()
            .map(text_of)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                body.lines()
                    .next()
                    .unwrap_or("untitled")
                    .chars()
                    .take(80)
                    .collect()
            });
        let authors: Vec<&str> = e.authors.iter().filter_map(|a| a.name.as_deref()).collect();
        let tags: Vec<String> = e
            .categories
            .iter()
            .map(|c| c.label.clone().unwrap_or_else(|| c.term.clone()))
            .collect();
        let mut meta = feed_meta.clone();
        meta["authors"] = json!(authors);
        meta["summary"] = json!(e.summary.as_ref().map(text_of));
        if let Some(l) = &link {
            meta["canonical_url"] = json!(crate::url_norm::normalize_url(l).ok());
        }
        docs.push(Document {
            external_id: e.id.clone(),
            uri: link.clone(),
            kind,
            title,
            body,
            origin,
            created_at: e.published.or(e.updated),
            modified_at: e.updated.or(e.published),
            tags,
            refs: link.into_iter().collect(),
            meta,
        });
    }
    Ok((docs, feed_meta))
}

fn doc_hash(d: &Document) -> String {
    sha256_hex(
        format!(
            "{}\u{0}{}\u{0}{:?}\u{0}{:?}",
            d.title, d.body, d.uri, d.modified_at
        )
        .as_bytes(),
    )
}

impl RssConnector {
    fn cfg(config: &Value) -> Result<RssConfig> {
        serde_json::from_value(config.clone())
            .map_err(|e| ConnectorError::InvalidConfig(vec![e.to_string()]))
    }

    /// Diff parsed documents against the cursor. Pure; used by `list` and tests.
    fn diff(docs: Vec<Document>, prev: &RssCursor) -> (Vec<Document>, BTreeMap<String, String>) {
        let mut seen = BTreeMap::new();
        let mut upserts = Vec::new();
        for d in docs {
            let h = doc_hash(&d);
            if prev.seen.get(&d.external_id) != Some(&h) {
                upserts.push(d.clone());
            }
            seen.insert(d.external_id, h);
        }
        (upserts, seen)
    }
}

fn parse_cursor(s: Option<&str>) -> Result<RssCursor> {
    match s {
        None => Ok(RssCursor {
            v: 1,
            ..Default::default()
        }),
        Some(s) => serde_json::from_str(s).map_err(|e| ConnectorError::BadCursor(e.to_string())),
    }
}

#[async_trait]
impl Connector for RssConnector {
    fn manifest(&self) -> Manifest {
        rss_manifest()
    }

    async fn check(&self, config: &Value) -> Result<CheckReport> {
        let cfg = Self::cfg(config)?;
        let timeout = Duration::from_secs(cfg.timeout_secs.unwrap_or(30));
        let f = match http::get(&cfg.url, &[], timeout).await {
            Ok(f) => f,
            Err(e) => {
                return Ok(CheckReport::failed(format!(
                    "could not reach the feed: {e}"
                )));
            }
        };
        if let Err(e) = http::ensure_ok(&f, false) {
            return Ok(CheckReport::failed(e.to_string()));
        }
        Ok(
            match parse_feed(&f.bytes, &f.final_url, Origin::External, ItemKind::Feed) {
                Ok((docs, _)) => {
                    CheckReport::ok(format!("feed answered with {} entries", docs.len()))
                }
                Err(e) => CheckReport::failed(format!("not a feed: {e}")),
            },
        )
    }

    async fn list(&self, config: &Value, cursor: Option<&str>) -> Result<Batch> {
        let cfg = Self::cfg(config)?;
        let prev = parse_cursor(cursor)?;
        let timeout = Duration::from_secs(cfg.timeout_secs.unwrap_or(30));
        let mut headers = Vec::new();
        if let Some(e) = &prev.etag {
            headers.push(("if-none-match".to_string(), e.clone()));
        }
        if let Some(lm) = &prev.last_modified {
            headers.push(("if-modified-since".to_string(), lm.clone()));
        }
        let f = http::get(&cfg.url, &headers, timeout).await?;
        http::ensure_ok(&f, true)?;
        if f.status == 304 {
            return Ok(Batch {
                next_cursor: cursor.map(str::to_owned),
                ..Default::default()
            });
        }
        let (docs, _) = parse_feed(
            &f.bytes,
            &cfg.url,
            cfg.origin.unwrap_or(Origin::External),
            cfg.kind.unwrap_or(ItemKind::Feed),
        )?;
        let (upserts, seen) = Self::diff(docs, &prev);
        let next = RssCursor {
            v: 1,
            etag: f.header("etag"),
            last_modified: f.header("last-modified"),
            seen,
        };
        Ok(Batch {
            upserts,
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

    pub const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
<title>Garden Notes</title><link>https://garden.example/</link><description>d</description>
<item><title>On &lt;b&gt;memory&lt;/b&gt;</title><link>https://garden.example/memory?utm_source=rss</link>
<guid>https://garden.example/memory</guid><pubDate>Tue, 05 Mar 2024 10:00:00 GMT</pubDate>
<category>memory</category><author>ana@example.org (Ana)</author>
<description><![CDATA[<p>Recollection is <em>not</em> retrieval.</p><p>Second para.</p>]]></description></item>
<item><title>Untagged</title><link>/relative</link><guid isPermaLink="false">id-2</guid><description>plain text</description></item>
</channel></rss>"#;

    pub const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom Log</title><id>urn:x:feed</id><updated>2024-04-01T00:00:00Z</updated>
<link href="https://log.example/"/>
<entry><title type="text">First</title><id>urn:x:1</id><updated>2024-04-01T00:00:00Z</updated><published>2024-03-31T12:00:00Z</published>
<link rel="alternate" href="https://log.example/1"/><author><name>Ana</name></author><category term="philosophy" label="Philosophy"/>
<content type="html">&lt;h2&gt;Sub&lt;/h2&gt;&lt;p&gt;Body &lt;a href="x"&gt;link&lt;/a&gt;&lt;/p&gt;</content></entry>
</feed>"#;

    #[test]
    fn parses_rss() {
        let (docs, meta) = parse_feed(
            RSS.as_bytes(),
            "https://garden.example/feed.xml",
            Origin::External,
            ItemKind::Feed,
        )
        .unwrap();
        assert_eq!(meta["feed_title"], "Garden Notes");
        assert_eq!(docs.len(), 2);
        let d = &docs[0];
        assert_eq!(d.external_id, "https://garden.example/memory");
        assert_eq!(d.kind, ItemKind::Feed);
        assert_eq!(d.origin, Origin::External);
        assert_eq!(d.tags, vec!["memory"]);
        assert!(
            d.body.contains("Recollection is *not* retrieval."),
            "{}",
            d.body
        );
        assert!(d.body.contains("Second para."));
        assert_eq!(d.meta["canonical_url"], "https://garden.example/memory");
        assert_eq!(
            d.created_at.unwrap().to_rfc3339(),
            "2024-03-05T10:00:00+00:00"
        );
        assert_eq!(docs[1].external_id, "id-2");
        assert_eq!(
            docs[1].uri.as_deref(),
            Some("https://garden.example/relative")
        );
    }

    #[test]
    fn parses_atom() {
        let (docs, meta) = parse_feed(
            ATOM.as_bytes(),
            "https://log.example/atom.xml",
            Origin::External,
            ItemKind::Feed,
        )
        .unwrap();
        assert_eq!(meta["feed_title"], "Atom Log");
        let d = &docs[0];
        assert_eq!(d.external_id, "urn:x:1");
        assert_eq!(d.title, "First");
        assert_eq!(d.uri.as_deref(), Some("https://log.example/1"));
        assert_eq!(d.tags, vec!["Philosophy"]);
        assert!(d.body.contains("## Sub"), "{}", d.body);
        assert!(d.body.contains("Body link"), "{}", d.body);
        assert_eq!(d.meta["authors"][0], "Ana");
        assert_eq!(
            d.created_at.unwrap().to_rfc3339(),
            "2024-03-31T12:00:00+00:00"
        );
    }

    #[test]
    fn diff_only_returns_changes() {
        let (docs, _) = parse_feed(
            RSS.as_bytes(),
            "https://garden.example/feed.xml",
            Origin::External,
            ItemKind::Feed,
        )
        .unwrap();
        let (up, seen) = RssConnector::diff(docs.clone(), &RssCursor::default());
        assert_eq!(up.len(), 2);
        let prev = RssCursor {
            v: 1,
            seen,
            ..Default::default()
        };
        let (up, _) = RssConnector::diff(docs.clone(), &prev);
        assert!(up.is_empty());
        let mut changed = docs;
        changed[1].body.push_str(" edited");
        let (up, _) = RssConnector::diff(changed, &prev);
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].external_id, "id-2");
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn live_feed() {
        let b = RssConnector
            .list(&json!({"url": "https://blog.rust-lang.org/feed.xml"}), None)
            .await
            .unwrap();
        assert!(!b.upserts.is_empty());
    }
}
