//! `pdf`: text extraction from PDF files (one file or a folder of them), in pure Rust via
//! `pdf-extract` (built on `lopdf`).
//!
//! The body is the text of each page joined by a blank line. `meta.pages` lists
//! `{ page, start, end }` byte offsets into the body (1-based page numbers), so a citation
//! span can be mapped back to a page. Scanned PDFs without a text layer produce an empty
//! body and `meta.text_layer = false` (no OCR here).

use std::path::Path;

use anamnesis_core::{Document, ItemKind, Origin, Privacy};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::connector::{Batch, CheckReport, Connector};
use crate::connectors::files::{expand_home, file_uri};
use crate::error::{ConnectorError, Result};
use crate::manifest::{API_VERSION, Capability, Manifest, Runtime};
use crate::scan::{self, FileCursor, ScanOptions};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PdfConfig {
    path: String,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    origin: Option<Origin>,
    #[serde(default)]
    kind: Option<ItemKind>,
    #[serde(default)]
    max_file_bytes: Option<u64>,
}

pub struct PdfConnector;

pub fn pdf_manifest() -> Manifest {
    Manifest {
        id: "pdf".into(),
        name: "pdfs".into(),
        description: "text of pdf files, with page markers for citations".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION,
        runtime: Runtime::Builtin,
        entry: None,
        config_schema: json!({
            "type": "object",
            "required": ["path"],
            "additionalProperties": false,
            "properties": {
                "path": {"type": "string", "minLength": 1, "description": "a pdf file or a folder of pdfs"},
                "ignore": {"type": "array", "items": {"type": "string"}},
                "origin": {"enum": ["owner_written", "owner_spoken", "owner_annotation", "owner_in_chat", "ai_generated", "ai_edited", "external"]},
                "kind": {"enum": ["note", "journal", "quote", "article", "pdf", "book", "chat", "clip", "feed", "skill"]},
                "max_file_bytes": {"type": "integer", "minimum": 1}
            }
        }),
        capabilities: vec![
            Capability::Check,
            Capability::Incremental,
            Capability::Deletes,
            Capability::Renames,
        ],
        network_hosts: vec![],
        fs_paths: vec!["<config.path>".into()],
        default_origin: Origin::External,
        default_privacy: Privacy::CloudOk,
        default_kind: Some(ItemKind::Pdf),
        poll_interval_secs: Some(300),
    }
}

/// Extracted text of a PDF.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfText {
    pub body: String,
    /// (page number from 1, start, end) byte offsets into `body`.
    pub pages: Vec<(u32, usize, usize)>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub page_count: usize,
}

fn info_string(doc: &pdf_extract::Document, key: &[u8]) -> Option<String> {
    let info = doc.trailer.get(b"Info").ok()?;
    let dict = match info {
        pdf_extract::Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok()?,
        pdf_extract::Object::Dictionary(d) => d,
        _ => return None,
    };
    let v = dict.get(key).ok()?;
    let v = match v {
        pdf_extract::Object::Reference(id) => doc.get_object(*id).ok()?,
        v => v,
    };
    pdf_extract::decode_text_string(v)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Extract per-page text and document info. Never panics: pdf-extract can panic on
/// malformed input, which is caught and turned into an error.
pub fn extract_pdf(bytes: &[u8]) -> Result<PdfText> {
    let owned = bytes.to_vec();
    let pages =
        std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem_by_pages(&owned))
            .map_err(|_| ConnectorError::Parse("pdf extractor panicked on this file".into()))?
            .map_err(|e| ConnectorError::Parse(format!("pdf: {e}")))?;
    let mut body = String::new();
    let mut spans = Vec::with_capacity(pages.len());
    for (i, p) in pages.iter().enumerate() {
        let text = normalise_page(p);
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        let start = body.len();
        body.push_str(&text);
        spans.push((i as u32 + 1, start, body.len()));
    }
    let doc = pdf_extract::Document::load_mem(bytes).ok();
    let get = |k: &[u8]| doc.as_ref().and_then(|d| info_string(d, k));
    Ok(PdfText {
        body,
        page_count: pages.len(),
        pages: spans,
        title: get(b"Title"),
        author: get(b"Author"),
        subject: get(b"Subject"),
        keywords: get(b"Keywords"),
    })
}

