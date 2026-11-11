//! Connector manifests.
//!
//! Every connector, built in or plugin, is described by a [`Manifest`]. The manifest is the
//! "spec" step of the Airbyte-style lifecycle: it names the connector, declares how it runs,
//! what config it accepts (a JSON schema), what it can do, and what it says it will touch
//! (network hosts, filesystem paths) so the ui can show that to the user before enabling it.
//!
//! Plugins ship their manifest as `anamnesis-plugin.json` next to their executable.

use std::path::Path;

use anamnesis_core::{ItemKind, Origin, Privacy};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ConnectorError, Result};

/// The connector api version this crate speaks. Plugins declare the version they target.
pub const API_VERSION: u32 = 1;

/// Connector ids the server uses for items it writes itself (no connector behind them).
/// No built-in connector or plugin may claim these.
pub const RESERVED_CONNECTOR_IDS: &[&str] = &["editor", "chat", "mcp"];

/// File name a plugin's manifest is expected to have.
pub const PLUGIN_MANIFEST_FILE: &str = "anamnesis-plugin.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    /// Compiled into anamnesis.
    Builtin,
    /// An external executable speaking json lines over stdin/stdout (see PLUGINS.md).
    Exec,
    /// A WebAssembly module. Reserved: declared in the format, not yet runnable.
    Wasm,
}

/// What a connector can do. Unknown values from newer plugins deserialise as `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// `check` does something meaningful (reachability, path exists...).
    Check,
    /// `list` honours the cursor and only returns changes.
    Incremental,
    /// `list` reports deletions.
    Deletes,
    /// `list` reports renames (see `Batch::renames`).
    Renames,
    /// `watch` returns change notices; otherwise the caller polls.
    Watch,
    /// `list` may return `has_more = true` and must be called again with `next_cursor`.
    Paged,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Lowercase id, `[a-z0-9][a-z0-9_-]*`, at most 64 chars. Stored as `Source.connector`.
    pub id: String,
    /// Human name, lowercase by convention ("obsidian vault").
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Version of the connector itself, `major.minor.patch`.
    pub version: String,
    /// Connector api version targeted (see [`API_VERSION`]).
    pub api_version: u32,
    pub runtime: Runtime,
    /// For `exec`: path of the executable, relative to the manifest's directory (or absolute).
    /// For `wasm`: path of the module. For `builtin`: unused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// JSON schema (draft 2020-12 unless `$schema` says otherwise) for `Source.config`.
    pub config_schema: Value,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// Hosts the connector says it will contact. Recorded and shown to the user; NOT enforced.
    /// A value like `<config.url host>` means "whatever the source config points at".
    #[serde(default)]
    pub network_hosts: Vec<String>,
    /// Paths the connector says it will read. Recorded and shown to the user; NOT enforced.
    #[serde(default)]
    pub fs_paths: Vec<String>,
    pub default_origin: Origin,
    #[serde(default)]
    pub default_privacy: Privacy,
    /// Kind used when a document does not say otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_kind: Option<ItemKind>,
    /// Suggested polling interval when the connector cannot `watch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_secs: Option<u64>,
}

impl Manifest {
    pub fn from_json(s: &str) -> Result<Self> {
        let m: Manifest =
            serde_json::from_str(s).map_err(|e| ConnectorError::InvalidManifest(e.to_string()))?;
        Ok(m)
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).map_err(|e| ConnectorError::io(path, e))?;
        Self::from_json(&s)
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Structural validation. Plugins should also pass [`Manifest::validate_plugin`].
    pub fn validate(&self) -> Result<()> {
        let mut errs = Vec::new();
        if !valid_id(&self.id) {
            errs.push(format!("id {:?} must match [a-z0-9][a-z0-9_-]* (max 64)", self.id));
        }
        if !valid_version(&self.version) {
            errs.push(format!("version {:?} must be major.minor.patch", self.version));
        }
        if self.api_version == 0 || self.api_version > API_VERSION {
            errs.push(format!(
                "api_version {} is not supported (this build speaks {API_VERSION})",
                self.api_version
            ));
        }
        match self.runtime {
            Runtime::Builtin => {}
            Runtime::Exec | Runtime::Wasm => match &self.entry {
                Some(e) if !e.trim().is_empty() => {}
                _ => errs.push(format!("runtime {:?} needs an entry", self.runtime)),
            },
        }
        if !self.config_schema.is_object() && !self.config_schema.is_boolean() {
            errs.push("config_schema must be a json schema object".into());
        } else if let Err(e) = jsonschema::validator_for(&self.config_schema) {
            errs.push(format!("config_schema is not a valid schema: {e}"));
        }
        for h in &self.network_hosts {
            if !valid_host_pattern(h) {
                errs.push(format!("network host {h:?} must be a bare host name (optionally *.domain)"));
            }
        }
        for p in &self.fs_paths {
            if p.trim().is_empty() {
                errs.push("fs_paths may not contain empty paths".into());
            }
        }
        if self.capabilities.contains(&Capability::Unknown) {
            tracing::warn!(connector = %self.id, "manifest declares capabilities this build does not know");
        }
        if errs.is_empty() { Ok(()) } else { Err(ConnectorError::InvalidManifest(errs.join("; "))) }
    }

