//! Portable bundles: a manifest line followed by every event as one json line.
//!
//! Items, links, threads... are all projections of the events, so the bundle is the whole
//! brain. Caches (embeddings, the 3d layout) are not included and are recomputed after import.

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

use super::{EVENT_COLS, Store, check_link, insert_event, now, project, read_events, schema};
use crate::canon::GENESIS_HASH;
use crate::error::{Error, Result};
use crate::model::{Event, Timestamp};

/// `format` value of a bundle manifest.
pub const BUNDLE_FORMAT: &str = "anamnesis-bundle";

/// First line of a bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub schema_version: i64,
    pub exported_at: Timestamp,
    pub events: u64,
    /// Hash of the last event (`None` for an empty brain).
    pub head_hash: Option<String>,
}

impl Store {
    /// Write the whole brain as a bundle (JSON lines: manifest, then events in order).
    pub fn export_bundle<W: Write>(&self, mut w: W) -> Result<Manifest> {
        let head = self.head()?;
        let manifest = Manifest {
            format: BUNDLE_FORMAT.into(),
            version: 1,
            schema_version: schema::SCHEMA_VERSION,
            exported_at: now(),
            events: head.as_ref().map(|h| h.0).unwrap_or(0),
            head_hash: head.map(|h| h.1),
        };
        serde_json::to_writer(&mut w, &manifest)?;
        w.write_all(b"\n")?;
        let mut last = 0u64;
        loop {
            let batch = read_events(
                &self.conn,
                &format!(
                    "SELECT {EVENT_COLS} FROM events WHERE seq > ?1 AND seq <= ?2 ORDER BY seq LIMIT 1000"
                ),
                rusqlite::params![last as i64, manifest.events as i64],
            )?;
            if batch.is_empty() {
                break;
            }
            for ev in &batch {
                serde_json::to_writer(&mut w, ev)?;
                w.write_all(b"\n")?;
                last = ev.seq;
            }
        }
        w.flush()?;
        Ok(manifest)
    }

    /// Import a bundle into an *empty* store by replaying its events. Every event's hash and
    /// link is verified; on any error nothing is imported.
    pub fn import_bundle<R: BufRead>(&self, r: R) -> Result<Manifest> {
        if self.event_count()? > 0 {
            return Err(Error::Invalid("import needs an empty store".into()));
        }
        let mut lines = r.lines();
        let first = lines
            .next()
            .ok_or_else(|| Error::Invalid("empty bundle".into()))??;
        let manifest: Manifest = serde_json::from_str(&first)?;
        if manifest.format != BUNDLE_FORMAT {
            return Err(Error::Invalid(format!(
                "not an anamnesis bundle: {}",
                manifest.format
            )));
        }
        if manifest.version != 1 {
            return Err(Error::Invalid(format!(
                "unsupported bundle version {}",
                manifest.version
            )));
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut prev = GENESIS_HASH.to_string();
        let mut n = 0u64;
        for line in lines {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let ev: Event = serde_json::from_str(&line)?;
            check_link(&ev, n + 1, &prev)?;
            insert_event(&tx, &ev)?;
            project::apply(&tx, &ev)?;
            prev = ev.hash;
            n += 1;
        }
        if n != manifest.events || (n > 0 && manifest.head_hash.as_deref() != Some(prev.as_str())) {
            return Err(Error::Invalid(format!(
                "bundle truncated or inconsistent: manifest says {} events, found {n}",
                manifest.events
            )));
        }
        tx.commit()?;
        self.invalidate_vectors();
        Ok(manifest)
    }
}
