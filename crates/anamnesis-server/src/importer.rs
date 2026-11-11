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

/// Where the demo bundle is: configured path (`--demo-bundle`), `ANAMNESIS_DEMO_BUNDLE`,
/// `demo/` next to the binary, or the lab's output in the repository (dev):
/// `lab/demo-brain`, `lab/out/demo-brain`, then `lab/out/<theme>/notes` (the
/// transcendentalist theme first).
///
/// A lab bundle folder (`lab/out/<theme>/` with `notes/`, `manifest.json`, `SOURCES.md`,
/// `COLOPHON.md`) resolves to its `notes/` folder, so the bundle's metadata files never
/// become items.
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

/// The theme the demo prefers among the lab's outputs.
pub const DEMO_THEME: &str = "transcendentalist";

/// The demo inside a repository checkout, if the lab has built one.
pub fn demo_in_repo(repo: &Path) -> Option<PathBuf> {
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

/// A lab bundle folder (no `.jsonl` bundle, a `notes/` folder inside) → its `notes/`.
fn demo_root(p: &Path) -> PathBuf {
    if p.is_dir() && bundle_in(p).is_none() && p.join("notes").is_dir() {
        return p.join("notes");
    }
    p.to_path_buf()
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
                }
                None => {
                    app.set_demo(false)?;
                    bail!("no demo bundle found (set ANAMNESIS_DEMO_BUNDLE or --demo-bundle)");
                }
            }
        }
        // a layout and regions right away: run the jobs once in the background when the
        // demo brain has none yet (minutes for the full demo; progress in /api/status)
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
