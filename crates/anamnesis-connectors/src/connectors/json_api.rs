//! `json-api`: a generic paged JSON endpoint (a personal site's content api, a notes service).
//!
//! Config maps fields of each item (dot paths) onto a document, e.g.
//!
//! ```json
//! { "url": "https://me.example/api/posts", "items_path": "data",
//!   "fields": { "id": "id", "title": "title", "body": "content", "updated_at": "updated_at",
//!               "url": "links.html", "tags": "tags", "deleted": "deleted" },
//!   "since_param": "updated_after", "page_param": "page", "next_path": "links.next" }
//! ```
//!
//! **Incremental.** The cursor remembers the largest `updated_at` seen. Each sync sends it as
//! `since_param` (if configured) and also filters client-side (`updated_at` strictly greater),
//! so an api that ignores the parameter still works, just slower. Values are compared as
//! timestamps when both parse, else as strings.
//!
//! **Paging.** One page per `list` call: the batch has `has_more = true` while there are more
//! pages (via `next_path` in the response, or `page_param` counting up until a page has fewer
//! than `per_page` items or is empty), capped by `max_pages`.
//!
//! **Auth.** Config never holds secrets: `token_env` names an environment variable whose value
//! is sent as `Authorization: Bearer <value>` (or in `auth_header` if set).

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
use crate::markdown::parse_date;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FieldMap {
    pub id: String,
    pub title: Option<String>,
    pub body: String,
    pub updated_at: String,
    pub created_at: Option<String>,
    pub url: Option<String>,
    pub tags: Option<String>,
    pub deleted: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct JsonApiConfig {
    pub url: String,
    /// Dot path to the array of items; empty / absent = the response is the array.
    #[serde(default)]
    pub items_path: Option<String>,
    pub fields: FieldMap,
    #[serde(default)]
    pub since_param: Option<String>,
    #[serde(default)]
    pub page_param: Option<String>,
    #[serde(default)]
    pub page_start: Option<u64>,
    #[serde(default)]
    pub per_page_param: Option<String>,
    #[serde(default)]
    pub per_page: Option<u64>,
    /// Dot path to the next page's url in the response.
    #[serde(default)]
    pub next_path: Option<String>,
    #[serde(default)]
    pub max_pages: Option<u64>,
    /// "markdown" (default), "html" or "text".
    #[serde(default)]
    pub body_format: Option<String>,
    #[serde(default)]
    pub kind: Option<ItemKind>,
    #[serde(default)]
    pub origin: Option<Origin>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
struct ApiCursor {
    v: u32,
    /// Largest updated_at fully synced.
    since: Option<String>,
    /// While paging: the page/url to fetch next and the max updated_at seen so far in this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    page: Option<u64>,
    #[serde(default)]
    pages_done: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_max: Option<String>,
}

pub struct JsonApiConnector;

pub fn json_api_manifest() -> Manifest {
    let field = json!({"type": "string", "minLength": 1});
    Manifest {
        id: "json-api".into(),
        name: "json api".into(),
        description: "a paged json endpoint with an updated_at cursor and a field mapping".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION,
        runtime: Runtime::Builtin,
        entry: None,
        config_schema: json!({
            "type": "object",
            "required": ["url", "fields"],
            "additionalProperties": false,
            "properties": {
                "url": {"type": "string", "pattern": "^https?://"},
                "items_path": {"type": "string"},
                "fields": {
                    "type": "object",
                    "required": ["id", "body", "updated_at"],
                    "additionalProperties": false,
                    "properties": {
                        "id": field, "title": field, "body": field, "updated_at": field, "created_at": field,
                        "url": field, "tags": field, "deleted": field, "kind": field
                    }
                },
                "since_param": {"type": "string"},
                "page_param": {"type": "string"},
                "page_start": {"type": "integer", "minimum": 0},
                "per_page_param": {"type": "string"},
                "per_page": {"type": "integer", "minimum": 1},
                "next_path": {"type": "string"},
                "max_pages": {"type": "integer", "minimum": 1},
                "body_format": {"enum": ["markdown", "html", "text"]},
                "kind": {"enum": ["note", "journal", "quote", "article", "pdf", "book", "chat", "clip", "feed", "skill"]},
                "origin": {"enum": ["owner_written", "owner_spoken", "owner_annotation", "owner_in_chat", "ai_generated", "ai_edited", "external"]},
                "token_env": {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
                "auth_header": {"type": "string"},
                "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 600}
            }
        }),
        capabilities: vec![Capability::Check, Capability::Incremental, Capability::Deletes, Capability::Paged],
        network_hosts: vec!["<config.url host>".into()],
        fs_paths: vec![],
        default_origin: Origin::OwnerWritten,
        default_privacy: Privacy::CloudOk,
        default_kind: Some(ItemKind::Article),
        poll_interval_secs: Some(3600),
    }
}

/// `a.b.0.c` lookup.
pub fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(v);
    }
    let mut cur = v;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn as_string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => matches!(s.as_str(), "true" | "1" | "yes" | "deleted"),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

fn tags_of(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|t| match t {
                Value::Object(o) => as_string(o.get("name").or_else(|| o.get("slug")).or_else(|| o.get("title"))),
                other => as_string(Some(other)),
            })
            .collect(),
        Some(Value::String(s)) => s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
        _ => vec![],
    }
}

