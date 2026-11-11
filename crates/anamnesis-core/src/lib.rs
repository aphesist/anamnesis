//! anamnesis-core: the shared data model, the append-only event log and local storage.
//!
//! - [`model`]: types every crate speaks ([`Item`], [`Event`], [`EventKind`], ...).
//! - [`Store`]: sqlite storage (WAL) with a hash-chained event log; all projections (items,
//!   links, threads, regions, suggestions, ...) are derived from the log and can be rebuilt.
//! - [`chunk`]: normalisation, content hashing, chunking with byte spans.
//! - [`refs`]: reference extraction (`[[wikilinks]]`, Markdown links).
//! - [`canon`]: canonical json and event hashing.

pub mod canon;
pub mod chunk;
pub mod error;
pub mod model;
pub mod refs;
pub mod store;

pub use error::{Error, Result};
pub use model::*;
pub use store::*;
