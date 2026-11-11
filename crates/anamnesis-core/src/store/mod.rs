//! The sqlite-backed store: event log, projections, ingest, vectors, search.
//!
//! Every state-changing method appends one or more [`Event`]s and applies them to the
//! projection tables inside a single sqlite transaction, so the projections are always a pure
//! function of the log ([`Store::rebuild_projections`] proves it by replaying). Two tables are
//! caches that are *not* in the log: `embeddings` (recomputable from chunk text with the same
//! model) and `placements` (the 3d layout; recorded in the log only as a
//! `LayoutUpdated { placements_hash, count }` digest).

mod bundle;
mod ingest;
mod project;
mod queries;
pub(crate) mod schema;
mod search;
mod vectors;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::canon::{GENESIS_HASH, canonical_json, event_hash};
use crate::error::{Error, Result};
use crate::model::{Actor, Event, EventKind, Timestamp};

pub use bundle::{BUNDLE_FORMAT, Manifest};
pub use ingest::{ItemFilter, ItemPatch, ItemSummary, NewNote, PendingRef, UpsertOutcome};
pub use queries::{
    ChatTurnRecord, DayCount, RegionCounts, RegionRun, Resurfacing, RetrievalLabel, SpeakerCounts,
    Suggestion, SuggestionStatus, ThreadCounts, ThreadFilter, Timeline,
};
pub use schema::SCHEMA_VERSION;
pub use search::{Granularity, QueryVector, SearchQuery};
pub use vectors::{EmbeddingModelInfo, VectorHit};

use vectors::VecIndex;

/// Summary of a successful chain verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainReport {
    /// Number of events checked.
    pub events: u64,
    /// Hash of the last event, if any.
    pub head: Option<String>,
}

/// The local brain: one sqlite database (WAL mode) holding the event log and its projections.
///
/// `Store` is `Send` but not `Sync`; share it behind a `Mutex` (or one per thread, opened on
/// the same path — WAL allows concurrent readers).
pub struct Store {
    conn: Connection,
    vec_cache: Mutex<HashMap<String, Arc<VecIndex>>>,
}

/// Format a timestamp for sortable sqlite TEXT columns (microsecond precision, `Z`).
pub(crate) fn ts(t: &Timestamp) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// Parse a timestamp written by [`ts`] (or any rfc3339 string).
pub(crate) fn parse_ts(s: &str) -> Result<Timestamp> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .map_err(|e| Error::Invalid(format!("bad timestamp {s:?}: {e}")))
}

/// Current time truncated to microseconds (so it round-trips through every encoding).
pub fn now() -> Timestamp {
    let n = chrono::Utc::now();
    let micros = n.timestamp_micros();
    chrono::DateTime::from_timestamp_micros(micros).unwrap_or(n)
}

/// The serde string form of a unit enum (e.g. `ItemKind::Note` → `"note"`).
pub(crate) fn enum_str<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

pub(crate) fn parse_enum<T: DeserializeOwned>(s: &str) -> Result<T> {
    Ok(serde_json::from_value(Value::String(s.to_string()))?)
}

/// The serde `type` tag of an event kind, e.g. `"item_created"`.
pub fn kind_type(kind: &EventKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// A write in progress: events appended here are applied to the projections immediately and
/// committed (or rolled back) together.
pub(crate) struct Writer<'a> {
    pub(crate) tx: &'a Connection,
    pub(crate) events: Vec<Event>,
    pub(crate) vectors_dirty: bool,
}

impl Writer<'_> {
    /// Append an event to the log and apply it to the projections.
    pub(crate) fn append(&mut self, actor: &Actor, kind: EventKind) -> Result<Event> {
        let head: Option<(i64, String)> = self
            .tx
            .query_row(
                "SELECT seq, hash FROM events ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (seq, prev_hash) = match head {
            Some((s, h)) => (s as u64 + 1, h),
            None => (1, GENESIS_HASH.to_string()),
        };
        let at = now();
        let hash = event_hash(&prev_hash, seq, &at, actor, &kind)?;
        let ev = Event {
            seq,
            at,
            actor: actor.clone(),
            kind,
            prev_hash,
            hash,
        };
        insert_event(self.tx, &ev)?;
        project::apply(self.tx, &ev)?;
        if matches!(
            ev.kind,
            EventKind::ItemCreated { .. }
                | EventKind::ItemUpdated { .. }
                | EventKind::ItemDeleted { .. }
        ) {
            self.vectors_dirty = true;
        }
        self.events.push(ev.clone());
        Ok(ev)
    }
}

