//! Importing folders and bundles, exporting bundles, and the demo brain.
//!
//! The demo brain is seeded from a prebuilt snapshot (`demo/transcendentalist/snapshot/`,
//! embedded in release builds) so it opens at once, with its embeddings, regions, layout and
//! suggestions, and without a model download. It is rebuilt from the demo notes only when an
//! explicit demo bundle is given or no snapshot ships.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anamnesis_core::{Id, Origin, Privacy, Source, now};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::brain::Brain;
use crate::sync::{SyncReport, sync_source};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportReport {
    Bundle {
        events: u64,
    },
    Folder {
        source_id: Id,
        connector: String,
        sync: SyncReport,
    },
}

/// Is this path an anamnesis bundle (json lines whose first line is a bundle manifest)?
fn is_bundle_file(p: &Path) -> bool {
    use std::io::BufRead;
    let Ok(f) = std::fs::File::open(p) else {
        return false;
    };
    let mut first = String::new();
    if BufReader::new(f).read_line(&mut first).is_err() {
        return false;
    }
    serde_json::from_str::<anamnesis_core::Manifest>(&first)
        .is_ok_and(|m| m.format == anamnesis_core::BUNDLE_FORMAT)
}

/// A bundle file inside a folder (`bundle.jsonl`, `brain.jsonl`, or the only `*.jsonl`).
fn bundle_in(dir: &Path) -> Option<PathBuf> {
    for n in ["bundle.jsonl", "brain.jsonl", "anamnesis.jsonl"] {
        let p = dir.join(n);
        if p.is_file() && is_bundle_file(&p) {
            return Some(p);
        }
    }
    let jsonl: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl") && is_bundle_file(p))
        .collect();
    (jsonl.len() == 1).then(|| jsonl[0].clone())
}

/// Import a bundle file into an empty brain.
pub async fn import_bundle(brain: &Arc<Brain>, path: &Path) -> Result<u64> {
    let p = path.to_path_buf();
    brain
        .db(move |s| {
            let f = std::fs::File::open(&p).with_context(|| format!("opening {}", p.display()))?;
            Ok(s.import_bundle(BufReader::new(f))?.events)
        })
        .await
}

/// Add a folder (or obsidian vault, if it has `.obsidian/`) as a source and sync it once.
/// An already registered folder is synced again instead of added twice.
pub async fn import_folder(brain: &Arc<Brain>, dir: &Path) -> Result<ImportReport> {
    let dir = std::fs::canonicalize(dir).with_context(|| format!("{}", dir.display()))?;
    let connector = if dir.join(".obsidian").is_dir() {
        "obsidian"
    } else {
        "folder"
    };
    let path = dir.display().to_string();
    // `lab/out/<theme>/notes` is named after its theme
    let named = if dir.file_name().is_some_and(|n| n == "notes") {
        dir.parent().unwrap_or(&dir)
    } else {
        &dir
    };
    let name = named
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let c = connector.to_string();
    let source = brain
        .db(move |s| {
            if let Some(src) = s.list_sources()?.into_iter().find(|x| {
                x.connector == c
                    && x.config.get("path").and_then(|p| p.as_str()) == Some(path.as_str())
            }) {
                return Ok(src);
            }
            let privacy = crate::privacy::default_setting(s).unwrap_or(Privacy::CloudOk);
            Ok(s.add_source(Source {
                id: Id::new(),
                connector: c,
                name,
                config: json!({ "path": path }),
                default_origin: Origin::OwnerWritten,
                default_privacy: privacy,
                cursor: None,
                created_at: now(),
            })?)
        })
        .await?;
    // folder / obsidian sources are built in: no plugins needed
    let sync = sync_source(brain, &crate::plugins::Plugins::default(), &source.id).await?;
    Ok(ImportReport::Folder {
        source_id: source.id,
        connector: connector.into(),
        sync,
    })
}

/// `anamnesis import <folder|bundle>`.
pub async fn import_path(brain: &Arc<Brain>, path: &Path) -> Result<ImportReport> {
    if path.is_file() {
        if !is_bundle_file(path) {
            bail!("{} is not an anamnesis bundle", path.display());
        }
        return Ok(ImportReport::Bundle {
            events: import_bundle(brain, path).await?,
        });
    }
    if path.is_dir() {
        if let Some(b) = bundle_in(path) {
            return Ok(ImportReport::Bundle {
                events: import_bundle(brain, &b).await?,
            });
        }
        return import_folder(brain, path).await;
    }
    bail!("{} does not exist", path.display())
}

/// Export the active brain to a bundle file.
pub async fn export_to(brain: &Arc<Brain>, path: &Path) -> Result<anamnesis_core::Manifest> {
    let p = path.to_path_buf();
    brain
        .db(move |s| {
            let f =
                std::fs::File::create(&p).with_context(|| format!("creating {}", p.display()))?;
            Ok(s.export_bundle(std::io::BufWriter::new(f))?)
        })
        .await
}

/// The theme the demo prefers among the lab's outputs.
pub const DEMO_THEME: &str = "transcendentalist";

/// The curated demo shipped in the repository (and embedded in release builds), relative
/// to the repository root. A full lab build is preferred when there is one.
pub const CURATED_DEMO: &str = "demo/transcendentalist";