/// Is `a` later than `b`? Timestamps when both parse, else string order.
fn later(a: &str, b: &str) -> bool {
    match (parse_date(a), parse_date(b)) {
        (Some(x), Some(y)) => x > y,
        _ => a > b,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub upserts: Vec<Document>,
    pub deletes: Vec<String>,
    pub max_updated: Option<String>,
    pub next_url: Option<String>,
    /// Items in the page before client-side filtering.
    pub count: usize,
}

/// Map one response page onto documents. Pure; `since` filters out unchanged items.
pub fn map_page(resp: &Value, cfg: &JsonApiConfig, since: Option<&str>) -> Result<Page> {
    let items_path = cfg.items_path.as_deref().unwrap_or("");
    let items = get_path(resp, items_path)
        .and_then(Value::as_array)
        .ok_or_else(|| ConnectorError::Parse(format!("no array at items_path {items_path:?}")))?;
    let f = &cfg.fields;
    let mut page = Page {
        count: items.len(),
        next_url: cfg.next_path.as_deref().and_then(|p| as_string(get_path(resp, p))).filter(|s| !s.is_empty()),
        ..Default::default()
    };
    for it in items {
        let Some(id) = as_string(get_path(it, &f.id)) else {
            tracing::warn!("json-api item without id; skipped");
            continue;
        };
        let updated = as_string(get_path(it, &f.updated_at));
        if let Some(u) = &updated {
            if page.max_updated.as_deref().is_none_or(|m| later(u, m)) {
                page.max_updated = Some(u.clone());
            }
            if let Some(s) = since
                && !later(u, s)
            {
                continue;
            }
        }
        if f.deleted.as_deref().is_some_and(|d| truthy(get_path(it, d))) {
            page.deletes.push(id);
            continue;
        }
        let raw_body = as_string(get_path(it, &f.body)).unwrap_or_default();
        let body = match cfg.body_format.as_deref() {
            Some("html") => html_to_text(&raw_body),
            _ => raw_body,
        };
        let title = f
            .title
            .as_deref()
            .and_then(|t| as_string(get_path(it, t)))
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| body.lines().next().unwrap_or(&id).chars().take(80).collect());
        let kind = f
            .kind
            .as_deref()
            .and_then(|k| get_path(it, k).cloned())
            .and_then(|k| serde_json::from_value::<ItemKind>(k).ok())
            .or(cfg.kind)
            .unwrap_or(ItemKind::Article);
        let uri = f.url.as_deref().and_then(|u| as_string(get_path(it, u)));
        page.upserts.push(Document {
            external_id: id,
            uri: uri.clone(),
            kind,
            title,
            body,
            origin: cfg.origin.unwrap_or(Origin::OwnerWritten),
            created_at: f.created_at.as_deref().and_then(|c| as_string(get_path(it, c))).and_then(|c| parse_date(&c)),
            modified_at: updated.as_deref().and_then(parse_date),
            tags: tags_of(f.tags.as_deref().and_then(|t| get_path(it, t))),
            refs: uri.into_iter().collect(),
            meta: json!({ "source_url": cfg.url, "item": it, "connector": "json-api" }),
        });
    }
    Ok(page)
}

impl JsonApiConnector {
    fn cfg(config: &Value) -> Result<JsonApiConfig> {
        serde_json::from_value(config.clone()).map_err(|e| ConnectorError::InvalidConfig(vec![e.to_string()]))
    }

    fn page_url(cfg: &JsonApiConfig, since: Option<&str>, page: Option<u64>) -> Result<String> {
        let mut u = url::Url::parse(&cfg.url).map_err(|e| ConnectorError::InvalidConfig(vec![format!("url: {e}")]))?;
        {
            let mut q = u.query_pairs_mut();
            if let (Some(p), Some(s)) = (&cfg.since_param, since) {
                q.append_pair(p, s);
            }
            if let (Some(p), Some(n)) = (&cfg.page_param, page) {
                q.append_pair(p, &n.to_string());
            }
            if let (Some(p), Some(n)) = (&cfg.per_page_param, cfg.per_page) {
                q.append_pair(p, &n.to_string());
            }
        }
        let s = u.to_string();
        Ok(s.strip_suffix('?').map(str::to_string).unwrap_or(s))
    }

