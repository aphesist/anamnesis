//! Incremental filesystem scanning shared by the folder, obsidian and pdf connectors.
//!
//! The cursor is json: `{ "v": 1, "files": { "<relative path>": { "mtime_ns", "size", "hash" } } }`.
//! A file is re-read only when its mtime or size changed, and reported only when its sha256
//! changed, so touching a file is free. Paths use `/` on every platform.
//!
//! **Renames.** A file that disappeared and a new file that appeared in the same sync with the
//! same content hash (and no other candidate with that hash) is reported as a rename. The
//! external id is the relative path, so a rename changes it; the connector reports it in
//! `Batch::renames` so the consumer can re-key the stored item and keep its identity. Edits
//! made together with the move, or ambiguous moves (several files with identical content),
//! fall back to delete + create.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anamnesis_core::Timestamp;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::connector::Rename;
use crate::error::{ConnectorError, Result};

pub const CURSOR_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    pub mtime_ns: i64,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FileCursor {
    pub v: u32,
    pub files: BTreeMap<String, FileState>,
}

impl FileCursor {
    pub fn parse(s: Option<&str>) -> Result<Self> {
        match s {
            None => Ok(FileCursor {
                v: CURSOR_VERSION,
                files: BTreeMap::new(),
            }),
            Some(s) => {
                let c: FileCursor = serde_json::from_str(s)
                    .map_err(|e| ConnectorError::BadCursor(e.to_string()))?;
                if c.v != CURSOR_VERSION {
                    return Err(ConnectorError::BadCursor(format!(
                        "cursor version {} unknown",
                        c.v
                    )));
                }
                Ok(c)
            }
        }
    }
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("cursor serialises")
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub root: PathBuf,
    /// Lowercase extensions without dot. Files with other extensions are only listed in
    /// `ScanResult::other_paths` (for link resolution), never read.
    pub extensions: Vec<String>,
    /// gitignore-flavoured globs: a pattern without `/` matches at any depth; every pattern
    /// also matches everything below a matching directory.
    pub ignore: Vec<String>,
    pub include_hidden: bool,
    pub follow_symlinks: bool,
    /// Larger files are skipped (logged).
    pub max_file_bytes: u64,
    /// Only consider this one relative path (single-file sources).
    pub only: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FileMeta {
    pub size: u64,
    pub modified: Option<Timestamp>,
    pub created: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub rel: String,
    pub abs: PathBuf,
    pub bytes: Vec<u8>,
    pub hash: String,
    pub meta: FileMeta,
    /// Not present in the previous cursor.
    pub is_new: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub changed: Vec<ChangedFile>,
    pub deleted: Vec<String>,
    pub renames: Vec<Rename>,
    /// Every matching file currently present (changed or not).
    pub all_paths: Vec<String>,
    /// Every non-matching, non-ignored file (attachments), for link resolution.
    pub other_paths: Vec<String>,
    pub next: FileCursor,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn build_ignore(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let p = p.trim().trim_start_matches("./");
        if p.is_empty() || p.starts_with('#') {
            continue;
        }
        let p = p.trim_end_matches('/');
        let base = if p.contains('/') {
            p.trim_start_matches('/').to_string()
        } else {
            format!("**/{p}")
        };
        for pat in [base.clone(), format!("{base}/**")] {
            let g = GlobBuilder::new(&pat)
                .literal_separator(true)
                .build()
                .map_err(|e| {
                    ConnectorError::InvalidConfig(vec![format!("ignore glob {p:?}: {e}")])
                })?;
            b.add(g);
        }
    }
    b.build()
        .map_err(|e| ConnectorError::InvalidConfig(vec![e.to_string()]))
}

fn to_ts(t: std::io::Result<std::time::SystemTime>) -> Option<Timestamp> {
    t.ok().map(Timestamp::from)
}

fn mtime_ns(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// `p` relative to `root`, joined with `/` on every os (the form used for external ids).
pub(crate) fn rel_string(root: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Blocking: walks the tree and reads changed files. Run it on a blocking thread.
pub fn scan(opts: &ScanOptions, prev: &FileCursor) -> Result<ScanResult> {
    let root = &opts.root;
    let md = std::fs::metadata(root).map_err(|e| ConnectorError::io(root, e))?;
    if !md.is_dir() {
        return Err(ConnectorError::InvalidConfig(vec![format!(
            "{} is not a directory",
            root.display()
        )]));
    }
    let ignore = build_ignore(&opts.ignore)?;
    let mut out = ScanResult {
        next: FileCursor {
            v: CURSOR_VERSION,
            files: BTreeMap::new(),
        },
        ..Default::default()
    };

    let max_depth = if opts.only.is_some() { 1 } else { usize::MAX };
    let walker = walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(opts.follow_symlinks)
        .sort_by_file_name();
    let iter = walker.into_iter().filter_entry(|e| {
        if e.depth() == 0 {
            return true;
        }
        let name = e.file_name().to_string_lossy();
        if !opts.include_hidden && name.starts_with('.') {
            return false;
        }
        match rel_string(root, e.path()) {
            Some(rel) => !ignore.is_match(&rel),
            None => true,
        }
    });
    for entry in iter {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "skipping unreadable entry");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(rel) = rel_string(root, entry.path()) else {
            continue;
        };
        if opts.only.as_ref().is_some_and(|o| o != &rel) {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !opts.extensions.iter().any(|x| x == &ext) {
            out.other_paths.push(rel);
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(path = %rel, error = %e, "skipping file without metadata");
                continue;
            }
        };
        if meta.len() > opts.max_file_bytes {
            tracing::warn!(path = %rel, size = meta.len(), "skipping file larger than max_file_bytes");
            continue;
        }
        let state_now = (mtime_ns(&meta), meta.len());
        let old = prev.files.get(&rel);
        if let Some(o) = old
            && (o.mtime_ns, o.size) == state_now
        {
            out.next.files.insert(rel.clone(), o.clone());
            out.all_paths.push(rel);
            continue;
        }
        let bytes = match std::fs::read(entry.path()) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(path = %rel, error = %e, "skipping unreadable file");
                // Keep the old state so a transient error does not look like a deletion.
                if let Some(o) = old {
                    out.next.files.insert(rel.clone(), o.clone());
                    out.all_paths.push(rel);
                }
                continue;
            }
        };
        let hash = sha256_hex(&bytes);
        out.next.files.insert(
            rel.clone(),
            FileState {
                mtime_ns: state_now.0,
                size: state_now.1,
                hash: hash.clone(),
            },
        );
        out.all_paths.push(rel.clone());
        if old.is_some_and(|o| o.hash == hash) {
            continue;
        }
        out.changed.push(ChangedFile {
            rel,
            abs: entry.path().to_path_buf(),
            bytes,
            hash,
            meta: FileMeta {
                size: meta.len(),
                modified: to_ts(meta.modified()),
                created: to_ts(meta.created()),
            },
            is_new: old.is_none(),
        });
    }