/// The curated demo, embedded at build time (feature `embed-demo`). Debug builds read it
/// from the repository instead.
#[cfg(feature = "embed-demo")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../demo/transcendentalist/"]
#[include = "notes/*"]
#[include = "COLOPHON.md"]
#[include = "SOURCES.md"]
#[include = "manifest.json"]
struct EmbeddedDemo;

/// Paths of the embedded demo's files (empty without the `embed-demo` feature).
fn embedded_demo_files() -> Vec<String> {
    #[cfg(feature = "embed-demo")]
    {
        let mut v: Vec<String> = EmbeddedDemo::iter().map(|p| p.into_owned()).collect();
        v.sort();
        v
    }
    #[cfg(not(feature = "embed-demo"))]
    {
        Vec::new()
    }
}

/// Is a copy of the curated demo embedded in this binary?
pub fn has_embedded_demo() -> bool {
    embedded_demo_files()
        .iter()
        .any(|p| p.starts_with("notes/"))
}

/// Write the embedded demo into `dest` (`dest/notes/…`, `COLOPHON.md`, `SOURCES.md`), only
/// rewriting files that differ and removing notes the embedded copy no longer has. Returns
/// the `notes/` folder, or `None` when nothing is embedded.
pub fn extract_embedded_demo(dest: &Path) -> Result<Option<PathBuf>> {
    let files = embedded_demo_files();
    if !files.iter().any(|p| p.starts_with("notes/")) {
        return Ok(None);
    }
    #[cfg(feature = "embed-demo")]
    {
        let mut keep = std::collections::HashSet::new();
        for rel in &files {
            let Some(f) = EmbeddedDemo::get(rel) else {
                continue;
            };
            let path = dest.join(rel);
            keep.insert(path.clone());
            if std::fs::read(&path).is_ok_and(|b| b == f.data.as_ref()) {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, f.data.as_ref())
                .with_context(|| format!("writing {}", path.display()))?;
        }
        remove_stale(&dest.join("notes"), &keep)?;
    }
    Ok(Some(dest.join("notes")))
}

#[cfg(feature = "embed-demo")]
fn remove_stale(dir: &Path, keep: &std::collections::HashSet<PathBuf>) -> Result<()> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            remove_stale(&p, keep)?;
        } else if p.extension().is_some_and(|x| x == "md") && !keep.contains(&p) {
            std::fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
        }
    }
    Ok(())
}

/// Where the demo notes are on disk: the configured path (`--demo-bundle`),
/// `ANAMNESIS_DEMO_BUNDLE`, `demo/` next to the binary, then the repository (a full lab
/// build in `lab/out/…` first, else the curated `demo/transcendentalist`).
///
/// A bundle folder (with `notes/`, `manifest.json`, `SOURCES.md`, `COLOPHON.md`) resolves to
/// its `notes/` folder, so the bundle's metadata files never become items. The embedded copy
/// is not considered here: see [`find_demo`].
pub fn locate_demo(cfg_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = cfg_path {
        return p.exists().then(|| demo_root(p));
    }
    if let Ok(p) = std::env::var("ANAMNESIS_DEMO_BUNDLE")
        && !p.is_empty()
    {
        let p = PathBuf::from(p);
        return p.exists().then(|| demo_root(&p));
    }
    let mut repos = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let d = dir.join("demo");
        if d.exists() {
            return Some(demo_root(&d));
        }
        // target/<profile>/anamnesis → the repository
        if let Some(repo) = dir.parent().and_then(|p| p.parent()) {
            repos.push(repo.to_path_buf());
        }
    }
    repos.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    repos.iter().find_map(|r| demo_in_repo(r))
}

/// [`locate_demo`], falling back to the copy embedded in the binary, which is extracted to
/// `<demo brain dir>/transcendentalist/` on first use.
pub fn find_demo(app_cfg: &crate::app::ServerConfig) -> Result<Option<PathBuf>> {
    if let Some(p) = locate_demo(app_cfg.demo_bundle.as_deref()) {
        return Ok(Some(p));
    }
    extract_embedded_demo(&app_cfg.demo_brain_dir().join(DEMO_THEME))
}

/// The demo inside a repository checkout: a full lab build if there is one, else the
/// curated demo in `demo/transcendentalist`.
pub fn demo_in_repo(repo: &Path) -> Option<PathBuf> {
    lab_build_in(repo).or_else(|| {
        let curated = repo.join(CURATED_DEMO).join("notes");
        curated.is_dir().then_some(curated)
    })
}

/// The lab's output in a repository checkout, the transcendentalist theme first.
fn lab_build_in(repo: &Path) -> Option<PathBuf> {
    let lab = repo.join("lab");
    for c in [lab.join("demo-brain"), lab.join("out/demo-brain")] {
        if c.exists() {
            return Some(demo_root(&c));
        }
    }
    let out = lab.join("out");
    let preferred = out.join(DEMO_THEME).join("notes");
    if preferred.is_dir() {
        return Some(preferred);
    }
    let mut themes: Vec<PathBuf> = std::fs::read_dir(&out)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path().join("notes")))
        .filter(|p| p.is_dir())
        .collect();
    themes.sort();
    themes.into_iter().next()
}

