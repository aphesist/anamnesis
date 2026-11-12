//! Sources, items and written links.

use std::collections::{BTreeSet, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Store, Writer, enum_str, now, parse_ts, ts};
use crate::chunk::{content_hash, normalize_body};
use crate::error::{Error, Result};
use crate::model::{
    Actor, Document, EventKind, Id, Item, ItemKind, Link, LinkKind, Origin, Privacy, Source,
    Timestamp,
};
use crate::refs::{extract_refs, item_keys};

/// What [`Store::upsert_document`] did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UpsertOutcome {
    /// A new item was stored.
    Created(Item),
    /// The item with the same `(source_id, external_id)` changed (or was revived).
    Updated(Item),
    /// Nothing changed (only `modified_at` may differ); no event was written.
    Unchanged(Id),
    /// The normalised body is identical to an existing live item; nothing stored.
    Duplicate { of: Id },
}

/// Filters for [`Store::list_items`]. All fields are optional; results are ordered by
/// `created_at`, then id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemFilter {
    pub kind: Option<ItemKind>,
    pub origin: Option<Origin>,
    pub source_id: Option<Id>,
    /// Only items written at or after this time.
    pub since: Option<Timestamp>,
    pub include_deleted: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// A lightweight item row (no body) for clouds and lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemSummary {
    pub id: Id,
    pub source_id: Id,
    pub kind: ItemKind,
    pub origin: Origin,
    pub title: String,
    pub created_at: Timestamp,
    pub modified_at: Timestamp,
}

/// Editor edit of an item (`PUT /api/items/:id`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
    pub kind: Option<ItemKind>,
}

/// A note written in the app's editor (`POST /api/items`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewNote {
    pub title: String,
    pub body: String,
    pub kind: Option<ItemKind>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Defaults to `OwnerWritten`.
    pub origin: Option<Origin>,
}

/// A reference written in an item that does not (yet) resolve to any item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRef {
    pub item_id: Id,
    pub reference: String,
}

/// Connector id of the built-in editor source.
pub const EDITOR_CONNECTOR: &str = "editor";

// ---------- read helpers on a connection ----------

pub(crate) fn get_item_in(conn: &Connection, id: &Id) -> Result<Option<Item>> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM items WHERE id = ?1", params![id.0], |r| {
            r.get(0)
        })
        .optional()?;
    json.map(|j| serde_json::from_str(&j).map_err(Error::from))
        .transpose()
}

fn get_source_in(conn: &Connection, id: &Id) -> Result<Option<Source>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT json FROM sources WHERE id = ?1",
            params![id.0],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|j| serde_json::from_str(&j).map_err(Error::from))
        .transpose()
}

fn item_by_external(conn: &Connection, source_id: &Id, external_id: &str) -> Result<Option<Item>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT json FROM items WHERE source_id = ?1 AND external_id = ?2",
            params![source_id.0, external_id],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|j| serde_json::from_str(&j).map_err(Error::from))
        .transpose()
}

