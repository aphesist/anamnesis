//! Applying events to the projection tables. `apply` must be a pure function of the current
//! projection state and the event, so that replaying the log reproduces the live state.

use rusqlite::{Connection, OptionalExtension, params};

use super::{enum_str, ts};
use crate::chunk::{chunk_item, text_hash};
use crate::error::Result;
use crate::model::{Event, EventKind, Id, Item};
use crate::refs::{item_keys, ref_candidates};

pub(crate) fn apply(conn: &Connection, ev: &Event) -> Result<()> {
    let at = ts(&ev.at);
    match &ev.kind {
        EventKind::SourceAdded { source } | EventKind::SourceUpdated { source } => {
            conn.execute(
                "INSERT INTO sources (id, connector, json) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET connector = excluded.connector, json = excluded.json",
                params![source.id.0, source.connector, serde_json::to_string(source)?],
            )?;
        }
        EventKind::SourceRemoved { source_id } => {
            conn.execute("DELETE FROM sources WHERE id = ?1", params![source_id.0])?;
        }
        EventKind::ItemCreated { item } | EventKind::ItemUpdated { item } => put_item(conn, item)?,
        EventKind::ItemDeleted { item_id } => {
            let json: Option<String> = conn
                .query_row(
                    "SELECT json FROM items WHERE id = ?1",
                    params![item_id.0],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(json) = json {
                let mut item: Item = serde_json::from_str(&json)?;
                item.deleted = true;
                put_item(conn, &item)?;
            }
        }
        EventKind::RefsSet { item_id, refs } => {
            let eid: String = conn
                .query_row(
                    "SELECT external_id FROM items WHERE id = ?1",
                    params![item_id.0],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default();
            conn.execute(
                "DELETE FROM item_refs WHERE item_id = ?1",
                params![item_id.0],
            )?;
            let mut stmt = conn.prepare_cached(
                "INSERT OR IGNORE INTO item_refs (item_id, ord, ref, cand, key) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (ord, r) in refs.iter().enumerate() {
                for (cand, key) in ref_candidates(&eid, r).iter().enumerate() {
                    stmt.execute(params![item_id.0, ord as i64, r, cand as i64, key])?;
                }
            }
        }
        EventKind::LinkAdded { link } => {
            conn.execute(
                "INSERT OR IGNORE INTO links (from_id, to_id, kind, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![link.from.0, link.to.0, enum_str(&link.kind), ts(&link.created_at)],
            )?;
        }
        EventKind::LinkRemoved { from, to } => {
            conn.execute(
                "DELETE FROM links WHERE from_id = ?1 AND to_id = ?2",
                params![from.0, to.0],
            )?;
        }
        EventKind::SuggestionMade {
            suggestion_id,
            from,
            to,
            score,
            rank,
            shown,
            model,
        } => {
            conn.execute(
                "INSERT OR REPLACE INTO suggestions (id, from_id, to_id, score, rank, shown, model, made_at, status, decided_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL)",
                params![suggestion_id.0, from.0, to.0, *score as f64, *rank as i64, *shown, model, at],
            )?;
        }
        EventKind::SuggestionAccepted { suggestion_id } => {
            conn.execute(
                "UPDATE suggestions SET status = 'accepted', decided_at = ?2 WHERE id = ?1",
                params![suggestion_id.0, at],
            )?;
        }
        EventKind::SuggestionDismissed { suggestion_id } => {
            conn.execute(
                "UPDATE suggestions SET status = 'dismissed', decided_at = ?2 WHERE id = ?1",
                params![suggestion_id.0, at],
            )?;
        }
        EventKind::SuggestionDisplayed { suggestion_ids } => {
            let mut stmt = conn.prepare_cached(
                "UPDATE suggestions SET displayed_at = ?2 WHERE id = ?1 AND displayed_at IS NULL",
            )?;
            for id in suggestion_ids {
                stmt.execute(params![id.0, at])?;
            }
        }
        EventKind::ThreadRemoved { thread_id } => {
            conn.execute("DELETE FROM threads WHERE id = ?1", params![thread_id.0])?;
        }
        EventKind::ThreadAdded { thread } | EventKind::ThreadUpdated { thread } => {
            conn.execute(
                "INSERT INTO threads (id, kind, status, item_id, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, status = excluded.status,
                   item_id = excluded.item_id, created_at = excluded.created_at, json = excluded.json",
                params![
                    thread.id.0,
                    enum_str(&thread.kind),
                    enum_str(&thread.status),
                    thread.item_id.0,
                    ts(&thread.created_at),
                    serde_json::to_string(thread)?
                ],
            )?;
        }
        EventKind::RegionsRecomputed {
            run_id,
            regions,
            transitions,
        } => {
            conn.execute(
                "INSERT OR REPLACE INTO region_runs (run_id, seq, at, regions, transitions) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    run_id.0,
                    ev.seq as i64,
                    at,
                    serde_json::to_string(regions)?,
                    serde_json::to_string(transitions)?
                ],
            )?;
            for r in regions {
                conn.execute(
                    "INSERT OR REPLACE INTO regions (id, run_id, json) VALUES (?1, ?2, ?3)",
                    params![r.id.0, run_id.0, serde_json::to_string(r)?],
                )?;
            }
        }
        EventKind::LayoutUpdated {
            run_id,
            placements_hash,
            count,
        } => {
            conn.execute(
                "INSERT OR REPLACE INTO layout_runs (run_id, seq, at, placements_hash, count) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![run_id.0, ev.seq as i64, at, placements_hash, *count as i64],
            )?;
        }
        EventKind::ResurfaceShown {
            resurface_id,
            item_id,
            arm,
        } => {
            conn.execute(
                "INSERT OR REPLACE INTO resurfacings (id, item_id, arm, shown_at, useful, rated_at) VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
                params![resurface_id.0, item_id.0, arm, at],
            )?;
        }
        EventKind::ResurfaceRated {
            resurface_id,
            useful,
        } => {
            conn.execute(
                "UPDATE resurfacings SET useful = ?2, rated_at = ?3 WHERE id = ?1",
                params![resurface_id.0, *useful, at],
            )?;
        }
        EventKind::RetrievalLabeled {
            query,
            item_id,
            grade,
        } => {
            conn.execute(
                "INSERT OR REPLACE INTO retrieval_labels (query, item_id, grade, at) VALUES (?1, ?2, ?3, ?4)",
                params![query, item_id.0, *grade as i64, at],
            )?;
        }
        EventKind::Egress { .. } => {} // read straight from the log
        EventKind::ChatTurn {
            chat_id,
            role,
            item_id,
            cited,
        } => {
            conn.execute(
                "INSERT OR REPLACE INTO chat_turns (seq, chat_id, role, item_id, cited, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![ev.seq as i64, chat_id.0, role, item_id.0, serde_json::to_string(cited)?, at],
            )?;
        }
        EventKind::SettingSet { key, value } => {
            if value.is_null() {
                conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
            } else {
                conn.execute(
                    "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                    params![key, serde_json::to_string(value)?],
                )?;
            }
        }
    }
    Ok(())
}

fn put_item(conn: &Connection, item: &Item) -> Result<()> {
    conn.execute(
        "INSERT INTO items (id, source_id, external_id, kind, origin, title, created_at, modified_at, content_hash, deleted, json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET source_id = excluded.source_id, external_id = excluded.external_id,
           kind = excluded.kind, origin = excluded.origin, title = excluded.title, created_at = excluded.created_at,
           modified_at = excluded.modified_at, content_hash = excluded.content_hash, deleted = excluded.deleted,
           json = excluded.json",
        params![
            item.id.0,
            item.source_id.0,
            item.external_id,
            enum_str(&item.kind),
            enum_str(&item.origin),
            item.title,
            ts(&item.created_at),
            ts(&item.modified_at),
            item.content_hash,
            item.deleted,
            serde_json::to_string(item)?
        ],
    )?;
    conn.execute(
        "DELETE FROM item_keys WHERE item_id = ?1",
        params![item.id.0],
    )?;
    delete_chunks(conn, &item.id)?;
    if item.deleted {
        return Ok(());
    }
    {
        let mut stmt =
            conn.prepare_cached("INSERT OR IGNORE INTO item_keys (key, item_id) VALUES (?1, ?2)")?;
        for k in item_keys(item) {
            stmt.execute(params![k, item.id.0])?;
        }
    }
    let mut ins = conn.prepare_cached(
        "INSERT INTO chunks (id, item_id, ord, span_start, span_end, text, text_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut fts =
        conn.prepare_cached("INSERT INTO chunks_fts (rowid, title, text) VALUES (?1, ?2, ?3)")?;
    for c in chunk_item(&item.id, &item.body) {
        ins.execute(params![
            c.id.0,
            c.item_id.0,
            c.ord as i64,
            c.start as i64,
            c.end as i64,
            c.text,
            text_hash(&c.text)
        ])?;
        let rowid = conn.last_insert_rowid();
        fts.execute(params![rowid, item.title, c.text])?;
    }
    Ok(())
}

fn delete_chunks(conn: &Connection, item_id: &Id) -> Result<()> {
    conn.execute(
        "DELETE FROM chunks_fts WHERE rowid IN (SELECT rowid FROM chunks WHERE item_id = ?1)",
        params![item_id.0],
    )?;
    conn.execute("DELETE FROM chunks WHERE item_id = ?1", params![item_id.0])?;
    Ok(())
}