/// A bundle folder (no `.jsonl` bundle, a `notes/` folder inside) → its `notes/`; a folder
/// holding `transcendentalist/notes` (a release's `demo/`) → that.
fn demo_root(p: &Path) -> PathBuf {
    if p.is_dir() && bundle_in(p).is_none() {
        if p.join("notes").is_dir() {
            return p.join("notes");
        }
        let themed = p.join(DEMO_THEME).join("notes");
        if themed.is_dir() {
            return themed;
        }
    }
    p.to_path_buf()
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoStatus {
    pub active: bool,
    pub available: bool,
    pub bundle: Option<String>,
    /// A copy of the curated demo is embedded in this binary.
    pub embedded: bool,
    /// A prebuilt demo brain (snapshot) ships with this binary: the demo opens at once.
    pub snapshot: bool,
    /// The person's own brain holds no items yet.
    pub user_empty: bool,
    /// A brand-new install: the person's brain is empty and they have not yet chosen
    /// between the demo and their own notes. The ui then opens the demo brain with a banner.
    pub first_launch: bool,
    pub items: u64,
    /// The demo brain has a layout (the background jobs ran at least once).
    pub ready: bool,
    /// The background jobs (`running`, `phase`, `done` / `total` phases).
    pub jobs: crate::app::JobsProgress,
    /// Chunks of the active brain, and how many have an embedding for the current model.
    pub chunks: u64,
    pub chunks_embedded: u64,
}

/// (chunks, chunks with an embedding for `model`) of a brain.
pub async fn embed_progress(brain: &Arc<Brain>, model: Option<String>) -> Result<(u64, u64)> {
    brain
        .db(move |s| {
            let c = s.connection();
            let chunks: i64 =
                c.query_row("SELECT COUNT(*) FROM chunks WHERE text != ''", [], |r| {
                    r.get(0)
                })?;
            let done: i64 = match &model {
                Some(m) => c.query_row(
                    "SELECT COUNT(*) FROM chunks c WHERE c.text != '' AND EXISTS
                     (SELECT 1 FROM embeddings e WHERE e.text_hash = c.text_hash AND e.model_id = ?1)",
                    [m],
                    |r| r.get(0),
                )?,
                // no embedder loaded yet: count chunks with a vector of any model
                None => c.query_row(
                    "SELECT COUNT(*) FROM chunks c WHERE c.text != '' AND EXISTS
                     (SELECT 1 FROM embeddings e WHERE e.text_hash = c.text_hash)",
                    [],
                    |r| r.get(0),
                )?,
            };
            Ok((chunks as u64, done as u64))
        })
        .await
}

pub async fn demo_status(app: &Arc<App>) -> Result<DemoStatus> {
    let bundle = locate_demo(app.cfg.demo_bundle.as_deref());
    let brain = app.brain();
    let items = brain.db(|s| Ok(s.count_items()?)).await?;
    let model = app.embedder().map(|e| e.id().to_string());
    let (chunks, chunks_embedded) = embed_progress(&brain, model).await?;
    let embedded = has_embedded_demo();
    let snapshot = snapshot_manifest().is_some();
    let user_empty = if brain.is_demo {
        let dir = app.cfg.user_brain_dir();
        tokio::task::spawn_blocking(move || user_brain_is_empty(&dir)).await??
    } else {
        items == 0
    };
    Ok(DemoStatus {
        active: brain.is_demo,
        available: bundle.is_some()
            || embedded
            || snapshot
            || app.cfg.demo_brain_dir().join("brain.sqlite").is_file(),
        bundle: bundle.map(|b| b.display().to_string()),
        embedded,
        snapshot,
        user_empty,
        first_launch: user_empty && !first_launch_done(&app.cfg),
        items,
        ready: brain.views().is_some(),
        jobs: app.jobs_progress.read().unwrap().clone(),
        chunks,
        chunks_embedded,
    })
}

/// Import the demo notes into the (empty) demo brain. Returns where they came from.
async fn import_demo(app: &Arc<App>, brain: &Arc<Brain>) -> Result<(PathBuf, ImportReport)> {
    let Some(p) = find_demo(&app.cfg)? else {
        bail!(
            "no demo found (build one with the lab, or set ANAMNESIS_DEMO_BUNDLE or --demo-bundle)"
        );
    };
    let r = import_path(brain, &p).await?;
    tracing::info!("demo brain imported from {}: {r:?}", p.display());
    Ok((p, r))
}

/// Switch to (or away from) the demo brain. On first use the demo brain is seeded from the
/// prebuilt snapshot (instant, no model needed), else imported from the demo notes and
/// arranged in the background. Leaving the demo records that the first-launch choice was made.
pub async fn set_demo(app: &Arc<App>, on: bool) -> Result<DemoStatus> {
    if on && !app.is_demo() && !explicit_demo_bundle(&app.cfg) {
        let cfg = app.cfg.clone();
        let seeded = tokio::task::spawn_blocking(move || seed_demo_if_empty(&cfg)).await?;
        match seeded {
            Ok(Some(r)) => tracing::info!(
                "demo brain seeded from the snapshot: {} items, {} events, chain verified ({} ms)",
                r.items,
                r.events,
                r.millis
            ),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("could not seed the demo brain, building it from the notes: {e:#}")
            }
        }
    }
    if !on {
        mark_first_launch_done(&app.cfg);
    }
    let brain = app.set_demo(on)?;
    if on {
        let empty = brain.db(|s| Ok(s.event_count()? == 0)).await?;
        if empty && let Err(e) = import_demo(app, &brain).await {
            app.set_demo(false)?;
            return Err(e);
        }
        // a layout and regions right away: run the jobs once in the background when the
        // demo brain has none yet (minutes for a full lab build; progress in /api/demo
        // and /api/status)
        if brain.views().is_none() && !app.jobs_progress.read().unwrap().running {
            app.jobs_progress.write().unwrap().running = true;
            let app2 = app.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::jobs::run_jobs_once(&app2).await {
                    tracing::warn!("demo jobs: {e:#}");
                }
            });
        }
    }
    crate::backends::detect_in_background(app);
    demo_status(app).await
}

