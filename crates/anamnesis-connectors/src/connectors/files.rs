//! `folder` and `obsidian`: Markdown / plain-text files on disk.
//!
//! Both share the incremental scanner in [`crate::scan`] and the Markdown analysis in
//! [`crate::markdown`]. The obsidian flavour adds vault defaults (ignores `.obsidian/`,
//! `.trash/`), daily-note detection, and `obsidian://` uris in meta.
//!
//! `Document.refs` holds, deduplicated and in order of appearance:
//! - wikilinks and embeds, resolved to the target's external id (relative path) when the
//!   target exists in the source, otherwise the raw target text (`[[Missing note]]` →
//!   `Missing note`);
//! - relative markdown links, resolved the same way;
//! - absolute urls verbatim.
//!
//! The structured form (target, heading, alias, embed, resolved) is in `meta.wikilinks`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anamnesis_core::{Document, ItemKind, Origin, Privacy};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::connector::{Batch, ChangeNotice, CheckReport, Connector, WatchHandle};
use crate::error::{ConnectorError, Result};
use crate::manifest::{API_VERSION, Capability, Manifest, Runtime};
use crate::markdown;
use crate::scan::{self, ChangedFile, FileCursor, Resolver, ScanOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Folder,
    Obsidian,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilesConfig {
    path: String,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    follow_symlinks: bool,
    #[serde(default)]
    kind: Option<ItemKind>,
    #[serde(default)]
    origin: Option<Origin>,
    #[serde(default)]
    max_file_bytes: Option<u64>,
}

pub struct FilesConnector {
    flavor: Flavor,
}

impl FilesConnector {
    pub fn folder() -> Self {
        FilesConnector {
            flavor: Flavor::Folder,
        }
    }
    pub fn obsidian() -> Self {
        FilesConnector {
            flavor: Flavor::Obsidian,
        }
    }

    fn id(&self) -> &'static str {
        match self.flavor {
            Flavor::Folder => "folder",
            Flavor::Obsidian => "obsidian",
        }
    }

    fn parse_config(&self, config: &Value) -> Result<FilesConfig> {
        serde_json::from_value(config.clone())
            .map_err(|e| ConnectorError::InvalidConfig(vec![e.to_string()]))
    }

    fn scan_options(&self, c: &FilesConfig) -> ScanOptions {
        let default_ext: &[&str] = match self.flavor {
            Flavor::Folder => &["md", "markdown", "txt"],
            Flavor::Obsidian => &["md"],
        };
        let mut ignore = c.ignore.clone();
        if self.flavor == Flavor::Obsidian {
            // Explicit so they stay ignored even with include_hidden.
            ignore.extend([".obsidian".to_string(), ".trash".to_string()]);
        }
        ScanOptions {
            root: expand_home(&c.path),
            extensions: c
                .extensions
                .clone()
                .unwrap_or_else(|| default_ext.iter().map(|s| s.to_string()).collect())
                .into_iter()
                .map(|e| e.trim_start_matches('.').to_lowercase())
                .collect(),
            ignore,
            include_hidden: c.include_hidden,
            follow_symlinks: c.follow_symlinks,
            max_file_bytes: c.max_file_bytes.unwrap_or(16 * 1024 * 1024),
            only: None,
        }
    }
}

/// Expand a leading `~` (`~`, `~/…`, and `~\…` on windows) to the home directory:
/// `HOME`, or `USERPROFILE` on windows when `HOME` is unset.
pub fn expand_home(p: &str) -> PathBuf {
    let rest = if p == "~" {
        Some("")
    } else {
        p.strip_prefix("~/")
            .or_else(|| p.strip_prefix("~\\").filter(|_| cfg!(windows)))
    };
    if let Some(rest) = rest
        && let Some(home) = home_dir()
    {
        return if rest.is_empty() {
            home
        } else {
            home.join(rest)
        };
    }
    PathBuf::from(p)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|_| cfg!(windows)))
        .map(PathBuf::from)
}

pub(crate) fn file_uri(p: &Path) -> Option<String> {
    url::Url::from_file_path(p).ok().map(|u| u.to_string())
}

fn files_schema(obsidian: bool) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["path"],
        "additionalProperties": false,
        "properties": {
            "path": {"type": "string", "minLength": 1, "description": if obsidian { "vault folder" } else { "folder to read, recursively" }},
            "extensions": {"type": "array", "items": {"type": "string"}, "description": "file extensions to read"},
            "ignore": {"type": "array", "items": {"type": "string"}, "description": "gitignore-style globs"},
            "include_hidden": {"type": "boolean"},
            "follow_symlinks": {"type": "boolean"},
            "kind": {"enum": ["note", "journal", "quote", "article", "pdf", "book", "chat", "clip", "feed", "skill"]},
            "origin": {"enum": ["owner_written", "owner_spoken", "owner_annotation", "owner_in_chat", "ai_generated", "ai_edited", "external"]},
            "max_file_bytes": {"type": "integer", "minimum": 1}
        }
    })
}