fn live_item_with_hash(conn: &Connection, hash: &str) -> Result<Option<Id>> {
    Ok(conn
        .query_row(
            "SELECT id FROM items WHERE content_hash = ?1 AND deleted = 0 ORDER BY id LIMIT 1",
            params![hash],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(Id))
}

fn stored_refs(conn: &Connection, item_id: &Id) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT ord, ref FROM item_refs WHERE item_id = ?1 ORDER BY ord",
    )?;
    let rows = stmt
        .query_map(params![item_id.0], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Resolve reference number `ord` of item `from`.
fn resolve_ref(conn: &Connection, from: &Id, from_source: &str, ord: i64) -> Result<Option<Id>> {
    let mut stmt = conn.prepare_cached(
        "SELECT r.cand, k.item_id, i.source_id FROM item_refs r
         JOIN item_keys k ON k.key = r.key
         JOIN items i ON i.id = k.item_id
         WHERE r.item_id = ?1 AND r.ord = ?2 AND i.deleted = 0 AND k.item_id != ?1",
    )?;
    let best = stmt
        .query_map(params![from.0, ord], |r| {
            let cand: i64 = r.get(0)?;
            let id: String = r.get(1)?;
            let src: String = r.get(2)?;
            Ok((src != from_source, cand, id))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .min();
    Ok(best.map(|(_, _, id)| Id(id)))
}

fn ref_ords(conn: &Connection, item_id: &Id) -> Result<Vec<i64>> {
    let mut stmt =
        conn.prepare_cached("SELECT DISTINCT ord FROM item_refs WHERE item_id = ?1 ORDER BY ord")?;
    let rows = stmt
        .query_map(params![item_id.0], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn link_kinds_between(conn: &Connection, from: &Id, to: &Id) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare_cached("SELECT kind FROM links WHERE from_id = ?1 AND to_id = ?2")?;
    let rows = stmt
        .query_map(params![from.0, to.0], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn has_link(conn: &Connection, from: &Id, to: &Id, kind: LinkKind) -> Result<bool> {
    Ok(link_kinds_between(conn, from, to)?.contains(&enum_str(&kind)))
}

/// Recompute the written links of `item` (outgoing) and resolve other items' pending
/// references that now point at it (incoming). Emits `LinkAdded`/`LinkRemoved` events.
fn sync_written_links(w: &mut Writer<'_>, actor: &Actor, item: &Item) -> Result<()> {
    let conn = w.tx;
    // outgoing
    let mut targets: BTreeSet<Id> = BTreeSet::new();
    for ord in ref_ords(conn, &item.id)? {
        if let Some(t) = resolve_ref(conn, &item.id, &item.source_id.0, ord)? {
            targets.insert(t);
        }
    }
    let existing: BTreeSet<Id> = {
        let mut stmt =
            conn.prepare_cached("SELECT to_id FROM links WHERE from_id = ?1 AND kind = 'written'")?;
        stmt.query_map(params![item.id.0], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(Id)
            .collect()
    };
    let at = now();
    for t in targets.difference(&existing) {
        w.append(
            actor,
            EventKind::LinkAdded {
                link: Link {
                    from: item.id.clone(),
                    to: t.clone(),
                    kind: LinkKind::Written,
                    created_at: at,
                },
            },
        )?;
    }
    for t in existing.difference(&targets) {
        // `LinkRemoved` drops every kind; keep a written link that shares the pair with a
        // manual or accepted one rather than lose the user's own link.
        if link_kinds_between(conn, &item.id, t)?.len() == 1 {
            w.append(
                actor,
                EventKind::LinkRemoved {
                    from: item.id.clone(),
                    to: t.clone(),
                },
            )?;
        }
    }
    // incoming: references elsewhere whose keys match this item's keys
    let keys = item_keys(item);
    let mut seen: HashSet<(String, i64)> = HashSet::new();
    for k in keys {
        let rows: Vec<(String, i64, String)> = {
            let mut stmt = conn.prepare_cached(
                "SELECT DISTINCT r.item_id, r.ord, i.source_id FROM item_refs r JOIN items i ON i.id = r.item_id
                 WHERE r.key = ?1 AND r.item_id != ?2 AND i.deleted = 0 ORDER BY r.item_id, r.ord",
            )?;
            stmt.query_map(params![k, item.id.0], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        for (from, ord, src) in rows {
            if !seen.insert((from.clone(), ord)) {
                continue;
            }
            let from = Id(from);
            if resolve_ref(conn, &from, &src, ord)?.as_ref() == Some(&item.id)
                && !has_link(conn, &from, &item.id, LinkKind::Written)?
            {
                w.append(
                    actor,
                    EventKind::LinkAdded {
                        link: Link {
                            from,
                            to: item.id.clone(),
                            kind: LinkKind::Written,
                            created_at: at,
                        },
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn source_actor(source: &Source) -> Actor {
    if source.connector == EDITOR_CONNECTOR {
        Actor::User
    } else {
        Actor::Connector(source.connector.clone())
    }
}

fn upsert_in(
    w: &mut Writer<'_>,
    source: &Source,
    doc: Document,
    actor: &Actor,
    dedup: bool,
) -> Result<UpsertOutcome> {
    let conn = w.tx;
    let hash = content_hash(&doc.body);
    let existing = item_by_external(conn, &source.id, &doc.external_id)?;
    let t = now();
    match existing {
        Some(old) => {
            let item = Item {
                id: old.id.clone(),
                source_id: source.id.clone(),
                external_id: doc.external_id,
                uri: doc.uri,
                kind: doc.kind,
                title: doc.title,
                body: doc.body,
                origin: doc.origin,
                privacy: old.privacy,
                created_at: doc.created_at.unwrap_or(old.created_at),
                captured_at: old.captured_at,
                modified_at: doc.modified_at.unwrap_or(t),
                content_hash: hash,
                tags: doc.tags,
                meta: doc.meta,
                deleted: false,
            };
            let old_refs = stored_refs(conn, &old.id)?;
            let same_item = Item {
                modified_at: old.modified_at,
                ..item.clone()
            } == old;
            if same_item && old_refs == doc.refs {
                return Ok(UpsertOutcome::Unchanged(old.id));
            }
            w.append(actor, EventKind::ItemUpdated { item: item.clone() })?;
            if old_refs != doc.refs {
                w.append(
                    actor,
                    EventKind::RefsSet {
                        item_id: item.id.clone(),
                        refs: doc.refs,
                    },
                )?;
            }
            sync_written_links(w, actor, &item)?;
            Ok(UpsertOutcome::Updated(item))
        }
        None => {
            if dedup
                && !normalize_body(&doc.body).is_empty()
                && let Some(of) = live_item_with_hash(conn, &hash)?
            {
                return Ok(UpsertOutcome::Duplicate { of });
            }
            let created_at = doc.created_at.or(doc.modified_at).unwrap_or(t);
            let item = Item {
                id: Id::new(),
                source_id: source.id.clone(),
                external_id: doc.external_id,
                uri: doc.uri,
                kind: doc.kind,
                title: doc.title,
                body: doc.body,
                origin: doc.origin,
                privacy: source.default_privacy,
                created_at,
                captured_at: t,
                modified_at: doc.modified_at.unwrap_or(created_at),
                content_hash: hash,
                tags: doc.tags,
                meta: doc.meta,
                deleted: false,
            };
            w.append(actor, EventKind::ItemCreated { item: item.clone() })?;
            if !doc.refs.is_empty() {
                w.append(
                    actor,
                    EventKind::RefsSet {
                        item_id: item.id.clone(),
                        refs: doc.refs,
                    },
                )?;
            }
            sync_written_links(w, actor, &item)?;
            Ok(UpsertOutcome::Created(item))
        }
    }
}

fn read_items(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Item>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(p, |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|j| serde_json::from_str(j).map_err(Error::from))
        .collect()
}

impl Store {
    // ---------- sources ----------

    /// Register a source (`SourceAdded`). The caller builds the [`Source`] (use `Id::new()`).
    pub fn add_source(&self, source: Source) -> Result<Source> {
        self.write(|w| {
            if get_source_in(w.tx, &source.id)?.is_some() {
                return Err(Error::Invalid(format!(
                    "source {} already exists",
                    source.id
                )));
            }
            w.append(
                &Actor::User,
                EventKind::SourceAdded {
                    source: source.clone(),
                },
            )?;
            Ok(source)
        })
    }

    /// Replace a source's settings (`SourceUpdated`).
    pub fn update_source(&self, source: Source, actor: Actor) -> Result<Source> {
        self.write(|w| {
            if get_source_in(w.tx, &source.id)?.is_none() {
                return Err(Error::NotFound(format!("source {}", source.id)));
            }
            w.append(
                &actor,
                EventKind::SourceUpdated {
                    source: source.clone(),
                },
            )?;
            Ok(source)
        })
    }

    /// Store a connector's incremental-sync cursor (`SourceUpdated`).
    pub fn set_source_cursor(&self, source_id: &Id, cursor: Option<String>) -> Result<Source> {
        let mut s = self
            .get_source(source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        if s.cursor == cursor {
            return Ok(s);
        }
        s.cursor = cursor;
        let actor = source_actor(&s);
        self.update_source(s, actor)
    }

    /// Remove a source: soft-deletes all its live items (`ItemDeleted` each), then
    /// `SourceRemoved`. Returns the number of items deleted.
    pub fn remove_source(&self, source_id: &Id) -> Result<usize> {
        self.write(|w| {
            if get_source_in(w.tx, source_id)?.is_none() {
                return Err(Error::NotFound(format!("source {source_id}")));
            }
            let ids: Vec<String> = {
                let mut stmt = w.tx.prepare(
                    "SELECT id FROM items WHERE source_id = ?1 AND deleted = 0 ORDER BY id",
                )?;
                stmt.query_map(params![source_id.0], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            for id in &ids {
                w.append(
                    &Actor::User,
                    EventKind::ItemDeleted {
                        item_id: Id(id.clone()),
                    },
                )?;
            }
            w.append(
                &Actor::User,
                EventKind::SourceRemoved {
                    source_id: source_id.clone(),
                },
            )?;
            Ok(ids.len())
        })
    }

    pub fn get_source(&self, id: &Id) -> Result<Option<Source>> {
        get_source_in(&self.conn, id)
    }

    /// All sources, oldest first.
    pub fn list_sources(&self) -> Result<Vec<Source>> {
        let mut stmt = self.conn.prepare("SELECT json FROM sources ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter()
            .map(|j| serde_json::from_str(j).map_err(Error::from))
            .collect()
    }

    /// The built-in editor source, created on first use.
    pub fn editor_source(&self) -> Result<Source> {
        if let Some(s) = self
            .list_sources()?
            .into_iter()
            .find(|s| s.connector == EDITOR_CONNECTOR)
        {
            return Ok(s);
        }
        self.add_source(Source {
            id: Id::new(),
            connector: EDITOR_CONNECTOR.into(),
            name: "editor".into(),
            config: serde_json::json!({}),
            default_origin: Origin::OwnerWritten,
            default_privacy: Privacy::CloudOk,
            cursor: None,
            created_at: now(),
        })
    }

    // ---------- ingest ----------

    /// Insert or update a connector document. Updates are matched by
    /// `(source_id, external_id)`; a *new* document whose normalised body equals a live item's
    /// (in any source) is reported as [`UpsertOutcome::Duplicate`] and not stored. Written
    /// links are resolved from `doc.refs`; unresolved references stay pending and are linked
    /// when a matching item appears. The actor is `Connector(<connector>)`.
    pub fn upsert_document(&self, source_id: &Id, doc: Document) -> Result<UpsertOutcome> {
        let source = self
            .get_source(source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        let actor = source_actor(&source);
        self.upsert_document_as(source_id, doc, actor)
    }

    /// [`Store::upsert_document`] with an explicit actor (e.g. `Mcp("claude-code")`).
    pub fn upsert_document_as(
        &self,
        source_id: &Id,
        doc: Document,
        actor: Actor,
    ) -> Result<UpsertOutcome> {
        let source = self
            .get_source(source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        self.write(|w| upsert_in(w, &source, doc, &actor, true))
    }

    /// Soft-delete the item with this external id (`ItemDeleted`). Returns its id if a live
    /// item was deleted.
    pub fn delete_by_external_id(&self, source_id: &Id, external_id: &str) -> Result<Option<Id>> {
        let source = self
            .get_source(source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        let actor = source_actor(&source);
        self.write(|w| match item_by_external(w.tx, source_id, external_id)? {
            Some(it) if !it.deleted => {
                w.append(
                    &actor,
                    EventKind::ItemDeleted {
                        item_id: it.id.clone(),
                    },
                )?;
                Ok(Some(it.id))
            }
            _ => Ok(None),
        })
    }

    /// Create a note from the editor (no dedup: the user asked for it). Wikilinks and Markdown
    /// links in the body become references.
    pub fn create_note(&self, note: NewNote, actor: Actor) -> Result<Item> {
        let source = self.editor_source()?;
        let id = Id::new();
        let doc = Document {
            external_id: id.0.clone(),
            uri: None,
            kind: note.kind.unwrap_or(ItemKind::Note),
            title: note.title,
            refs: extract_refs(&note.body),
            body: note.body,
            origin: note.origin.unwrap_or(Origin::OwnerWritten),
            created_at: None,
            modified_at: None,
            tags: note.tags,
            meta: serde_json::json!({}),
        };
        match self.write(|w| upsert_in(w, &source, doc, &actor, false))? {
            UpsertOutcome::Created(i) | UpsertOutcome::Updated(i) => Ok(i),
            other => Err(Error::Invalid(format!("unexpected outcome {other:?}"))),
        }
    }

    /// Store a document the user asked for explicitly, without deduplication against other
    /// items (e.g. a distilled note whose text is also the whole chat message it came from).
    /// A document with the same external id in the source is updated.
    pub fn upsert_document_explicit(
        &self,
        source_id: &Id,
        doc: Document,
        actor: Actor,
    ) -> Result<UpsertOutcome> {
        let source = self
            .get_source(source_id)?
            .ok_or_else(|| Error::NotFound(format!("source {source_id}")))?;
        self.write(|w| upsert_in(w, &source, doc, &actor, false))
    }

    /// Edit an item (`ItemUpdated`). A changed body re-extracts its references.
    pub fn update_item(&self, id: &Id, patch: ItemPatch, actor: Actor) -> Result<Item> {
        self.write(|w| {
            let old =
                get_item_in(w.tx, id)?.ok_or_else(|| Error::NotFound(format!("item {id}")))?;
            if old.deleted {
                return Err(Error::NotFound(format!("item {id} is deleted")));
            }
            let mut item = old.clone();
            let body_changed = patch.body.as_ref().is_some_and(|b| *b != old.body);
            if let Some(t) = patch.title {
                item.title = t;
            }
            if let Some(b) = patch.body {
                item.content_hash = content_hash(&b);
                item.body = b;
            }
            if let Some(t) = patch.tags {
                item.tags = t;
            }
            if let Some(k) = patch.kind {
                item.kind = k;
            }
            if item == old {
                return Ok(old);
            }
            item.modified_at = now();
            w.append(&actor, EventKind::ItemUpdated { item: item.clone() })?;
            if body_changed {
                let refs = extract_refs(&item.body);
                if refs != stored_refs(w.tx, id)? {
                    w.append(
                        &actor,
                        EventKind::RefsSet {
                            item_id: id.clone(),
                            refs,
                        },
                    )?;
                }
            }
            sync_written_links(w, &actor, &item)?;
            Ok(item)
        })
    }

    /// Soft-delete an item (`ItemDeleted`). Returns false if it was missing or already deleted.
    pub fn delete_item(&self, id: &Id, actor: Actor) -> Result<bool> {
        self.write(|w| match get_item_in(w.tx, id)? {
            Some(it) if !it.deleted => {
                w.append(
                    &actor,
                    EventKind::ItemDeleted {
                        item_id: id.clone(),
                    },
                )?;
                Ok(true)
            }
            _ => Ok(false),
        })
    }

    // ---------- items ----------

    /// An item by id (including soft-deleted ones).
    pub fn get_item(&self, id: &Id) -> Result<Option<Item>> {
        get_item_in(&self.conn, id)
    }

    /// An item by `(source_id, external_id)`.
    pub fn get_item_by_external_id(
        &self,
        source_id: &Id,
        external_id: &str,
    ) -> Result<Option<Item>> {
        item_by_external(&self.conn, source_id, external_id)
    }

    fn filter_sql(f: &ItemFilter) -> (String, Vec<rusqlite::types::Value>) {
        use rusqlite::types::Value as V;
        let mut clauses: Vec<&str> = Vec::new();
        let mut args: Vec<V> = Vec::new();
        if !f.include_deleted {
            clauses.push("deleted = 0");
        }
        if let Some(k) = &f.kind {
            clauses.push("kind = ?");
            args.push(V::Text(enum_str(k)));
        }
        if let Some(o) = &f.origin {
            clauses.push("origin = ?");
            args.push(V::Text(enum_str(o)));
        }
        if let Some(s) = &f.source_id {
            clauses.push("source_id = ?");
            args.push(V::Text(s.0.clone()));
        }
        if let Some(t) = &f.since {
            clauses.push("created_at >= ?");
            args.push(V::Text(ts(t)));
        }
        let mut sql = String::new();
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at, id");
        sql.push_str(&format!(
            " LIMIT {} OFFSET {}",
            f.limit.map(|l| l as i64).unwrap_or(-1),
            f.offset.unwrap_or(0)
        ));
        (sql, args)
    }

    /// Items matching `filter`, with bodies.
    pub fn list_items(&self, filter: &ItemFilter) -> Result<Vec<Item>> {
        let (tail, args) = Self::filter_sql(filter);
        read_items(
            &self.conn,
            &format!("SELECT json FROM items{tail}"),
            rusqlite::params_from_iter(args),
        )
    }

    /// Items matching `filter`, without bodies (cheap; for the cloud and lists).
    pub fn list_item_summaries(&self, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        let (tail, args) = Self::filter_sql(filter);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, source_id, kind, origin, title, created_at, modified_at FROM items{tail}"
        ))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, src, kind, origin, title, c, m)| {
                Ok(ItemSummary {
                    id: Id(id),
                    source_id: Id(src),
                    kind: super::parse_enum(&kind)?,
                    origin: super::parse_enum(&origin)?,
                    title,
                    created_at: parse_ts(&c)?,
                    modified_at: parse_ts(&m)?,
                })
            })
            .collect()
    }

    /// Number of live items.
    pub fn count_items(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM items WHERE deleted = 0", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }

    /// The references an item declares (as written), in order.
    pub fn item_refs(&self, item_id: &Id) -> Result<Vec<String>> {
        stored_refs(&self.conn, item_id)
    }

    /// References in live items that resolve to no item yet.
    pub fn pending_refs(&self) -> Result<Vec<PendingRef>> {
        let rows: Vec<(String, i64, String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT r.item_id, r.ord, r.ref, i.source_id FROM item_refs r JOIN items i ON i.id = r.item_id
                 WHERE i.deleted = 0 ORDER BY r.item_id, r.ord",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut out = Vec::new();
        for (item, ord, reference, src) in rows {
            let id = Id(item);
            if resolve_ref(&self.conn, &id, &src, ord)?.is_none() {
                out.push(PendingRef {
                    item_id: id,
                    reference,
                });
            }
        }
        Ok(out)
    }

    /// Resolve a free-standing reference (a title, alias, path or url) to a live item, as a
    /// top-level wikilink would be. Useful for mcp `link` tools.
    pub fn resolve_reference(&self, reference: &str) -> Result<Option<Id>> {
        let cands = crate::refs::ref_candidates("", reference);
        for key in cands {
            let id: Option<String> = self
                .conn
                .query_row(
                    "SELECT k.item_id FROM item_keys k JOIN items i ON i.id = k.item_id
                     WHERE k.key = ?1 AND i.deleted = 0 ORDER BY k.item_id LIMIT 1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = id {
                return Ok(Some(Id(id)));
            }
        }
        Ok(None)
    }
}
