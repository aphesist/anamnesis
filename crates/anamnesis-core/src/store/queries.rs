//! Projections the http api reads: links, threads, regions and placements, suggestions,
//! resurfacing, retrieval labels, egress, settings, chat turns and timeline aggregates.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Store, enum_str, now, parse_enum, parse_ts};
use crate::canon::{canonical_json, sha256_hex};
use crate::error::{Error, Result};
use crate::model::{
    Actor, Event, EventKind, Id, Link, LinkKind, Origin, Placement, Region, RegionTransition,
    Speaker, Thread, ThreadKind, ThreadStatus, Timestamp,
};

/// Lifecycle of a link suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Pending,
    Accepted,
    Dismissed,
}

/// A recorded link suggestion. `shown = false` is the held-out control arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: Id,
    pub from: Id,
    pub to: Id,
    pub score: f32,
    pub rank: u32,
    pub shown: bool,
    pub model: String,
    pub made_at: Timestamp,
    pub status: SuggestionStatus,
    pub decided_at: Option<Timestamp>,
}

/// A resurfacing record (micro-randomised trial arm and rating).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resurfacing {
    pub id: Id,
    pub item_id: Id,
    pub arm: String,
    pub shown_at: Timestamp,
    pub useful: Option<bool>,
    pub rated_at: Option<Timestamp>,
}

/// A user relevance label (latest grade per `(query, item)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalLabel {
    pub query: String,
    pub item_id: Id,
    pub grade: u8,
    pub at: Timestamp,
}

/// One clustering run with its regions and transitions (lineage).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionRun {
    pub run_id: Id,
    pub at: Timestamp,
    pub regions: Vec<Region>,
    pub transitions: Vec<RegionTransition>,
}

/// A chat turn as recorded by `ChatTurn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatTurnRecord {
    pub seq: u64,
    pub chat_id: Id,
    pub role: String,
    pub item_id: Id,
    pub cited: Vec<Id>,
    pub at: Timestamp,
}

/// Filters for [`Store::threads`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThreadFilter {
    pub kind: Option<ThreadKind>,
    pub status: Option<ThreadStatus>,
    pub item_id: Option<Id>,
}

/// `{date, count}` per day (`YYYY-MM-DD`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayCount {
    pub date: String,
    pub count: u32,
}

/// Item counts per month for one region (aligned with [`Timeline::months`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionCounts {
    pub region_id: Id,
    pub counts: Vec<u32>,
}

/// Item counts per month per speaker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerCounts {
    pub you: Vec<u32>,
    pub assistants: Vec<u32>,
    pub other: Vec<u32>,
}

/// Threads created per month per kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadCounts {
    pub decision: Vec<u32>,
    pub open_question: Vec<u32>,
    pub plan: Vec<u32>,
    pub rejection: Vec<u32>,
}

/// `GET /api/timeline`. Every per-month vector is aligned with `months` (`YYYY-MM`,
/// contiguous from the first to the last month with any item or thread).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeline {
    pub days: Vec<DayCount>,
    pub months: Vec<String>,
    pub by_region: Vec<RegionCounts>,
    pub by_speaker: SpeakerCounts,
    pub threads_by_month: ThreadCounts,
}

