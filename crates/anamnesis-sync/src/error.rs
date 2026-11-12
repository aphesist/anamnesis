use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("storage: {0}")]
    Io(String),
    #[error("the passphrase does not open this sync vault")]
    WrongPassphrase,
    #[error("the stored sync key is not valid")]
    BadKey,
    #[error("encryption failed")]
    Crypto,
    /// A file failed authentication, or a device's chain does not link up.
    #[error("tampered or damaged: {0}")]
    Tampered(String),
    #[error("format: {0}")]
    Format(String),
    #[error("{0}")]
    Invalid(String),
    #[error("this device was unlinked from the vault by {by}")]
    Unlinked { by: String },
    #[error("store: {0}")]
    Store(#[from] anamnesis_core::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SyncError>;

impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        SyncError::Io(e.to_string())
    }
}
