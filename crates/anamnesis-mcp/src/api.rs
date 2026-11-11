//! The seam between the mcp server and the brain's storage.
//!
//! `anamnesis-mcp` has no storage dependency. The server crate implements
//! [`BrainApi`] on top of core storage (and mind, for neighbours and link
//! suggestions) and hands an `Arc` of it to [`crate::McpServer`].
//!
//! Everything here is append-only: there is deliberately no method that
//! deletes or overwrites data, and none that reaches the network.

use anamnesis_core::{
    Actor, Hit, Id, Item, ItemKind, Link, Origin, Privacy, Region, Thread, ThreadKind,
    ThreadStatus, Timestamp,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Errors a [`BrainApi`] implementation reports. They are shown to the calling
/// harness as tool errors, so messages should be short and contain no secrets.
#[derive(Debug, thiserror::Error)]
pub enum BrainError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for BrainError {
    fn from(e: anyhow::Error) -> Self {
        BrainError::Internal(e.to_string())
    }
}

pub type BrainResult<T> = Result<T, BrainError>;

/// A compact view of an item, used in lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemSummary {
    pub id: Id,
    pub title: String,
    pub kind: ItemKind,
    pub origin: Origin,
    pub privacy: Privacy,
    pub created_at: Timestamp,
    /// Region the item currently belongs to, if any.
    pub region_id: Option<Id>,
    /// A short excerpt of the body (the implementation chooses the length).
    pub snippet: String,
}

impl ItemSummary {
    /// Builds a summary from a full item, with a snippet of at most `max_chars` characters.
    pub fn from_item(item: &Item, region_id: Option<Id>, max_chars: usize) -> Self {
        ItemSummary {
            id: item.id.clone(),
            title: item.title.clone(),
            kind: item.kind,
            origin: item.origin,
            privacy: item.privacy,
            created_at: item.created_at,
            region_id,
            snippet: item.body.chars().take(max_chars).collect(),
        }
    }
}

/// A search hit together with the provenance needed to present it safely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    #[serde(flatten)]
    pub hit: Hit,
    pub kind: ItemKind,
    pub origin: Origin,
    pub privacy: Privacy,
}

/// An item with its surroundings (same shape as `GET /api/items/:id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemView {
    pub item: Item,
    pub links_out: Vec<Link>,
    pub links_in: Vec<Link>,
    pub threads: Vec<Thread>,
    pub region: Option<Region>,
}

/// A semantic neighbour of an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Neighbour {
    pub item: ItemSummary,
    pub score: f32,
}

/// A link suggestion as shown to the caller.
///
/// The implementation is responsible for the evaluation protocol: it must only
/// return suggestions that were recorded as shown (`SuggestionMade { shown: true }`)
/// and never leak the held-out control arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkSuggestion {
    pub suggestion_id: Id,
    pub to: ItemSummary,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Related {
    pub neighbours: Vec<Neighbour>,
    pub suggestions: Vec<LinkSuggestion>,
}

/// A region with a few representative items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionView {
    pub region: Region,
    /// Representative items (closest to the centroid, or most recent).
    pub items: Vec<ItemSummary>,
}

/// Something to append to the brain via the `capture` tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewCapture {
    pub title: String,
    pub body: String,
    pub kind: ItemKind,
    pub tags: Vec<String>,
    pub origin: Origin,
    /// Normalised client name (also carried in the `Actor::Mcp`), for the item's source/meta.
    pub client: String,
}

/// A record that brain content left anamnesis through mcp, towards a harness
/// that may forward it to a cloud model. Counts only, never text.
///
/// The server should turn this into an `EventKind::Egress`
/// (provider = `mcp:<client>`, model = "unknown", purpose = the tool name).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EgressNote {
    pub client: String,
    pub purpose: String,
    pub items: Vec<Id>,
    pub chars: u64,
}

/// The brain as seen by the mcp server. Implemented by the server crate.
///
/// Every write takes the [`Actor`] that caused it (always `Actor::Mcp(client)`
/// from this crate) so the implementation can record it in the event log.
#[async_trait]
pub trait BrainApi: Send + Sync + 'static {
    /// Hybrid (full-text + vector) search, best first, at most `k` hits.
    async fn search(&self, query: &str, k: usize) -> BrainResult<Vec<SearchHit>>;

    /// One item with links, threads and region. `Ok(None)` if unknown or deleted.
    async fn get_item(&self, id: &Id) -> BrainResult<Option<ItemView>>;

    /// Up to `k` semantic neighbours and shown link suggestions for an item.
    async fn related(&self, id: &Id, k: usize) -> BrainResult<Related>;

    /// Decisions, open questions, plans and rejections, optionally filtered.
    async fn list_threads(
        &self,
        kind: Option<ThreadKind>,
        status: Option<ThreadStatus>,
    ) -> BrainResult<Vec<Thread>>;

    /// All living regions (label, size, birth...).
    async fn regions(&self) -> BrainResult<Vec<Region>>;

    /// One region with representative items. `Ok(None)` if unknown.
    async fn get_region(&self, id: &Id) -> BrainResult<Option<RegionView>>;

    /// Items written between `from` and `to` (inclusive, either open), oldest
    /// first, at most `limit`.
    async fn timeline(
        &self,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
        limit: usize,
    ) -> BrainResult<Vec<ItemSummary>>;

    /// The `n` most recently captured items, newest first.
    async fn recent(&self, n: usize) -> BrainResult<Vec<ItemSummary>>;

    /// Appends a new item. Never overwrites an existing one.
    async fn capture(&self, actor: Actor, capture: NewCapture) -> BrainResult<Item>;

    /// Adds a manual link (`LinkKind::Manual`). `note` may be kept in the event or item meta.
    async fn link(
        &self,
        actor: Actor,
        from: &Id,
        to: &Id,
        note: Option<String>,
    ) -> BrainResult<Link>;

    /// Moves a thread through its lifecycle (appends `ThreadUpdated`).
    async fn update_thread(
        &self,
        actor: Actor,
        id: &Id,
        status: ThreadStatus,
        resolved_by: Option<Id>,
    ) -> BrainResult<Thread>;

    /// Called after a read tool returned brain content to a harness. Default: no-op.
    async fn note_egress(&self, _actor: Actor, _note: EgressNote) -> BrainResult<()> {
        Ok(())
    }
}