/// Remove the demo brain (`<data dir>/anamnesis/demo`) so the next use builds it afresh.
/// Refuses anything that does not look like a demo brain directory.
pub fn remove_demo_brain(cfg: &crate::app::ServerConfig) -> Result<bool> {
    let dir = cfg.demo_brain_dir();
    if !dir.exists() {
        return Ok(false);
    }
    let ours = dir.file_name().is_some_and(|n| n == "demo")
        && std::fs::read_dir(&dir)?.flatten().all(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("brain.sqlite")
                || n.ends_with(".json")
                || n == SEEDING_DIR
                || n == "mcp_token"
                || n == DEMO_THEME
        });
    if !ours {
        bail!(
            "{} holds files that are not part of a demo brain; remove it by hand",
            dir.display()
        );
    }
    std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    Ok(true)
}

/// `anamnesis demo`: build the demo brain (import, embeddings, regions and layout,
/// suggestions) with progress on stderr. The app must have the demo brain active.
pub async fn build_demo(app: &Arc<App>) -> Result<()> {
    let t0 = std::time::Instant::now();
    let brain = app.brain();
    anyhow::ensure!(brain.is_demo, "the demo brain is not active");
    eprintln!("demo brain: {}", brain.dir.display());
    let empty = brain.db(|s| Ok(s.event_count()? == 0)).await?;
    if empty {
        eprintln!("importing the demo notes…");
        let (p, r) = import_demo(app, &brain).await?;
        let created = match &r {
            ImportReport::Folder { sync, .. } => sync.created as u64,
            ImportReport::Bundle { events } => *events,
        };
        eprintln!(
            "  {created} notes from {} ({:.1}s)",
            p.display(),
            t0.elapsed().as_secs_f32()
        );
    } else {
        let items = brain.db(|s| Ok(s.count_items()?)).await?;
        eprintln!("  already imported: {items} items (use --rebuild to start afresh)");
    }

    eprintln!("loading the embedder…");
    app.load_embedder().await;
    let st = app.embed_status();
    eprintln!(
        "  {} ({}){}",
        st.id.as_deref().unwrap_or("none"),
        st.status,
        if st.fallback { ", fallback" } else { "" }
    );

    let t1 = std::time::Instant::now();
    let jobs = {
        let app = app.clone();
        tokio::spawn(async move { crate::jobs::run_jobs_once(&app).await })
    };
    let model = app.embedder().map(|e| e.id().to_string());
    let mut last = String::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
    while !jobs.is_finished() {
        tick.tick().await;
        let p = app.jobs_progress.read().unwrap().clone();
        let Some(phase) = p.phase.filter(|_| p.running) else {
            continue;
        };
        let line = if phase == "embeddings" {
            let (c, d) = embed_progress(&brain, model.clone())
                .await
                .unwrap_or((0, 0));
            format!("  {phase}: {d} / {c} chunks")
        } else {
            format!("  {phase} ({} / {})", p.done + 1, p.total.max(1))
        };
        if line != last {
            eprintln!("{line}  [{:.0}s]", t1.elapsed().as_secs_f32());
            last = line;
        }
    }
    let r = jobs.await??;
    let regions = brain.db(|s| Ok(s.current_regions()?)).await?;
    let items = brain.db(|s| Ok(s.count_items()?)).await?;
    eprintln!(
        "done in {:.0}s: {items} items, {} chunks embedded, {} regions, {} suggestions",
        t0.elapsed().as_secs_f32(),
        r.embedded,
        regions.len(),
        r.suggestions
    );
    let mut labels: Vec<_> = regions.iter().map(|r| (r.size, r.label.clone())).collect();
    labels.sort_by(|a, b| b.cmp(a));
    for (size, label) in labels.iter().take(12) {
        eprintln!("  {size:>5}  {label}");
    }
    println!(
        "{}",
        serde_json::to_string(
            &json!({ "brain": brain.dir, "items": items, "regions": regions.len(), "jobs": r })
        )?
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------
// the prebuilt demo brain

/// Where the prebuilt demo brain lives, relative to the repository root.
pub const DEMO_SNAPSHOT: &str = "demo/transcendentalist/snapshot";

/// The files of a brain directory a snapshot carries, each gzip-compressed as `<name>.gz`.
/// `brain.sqlite` is required; the job caches are optional (the jobs recompute them).
pub const SNAPSHOT_FILES: [&str; 4] =
    ["brain.sqlite", "mind_state.json", "views.json", "jobs.json"];

/// The snapshot format this binary reads.
pub const SNAPSHOT_FORMAT: u32 = 1;

/// Scratch folder inside the demo brain directory while a snapshot is unpacked.
const SEEDING_DIR: &str = ".seeding";

/// Marker in the data root: the person made the first-launch choice (left the demo once).
const FIRST_LAUNCH_MARKER: &str = "first_launch_done";

/// `manifest.json` of a snapshot, written by `lab/scripts/demo_snapshot.py`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub format: u32,
    /// Events in the log, and the hash of the last one: seeding checks the chain ends there.
    pub events: u64,
    pub head: String,
    pub items: u64,
    #[serde(default)]
    pub chunks: u64,
    /// The embedding model the vectors, regions and layout were computed with.
    pub embedding_model: String,
    /// file name → checksum and sizes of the uncompressed file.
    pub files: BTreeMap<String, SnapshotFile>,
    #[serde(default)]
    pub anamnesis_version: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub sha256: String,
    pub bytes: u64,
    #[serde(default)]
    pub gz_bytes: u64,
}

/// What seeding did.
#[derive(Debug, Clone, Serialize)]
pub struct SeedReport {
    pub events: u64,
    pub items: u64,
    pub head: String,
    pub embedding_model: String,
    pub millis: u64,
}

/// The snapshot, embedded at build time (feature `embed-demo`; debug builds read the files
/// from the repository at run time).
#[cfg(feature = "embed-demo")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../demo/transcendentalist/snapshot/"]
#[include = "*.gz"]
#[include = "manifest.json"]
struct EmbeddedSnapshot;

/// Folders a snapshot is looked for on disk: `demo/transcendentalist/snapshot` next to the
/// binary, then in the repository this binary was built from.
fn snapshot_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        v.push(dir.join("demo").join(DEMO_THEME).join("snapshot"));
    }
    v.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(DEMO_SNAPSHOT),
    );
    v
}