pub fn folder_manifest() -> Manifest {
    Manifest {
        id: "folder".into(),
        name: "folder".into(),
        description: "markdown and plain-text files in a folder, read recursively".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION,
        runtime: Runtime::Builtin,
        entry: None,
        config_schema: files_schema(false),
        capabilities: vec![
            Capability::Check,
            Capability::Incremental,
            Capability::Deletes,
            Capability::Renames,
            Capability::Watch,
        ],
        network_hosts: vec![],
        fs_paths: vec!["<config.path>".into()],
        default_origin: Origin::OwnerWritten,
        default_privacy: Privacy::CloudOk,
        default_kind: Some(ItemKind::Note),
        poll_interval_secs: Some(60),
    }
}

pub fn obsidian_manifest() -> Manifest {
    Manifest {
        id: "obsidian".into(),
        name: "obsidian vault".into(),
        description: "an obsidian vault: frontmatter, aliases, tags, wikilinks and embeds".into(),
        config_schema: files_schema(true),
        ..folder_manifest()
    }
}

/// `Document.meta` key: the per-document privacy flag (`local_only` | `cloud_ok` | `ask`),
/// set when the document itself states one (e.g. frontmatter `privacy:`). The server
/// applies it to the item unless the user set the item's privacy by hand.
pub const META_PRIVACY: &str = "privacy";
/// `Document.meta` key: true when `Document.origin` was stated explicitly (frontmatter or
/// connector config) rather than taken from the connector's default, so the server does
/// not replace it with the source's `default_origin`.
pub const META_ORIGIN_EXPLICIT: &str = "origin_explicit";

/// A snake_case enum value from frontmatter (`origin: ai_generated`). Tolerates case and
/// hyphens (`Local-Only`); anything else is ignored.
fn frontmatter_enum<T: serde::de::DeserializeOwned>(fm: &Value, key: &str) -> Option<T> {
    let raw = fm.get(key)?.as_str()?;
    let norm = raw.trim().to_lowercase().replace(['-', ' '], "_");
    serde_json::from_value(Value::String(norm)).ok()
}

static DAILY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());

impl FilesConnector {
    fn to_document(
        &self,
        cfg: &FilesConfig,
        root: &Path,
        f: &ChangedFile,
        resolver: &Resolver,
        renamed_from: Option<&str>,
    ) -> Document {
        let text = String::from_utf8_lossy(&f.bytes).into_owned();
        let stem = Path::new(&f.rel)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_md = !f.rel.to_lowercase().ends_with(".txt");
        let origin = cfg.origin.unwrap_or(Origin::OwnerWritten);
        let mut meta = json!({
            "path": f.rel,
            "content_hash": f.hash,
            "size": f.meta.size,
            "connector": self.id(),
        });
        if let Some(from) = renamed_from {
            meta["renamed_from"] = json!(from);
        }
        if self.flavor == Flavor::Obsidian
            && let Some(vault) = root.file_name()
        {
            let file = f.rel.strip_suffix(".md").unwrap_or(&f.rel);
            let mut u = url::Url::parse("obsidian://open").unwrap();
            u.query_pairs_mut()
                .append_pair("vault", &vault.to_string_lossy())
                .append_pair("file", file);
            meta["obsidian_uri"] = json!(u.to_string());
        }
        let default_kind = match self.flavor {
            Flavor::Obsidian if DAILY_RE.is_match(&stem) => ItemKind::Journal,
            _ => ItemKind::Note,
        };
        let mut doc = Document {
            external_id: f.rel.clone(),
            uri: file_uri(&f.abs),
            kind: cfg.kind.unwrap_or(default_kind),
            title: stem.clone(),
            body: text.clone(),
            origin,
            created_at: f.meta.created.or(f.meta.modified),
            modified_at: f.meta.modified,
            tags: vec![],
            refs: vec![],
            meta: Value::Null,
        };
        if is_md {
            let p = markdown::parse(&text);
            if let Some(t) = &p.title {
                doc.title = t.clone();
            }
            doc.body = p.body.clone();
            doc.tags = p.tags.clone();
            if let Some(c) = p.created {
                doc.created_at = Some(c);
            }
            if let Some(m) = p.modified {
                doc.modified_at = Some(m);
            }
            let mut refs: Vec<String> = Vec::new();
            let mut wl_meta = Vec::new();
            for w in &p.wikilinks {
                let resolved = if w.target.is_empty() {
                    // [[#heading]] points into this note.
                    Some(f.rel.clone())
                } else {
                    resolver.resolve_wikilink(&w.target, &f.rel)
                };
                let r = resolved.clone().unwrap_or_else(|| w.target.clone());
                if r != f.rel && !refs.contains(&r) {
                    refs.push(r);
                }
                let mut m = serde_json::to_value(w).unwrap();
                m["resolved"] = json!(resolved);
                wl_meta.push(m);
            }
            for l in &p.links {
                let r = resolver
                    .resolve_relative(l, &f.rel)
                    .unwrap_or_else(|| l.clone());
                if r != f.rel && !refs.contains(&r) {
                    refs.push(r);
                }
            }
            doc.refs = refs;
            meta["frontmatter"] = p.frontmatter.clone();
            if let Some(e) = &p.frontmatter_error {
                meta["frontmatter_error"] = json!(e);
            }
            meta["aliases"] = json!(p.aliases);
            meta["wikilinks"] = Value::Array(wl_meta);
            // Frontmatter may override kind / origin / privacy per note.
            if cfg.kind.is_none()
                && let Some(k) = p
                    .frontmatter
                    .get("kind")
                    .cloned()
                    .and_then(|k| serde_json::from_value::<ItemKind>(k).ok())
            {
                doc.kind = k;
            }
            // A note's own `origin` is more specific than the source's (or config's)
            // default: an ai-written draft in a folder of owner notes stays ai-written.
            if let Some(o) = frontmatter_enum::<Origin>(&p.frontmatter, "origin") {
                doc.origin = o;
                meta[META_ORIGIN_EXPLICIT] = json!(true);
            }
            // `privacy: local_only | cloud_ok | ask` per note; the server applies it.
            if let Some(pv) = frontmatter_enum::<Privacy>(&p.frontmatter, "privacy") {
                meta[META_PRIVACY] = json!(pv);
            }
        }
        if cfg.origin.is_some() {
            meta[META_ORIGIN_EXPLICIT] = json!(true);
        }
        doc.meta = meta;
        doc
    }
}