pub(crate) fn insert_event(conn: &Connection, ev: &Event) -> Result<()> {
    let at = match serde_json::to_value(ev.at)? {
        Value::String(s) => s,
        other => other.to_string(),
    };
    conn.execute(
        "INSERT INTO events (seq, at, actor, kind_type, kind, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            ev.seq as i64,
            at,
            canonical_json(&serde_json::to_value(&ev.actor)?),
            kind_type(&ev.kind),
            canonical_json(&serde_json::to_value(&ev.kind)?),
            ev.prev_hash,
            ev.hash
        ],
    )?;
    Ok(())
}

type RawEvent = (i64, String, String, String, String, String);

fn raw_to_event(raw: RawEvent) -> Result<Event> {
    let (seq, at, actor, kind, prev_hash, hash) = raw;
    Ok(Event {
        seq: seq as u64,
        at: serde_json::from_value(Value::String(at))?,
        actor: serde_json::from_str(&actor)?,
        kind: serde_json::from_str(&kind)?,
        prev_hash,
        hash,
    })
}

pub(crate) fn read_events(
    conn: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(sql)?;
    let raws = stmt
        .query_map(p, |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<RawEvent>>>()?;
    raws.into_iter().map(raw_to_event).collect()
}

const EVENT_COLS: &str = "seq, at, actor, kind, prev_hash, hash";

impl Store {
    /// Open (or create) a store at `path`, enable WAL and run migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// Open a fresh in-memory store (tests, previews).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&conn)?;
        Ok(Store {
            conn,
            vec_cache: Mutex::new(HashMap::new()),
        })
    }

    /// The schema version of this database.
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Raw connection, for read-only ad-hoc queries (e.g. diagnostics). Writing through it
    /// bypasses the event log and breaks derivability.
    #[doc(hidden)]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn write<T>(&self, f: impl FnOnce(&mut Writer<'_>) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let mut w = Writer {
            tx: &tx,
            events: Vec::new(),
            vectors_dirty: false,
        };
        let out = f(&mut w)?;
        let dirty = w.vectors_dirty;
        tx.commit()?;
        if dirty {
            self.invalidate_vectors();
        }
        Ok(out)
    }

    pub(crate) fn invalidate_vectors(&self) {
        if let Ok(mut c) = self.vec_cache.lock() {
            c.clear();
        }
    }

    /// Append a single event (and apply it) in its own transaction. Use this for events that
    /// have no dedicated method (`Egress`, `ChatTurn`, ...). Prefer the typed methods for items,
    /// sources and links: they also maintain written links and deduplication.
    pub fn append(&self, actor: Actor, kind: EventKind) -> Result<Event> {
        self.write(|w| w.append(&actor, kind))
    }

    /// The last event's `(seq, hash)`.
    pub fn head(&self) -> Result<Option<(u64, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT seq, hash FROM events ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
            )
            .optional()?)
    }

    /// Number of events in the log.
    pub fn event_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?
            as u64)
    }

    /// Up to `limit` events with `seq > after`, in order.
    pub fn events_after(&self, after: u64, limit: usize) -> Result<Vec<Event>> {
        read_events(
            &self.conn,
            &format!("SELECT {EVENT_COLS} FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2"),
            params![after as i64, limit as i64],
        )
    }

    /// Every event, in order (for eval; prefer [`Store::events_after`] for paging).
    pub fn all_events(&self) -> Result<Vec<Event>> {
        read_events(
            &self.conn,
            &format!("SELECT {EVENT_COLS} FROM events ORDER BY seq"),
            [],
        )
    }

    /// Events of the given serde `type` tags (e.g. `["egress"]`), newest first.
    pub fn events_of_type(&self, types: &[&str], limit: usize) -> Result<Vec<Event>> {
        let mut out = Vec::new();
        for t in types {
            out.extend(read_events(
                &self.conn,
                &format!("SELECT {EVENT_COLS} FROM events WHERE kind_type = ?1 ORDER BY seq DESC LIMIT ?2"),
                params![t, limit as i64],
            )?);
        }
        out.sort_by_key(|e| std::cmp::Reverse(e.seq));
        out.truncate(limit);
        Ok(out)
    }

    /// Recompute every hash and check the links between events. Returns
    /// [`Error::Chain`] at the first broken event (edited payload, edited hash, gap or reorder).
    pub fn verify_chain(&self) -> Result<ChainReport> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {EVENT_COLS} FROM events ORDER BY seq"))?;
        let mut rows = stmt.query([])?;
        let mut prev = GENESIS_HASH.to_string();
        let mut expected_seq = 1u64;
        let mut n = 0u64;
        while let Some(r) = rows.next()? {
            let raw: RawEvent = (
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            );
            let seq = raw.0 as u64;
            let ev = raw_to_event(raw).map_err(|e| Error::Chain {
                seq,
                reason: format!("unreadable: {e}"),
            })?;
            check_link(&ev, expected_seq, &prev)?;
            prev = ev.hash.clone();
            expected_seq += 1;
            n += 1;
        }
        Ok(ChainReport {
            events: n,
            head: (n > 0).then_some(prev),
        })
    }

    /// Drop every projection table and rebuild it by replaying the log. Caches (`embeddings`,
    /// `placements`) are kept.
    pub fn rebuild_projections(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for t in schema::PROJECTION_TABLES {
            tx.execute(&format!("DELETE FROM {t}"), [])?;
        }
        let mut last = 0u64;
        loop {
            let batch = read_events(
                &tx,
                &format!("SELECT {EVENT_COLS} FROM events WHERE seq > ?1 ORDER BY seq LIMIT 1000"),
                params![last as i64],
            )?;
            if batch.is_empty() {
                break;
            }
            for ev in &batch {
                project::apply(&tx, ev)?;
                last = ev.seq;
            }
        }
        tx.commit()?;
        self.invalidate_vectors();
        Ok(())
    }

    /// A deterministic text dump of every projection table, used to prove replay equivalence.
    #[doc(hidden)]
    pub fn projection_snapshot(&self) -> Result<String> {
        let mut out = String::new();
        for t in schema::PROJECTION_TABLES {
            let sql = match *t {
                // rowids are an implementation detail; compare by chunk id
                "chunks" => "SELECT id, item_id, ord, span_start, span_end, text, text_hash FROM chunks".to_string(),
                "chunks_fts" => "SELECT c.id, f.title, f.text FROM chunks_fts f JOIN chunks c ON c.rowid = f.rowid".to_string(),
                t => format!("SELECT * FROM {t}"),
            };
            let mut stmt = self.conn.prepare(&sql)?;
            let cols = stmt.column_count();
            let mut rows: Vec<String> = stmt
                .query_map([], |r| {
                    let mut s = Vec::with_capacity(cols);
                    for i in 0..cols {
                        let v: rusqlite::types::Value = r.get(i)?;
                        s.push(format!("{v:?}"));
                    }
                    Ok(s.join("|"))
                })?
                .collect::<rusqlite::Result<_>>()?;
            rows.sort();
            out.push_str(&format!("## {t} ({})\n", rows.len()));
            for r in rows {
                out.push_str(&r);
                out.push('\n');
            }
        }
        Ok(out)
    }
}

pub(crate) fn check_link(ev: &Event, expected_seq: u64, prev: &str) -> Result<()> {
    let seq = ev.seq;
    if seq != expected_seq {
        return Err(Error::Chain {
            seq,
            reason: format!("expected seq {expected_seq}"),
        });
    }
    if ev.prev_hash != prev {
        return Err(Error::Chain {
            seq,
            reason: "prev_hash does not match previous event".into(),
        });
    }
    let h = event_hash(&ev.prev_hash, ev.seq, &ev.at, &ev.actor, &ev.kind)?;
    if h != ev.hash {
        return Err(Error::Chain {
            seq,
            reason: "hash does not match content".into(),
        });
    }
    Ok(())
}
