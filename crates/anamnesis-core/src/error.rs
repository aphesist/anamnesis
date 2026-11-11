//! Error type for the core crate.

/// Everything that can go wrong in core storage.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    /// The hash chain is broken at `seq`.
    #[error("event chain broken at seq {seq}: {reason}")]
    Chain { seq: u64, reason: String },
    /// The schema on disk is newer than this build understands.
    #[error("schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
}

/// Result alias for core operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;