#[async_trait]
impl Connector for FilesConnector {
    fn manifest(&self) -> Manifest {
        match self.flavor {
            Flavor::Folder => folder_manifest(),
            Flavor::Obsidian => obsidian_manifest(),
        }
    }

    async fn check(&self, config: &Value) -> Result<CheckReport> {
        let cfg = self.parse_config(config)?;
        let opts = self.scan_options(&cfg);
        let flavor = self.flavor;
        tokio::task::spawn_blocking(move || {
            if !opts.root.is_dir() {
                return Ok(CheckReport::failed(format!("no folder at {}", opts.root.display())));
            }
            if flavor == Flavor::Obsidian && !opts.root.join(".obsidian").is_dir() {
                // Still usable: a vault is just a folder of markdown.
                tracing::info!(path = %opts.root.display(), "no .obsidian folder; reading as a plain vault");
            }
            // Count without reading: an empty cursor + max_file_bytes 0 would skip; walk instead.
            let n = walkdir::WalkDir::new(&opts.root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter(|e| {
                    let ext = e.path().extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                    opts.extensions.contains(&ext)
                })
                .count();
            Ok(CheckReport::ok(format!("found {n} files")))
        })
        .await
        .map_err(|e| ConnectorError::Unsupported(e.to_string()))?
    }

    async fn list(&self, config: &Value, cursor: Option<&str>) -> Result<Batch> {
        let cfg = self.parse_config(config)?;
        let opts = self.scan_options(&cfg);
        let prev = FileCursor::parse(cursor)?;
        let scanned = {
            let opts = opts.clone();
            tokio::task::spawn_blocking(move || scan::scan(&opts, &prev))
                .await
                .map_err(|e| ConnectorError::Unsupported(e.to_string()))??
        };
        let resolver = Resolver::new(scanned.all_paths.iter().chain(scanned.other_paths.iter()));
        let upserts = scanned
            .changed
            .iter()
            .map(|f| {
                let from = scanned
                    .renames
                    .iter()
                    .find(|r| r.to == f.rel)
                    .map(|r| r.from.as_str());
                self.to_document(&cfg, &opts.root, f, &resolver, from)
            })
            .collect();
        Ok(Batch {
            upserts,
            deletes: scanned.deleted,
            renames: scanned.renames,
            next_cursor: Some(scanned.next.encode()),
            has_more: false,
        })
    }

    fn watch(&self, config: &Value) -> Result<Option<WatchHandle>> {
        use notify::{RecursiveMode, Watcher};
        let cfg = self.parse_config(config)?;
        let root = expand_home(&cfg.path);
        let (tx, rx) = mpsc::channel(256);
        let root2 = root.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if ev.kind.is_access() {
                return;
            }
            for p in ev.paths {
                // the same `/`-joined form as a scan's external ids, on every os
                let rel = crate::scan::rel_string(&root2, &p);
                if rel
                    .as_deref()
                    .is_some_and(|r| r.starts_with(".obsidian") || r.starts_with(".trash"))
                {
                    continue;
                }
                // Full channel: drop; the next list() catches up anyway.
                let _ = tx.try_send(ChangeNotice {
                    external_id: rel,
                    location: Some(p.display().to_string()),
                });
            }
        })
        .map_err(|e| ConnectorError::Unsupported(format!("watch: {e}")))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| ConnectorError::Unsupported(format!("watch {}: {e}", root.display())))?;
        Ok(Some(WatchHandle::new(rx, watcher)))
    }
}