    /// Validation for third-party plugins: structure, plus no claiming reserved or built-in ids
    /// and no `builtin` runtime.
    pub fn validate_plugin(&self) -> Result<()> {
        self.validate()?;
        if self.runtime == Runtime::Builtin {
            return Err(ConnectorError::InvalidManifest("plugins cannot use the builtin runtime".into()));
        }
        if is_reserved_id(&self.id) {
            return Err(ConnectorError::InvalidManifest(format!("id {:?} is reserved", self.id)));
        }
        Ok(())
    }

    /// Validate a source config against `config_schema`. Returns every violation found.
    pub fn validate_config(&self, config: &Value) -> Result<()> {
        let validator = jsonschema::validator_for(&self.config_schema)
            .map_err(|e| ConnectorError::InvalidManifest(format!("config_schema: {e}")))?;
        let errs: Vec<String> = validator
            .iter_errors(config)
            .map(|e| {
                let path = e.instance_path().to_string();
                if path.is_empty() { e.to_string() } else { format!("{path}: {e}") }
            })
            .collect();
        if errs.is_empty() { Ok(()) } else { Err(ConnectorError::InvalidConfig(errs)) }
    }
}

/// True for ids nobody but anamnesis itself may use: the reserved ids and the built-ins.
pub fn is_reserved_id(id: &str) -> bool {
    RESERVED_CONNECTOR_IDS.contains(&id) || crate::registry::BUILTIN_IDS.contains(&id)
}

fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
}

fn valid_version(v: &str) -> bool {
    // major.minor.patch with an optional -pre / +build suffix.
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
}

/// `<config.url host>`-style placeholders: the value depends on the source config.
fn is_placeholder(s: &str) -> bool {
    s.starts_with("<config.") && s.ends_with('>')
}

fn valid_host_pattern(h: &str) -> bool {
    if is_placeholder(h) {
        return true;
    }
    let h = h.strip_prefix("*.").unwrap_or(h);
    !h.is_empty()
        && !h.contains("://")
        && !h.contains('/')
        && h.split('.').all(|l| {
            !l.is_empty() && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plugin() -> Manifest {
        Manifest::from_json(
            &json!({
                "id": "my-notes",
                "version": "0.1.0",
                "api_version": 1,
                "runtime": "exec",
                "entry": "./plugin.sh",
                "config_schema": {"type": "object", "required": ["dir"], "properties": {"dir": {"type": "string"}}},
                "capabilities": ["check", "incremental", "teleport"],
                "network_hosts": ["api.example.com", "*.cdn.example.com"],
                "fs_paths": ["~/notes"],
                "default_origin": "owner_written"
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn plugin_manifest_roundtrip_and_validate() {
        let m = plugin();
        m.validate_plugin().unwrap();
        assert_eq!(m.default_privacy, Privacy::CloudOk);
        assert!(m.has(Capability::Incremental));
        assert!(m.capabilities.contains(&Capability::Unknown));
        let back = Manifest::from_json(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.id, m.id);
    }

    #[test]
    fn rejects_bad_manifests() {
        let mut m = plugin();
        m.id = "Bad Id".into();
        assert!(m.validate().is_err());
        let mut m = plugin();
        m.version = "1.0".into();
        assert!(m.validate().is_err());
        let mut m = plugin();
        m.entry = None;
        assert!(m.validate().is_err());
        let mut m = plugin();
        m.api_version = API_VERSION + 1;
        assert!(m.validate().is_err());
        let mut m = plugin();
        m.network_hosts = vec!["https://x.com/".into()];
        assert!(m.validate().is_err());
        let mut m = plugin();
        m.config_schema = json!({"type": 12});
        assert!(m.validate().is_err());
        for id in ["chat", "editor", "mcp", "obsidian"] {
            let mut m = plugin();
            m.id = id.into();
            assert!(m.validate_plugin().is_err(), "{id} should be reserved");
        }
    }

    #[test]
    fn config_validation_reports_errors() {
        let m = plugin();
        m.validate_config(&json!({"dir": "/x"})).unwrap();
        match m.validate_config(&json!({"dir": 3})) {
            Err(ConnectorError::InvalidConfig(errs)) => assert!(errs[0].contains("/dir"), "{errs:?}"),
            other => panic!("{other:?}"),
        }
        assert!(m.validate_config(&json!({})).is_err());
    }
}
