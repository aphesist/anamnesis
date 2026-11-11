//! The registry of built-in connectors, and plugin loading.

use std::path::Path;

use crate::connector::Connector;
use crate::connectors::{exec::ExecConnector, files, json_api, pdf, rss, web};
use crate::error::{ConnectorError, Result};
use crate::manifest::{Manifest, RESERVED_CONNECTOR_IDS, Runtime};

/// Ids of the connectors compiled into anamnesis.
pub const BUILTIN_IDS: &[&str] = &["folder", "obsidian", "pdf", "rss", "web", "json-api"];

/// A built-in connector: its manifest and a constructor.
#[derive(Clone)]
pub struct BuiltinConnector {
    pub manifest: Manifest,
    pub create: fn() -> Box<dyn Connector>,
}

impl std::fmt::Debug for BuiltinConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinConnector").field("id", &self.manifest.id).finish()
    }
}

pub fn builtin_connectors() -> Vec<BuiltinConnector> {
    vec![
        BuiltinConnector { manifest: files::folder_manifest(), create: || Box::new(files::FilesConnector::folder()) },
        BuiltinConnector { manifest: files::obsidian_manifest(), create: || Box::new(files::FilesConnector::obsidian()) },
        BuiltinConnector { manifest: pdf::pdf_manifest(), create: || Box::new(pdf::PdfConnector) },
        BuiltinConnector { manifest: rss::rss_manifest(), create: || Box::new(rss::RssConnector) },
        BuiltinConnector { manifest: web::web_manifest(), create: || Box::new(web::WebConnector) },
        BuiltinConnector { manifest: json_api::json_api_manifest(), create: || Box::new(json_api::JsonApiConnector) },
    ]
}

/// Construct a built-in connector by id. Reserved ids ("editor", "chat", "mcp") have no
/// connector: the server writes those items directly.
pub fn builtin(id: &str) -> Result<Box<dyn Connector>> {
    if RESERVED_CONNECTOR_IDS.contains(&id) {
        return Err(ConnectorError::Unsupported(format!("{id:?} items are written by the server, not a connector")));
    }
    builtin_connectors()
        .into_iter()
        .find(|b| b.manifest.id == id)
        .map(|b| (b.create)())
        .ok_or_else(|| ConnectorError::UnknownConnector(id.to_string()))
}

/// Load a plugin from its directory or manifest file.
pub fn load_plugin(path: &Path) -> Result<Box<dyn Connector>> {
    let manifest_path =
        if path.is_dir() { path.join(crate::manifest::PLUGIN_MANIFEST_FILE) } else { path.to_path_buf() };
    let m = Manifest::from_path(&manifest_path)?;
    match m.runtime {
        Runtime::Exec => Ok(Box::new(ExecConnector::load(&manifest_path)?)),
        Runtime::Wasm => Err(ConnectorError::Unsupported("the wasm plugin runtime is not implemented yet".into())),
        Runtime::Builtin => Err(ConnectorError::InvalidManifest("plugins cannot use the builtin runtime".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid_and_match_ids() {
        let all = builtin_connectors();
        assert_eq!(all.iter().map(|b| b.manifest.id.as_str()).collect::<Vec<_>>(), BUILTIN_IDS);
        for b in &all {
            b.manifest.validate().unwrap_or_else(|e| panic!("{}: {e}", b.manifest.id));
            assert_eq!(b.manifest.runtime, Runtime::Builtin);
            assert_eq!((b.create)().manifest().id, b.manifest.id);
        }
        for r in RESERVED_CONNECTOR_IDS {
            assert!(!BUILTIN_IDS.contains(r));
            assert!(builtin(r).is_err());
        }
        assert!(matches!(builtin("nope"), Err(ConnectorError::UnknownConnector(_))));
    }
}
