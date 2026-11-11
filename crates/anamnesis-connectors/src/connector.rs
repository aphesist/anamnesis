//! The connector lifecycle, modelled on Airbyte's spec / check / discover / read:
//!
//! - **spec** → [`Connector::manifest`]: id, runtime, config schema, capabilities.
//! - **check** → [`Connector::check`]: can this config work (path exists, url answers)?
//! - **discover + read** → [`Connector::list`]: given the opaque cursor from the previous sync,
//!   return what changed as a [`Batch`] plus the next cursor. anamnesis has a single stream per
//!   source (documents), so discover collapses into list.
//! - **watch** (optional) → [`Connector::watch`]: change notices so the caller can call `list`
//!   early; connectors that cannot watch return `None` and the caller polls.
//!
//! ## applying a batch
//!
//! Apply `renames` first (re-key the stored item from `from` to `to`, keeping its id, links and
//! history), then `upserts` (insert or update by `external_id`), then `deletes`. A renamed
//! document is also present in `upserts` under its new external id. A consumer that cannot
//! re-key items calls [`Batch::flatten_renames`], which turns each rename into a delete of the
//! old id (so the item is recreated under the new one). Persist `next_cursor` only after the
//! batch is applied. If `has_more` is set, call `list` again with `next_cursor`.

use anamnesis_core::Document;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::error::Result;
use crate::manifest::Manifest;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Batch {
    /// New or changed documents.
    pub upserts: Vec<Document>,
    /// External ids that no longer exist in the source.
    pub deletes: Vec<String>,
    /// Documents that moved: same content, new external id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renames: Vec<Rename>,
    /// Opaque cursor to pass to the next `list` call. `None` means "no incremental state".
    pub next_cursor: Option<String>,
    /// More pages are waiting; call `list` again with `next_cursor` right away.
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.upserts.is_empty() && self.deletes.is_empty() && self.renames.is_empty()
    }

    /// For consumers that cannot re-key items: turn each rename into a delete of the old id.
    pub fn flatten_renames(mut self) -> Self {
        for r in self.renames.drain(..) {
            self.deletes.push(r.from);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckReport {
    pub ok: bool,
    /// Lowercase, user-facing ("found 412 notes", "feed answered with 20 entries").
    pub message: String,
}

impl CheckReport {
    pub fn ok(message: impl Into<String>) -> Self {
        CheckReport {
            ok: true,
            message: message.into(),
        }
    }
    pub fn failed(message: impl Into<String>) -> Self {
        CheckReport {
            ok: false,
            message: message.into(),
        }
    }
}

/// A hint that something in the source changed. Notices are coalesced by nobody: the caller
/// should debounce and then call `list` with its cursor, which computes the actual changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeNotice {
    /// External id, when the connector can tell.
    pub external_id: Option<String>,
    /// Path or url that changed, for logging.
    pub location: Option<String>,
}

/// A live watch. Dropping it stops watching.
pub struct WatchHandle {
    pub notices: mpsc::Receiver<ChangeNotice>,
    _guard: Box<dyn std::any::Any + Send>,
}

impl WatchHandle {
    pub fn new(notices: mpsc::Receiver<ChangeNotice>, guard: impl std::any::Any + Send) -> Self {
        WatchHandle {
            notices,
            _guard: Box::new(guard),
        }
    }
}

impl std::fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchHandle").finish_non_exhaustive()
    }
}

#[async_trait]
pub trait Connector: Send + Sync {
    fn manifest(&self) -> Manifest;

    /// Can this config work? Errors are for broken configs; an unreachable source is
    /// `Ok(CheckReport { ok: false, .. })`.
    async fn check(&self, config: &Value) -> Result<CheckReport>;

    /// Changes since `cursor` (`None` = full sync).
    async fn list(&self, config: &Value, cursor: Option<&str>) -> Result<Batch>;

    /// Change notices, if the connector can watch. Default: `None` → poll.
    /// Must be called from within a tokio runtime.
    fn watch(&self, _config: &Value) -> Result<Option<WatchHandle>> {
        Ok(None)
    }
}

/// Validate `config` against the connector's schema, then `check`.
pub async fn validated_check(c: &dyn Connector, config: &Value) -> Result<CheckReport> {
    c.manifest().validate_config(config)?;
    c.check(config).await
}

/// Validate `config` against the connector's schema, then `list`.
pub async fn validated_list(
    c: &dyn Connector,
    config: &Value,
    cursor: Option<&str>,
) -> Result<Batch> {
    c.manifest().validate_config(config)?;
    c.list(config, cursor).await
}

/// Run `list` until `has_more` is false and merge the pages (handy for tests and small sources).
pub async fn list_all(c: &dyn Connector, config: &Value, cursor: Option<&str>) -> Result<Batch> {
    let mut cursor = cursor.map(str::to_owned);
    let mut out = Batch::default();
    for _ in 0..10_000 {
        let page = validated_list(c, config, cursor.as_deref()).await?;
        out.upserts.extend(page.upserts);
        out.deletes.extend(page.deletes);
        out.renames.extend(page.renames);
        cursor = page.next_cursor;
        if !page.has_more {
            break;
        }
    }
    out.next_cursor = cursor;
    Ok(out)
}
