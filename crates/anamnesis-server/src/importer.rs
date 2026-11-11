//! Importing folders and bundles, exporting bundles, and the demo brain.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anamnesis_core::{Id, Origin, Privacy, Source, now};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;

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
    let name = dir
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
            Ok(s.add_source(Source {
                id: Id::new(),
                connector: c,
                name,
                config: json!({ "path": path }),
                default_origin: Origin::OwnerWritten,
                default_privacy: Privacy::CloudOk,
                cursor: None,
                created_at: now(),
            })?)
        })
        .await?;
    let sync = sync_source(brain, &source.id).await?;
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

/// Where the demo bundle is: configured path, `ANAMNESIS_DEMO_BUNDLE`, `demo/` next to the
/// binary, or the lab's output in the repository (dev).
pub fn locate_demo(cfg_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = cfg_path {
        return p.exists().then(|| p.to_path_buf());
    }
    if let Ok(p) = std::env::var("ANAMNESIS_DEMO_BUNDLE")
        && !p.is_empty()
    {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    let mut cands = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        cands.push(dir.join("demo"));
        if let Some(repo) = dir.parent().and_then(|p| p.parent()) {
            cands.push(repo.join("lab/demo-brain"));
            cands.push(repo.join("lab/out/demo-brain"));
        }
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    cands.push(repo.join("lab/demo-brain"));
    cands.push(repo.join("lab/out/demo-brain"));
    cands.into_iter().find(|p| p.exists())
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoStatus {
    pub active: bool,
    pub available: bool,
    pub bundle: Option<String>,
    pub items: u64,
}

pub async fn demo_status(app: &Arc<App>) -> Result<DemoStatus> {
    let bundle = locate_demo(app.cfg.demo_bundle.as_deref());
    let brain = app.brain();
    let items = brain.db(|s| Ok(s.count_items()?)).await?;
    Ok(DemoStatus {
        active: brain.is_demo,
        available: bundle.is_some() || app.cfg.demo_brain_dir().join("brain.sqlite").is_file(),
        bundle: bundle.map(|b| b.display().to_string()),
        items,
    })
}

/// Switch to (or away from) the demo brain, importing the demo bundle on first use.
pub async fn set_demo(app: &Arc<App>, on: bool) -> Result<DemoStatus> {
    let brain = app.set_demo(on)?;
    if on {
        let empty = brain.db(|s| Ok(s.event_count()? == 0)).await?;
        if empty {
            match locate_demo(app.cfg.demo_bundle.as_deref()) {
                Some(p) => {
                    let r = import_path(&brain, &p).await?;
                    tracing::info!("demo brain imported from {}: {r:?}", p.display());
                    let app2 = app.clone();
                    tokio::spawn(async move {
                        if let Err(e) = crate::jobs::run_jobs_once(&app2).await {
                            tracing::warn!("demo jobs: {e:#}");
                        }
                    });
                }
                None => {
                    app.set_demo(false)?;
                    bail!("no demo bundle found (set ANAMNESIS_DEMO_BUNDLE or --demo-bundle)");
                }
            }
        }
    }
    crate::backends::detect_in_background(app);
    demo_status(app).await
}
