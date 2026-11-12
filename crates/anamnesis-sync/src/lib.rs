//! anamnesis-sync: the user's brain on several devices, through storage the user
//! manages (a folder their own cloud drive syncs, or an s3-compatible bucket), end-to-end
//! encrypted. There is no anamnesis account and no anamnesis server.
//!
//! - [`crypto`]: passphrase → key (argon2id), files sealed with XChaCha20-Poly1305.
//! - [`storage`]: the sync root as a folder ([`FolderStorage`]) or a bucket ([`S3Storage`]).
//! - [`format`]: the documented layout of a sync root and its batch files.
//! - [`hlc`]: hybrid logical clocks, the order every device merges in.
//! - [`engine`]: one sync pass: inbox, export, import, conflicts, unlinking.
//!
//! The event log is synced, never the sqlite file: each device publishes its own
//! hash-chained segment and replays the others'.

pub mod crypto;
pub mod engine;
pub mod error;
pub mod format;
pub mod hlc;
pub mod s3;
pub mod storage;

pub use crypto::{KdfParams, Key, Vault};
pub use engine::{
    Conflict, ConflictVersion, Resolution, SyncReport, SyncState, create_or_open, read_vault,
    resolve, sync, unlink,
};
pub use error::{Result, SyncError};
pub use s3::{S3Config, S3Storage};
pub use storage::{FolderStorage, Storage};
