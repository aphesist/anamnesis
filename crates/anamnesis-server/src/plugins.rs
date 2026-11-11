//! Connector plugins found on disk.
//!
//! The server scans `<data dir>/anamnesis/plugins/*/` (and every `--plugins-dir`) for a
//! plugin manifest (`manifest.json`, or `anamnesis-plugin.json` as in `PLUGINS.md`) and
//! loads each with [`anamnesis_connectors::load_plugin`]. A plugin whose manifest is
//! invalid, whose id clashes with a built-in or an earlier plugin, or whose runtime is not
//! runnable is skipped and reported, never fatal.
//!
//! Declared `network_hosts` / `fs_paths` are shown to the user; they are not enforced.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anamnesis_connectors::{BUILTIN_IDS, Connector, Manifest, PLUGIN_MANIFEST_FILE, load_plugin};
use serde::Serialize;

/// Manifest file names looked for in each plugin folder, in order.
pub const MANIFEST_NAMES: &[&str] = &["manifest.json", PLUGIN_MANIFEST_FILE];

#[derive(Debug, Clone, Serialize)]
pub struct LoadedPlugin {
    pub manifest: Manifest,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginError {
    pub path: PathBuf,
    pub error: String,
}

#[derive(Debug, Default)]
pub struct Plugins {
    dirs: Vec<PathBuf>,
    state: RwLock<(Vec<LoadedPlugin>, Vec<PluginError>)>,
}

impl Plugins {
    /// A registry over these plugin directories (not scanned yet).
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Plugins {
            dirs,
            state: RwLock::default(),
        }
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Re-read every plugin directory.
    pub fn scan(&self) {
        let mut loaded: Vec<LoadedPlugin> = Vec::new();
        let mut errors = Vec::new();
        for dir in &self.dirs {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue; // a missing plugins folder is normal
            };
            let mut subdirs: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect();
            subdirs.sort();
            for sub in subdirs {
                let Some(mpath) = manifest_in(&sub) else {
                    continue;
                };
                match load_plugin(&mpath) {
                    Ok(conn) => {
                        let manifest = conn.manifest();
                        if BUILTIN_IDS.contains(&manifest.id.as_str())
                            || loaded.iter().any(|p| p.manifest.id == manifest.id)
                        {
                            errors.push(PluginError {
                                path: mpath,
                                error: format!("connector id {:?} is already taken", manifest.id),
                            });
                            continue;
                        }
                        loaded.push(LoadedPlugin {
                            manifest,
                            manifest_path: mpath,
                        });
                    }
                    Err(e) => errors.push(PluginError {
                        path: mpath,
                        error: e.to_string(),
                    }),
                }
            }
        }
        for e in &errors {
            tracing::warn!("plugin {}: {}", e.path.display(), e.error);
        }
        *self.state.write().unwrap() = (loaded, errors);
    }

    pub fn loaded(&self) -> Vec<LoadedPlugin> {
        self.state.read().unwrap().0.clone()
    }

    pub fn errors(&self) -> Vec<PluginError> {
        self.state.read().unwrap().1.clone()
    }

    /// A fresh connector for a loaded plugin id.
    pub fn connector(&self, id: &str) -> Option<anamnesis_connectors::Result<Box<dyn Connector>>> {
        let path = self
            .state
            .read()
            .unwrap()
            .0
            .iter()
            .find(|p| p.manifest.id == id)
            .map(|p| p.manifest_path.clone())?;
        Some(load_plugin(&path))
    }

    /// A built-in connector, else a loaded plugin (rescanning once if the id is unknown).
    pub fn resolve(&self, id: &str) -> anamnesis_connectors::Result<Box<dyn Connector>> {
        match anamnesis_connectors::builtin(id) {
            Err(anamnesis_connectors::ConnectorError::UnknownConnector(_)) => {}
            other => return other,
        }
        if let Some(c) = self.connector(id) {
            return c;
        }
        self.scan();
        self.connector(id).unwrap_or_else(|| {
            Err(anamnesis_connectors::ConnectorError::UnknownConnector(
                id.to_string(),
            ))
        })
    }
}

fn manifest_in(dir: &Path) -> Option<PathBuf> {
    MANIFEST_NAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}
