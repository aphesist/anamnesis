//! anamnesis-connectors: turn sources (folders, obsidian vaults, pdfs, feeds, web pages, json
//! apis, plugins) into [`anamnesis_core::Document`]s, with incremental sync and provenance.
//!
//! - [`Connector`]: the lifecycle (manifest / check / list / watch) — see [`connector`].
//! - [`Manifest`]: what a connector is, runs as, accepts and touches — see [`manifest`].
//! - [`builtin_connectors`] / [`builtin`]: the connectors compiled in;
//!   [`load_plugin`]: an `exec` plugin (protocol in `PLUGINS.md`).
//! - [`RESERVED_CONNECTOR_IDS`]: "editor", "chat", "mcp" — the server writes those items itself.

pub mod connector;
pub mod connectors;
pub mod error;
pub mod html;
pub mod http;
pub mod manifest;
pub mod markdown;
pub mod registry;
pub mod scan;
pub mod url_norm;

pub use connector::{
    Batch, ChangeNotice, CheckReport, Connector, Rename, WatchHandle, list_all, validated_check,
    validated_list,
};
pub use connectors::files::{META_ORIGIN_EXPLICIT, META_PRIVACY};
pub use error::{ConnectorError, Result};
pub use manifest::{
    API_VERSION, Capability, Manifest, PLUGIN_MANIFEST_FILE, RESERVED_CONNECTOR_IDS, Runtime,
};
pub use registry::{BUILTIN_IDS, BuiltinConnector, builtin, builtin_connectors, load_plugin};
pub use url_norm::normalize_url;