/// One file of the snapshot: the embedded copy, else the first one found on disk.
fn snapshot_file(name: &str) -> Option<Cow<'static, [u8]>> {
    #[cfg(feature = "embed-demo")]
    if let Some(f) = EmbeddedSnapshot::get(name) {
        return Some(f.data);
    }
    snapshot_dirs()
        .into_iter()
        .find_map(|d| std::fs::read(d.join(name)).ok())
        .map(Cow::Owned)
}

/// The manifest of the prebuilt demo brain, if one ships with this binary and this binary
/// can read its format.
pub fn snapshot_manifest() -> Option<SnapshotManifest> {
    manifest_from(&snapshot_file)
}

/// Where the files of a snapshot come from (the shipped one, or a test's).
type Fetch<'a> = &'a dyn Fn(&str) -> Option<Cow<'static, [u8]>>;

fn manifest_from(fetch: Fetch<'_>) -> Option<SnapshotManifest> {
    let raw = fetch("manifest.json")?;
    match serde_json::from_slice::<SnapshotManifest>(&raw) {
        Ok(m) if m.format == SNAPSHOT_FORMAT => Some(m),
        Ok(m) => {
            tracing::warn!("ignoring a demo snapshot of format {}", m.format);
            None
        }
        Err(e) => {
            tracing::warn!("ignoring an unreadable demo snapshot manifest: {e}");
            None
        }
    }
}

/// Demo notes were named explicitly (`--demo-bundle`, `ANAMNESIS_DEMO_BUNDLE`): build from
/// those instead of seeding the snapshot.
fn explicit_demo_bundle(cfg: &crate::app::ServerConfig) -> bool {
    cfg.demo_bundle.is_some() || std::env::var("ANAMNESIS_DEMO_BUNDLE").is_ok_and(|v| !v.is_empty())
}

/// Does a brain directory hold no events (or no database at all)?
fn brain_dir_is_empty(dir: &Path) -> Result<bool> {
    let db = dir.join("brain.sqlite");
    if !db.is_file() {
        return Ok(true);
    }
    let s =
        anamnesis_core::Store::open(&db).with_context(|| format!("opening {}", db.display()))?;
    Ok(s.event_count()? == 0)
}

/// Does the person's brain hold no items? (Read through its own connection: the brain may be
/// open elsewhere in this process, which sqlite in wal mode allows.)
fn user_brain_is_empty(dir: &Path) -> Result<bool> {
    let db = dir.join("brain.sqlite");
    if !db.is_file() {
        return Ok(true);
    }
    let s =
        anamnesis_core::Store::open(&db).with_context(|| format!("opening {}", db.display()))?;
    Ok(s.count_items()? == 0)
}

fn first_launch_done(cfg: &crate::app::ServerConfig) -> bool {
    cfg.root.join(FIRST_LAUNCH_MARKER).exists()
}

fn mark_first_launch_done(cfg: &crate::app::ServerConfig) {
    let p = cfg.root.join(FIRST_LAUNCH_MARKER);
    if !p.exists()
        && let Err(e) = std::fs::write(&p, now().to_rfc3339())
    {
        tracing::warn!("could not write {}: {e}", p.display());
    }
}

/// Seed the demo brain from the snapshot when it is empty (or missing) and a snapshot ships.
/// Returns `None` when there was nothing to do. The demo brain must not be open.
pub fn seed_demo_if_empty(cfg: &crate::app::ServerConfig) -> Result<Option<SeedReport>> {
    if snapshot_manifest().is_none() || !brain_dir_is_empty(&cfg.demo_brain_dir())? {
        return Ok(None);
    }
    seed_demo_from_snapshot(&cfg.demo_brain_dir())
}

/// Unpack the snapshot into `dest` (a brain directory): decompress each file, check its
/// sha256, open the database, verify the whole event chain and that it ends at the
/// manifest's head, and only then move the files into place (replacing an empty brain).
/// Returns `None` when no snapshot ships.
pub fn seed_demo_from_snapshot(dest: &Path) -> Result<Option<SeedReport>> {
    seed_from(dest, &snapshot_file)
}