    let mut deleted: Vec<String> = prev
        .files
        .keys()
        .filter(|k| !out.next.files.contains_key(*k))
        .cloned()
        .collect();

    // Rename detection: unique hash match between a vanished file and a new file.
    let mut new_by_hash: HashMap<&str, Vec<&str>> = HashMap::new();
    for c in out.changed.iter().filter(|c| c.is_new) {
        new_by_hash
            .entry(c.hash.as_str())
            .or_default()
            .push(c.rel.as_str());
    }
    let mut gone_by_hash: HashMap<&str, Vec<&str>> = HashMap::new();
    for d in &deleted {
        gone_by_hash
            .entry(prev.files[d].hash.as_str())
            .or_default()
            .push(d.as_str());
    }
    let mut renames = Vec::new();
    for (hash, gone) in &gone_by_hash {
        if let (1, Some(new)) = (gone.len(), new_by_hash.get(hash))
            && new.len() == 1
        {
            renames.push(Rename {
                from: gone[0].to_string(),
                to: new[0].to_string(),
            });
        }
    }
    renames.sort_by(|a, b| a.from.cmp(&b.from));
    deleted.retain(|d| !renames.iter().any(|r| &r.from == d));
    out.deleted = deleted;
    out.renames = renames;
    Ok(out)
}

/// Resolve note names, wikilink targets and relative markdown links to relative paths.
#[derive(Debug, Default, Clone)]
pub struct Resolver {
    /// lowercase full path → path
    by_path: HashMap<String, String>,
    /// lowercase path without `.md` → path
    by_path_noext: HashMap<String, String>,
    /// lowercase file stem (for .md) or file name (others) → paths
    by_name: HashMap<String, Vec<String>>,
}

fn stem_of(p: &str) -> &str {
    let name = p.rsplit('/').next().unwrap_or(p);
    name.strip_suffix(".md")
        .or_else(|| name.strip_suffix(".markdown"))
        .unwrap_or(name)
}