    fn headers(cfg: &JsonApiConfig) -> Result<Vec<(String, String)>> {
        let mut h = vec![("accept".to_string(), "application/json".to_string())];
        if let Some(var) = &cfg.token_env {
            let tok = std::env::var(var)
                .map_err(|_| ConnectorError::InvalidConfig(vec![format!("environment variable {var} is not set")]))?;
            match cfg.auth_header.as_deref().map(str::to_ascii_lowercase) {
                None => h.push(("authorization".into(), format!("Bearer {tok}"))),
                Some(name) if name == "authorization" => h.push((name, format!("Bearer {tok}"))),
                Some(name) => h.push((name, tok)),
            }
        }
        Ok(h)
    }

    async fn fetch_json(cfg: &JsonApiConfig, url: &str) -> Result<Value> {
        let f = http::get(url, &Self::headers(cfg)?, Duration::from_secs(cfg.timeout_secs.unwrap_or(30))).await?;
        http::ensure_ok(&f, false)?;
        serde_json::from_slice(&f.bytes).map_err(|e| ConnectorError::Parse(format!("json from {url}: {e}")))
    }
}

#[async_trait]
impl Connector for JsonApiConnector {
    fn manifest(&self) -> Manifest {
        json_api_manifest()
    }

    async fn check(&self, config: &Value) -> Result<CheckReport> {
        let cfg = Self::cfg(config)?;
        let url = Self::page_url(&cfg, None, cfg.page_param.as_ref().map(|_| cfg.page_start.unwrap_or(1)))?;
        let v = match Self::fetch_json(&cfg, &url).await {
            Ok(v) => v,
            Err(e) => return Ok(CheckReport::failed(e.to_string())),
        };
        Ok(match map_page(&v, &cfg, None) {
            Ok(p) => CheckReport::ok(format!("endpoint answered with {} items", p.count)),
            Err(e) => CheckReport::failed(e.to_string()),
        })
    }

