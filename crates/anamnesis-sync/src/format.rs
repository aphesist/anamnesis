//! The files of a sync root (see docs/sync.md):
//!
//! ```text
//! <root>/
//!   vault.json                              plain: format, kdf parameters, passphrase check
//!   devices/<device id>/device.anb          sealed: the device's name, platform, last sync
//!   devices/<device id>/unlinked.anb        sealed: present once another device unlinked it
//!   devices/<device id>/batches/<first>-<last>.anb
//!                                           sealed: the device's events <first>..=<last>
//!   inbox/                                  plain: drop .md / .txt files here
//!   inbox/imported/<yyyy-mm-dd>/            files already taken in
//! ```
//!
//! A batch's plaintext is json lines: one [`SyncEvent`] per line. Every device numbers
//! its own events 1, 2, 3… and chains them: `hash = sha256(prev ‖ canonical json of the
//! event without its hash)`, `prev` of the first is 64 zeros. A gap, a reordering or a
//! replaced batch breaks the chain.

use anamnesis_core::canon::canonical_json;
use anamnesis_core::{Actor, EventKind, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, SyncError};
use crate::hlc::Hlc;

pub const VAULT_FILE: &str = "vault.json";
pub const DEVICES_DIR: &str = "devices";
pub const INBOX_DIR: &str = "inbox";
pub const INBOX_IMPORTED: &str = "inbox/imported";
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One event of a device's own chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncEvent {
    /// The device that recorded it.
    pub device: String,
    /// Position in that device's chain, from 1.
    pub dseq: u64,
    pub hlc: Hlc,
    /// When it happened on the device (the store's own timestamp).
    pub at: Timestamp,
    pub actor: Actor,
    pub kind: EventKind,
    /// For item events: the item versions this one replaces (empty for a creation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    pub prev: String,
    pub hash: String,
}

impl SyncEvent {
    /// The chain hash of this event (over everything but `hash`).
    pub fn compute_hash(&self) -> Result<String> {
        let mut v = serde_json::to_value(self)?;
        if let Some(o) = v.as_object_mut() {
            o.remove("hash");
        }
        let mut h = Sha256::new();
        h.update(self.prev.as_bytes());
        h.update(canonical_json(&v).as_bytes());
        Ok(hex::encode(h.finalize()))
    }
}

/// The sealed `device.anb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub created_at: Timestamp,
    pub last_sync_at: Timestamp,
    /// The last event of its chain it has published.
    pub dseq: u64,
}

/// The sealed `unlinked.anb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnlinkRecord {
    pub by_device: String,
    pub by_name: String,
    pub at: Timestamp,
    /// Events of the unlinked device up to this one are kept; later ones are ignored.
    pub up_to_dseq: u64,
}

pub fn device_dir(id: &str) -> String {
    format!("{DEVICES_DIR}/{id}")
}
pub fn device_file(id: &str) -> String {
    format!("{DEVICES_DIR}/{id}/device.anb")
}
pub fn unlinked_file(id: &str) -> String {
    format!("{DEVICES_DIR}/{id}/unlinked.anb")
}
pub fn batch_file(id: &str, first: u64, last: u64) -> String {
    format!("{DEVICES_DIR}/{id}/batches/{first:012}-{last:012}.anb")
}

/// `(first, last)` of a batch path, if it is one.
pub fn parse_batch(path: &str) -> Option<(String, u64, u64)> {
    let rest = path.strip_prefix(&format!("{DEVICES_DIR}/"))?;
    let (dev, file) = rest.split_once("/batches/")?;
    let (a, b) = file.strip_suffix(".anb")?.split_once('-')?;
    Some((dev.to_string(), a.parse().ok()?, b.parse().ok()?))
}

/// Serialise a batch's events as json lines.
pub fn encode_batch(events: &[SyncEvent]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for e in events {
        out.extend_from_slice(serde_json::to_string(e)?.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

pub fn decode_batch(bytes: &[u8]) -> Result<Vec<SyncEvent>> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| SyncError::Format("batch is not utf-8".into()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(SyncError::from))
        .collect()
}

/// Check that `events` continue a device's chain from `(dseq, hash)`, in order.
pub fn verify_chain(
    device: &str,
    mut dseq: u64,
    mut prev: String,
    events: &[SyncEvent],
) -> Result<(u64, String)> {
    for e in events {
        if e.device != device {
            return Err(SyncError::Tampered(format!(
                "device {device}: an event claims to be from {}",
                e.device
            )));
        }
        if e.dseq != dseq + 1 {
            return Err(SyncError::Tampered(format!(
                "device {device}: event {} follows {dseq}",
                e.dseq
            )));
        }
        if e.prev != prev || e.compute_hash()? != e.hash {
            return Err(SyncError::Tampered(format!(
                "device {device}: event {} does not link to the chain",
                e.dseq
            )));
        }
        dseq = e.dseq;
        prev = e.hash.clone();
    }
    Ok((dseq, prev))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_names_round_trip() {
        let p = batch_file("01ABC", 1, 42);
        assert_eq!(p, "devices/01ABC/batches/000000000001-000000000042.anb");
        assert_eq!(parse_batch(&p), Some(("01ABC".into(), 1, 42)));
        assert_eq!(parse_batch("devices/01ABC/device.anb"), None);
        assert_eq!(parse_batch("inbox/x.md"), None);
    }
}