impl Resolver {
    pub fn new<'a>(paths: impl IntoIterator<Item = &'a String>) -> Self {
        let mut r = Resolver::default();
        for p in paths {
            let low = p.to_lowercase();
            r.by_path.insert(low.clone(), p.clone());
            let is_note = low.ends_with(".md") || low.ends_with(".markdown");
            if is_note {
                r.by_path_noext
                    .insert(stem_path(&low).to_string(), p.clone());
            }
            let name = if is_note {
                stem_of(&low).to_string()
            } else {
                low.rsplit('/').next().unwrap_or(&low).to_string()
            };
            r.by_name.entry(name).or_default().push(p.clone());
        }
        for v in r.by_name.values_mut() {
            v.sort_by(|a, b| {
                a.matches('/')
                    .count()
                    .cmp(&b.matches('/').count())
                    .then(a.cmp(b))
            });
        }
        r
    }

    /// Obsidian-style: exact path, path without extension, then by name (preferring a note in
    /// the same folder as `from`, then the shallowest). A root-level note wins an exact match.
    pub fn resolve_wikilink(&self, target: &str, from: &str) -> Option<String> {
        let t = target.trim().trim_start_matches('/').to_lowercase();
        if t.is_empty() {
            return None;
        }
        if let Some(p) = self.by_path.get(&t).or_else(|| self.by_path_noext.get(&t)) {
            return Some(p.clone());
        }
        if t.contains('/') {
            // Partial path: match a suffix.
            let suffix_md = format!("/{t}.md");
            let suffix = format!("/{t}");
            return self
                .by_path
                .iter()
                .filter(|(k, _)| k.ends_with(&suffix_md) || k.ends_with(&suffix))
                .map(|(_, v)| v.clone())
                .min_by_key(|v| v.len());
        }
        let cands = self.by_name.get(&t)?;
        let dir = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        cands
            .iter()
            .find(|c| c.rsplit_once('/').map(|(d, _)| d).unwrap_or("") == dir)
            .or_else(|| cands.first())
            .cloned()
    }

    /// A relative markdown link (`../a/b.md`, percent-encoded) from note `from`.
    pub fn resolve_relative(&self, dest: &str, from: &str) -> Option<String> {
        if dest.contains("://") || dest.starts_with("mailto:") {
            return None;
        }
        let dest = dest.split(['#', '?']).next().unwrap_or("");
        let dest = percent_decode(dest);
        let mut parts: Vec<&str> = if dest.starts_with('/') {
            Vec::new()
        } else {
            from.rsplit_once('/')
                .map(|(d, _)| d.split('/').collect())
                .unwrap_or_default()
        };
        for seg in dest.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                s => parts.push(s),
            }
        }
        let joined = parts.join("/").to_lowercase();
        self.by_path
            .get(&joined)
            .or_else(|| self.by_path_noext.get(&joined))
            .cloned()
            .or_else(|| {
                self.resolve_wikilink(&dest, from)
                    .filter(|_| !dest.contains('/'))
            })
    }
}

fn stem_path(p: &str) -> &str {
    p.strip_suffix(".md")
        .or_else(|| p.strip_suffix(".markdown"))
        .unwrap_or(p)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_globs() {
        let g = build_ignore(&["drafts/".into(), "*.tmp".into(), "a/b/*.md".into()]).unwrap();
        assert!(g.is_match("drafts"));
        assert!(g.is_match("drafts/x.md"));
        assert!(g.is_match("deep/x.tmp"));
        assert!(g.is_match("a/b/c.md"));
        assert!(!g.is_match("a/c.md"));
        assert!(!g.is_match("notdrafts/x.md"));
    }

    #[test]
    fn resolver() {
        let paths: Vec<String> = [
            "Meno.md",
            "folder/Other Note.md",
            "x/Meno.md",
            "img/diagram.png",
            "a/b.md",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let r = Resolver::new(&paths);
        assert_eq!(
            r.resolve_wikilink("meno", "z.md").as_deref(),
            Some("Meno.md")
        );
        assert_eq!(
            r.resolve_wikilink("Meno", "x/y.md").as_deref(),
            Some("Meno.md")
        );
        assert_eq!(
            r.resolve_wikilink("x/Meno", "z.md").as_deref(),
            Some("x/Meno.md")
        );
        assert_eq!(
            r.resolve_wikilink("folder/Other Note", "z.md").as_deref(),
            Some("folder/Other Note.md")
        );
        assert_eq!(
            r.resolve_wikilink("diagram.png", "z.md").as_deref(),
            Some("img/diagram.png")
        );
        assert_eq!(r.resolve_wikilink("nope", "z.md"), None);
        assert_eq!(
            r.resolve_relative("../folder/Other%20Note.md", "a/b.md")
                .as_deref(),
            Some("folder/Other Note.md")
        );
        assert_eq!(
            r.resolve_relative("b.md#h", "a/c.md").as_deref(),
            Some("a/b.md")
        );
        assert_eq!(r.resolve_relative("https://x.org/a.md", "a/c.md"), None);
    }
}