    async fn list(&self, config: &Value, cursor: Option<&str>) -> Result<Batch> {
        let cfg = Self::cfg(config)?;
        let mut cur: ApiCursor = match cursor {
            Some(c) => serde_json::from_str(c).map_err(|e| ConnectorError::BadCursor(e.to_string()))?,
            None => ApiCursor { v: 1, ..Default::default() },
        };
        let first_page = cfg.page_start.unwrap_or(1);
        let url = match (&cur.next_url, cur.page) {
            (Some(u), _) => u.clone(),
            (None, page) => Self::page_url(
                &cfg,
                cur.since.as_deref(),
                cfg.page_param.as_ref().map(|_| page.unwrap_or(first_page)),
            )?,
        };
        let resp = Self::fetch_json(&cfg, &url).await?;
        let page = map_page(&resp, &cfg, cur.since.as_deref())?;
        if let Some(m) = &page.max_updated
            && cur.run_max.as_deref().is_none_or(|r| later(m, r))
        {
            cur.run_max = Some(m.clone());
        }
        cur.pages_done += 1;
        let max_pages = cfg.max_pages.unwrap_or(100);
        let more = cur.pages_done < max_pages
            && page.count > 0
            && if page.next_url.is_some() {
                true
            } else if cfg.page_param.is_some() {
                cfg.per_page.is_none_or(|n| page.count as u64 >= n)
            } else {
                false
            };
        if more {
            match &page.next_url {
                Some(n) => {
                    // Resolve relative next links against the current url.
                    let abs = url::Url::parse(&url).and_then(|b| b.join(n)).map(|u| u.to_string()).unwrap_or(n.clone());
                    cur.next_url = Some(abs);
                }
                None => cur.page = Some(cur.page.unwrap_or(first_page) + 1),
            }
        } else {
            if let Some(m) = cur.run_max.take()
                && cur.since.as_deref().is_none_or(|s| later(&m, s))
            {
                cur.since = Some(m);
            }
            cur.next_url = None;
            cur.page = None;
            cur.pages_done = 0;
        }
        Ok(Batch {
            upserts: page.upserts,
            deletes: page.deletes,
            renames: vec![],
            next_cursor: Some(serde_json::to_string(&cur).expect("cursor serialises")),
            has_more: more,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::list_all;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn cfg() -> JsonApiConfig {
        serde_json::from_value(json!({
            "url": "http://127.0.0.1:1/api/posts",
            "items_path": "data",
            "fields": {"id": "id", "title": "title", "body": "content", "updated_at": "updated_at",
                       "url": "links.html", "tags": "tags", "deleted": "deleted", "created_at": "created"},
            "body_format": "html"
        }))
        .unwrap()
    }

    fn page_json() -> Value {
        json!({"data": [
            {"id": 1, "title": "first", "content": "<p>hello <b>there</b></p>", "updated_at": "2024-01-02T00:00:00Z",
             "created": "2024-01-01", "links": {"html": "https://me.example/1"}, "tags": [{"name": "a"}, "b"]},
            {"id": 2, "title": "", "content": "body line\nmore", "updated_at": "2024-01-05T00:00:00Z", "tags": "x, y"},
            {"id": 3, "deleted": true, "updated_at": "2024-01-04T00:00:00Z", "content": ""}
        ], "links": {"next": null}})
    }

    #[test]
    fn maps_fields() {
        let p = map_page(&page_json(), &cfg(), None).unwrap();
        assert_eq!(p.count, 3);
        assert_eq!(p.upserts.len(), 2);
        assert_eq!(p.deletes, vec!["3"]);
        assert_eq!(p.max_updated.as_deref(), Some("2024-01-05T00:00:00Z"));
        let d = &p.upserts[0];
        assert_eq!(d.external_id, "1");
        assert_eq!(d.body, "hello **there**");
        assert_eq!(d.tags, vec!["a", "b"]);
        assert_eq!(d.uri.as_deref(), Some("https://me.example/1"));
        assert_eq!(d.created_at.unwrap().to_rfc3339(), "2024-01-01T00:00:00+00:00");
        assert_eq!(p.upserts[1].title, "body line more");
        assert_eq!(p.upserts[1].tags, vec!["x", "y"]);
        // Incremental filter.
        let p = map_page(&page_json(), &cfg(), Some("2024-01-03T00:00:00Z")).unwrap();
        assert_eq!(p.upserts.iter().map(|d| d.external_id.as_str()).collect::<Vec<_>>(), vec!["2"]);
        assert_eq!(p.deletes, vec!["3"]);
        assert!(map_page(&json!({"nope": 1}), &cfg(), None).is_err());
    }

    /// A tiny loopback http server answering canned json by `page` query param.
    fn serve(pages: Vec<Value>, expect: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let h = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..expect {
                let (mut s, _) = l.accept().unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap().to_string();
                let page: usize = path.split("page=").nth(1).and_then(|p| p.split('&').next()).and_then(|p| p.parse().ok()).unwrap_or(1);
                let body = pages.get(page - 1).cloned().unwrap_or(json!({"data": []})).to_string();
                write!(s, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", body.len(), body).unwrap();
                seen.push(path);
            }
            seen
        });
        (format!("http://{addr}/api/posts"), h)
    }

    #[tokio::test]
    async fn pages_and_cursor_over_loopback() {
        let p1 = json!({"data": [
            {"id": "a", "content": "one", "updated_at": "2024-01-01T00:00:00Z"},
            {"id": "b", "content": "two", "updated_at": "2024-01-03T00:00:00Z"}]});
        let p2 = json!({"data": [{"id": "c", "content": "three", "updated_at": "2024-01-02T00:00:00Z"}]});
        let (url, h) = serve(vec![p1, p2], 4);
        let config = json!({
            "url": url, "items_path": "data",
            "fields": {"id": "id", "body": "content", "updated_at": "updated_at"},
            "since_param": "updated_after", "page_param": "page", "per_page_param": "per_page", "per_page": 2
        });
        let c = JsonApiConnector;
        let b1 = c.list(&config, None).await.unwrap();
        assert!(b1.has_more);
        assert_eq!(b1.upserts.len(), 2);
        let b2 = c.list(&config, b1.next_cursor.as_deref()).await.unwrap();
        assert!(!b2.has_more);
        assert_eq!(b2.upserts[0].external_id, "c");
        let cur: ApiCursor = serde_json::from_str(b2.next_cursor.as_deref().unwrap()).unwrap();
        assert_eq!(cur.since.as_deref(), Some("2024-01-03T00:00:00Z"));
        // Next sync sends the cursor; the canned server ignores it, the client-side filter drops all.
        let b3 = list_all(&c, &config, b2.next_cursor.as_deref()).await.unwrap();
        assert!(b3.upserts.is_empty());
        let paths = h.join().unwrap();
        assert_eq!(paths[0], "/api/posts?page=1&per_page=2");
        assert_eq!(paths[1], "/api/posts?page=2&per_page=2");
        assert!(paths[2].contains("updated_after=2024-01-03T00%3A00%3A00Z"), "{}", paths[2]);
        assert!(paths[3].contains("page=2"), "{}", paths[3]);
    }
}
