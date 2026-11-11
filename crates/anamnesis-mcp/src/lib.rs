//! anamnesis-mcp: the brain as a model context protocol server.
//!
//! Any harness that speaks mcp (Claude Code, Claude Desktop, Codex CLI,
//! Gemini CLI...) can read the brain and append to it. The server is generic
//! over [`BrainApi`], which the `anamnesis` server crate implements on top of
//! core storage, so this crate has no storage dependency.
//!
//! - [`McpServer`]: tools, resources and instructions.
//! - [`serve_stdio`], [`http_router`], [`serve_http`]: transports.
//! - [`config_snippets`]: ready-to-paste client configuration.
//!
//! Safety: there is no tool that deletes data and none that fetches urls or
//! sends data anywhere. Every write is recorded as `Actor::Mcp(client)`.

pub mod api;
pub mod server;
pub mod snippets;
pub mod transport;

pub use api::{
    BrainApi, BrainError, BrainResult, EgressNote, ItemSummary, ItemView, LinkSuggestion,
    Neighbour, NewCapture, RegionView, Related, SearchHit,
};
pub use server::{McpServer, SERVER_INSTRUCTIONS, normalise_client_name};
pub use snippets::{ConfigSnippets, SnippetOptions, config_snippets};
pub use transport::{HttpOptions, http_router, serve_http, serve_stdio};

/// Names of every tool this server exposes. None of them deletes data.
pub const TOOL_NAMES: &[&str] = &[
    "search",
    "get_item",
    "related",
    "list_threads",
    "region_overview",
    "timeline",
    "recent",
    "capture",
    "link",
    "update_thread",
];