fn month_range(first: &str, last: &str) -> Vec<String> {
    let parse = |s: &str| -> Option<(i32, u32)> {
        Some((s.get(0..4)?.parse().ok()?, s.get(5..7)?.parse().ok()?))
    };
    let (Some((mut y, mut m)), Some((ly, lm))) = (parse(first), parse(last)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while (y, m) <= (ly, lm) {
        out.push(format!("{y:04}-{m:02}"));
        m += 1;
        if m > 12 {
            m = 1;
            y += 1;
        }
    }
    out
}

fn read_json_rows<T: serde::de::DeserializeOwned>(
    conn: &rusqlite::Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(p, |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|j| serde_json::from_str(j).map_err(Error::from))
        .collect()
}

fn opt_ts(s: Option<String>) -> Result<Option<Timestamp>> {
    s.map(|s| parse_ts(&s)).transpose()
}

impl Store {
    // ---------- links ----------

    fn links_where(&self, clause: &str, p: impl rusqlite::Params) -> Result<Vec<Link>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT from_id, to_id, kind, created_at FROM links {clause} ORDER BY created_at, from_id, to_id"))?;
        let rows = stmt
            .query_map(p, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(f, t, k, c)| {
                Ok(Link {
                    from: Id(f),
                    to: Id(t),
                    kind: parse_enum(&k)?,
                    created_at: parse_ts(&c)?,
                })
            })
            .collect()
    }

    /// Links going out of an item.
    pub fn links_from(&self, item_id: &Id) -> Result<Vec<Link>> {
        self.links_where("WHERE from_id = ?1", params![item_id.0])
    }

    /// Links pointing at an item.
    pub fn links_to(&self, item_id: &Id) -> Result<Vec<Link>> {
        self.links_where("WHERE to_id = ?1", params![item_id.0])
    }

    /// Every link between live items.
    pub fn all_links(&self) -> Result<Vec<Link>> {
        self.links_where(
            "WHERE from_id IN (SELECT id FROM items WHERE deleted = 0) AND to_id IN (SELECT id FROM items WHERE deleted = 0)",
            [],
        )
    }

    /// Add a link made by hand (`LinkAdded { kind: Manual }`). Both items must exist.
    pub fn add_manual_link(&self, from: &Id, to: &Id, actor: Actor) -> Result<Link> {
        self.add_link(from, to, LinkKind::Manual, actor)
    }

    /// Add a link of any kind (idempotent: an existing identical link is returned as is).
    pub fn add_link(&self, from: &Id, to: &Id, kind: LinkKind, actor: Actor) -> Result<Link> {
        if from == to {
            return Err(Error::Invalid("an item cannot link to itself".into()));
        }
        for id in [from, to] {
            if self.get_item(id)?.is_none_or(|i| i.deleted) {
                return Err(Error::NotFound(format!("item {id}")));
            }
        }
        if let Some(l) = self
            .links_from(from)?
            .into_iter()
            .find(|l| &l.to == to && l.kind == kind)
        {
            return Ok(l);
        }
        let link = Link {
            from: from.clone(),
            to: to.clone(),
            kind,
            created_at: now(),
        };
        self.append(actor, EventKind::LinkAdded { link: link.clone() })?;
        Ok(link)
    }

    /// Remove every link from `from` to `to` (`LinkRemoved`). Returns false if none existed.
    pub fn remove_link(&self, from: &Id, to: &Id, actor: Actor) -> Result<bool> {
        if !self.links_from(from)?.iter().any(|l| &l.to == to) {
            return Ok(false);
        }
        self.append(
            actor,
            EventKind::LinkRemoved {
                from: from.clone(),
                to: to.clone(),
            },
        )?;
        Ok(true)
    }

    // ---------- threads ----------

    pub fn get_thread(&self, id: &Id) -> Result<Option<Thread>> {
        Ok(read_json_rows(
            &self.conn,
            "SELECT json FROM threads WHERE id = ?1",
            params![id.0],
        )?
        .pop())
    }

    /// Threads matching the filter, newest first.
    pub fn threads(&self, f: &ThreadFilter) -> Result<Vec<Thread>> {
        let mut sql = "SELECT json FROM threads WHERE 1 = 1".to_string();
        let mut args: Vec<String> = Vec::new();
        if let Some(k) = &f.kind {
            sql.push_str(" AND kind = ?");
            args.push(enum_str(k));
        }
        if let Some(s) = &f.status {
            sql.push_str(" AND status = ?");
            args.push(enum_str(s));
        }
        if let Some(i) = &f.item_id {
            sql.push_str(" AND item_id = ?");
            args.push(i.0.clone());
        }
        sql.push_str(" ORDER BY created_at DESC, id DESC");
        read_json_rows(&self.conn, &sql, rusqlite::params_from_iter(args))
    }

    /// Insert a thread (`ThreadAdded`) or replace it (`ThreadUpdated`) if its id exists and
    /// differs. Returns the stored thread.
    pub fn upsert_thread(&self, thread: Thread, actor: Actor) -> Result<Thread> {
        match self.get_thread(&thread.id)? {
            None => {
                self.append(
                    actor,
                    EventKind::ThreadAdded {
                        thread: thread.clone(),
                    },
                )?;
            }
            Some(old) if old != thread => {
                self.append(
                    actor,
                    EventKind::ThreadUpdated {
                        thread: thread.clone(),
                    },
                )?;
            }
            Some(_) => {}
        }
        Ok(thread)
    }

    /// Change a thread's status (`POST /api/threads/:id`).
    pub fn set_thread_status(
        &self,
        id: &Id,
        status: ThreadStatus,
        resolved_by: Option<Id>,
        actor: Actor,
    ) -> Result<Thread> {
        let mut t = self
            .get_thread(id)?
            .ok_or_else(|| Error::NotFound(format!("thread {id}")))?;
        t.status = status;
        if resolved_by.is_some() {
            t.resolved_by = resolved_by;
        }
        t.updated_at = now();
        self.append(actor, EventKind::ThreadUpdated { thread: t.clone() })?;
        Ok(t)
    }

    // ---------- regions and placements ----------

    /// Record a clustering run (`RegionsRecomputed`).
    pub fn record_regions(
        &self,
        run_id: Id,
        regions: Vec<Region>,
        transitions: Vec<RegionTransition>,
        actor: Actor,
    ) -> Result<Event> {
        self.append(
            actor,
            EventKind::RegionsRecomputed {
                run_id,
                regions,
                transitions,
            },
        )
    }

    /// Regions of the latest run.
    pub fn current_regions(&self) -> Result<Vec<Region>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT regions FROM region_runs ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => serde_json::from_str(&j)?,
            None => Vec::new(),
        })
    }

    /// A region by id (its latest recorded state).
    pub fn get_region(&self, id: &Id) -> Result<Option<Region>> {
        Ok(read_json_rows(
            &self.conn,
            "SELECT json FROM regions WHERE id = ?1",
            params![id.0],
        )?
        .pop())
    }

    /// Every clustering run, oldest first (`GET /api/lineage`).
    pub fn region_runs(&self) -> Result<Vec<RegionRun>> {
        let mut stmt = self
            .conn
            .prepare("SELECT run_id, at, regions, transitions FROM region_runs ORDER BY seq")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, at, regs, tr)| {
                Ok(RegionRun {
                    run_id: Id(id),
                    at: parse_ts(&at)?,
                    regions: serde_json::from_str(&regs)?,
                    transitions: serde_json::from_str(&tr)?,
                })
            })
            .collect()
    }

    /// Replace the whole layout with a new run's placements. The placements are a cache;
    /// the log records `LayoutUpdated { run_id, placements_hash, count }` where the hash is
    /// sha256 of the canonical json of the placements sorted by item id.
    pub fn replace_placements(
        &self,
        run_id: Id,
        placements: &[Placement],
        actor: Actor,
    ) -> Result<Event> {
        let mut sorted: Vec<&Placement> = placements.iter().collect();
        sorted.sort_by(|a, b| a.item_id.cmp(&b.item_id));
        let hash = sha256_hex(canonical_json(&serde_json::to_value(&sorted)?).as_bytes());
        self.write(|w| {
            w.tx.execute("DELETE FROM placements", [])?;
            {
                let mut ins = w.tx.prepare_cached(
                    "INSERT OR REPLACE INTO placements (item_id, run_id, x, y, z, region_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?;
                for p in &sorted {
                    ins.execute(params![
                        p.item_id.0,
                        run_id.0,
                        p.xyz[0] as f64,
                        p.xyz[1] as f64,
                        p.xyz[2] as f64,
                        p.region_id.as_ref().map(|r| r.0.clone())
                    ])?;
                }
            }
            w.append(
                &actor,
                EventKind::LayoutUpdated { run_id: run_id.clone(), placements_hash: hash, count: sorted.len() as u32 },
            )
        })
    }

    /// The current layout, ordered by item id.
    pub fn placements(&self) -> Result<Vec<Placement>> {
        let mut stmt = self
            .conn
            .prepare("SELECT item_id, x, y, z, region_id FROM placements ORDER BY item_id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Placement {
                    item_id: Id(r.get(0)?),
                    xyz: [
                        r.get::<_, f64>(1)? as f32,
                        r.get::<_, f64>(2)? as f32,
                        r.get::<_, f64>(3)? as f32,
                    ],
                    region_id: r.get::<_, Option<String>>(4)?.map(Id),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The region an item is placed in, if any.
    pub fn item_region(&self, item_id: &Id) -> Result<Option<Region>> {
        let rid: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT region_id FROM placements WHERE item_id = ?1",
                params![item_id.0],
                |r| r.get(0),
            )
            .optional()?;
        match rid.flatten() {
            Some(r) => self.get_region(&Id(r)),
            None => Ok(None),
        }
    }

    // ---------- suggestions ----------

    /// Record a computed suggestion (`SuggestionMade`). `shown = false` holds it out (control).
    #[allow(clippy::too_many_arguments)]
    pub fn record_suggestion(
        &self,
        from: &Id,
        to: &Id,
        score: f32,
        rank: u32,
        shown: bool,
        model: &str,
        actor: Actor,
    ) -> Result<Id> {
        let id = Id::new();
        self.append(
            actor,
            EventKind::SuggestionMade {
                suggestion_id: id.clone(),
                from: from.clone(),
                to: to.clone(),
                score,
                rank,
                shown,
                model: model.to_string(),
            },
        )?;
        Ok(id)
    }

    fn suggestions_where(&self, clause: &str, p: impl rusqlite::Params) -> Result<Vec<Suggestion>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, from_id, to_id, score, rank, shown, model, made_at, status, decided_at FROM suggestions {clause}"
        ))?;
        type Row = (
            String,
            String,
            String,
            f64,
            i64,
            bool,
            String,
            String,
            String,
            Option<String>,
        );
        let rows = stmt
            .query_map(p, |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<Row>>>()?;
        rows.into_iter()
            .map(|(id, f, t, score, rank, shown, model, made, status, dec)| {
                Ok(Suggestion {
                    id: Id(id),
                    from: Id(f),
                    to: Id(t),
                    score: score as f32,
                    rank: rank as u32,
                    shown,
                    model,
                    made_at: parse_ts(&made)?,
                    status: parse_enum(&status)?,
                    decided_at: opt_ts(dec)?,
                })
            })
            .collect()
    }

    /// Pending, shown suggestions from an item, best rank first (what the ui displays).
    pub fn open_suggestions(&self, from: &Id) -> Result<Vec<Suggestion>> {
        self.suggestions_where(
            "WHERE from_id = ?1 AND shown = 1 AND status = 'pending' ORDER BY rank, score DESC",
            params![from.0],
        )
    }

    /// Every suggestion (both arms, all statuses), oldest first.
    pub fn all_suggestions(&self) -> Result<Vec<Suggestion>> {
        self.suggestions_where("ORDER BY made_at, id", [])
    }

    pub fn get_suggestion(&self, id: &Id) -> Result<Option<Suggestion>> {
        Ok(self
            .suggestions_where("WHERE id = ?1", params![id.0])?
            .pop())
    }

    /// Accept (`SuggestionAccepted` + `LinkAdded { AcceptedSuggestion }`) or dismiss
    /// (`SuggestionDismissed`) a pending suggestion, atomically.
    pub fn decide_suggestion(&self, id: &Id, accept: bool, actor: Actor) -> Result<Suggestion> {
        let s = self
            .get_suggestion(id)?
            .ok_or_else(|| Error::NotFound(format!("suggestion {id}")))?;
        if s.status != SuggestionStatus::Pending {
            return Ok(s);
        }
        self.write(|w| {
            if accept {
                w.append(
                    &actor,
                    EventKind::SuggestionAccepted {
                        suggestion_id: id.clone(),
                    },
                )?;
                w.append(
                    &actor,
                    EventKind::LinkAdded {
                        link: Link {
                            from: s.from.clone(),
                            to: s.to.clone(),
                            kind: LinkKind::AcceptedSuggestion,
                            created_at: now(),
                        },
                    },
                )?;
            } else {
                w.append(
                    &actor,
                    EventKind::SuggestionDismissed {
                        suggestion_id: id.clone(),
                    },
                )?;
            }
            Ok(())
        })?;
        self.get_suggestion(id)?
            .ok_or_else(|| Error::NotFound(format!("suggestion {id}")))
    }

    // ---------- resurfacing ----------

    /// Record that an item was resurfaced (`ResurfaceShown`); `arm` is "algorithm" or "random".
    pub fn record_resurface(&self, item_id: &Id, arm: &str, actor: Actor) -> Result<Id> {
        let id = Id::new();
        self.append(
            actor,
            EventKind::ResurfaceShown {
                resurface_id: id.clone(),
                item_id: item_id.clone(),
                arm: arm.to_string(),
            },
        )?;
        Ok(id)
    }

    /// Rate a resurfaced item (`ResurfaceRated`).
    pub fn rate_resurface(&self, resurface_id: &Id, useful: bool, actor: Actor) -> Result<()> {
        if self
            .resurfacings(false)?
            .iter()
            .all(|r| &r.id != resurface_id)
        {
            return Err(Error::NotFound(format!("resurface {resurface_id}")));
        }
        self.append(
            actor,
            EventKind::ResurfaceRated {
                resurface_id: resurface_id.clone(),
                useful,
            },
        )?;
        Ok(())
    }

    /// Resurfacing records, oldest first; `unrated_only` keeps those awaiting a rating.
    pub fn resurfacings(&self, unrated_only: bool) -> Result<Vec<Resurfacing>> {
        let sql = format!(
            "SELECT id, item_id, arm, shown_at, useful, rated_at FROM resurfacings {} ORDER BY shown_at, id",
            if unrated_only {
                "WHERE rated_at IS NULL"
            } else {
                ""
            }
        );
        let mut stmt = self.conn.prepare(&sql)?;
        type Row = (String, String, String, String, Option<bool>, Option<String>);
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<Row>>>()?;
        rows.into_iter()
            .map(|(id, item, arm, shown, useful, rated)| {
                Ok(Resurfacing {
                    id: Id(id),
                    item_id: Id(item),
                    arm,
                    shown_at: parse_ts(&shown)?,
                    useful,
                    rated_at: opt_ts(rated)?,
                })
            })
            .collect()
    }

    // ---------- retrieval labels ----------

    /// Record a relevance label (`RetrievalLabeled`, grade 0..=3).
    pub fn label_retrieval(
        &self,
        query: &str,
        item_id: &Id,
        grade: u8,
        actor: Actor,
    ) -> Result<()> {
        if grade > 3 {
            return Err(Error::Invalid("grade must be 0..=3".into()));
        }
        self.append(
            actor,
            EventKind::RetrievalLabeled {
                query: query.to_string(),
                item_id: item_id.clone(),
                grade,
            },
        )?;
        Ok(())
    }

    /// All labels (latest per query and item).
    pub fn retrieval_labels(&self) -> Result<Vec<RetrievalLabel>> {
        let mut stmt = self.conn.prepare(
            "SELECT query, item_id, grade, at FROM retrieval_labels ORDER BY query, item_id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(q, i, g, at)| {
                Ok(RetrievalLabel {
                    query: q,
                    item_id: Id(i),
                    grade: g as u8,
                    at: parse_ts(&at)?,
                })
            })
            .collect()
    }

    // ---------- egress ----------

    /// The most recent `Egress` events, newest first.
    pub fn egress_entries(&self, limit: usize) -> Result<Vec<Event>> {
        self.events_of_type(&["egress"], limit)
    }

    // ---------- chat turns ----------

    /// Turns of one chat, in order.
    pub fn chat_turns(&self, chat_id: &Id) -> Result<Vec<ChatTurnRecord>> {
        let mut stmt =
            self.conn.prepare("SELECT seq, chat_id, role, item_id, cited, at FROM chat_turns WHERE chat_id = ?1 ORDER BY seq")?;
        let rows = stmt
            .query_map(params![chat_id.0], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(seq, c, role, item, cited, at)| {
                Ok(ChatTurnRecord {
                    seq: seq as u64,
                    chat_id: Id(c),
                    role,
                    item_id: Id(item),
                    cited: serde_json::from_str(&cited)?,
                    at: parse_ts(&at)?,
                })
            })
            .collect()
    }

    // ---------- settings ----------

    /// All settings as one json object.
    pub fn settings(&self) -> Result<Value> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM settings ORDER BY key")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut map = serde_json::Map::new();
        for (k, v) in rows {
            map.insert(k, serde_json::from_str(&v)?);
        }
        Ok(Value::Object(map))
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<Value>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        v.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }

    /// Set one key (`SettingSet`); `Value::Null` removes it. No event if unchanged.
    pub fn set_setting(&self, key: &str, value: Value) -> Result<()> {
        if self.get_setting(key)?.unwrap_or(Value::Null) == value {
            return Ok(());
        }
        self.append(
            Actor::User,
            EventKind::SettingSet {
                key: key.to_string(),
                value,
            },
        )?;
        Ok(())
    }

    /// Merge a json object of settings (top-level keys; `null` removes). Returns all settings.
    pub fn put_settings(&self, obj: &Value) -> Result<Value> {
        let map = obj
            .as_object()
            .ok_or_else(|| Error::Invalid("settings must be a json object".into()))?;
        let current = self.settings()?;
        self.write(|w| {
            for (k, v) in map {
                if current.get(k).unwrap_or(&Value::Null) != v {
                    w.append(
                        &Actor::User,
                        EventKind::SettingSet {
                            key: k.clone(),
                            value: v.clone(),
                        },
                    )?;
                }
            }
            Ok(())
        })?;
        self.settings()
    }

    // ---------- timeline ----------

    /// Aggregates for `GET /api/timeline` over live items (by `created_at`) and threads.
    /// Regions come from the current placements.
    pub fn timeline(&self) -> Result<Timeline> {
        let rows: Vec<(String, String, Option<String>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT i.created_at, i.origin, p.region_id FROM items i LEFT JOIN placements p ON p.item_id = i.id
                 WHERE i.deleted = 0 ORDER BY i.created_at",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let threads: Vec<(String, String)> = {
            let mut stmt = self.conn.prepare("SELECT created_at, kind FROM threads")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut days: BTreeMap<String, u32> = BTreeMap::new();
        let mut all_months: Vec<&str> = Vec::new();
        for (c, _, _) in &rows {
            *days.entry(c[..10].to_string()).or_default() += 1;
            all_months.push(&c[..7]);
        }
        for (c, _) in &threads {
            all_months.push(&c[..7]);
        }
        let (Some(first), Some(last)) = (all_months.iter().min(), all_months.iter().max()) else {
            return Ok(Timeline::default());
        };
        let months = month_range(first, last);
        let idx: HashMap<&str, usize> = months
            .iter()
            .enumerate()
            .map(|(i, m)| (m.as_str(), i))
            .collect();
        let n = months.len();
        let mut speakers = SpeakerCounts {
            you: vec![0; n],
            assistants: vec![0; n],
            other: vec![0; n],
        };
        let mut regions: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for (c, origin, region) in &rows {
            let m = idx[&c[..7]];
            let origin: Origin = parse_enum(origin)?;
            match origin.speaker() {
                Speaker::You => speakers.you[m] += 1,
                Speaker::Assistants => speakers.assistants[m] += 1,
                Speaker::Other => speakers.other[m] += 1,
            }
            if let Some(r) = region {
                regions.entry(r.clone()).or_insert_with(|| vec![0; n])[m] += 1;
            }
        }
        let mut tc = ThreadCounts {
            decision: vec![0; n],
            open_question: vec![0; n],
            plan: vec![0; n],
            rejection: vec![0; n],
        };
        for (c, kind) in &threads {
            let m = idx[&c[..7]];
            match parse_enum::<ThreadKind>(kind)? {
                ThreadKind::Decision => tc.decision[m] += 1,
                ThreadKind::OpenQuestion => tc.open_question[m] += 1,
                ThreadKind::Plan => tc.plan[m] += 1,
                ThreadKind::Rejection => tc.rejection[m] += 1,
            }
        }
        Ok(Timeline {
            days: days
                .into_iter()
                .map(|(date, count)| DayCount { date, count })
                .collect(),
            months,
            by_region: regions
                .into_iter()
                .map(|(r, counts)| RegionCounts {
                    region_id: Id(r),
                    counts,
                })
                .collect(),
            by_speaker: speakers,
            threads_by_month: tc,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn month_range_spans_years() {
        assert_eq!(
            super::month_range("2025-11", "2026-02"),
            vec!["2025-11", "2025-12", "2026-01", "2026-02"]
        );
    }
}
