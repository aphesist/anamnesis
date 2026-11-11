//! Error type shared by every connector.

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    /// The source config does not match the connector's `config_schema`.
    #[error("invalid config: {}", .0.join("; "))]
    InvalidConfig(Vec<String>),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("unknown connector: {0}")]
    UnknownConnector(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("http: {0}")]
    Http(String),
    #[error("parse: {0}")]
    Parse(String),
    /// The cursor could not be read; callers may drop it and resync from scratch.
    #[error("bad cursor: {0}")]
    BadCursor(String),
    #[error("plugin: {0}")]
    Plugin(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl ConnectorError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        ConnectorError::Io { path: path.into(), source }
    }
}

impl From<reqwest::Error> for ConnectorError {
    fn from(e: reqwest::Error) -> Self {
        ConnectorError::Http(e.to_string())
    }
}

pub type Result<T, E = ConnectorError> = std::result::Result<T, E>;