/// Trim trailing spaces per line and collapse runs of blank lines.
fn normalise_page(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut blank = 0;
    for line in p.lines() {
        let l = line.trim_end();
        if l.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(l);
    }
    out
}

impl PdfConnector {
    fn cfg(config: &Value) -> Result<PdfConfig> {
        serde_json::from_value(config.clone())
            .map_err(|e| ConnectorError::InvalidConfig(vec![e.to_string()]))
    }

    fn options(cfg: &PdfConfig) -> ScanOptions {
        let p = expand_home(&cfg.path);
        let (root, only) = if p.is_file() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            (p.parent().map(Path::to_path_buf).unwrap_or_default(), name)
        } else {
            (p, None)
        };
        ScanOptions {
            root,
            extensions: vec!["pdf".into()],
            ignore: cfg.ignore.clone(),
            include_hidden: false,
            follow_symlinks: false,
            max_file_bytes: cfg.max_file_bytes.unwrap_or(256 * 1024 * 1024),
            only,
        }
    }
}

#[async_trait]
impl Connector for PdfConnector {
    fn manifest(&self) -> Manifest {
        pdf_manifest()
    }

    async fn check(&self, config: &Value) -> Result<CheckReport> {
        let cfg = Self::cfg(config)?;
        let p = expand_home(&cfg.path);
        Ok(if p.is_file() || p.is_dir() {
            CheckReport::ok(format!("found {}", p.display()))
        } else {
            CheckReport::failed(format!("nothing at {}", p.display()))
        })
    }

    async fn list(&self, config: &Value, cursor: Option<&str>) -> Result<Batch> {
        let cfg = Self::cfg(config)?;
        let opts = Self::options(&cfg);
        let prev = FileCursor::parse(cursor)?;
        tokio::task::spawn_blocking(move || {
            let mut scanned = scan::scan(&opts, &prev)?;
            let mut upserts = Vec::new();
            for f in &scanned.changed {
                let stem = Path::new(&f.rel)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let (body, mut meta, title) = match extract_pdf(&f.bytes) {
                    Ok(t) => {
                        let pages: Vec<Value> = t
                            .pages
                            .iter()
                            .map(|(n, s, e)| json!({"page": n, "start": s, "end": e}))
                            .collect();
                        let meta = json!({
                            "pages": pages,
                            "page_count": t.page_count,
                            "text_layer": !t.body.trim().is_empty(),
                            "author": t.author,
                            "subject": t.subject,
                            "keywords": t.keywords,
                        });
                        (t.body, meta, t.title.unwrap_or(stem))
                    }
                    Err(e) => {
                        // Keep the document so the user sees it; record why it is empty.
                        tracing::warn!(path = %f.rel, error = %e, "pdf text extraction failed");
                        (
                            String::new(),
                            json!({"extract_error": e.to_string(), "text_layer": false}),
                            stem,
                        )
                    }
                };
                meta["path"] = json!(f.rel);
                meta["content_hash"] = json!(f.hash);
                meta["size"] = json!(f.meta.size);
                meta["connector"] = json!("pdf");
                if let Some(r) = scanned.renames.iter().find(|r| r.to == f.rel) {
                    meta["renamed_from"] = json!(r.from);
                }
                upserts.push(Document {
                    external_id: f.rel.clone(),
                    uri: file_uri(&f.abs),
                    kind: cfg.kind.unwrap_or(ItemKind::Pdf),
                    title,
                    body,
                    origin: cfg.origin.unwrap_or(Origin::External),
                    created_at: f.meta.created.or(f.meta.modified),
                    modified_at: f.meta.modified,
                    tags: vec![],
                    refs: vec![],
                    meta,
                });
            }
            Ok(Batch {
                upserts,
                deletes: std::mem::take(&mut scanned.deleted),
                renames: std::mem::take(&mut scanned.renames),
                next_cursor: Some(scanned.next.encode()),
                has_more: false,
            })
        })
        .await
        .map_err(|e| ConnectorError::Unsupported(e.to_string()))?
    }
}