fn seed_from(dest: &Path, fetch: Fetch<'_>) -> Result<Option<SeedReport>> {
    let t0 = std::time::Instant::now();
    let Some(man) = manifest_from(fetch) else {
        return Ok(None);
    };
    anyhow::ensure!(
        man.files.contains_key("brain.sqlite"),
        "the demo snapshot has no brain.sqlite"
    );
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    let tmp = dest.join(SEEDING_DIR);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    let unpacked = unpack_snapshot(&man, &tmp, fetch);
    let report = match unpacked {
        Ok(items) => SeedReport {
            events: man.events,
            items,
            head: man.head.clone(),
            embedding_model: man.embedding_model.clone(),
            millis: 0,
        },
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    // install: the empty brain (and its wal) out, the snapshot in
    for f in ["brain.sqlite", "brain.sqlite-wal", "brain.sqlite-shm"]
        .into_iter()
        .chain(SNAPSHOT_FILES.into_iter().skip(1))
    {
        let p = dest.join(f);
        if p.exists() {
            std::fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
        }
    }
    for e in std::fs::read_dir(&tmp)?.flatten() {
        let to = dest.join(e.file_name());
        std::fs::rename(e.path(), &to).with_context(|| format!("moving {}", to.display()))?;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(Some(SeedReport {
        millis: t0.elapsed().as_millis() as u64,
        ..report
    }))
}

/// Decompress and check every file of the snapshot into `dir`; returns the item count.
fn unpack_snapshot(man: &SnapshotManifest, dir: &Path, fetch: Fetch<'_>) -> Result<u64> {
    for name in SNAPSHOT_FILES {
        let Some(info) = man.files.get(name) else {
            continue;
        };
        let gz = fetch(&format!("{name}.gz"))
            .with_context(|| format!("the demo snapshot is missing {name}.gz"))?;
        let mut out = Vec::with_capacity(info.bytes as usize);
        flate2::read::GzDecoder::new(gz.as_ref())
            .read_to_end(&mut out)
            .with_context(|| format!("decompressing {name}.gz"))?;
        let sum = hex::encode(Sha256::digest(&out));
        anyhow::ensure!(
            sum == info.sha256,
            "{name} in the demo snapshot does not match its checksum"
        );
        std::fs::write(dir.join(name), &out)
            .with_context(|| format!("writing {}", dir.join(name).display()))?;
    }
    let store = anamnesis_core::Store::open(dir.join("brain.sqlite"))
        .context("opening the demo snapshot")?;
    let chain = store
        .verify_chain()
        .context("the demo snapshot's event chain does not verify")?;
    anyhow::ensure!(
        chain.events == man.events && chain.head.as_deref() == Some(man.head.as_str()),
        "the demo snapshot's event chain ends at {} events, {}; its manifest says {}, {}",
        chain.events,
        chain.head.as_deref().unwrap_or("-"),
        man.events,
        man.head
    );
    Ok(store.count_items()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab_bundle(out: &Path, theme: &str) {
        let b = out.join(theme);
        std::fs::create_dir_all(b.join("notes/thoreau/journal")).unwrap();
        std::fs::write(b.join("notes/thoreau/journal/a day.md"), "a walk").unwrap();
        for f in ["COLOPHON.md", "SOURCES.md", "manifest.json"] {
            std::fs::write(b.join(f), "{}").unwrap();
        }
    }

    #[test]
    fn finds_the_labs_notes_preferring_transcendentalist() {
        let repo = tempfile::tempdir().unwrap();
        assert_eq!(demo_in_repo(repo.path()), None);
        let out = repo.path().join("lab/out");
        lab_bundle(&out, "aaa-letters");
        assert_eq!(
            demo_in_repo(repo.path()),
            Some(out.join("aaa-letters/notes"))
        );
        lab_bundle(&out, DEMO_THEME);
        assert_eq!(
            demo_in_repo(repo.path()),
            Some(out.join(DEMO_THEME).join("notes"))
        );
        // an explicit bundle folder resolves to its notes, a notes folder stays as it is
        let root = out.join(DEMO_THEME);
        assert_eq!(locate_demo(Some(&root)), Some(root.join("notes")));
        assert_eq!(
            locate_demo(Some(&root.join("notes"))),
            Some(root.join("notes"))
        );
        assert_eq!(locate_demo(Some(&out.join("missing"))), None);
    }

    #[test]
    fn a_full_lab_build_wins_over_the_curated_demo() {
        let repo = tempfile::tempdir().unwrap();
        let curated = repo.path().join(CURATED_DEMO);
        std::fs::create_dir_all(curated.join("notes/x")).unwrap();
        std::fs::write(curated.join("notes/x/a.md"), "a").unwrap();
        assert_eq!(demo_in_repo(repo.path()), Some(curated.join("notes")));
        lab_bundle(&repo.path().join("lab/out"), DEMO_THEME);
        assert_eq!(
            demo_in_repo(repo.path()),
            Some(repo.path().join("lab/out").join(DEMO_THEME).join("notes"))
        );
        // a release's `demo/` folder holding `transcendentalist/notes`
        let rel = repo.path().join("demo");
        assert_eq!(locate_demo(Some(&rel)), Some(curated.join("notes")));
    }

    #[test]
    fn the_shipped_curated_demo_is_found_in_this_repository() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let notes = repo.join(CURATED_DEMO).join("notes");
        assert!(notes.is_dir(), "{} is missing", notes.display());
        assert!(repo.join(CURATED_DEMO).join("SOURCES.md").is_file());
        assert!(repo.join(CURATED_DEMO).join("COLOPHON.md").is_file());
        let found = demo_in_repo(&repo).unwrap();
        assert!(found.ends_with("notes"), "{}", found.display());
    }

    #[cfg(feature = "embed-demo")]
    #[test]
    fn the_embedded_demo_extracts_and_prunes() {
        assert!(has_embedded_demo());
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(DEMO_THEME);
        std::fs::create_dir_all(dest.join("notes/old")).unwrap();
        std::fs::write(dest.join("notes/old/stale.md"), "gone").unwrap();
        let notes = extract_embedded_demo(&dest).unwrap().unwrap();
        assert_eq!(notes, dest.join("notes"));
        assert!(!dest.join("notes/old/stale.md").exists());
        assert!(dest.join("SOURCES.md").is_file() && dest.join("COLOPHON.md").is_file());
        let n = embedded_demo_files()
            .iter()
            .filter(|p| p.starts_with("notes/"))
            .count();
        let on_disk = walk(&notes);
        assert_eq!(on_disk, n);
        assert!(n > 500, "{n} notes embedded");
        // a second extraction rewrites nothing and keeps everything
        extract_embedded_demo(&dest).unwrap();
        assert_eq!(walk(&notes), n);
    }

    #[cfg(feature = "embed-demo")]
    fn walk(d: &Path) -> usize {
        std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| {
                let p = e.path();
                if p.is_dir() { walk(&p) } else { 1 }
            })
            .sum()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn demo_status_reports_the_jobs_and_embedding_progress() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = crate::app::ServerConfig::for_tests(&dir.path().join("anamnesis"));
        cfg.demo_bundle = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes"));
        let app = App::open(cfg, false).unwrap();
        let st = set_demo(&app, true).await.unwrap();
        assert!(st.active && st.available && st.items > 0);
        let mut st = st;
        for _ in 0..600 {
            st = demo_status(&app).await.unwrap();
            if !st.jobs.running && st.ready {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(st.ready, "{st:?}");
        assert!(st.chunks > 0 && st.chunks_embedded == st.chunks, "{st:?}");
        let v = serde_json::to_value(&st).unwrap();
        assert!(
            v["jobs"]["phase"].is_null() && v["jobs"]["total"] == 5,
            "{v}"
        );
    }

    /// A snapshot of a small brain built here with the hash embedder: name → bytes.
    async fn tiny_snapshot() -> (std::collections::HashMap<String, Vec<u8>>, SnapshotManifest) {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = crate::app::ServerConfig::for_tests(&dir.path().join("anamnesis"));
        cfg.demo_bundle = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes"));
        let app = App::open(cfg, true).unwrap();
        let brain = app.brain();
        let notes = cfg_notes();
        import_path(&brain, &notes).await.unwrap();
        crate::jobs::run_jobs_once(&app).await.unwrap();
        let (events, head, items) = brain
            .db(|s| Ok((s.event_count()?, s.head()?.unwrap().1, s.count_items()?)))
            .await
            .unwrap();
        drop(brain);
        drop(app);
        let mut files = std::collections::HashMap::new();
        let mut infos = BTreeMap::new();
        for name in SNAPSHOT_FILES {
            let p = dir.path().join("anamnesis/demo").join(name);
            let Ok(raw) = std::fs::read(&p) else { continue };
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            enc.write_all(&raw).unwrap();
            let gz = enc.finish().unwrap();
            infos.insert(
                name.to_string(),
                SnapshotFile {
                    sha256: hex::encode(Sha256::digest(&raw)),
                    bytes: raw.len() as u64,
                    gz_bytes: gz.len() as u64,
                },
            );
            files.insert(format!("{name}.gz"), gz);
        }
        assert!(infos.contains_key("brain.sqlite") && infos.contains_key("views.json"));
        let man = SnapshotManifest {
            format: SNAPSHOT_FORMAT,
            events,
            head,
            items,
            chunks: 0,
            embedding_model: "hash@256".into(),
            files: infos,
            anamnesis_version: String::new(),
            note: String::new(),
        };
        files.insert("manifest.json".into(), serde_json::to_vec(&man).unwrap());
        (files, man)
    }

    fn cfg_notes() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_snapshot_seeds_a_verified_brain_and_refuses_tampering() {
        let (files, man) = tiny_snapshot().await;
        // the wal of the source brain must have been folded in: the copy alone is complete
        let fetch = |n: &str| files.get(n).cloned().map(Cow::Owned);
        let dest = tempfile::tempdir().unwrap();
        let demo = dest.path().join("demo");
        // an empty brain is already there (a failed first try): it is replaced
        drop(Brain::open(&demo, true).unwrap());
        let r = seed_from(&demo, &fetch).unwrap().unwrap();
        assert_eq!((r.events, r.items), (man.events, man.items));
        assert!(!demo.join(SEEDING_DIR).exists());
        let brain = Brain::open(&demo, true).unwrap();
        let chain = brain.with_store(|s| Ok(s.verify_chain()?)).unwrap();
        assert_eq!(chain.head.as_deref(), Some(man.head.as_str()));
        assert!(brain.views().is_some());
        assert!(brain.job_state().mind_model.is_some());
        drop(brain);

        // a file that does not match its checksum
        let mut bad = files.clone();
        let mut man2 = man.clone();
        man2.files.get_mut("views.json").unwrap().sha256 = "00".repeat(32);
        bad.insert("manifest.json".into(), serde_json::to_vec(&man2).unwrap());
        let fetch = |n: &str| bad.get(n).cloned().map(Cow::Owned);
        let other = dest.path().join("other");
        let e = seed_from(&other, &fetch).unwrap_err();
        assert!(format!("{e:#}").contains("checksum"), "{e:#}");
        assert!(!other.join("brain.sqlite").exists() && !other.join(SEEDING_DIR).exists());

        // a manifest whose head is not where the chain ends
        let mut bad = files.clone();
        let mut man3 = man.clone();
        man3.head = "f".repeat(64);
        bad.insert("manifest.json".into(), serde_json::to_vec(&man3).unwrap());
        let fetch = |n: &str| bad.get(n).cloned().map(Cow::Owned);
        let e = seed_from(&other, &fetch).unwrap_err();
        assert!(format!("{e:#}").contains("event chain"), "{e:#}");

        // another format is ignored, not misread
        let mut bad = files.clone();
        let mut man4 = man.clone();
        man4.format = SNAPSHOT_FORMAT + 1;
        bad.insert("manifest.json".into(), serde_json::to_vec(&man4).unwrap());
        let fetch = |n: &str| bad.get(n).cloned().map(Cow::Owned);
        assert!(seed_from(&other, &fetch).unwrap().is_none());
    }

    /// The prebuilt demo brain that ships in the repository opens at once on a fresh data
    /// dir, verified, with its layout; the first-launch choice is remembered.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_shipped_snapshot_seeds_the_demo_on_first_launch() {
        let Some(man) = snapshot_manifest() else {
            panic!("no demo snapshot in {DEMO_SNAPSHOT}");
        };
        assert!(!man.embedding_model.starts_with("hash@"), "{man:?}");
        if std::env::var("ANAMNESIS_DEMO_BUNDLE").is_ok_and(|v| !v.is_empty()) {
            return; // an explicit bundle builds from notes instead
        }
        let dir = tempfile::tempdir().unwrap();
        let cfg = crate::app::ServerConfig::for_tests(&dir.path().join("anamnesis"));
        let app = App::open(cfg, false).unwrap();
        let st = demo_status(&app).await.unwrap();
        assert!(
            st.snapshot && st.available && st.user_empty && st.first_launch,
            "{st:?}"
        );
        let t0 = std::time::Instant::now();
        let st = set_demo(&app, true).await.unwrap();
        assert!(st.active && st.ready && st.items == man.items, "{st:?}");
        assert!(!st.jobs.running, "a seeded demo needs no rebuild: {st:?}");
        assert!(st.user_empty && st.first_launch);
        assert!(t0.elapsed().as_secs() < 30);
        let head = app
            .brain()
            .db(|s| Ok(s.verify_chain()?.head))
            .await
            .unwrap();
        assert_eq!(head.as_deref(), Some(man.head.as_str()));
        // words-only search works with no embedder loaded
        let hits = crate::chat::hybrid_search(&app, &app.brain(), "walden pond", 5)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        // leaving the demo is the first-launch choice; it is not offered again
        let st = set_demo(&app, false).await.unwrap();
        assert!(!st.active && st.user_empty && !st.first_launch, "{st:?}");
        // a second visit reuses the seeded brain as it is
        let st = set_demo(&app, true).await.unwrap();
        assert!(st.active && st.items == man.items);
    }

    #[test]
    fn rebuild_refuses_a_foreign_demo_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = crate::app::ServerConfig::for_tests(dir.path());
        assert!(!remove_demo_brain(&cfg).unwrap());
        std::fs::create_dir_all(cfg.demo_brain_dir().join(DEMO_THEME)).unwrap();
        std::fs::write(cfg.demo_brain_dir().join("brain.sqlite"), "").unwrap();
        assert!(remove_demo_brain(&cfg).unwrap());
        assert!(!cfg.demo_brain_dir().exists());
        std::fs::create_dir_all(cfg.demo_brain_dir()).unwrap();
        std::fs::write(cfg.demo_brain_dir().join("thesis.docx"), "").unwrap();
        assert!(remove_demo_brain(&cfg).is_err());
    }

    #[tokio::test]
    async fn demo_import_skips_the_bundle_metadata() {
        let repo = tempfile::tempdir().unwrap();
        let out = repo.path().join("lab/out");
        lab_bundle(&out, DEMO_THEME);
        let brain = Arc::new(Brain::open(&repo.path().join("brain"), true).unwrap());
        let notes = locate_demo(Some(&out.join(DEMO_THEME))).unwrap();
        let r = import_path(&brain, &notes).await.unwrap();
        let ImportReport::Folder { sync, .. } = r else {
            panic!("{r:?}")
        };
        assert_eq!(sync.created, 1);
        let items = brain
            .db(|s| Ok(s.list_items(&Default::default())?))
            .await
            .unwrap();
        assert!(items.iter().all(|i| !i.external_id.ends_with("COLOPHON.md")
            && !i.external_id.ends_with("SOURCES.md")
            && !i.external_id.contains("manifest")));
        let sources = brain.db(|s| Ok(s.list_sources()?)).await.unwrap();
        assert!(sources.iter().any(|s| s.name == DEMO_THEME));
    }
}
