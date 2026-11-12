//! Distilled-note proposals (`DistilProposed`, `DistilDecided`).

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::{Store, parse_ts};
use crate::error::{Error, Result};
use crate::model::{Actor, DistilProposal, EventKind, Id, Timestamp};

/// Where a proposal stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistilStatus {
    Pending,
    Kept,
    Dropped,
}

impl DistilStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            DistilStatus::Pending => "pending",
            DistilStatus::Kept => "kept",
            DistilStatus::Dropped => "dropped",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "kept" => DistilStatus::Kept,
            "dropped" => DistilStatus::Dropped,
            _ => DistilStatus::Pending,
        }
    }
}

/// A proposal with its outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistilRecord {
    #[serde(flatten)]
    pub proposal: DistilProposal,
    pub status: DistilStatus,
    /// The note made from it, once kept.
    pub item_id: Option<Id>,
    pub decided_at: Option<Timestamp>,
}

/// Filters for [`Store::distil_proposals`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DistilFilter {
    pub chat_id: Option<Id>,
    pub status: Option<DistilStatus>,
}

impl Store {
    /// Record proposals (one `DistilProposed` event). An empty list records nothing.
    pub fn propose_distilled(&self, proposals: Vec<DistilProposal>, actor: Actor) -> Result<()> {
        if proposals.is_empty() {
            return Ok(());
        }
        self.append(actor, EventKind::DistilProposed { proposals })?;
        Ok(())
    }

    /// Record the user's answer to a pending proposal. Errors if it is unknown or already
    /// answered.
    pub fn decide_distilled(
        &self,
        proposal_id: &Id,
        kept: bool,
        item_id: Option<Id>,
        actor: Actor,
    ) -> Result<DistilRecord> {
        let rec = self
            .get_distil_proposal(proposal_id)?
            .ok_or_else(|| Error::NotFound(format!("proposal {proposal_id}")))?;
        if rec.status != DistilStatus::Pending {
            return Err(Error::Invalid(format!(
                "proposal {proposal_id} was already {}",
                rec.status.as_str()
            )));
        }
        self.append(
            actor,
            EventKind::DistilDecided {
                proposal_id: proposal_id.clone(),
                kept,
                item_id,
            },
        )?;
        self.get_distil_proposal(proposal_id)?
            .ok_or_else(|| Error::NotFound(format!("proposal {proposal_id}")))
    }

    pub fn get_distil_proposal(&self, id: &Id) -> Result<Option<DistilRecord>> {
        Ok(self
            .read_distil(
                "SELECT json, status, item_id, decided_at FROM distil_proposals WHERE id = ?1",
                vec![id.0.clone()],
            )?
            .pop())
    }

    /// Proposals matching the filter, oldest first (the order they were proposed in).
    pub fn distil_proposals(&self, f: &DistilFilter) -> Result<Vec<DistilRecord>> {
        let mut sql = "SELECT json, status, item_id, decided_at FROM distil_proposals WHERE 1 = 1"
            .to_string();
        let mut args = Vec::new();
        if let Some(c) = &f.chat_id {
            sql.push_str(" AND chat_id = ?");
            args.push(c.0.clone());
        }
        if let Some(s) = f.status {
            sql.push_str(" AND status = ?");
            args.push(s.as_str().to_string());
        }
        sql.push_str(" ORDER BY seq, ord");
        self.read_distil(&sql, args)
    }

    /// Chats with a user turn after `since` (all chats when `None`), most recent first.
    pub fn chats_with_user_turns_since(&self, since: Option<Timestamp>) -> Result<Vec<Id>> {
        let since = since.map(|t| super::ts(&t)).unwrap_or_default();
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, MAX(at) AS last FROM chat_turns WHERE role = 'user' AND at > ?1
             GROUP BY chat_id ORDER BY last DESC",
        )?;
        let rows = stmt
            .query_map(params![since], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(Id).collect())
    }

    fn read_distil(&self, sql: &str, args: Vec<String>) -> Result<Vec<DistilRecord>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(json, status, item, at)| {
                Ok(DistilRecord {
                    proposal: serde_json::from_str(&json)?,
                    status: DistilStatus::parse(&status),
                    item_id: item.map(Id),
                    decided_at: at.as_deref().map(parse_ts).transpose()?,
                })
            })
            .collect()
    }
}

pub(crate) fn apply_proposed(
    conn: &rusqlite::Connection,
    seq: u64,
    proposals: &[DistilProposal],
) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT OR REPLACE INTO distil_proposals (id, seq, ord, chat_id, status, item_id, decided_at, json)
         VALUES (?1, ?2, ?3, ?4, 'pending', NULL, NULL, ?5)",
    )?;
    for (i, p) in proposals.iter().enumerate() {
        stmt.execute(params![
            p.id.0,
            seq as i64,
            i as i64,
            p.chat_id.as_ref().map(|c| c.0.clone()),
            serde_json::to_string(p)?
        ])?;
    }
    Ok(())
}

pub(crate) fn apply_decided(
    conn: &rusqlite::Connection,
    at: &str,
    proposal_id: &Id,
    kept: bool,
    item_id: Option<&Id>,
) -> Result<()> {
    let status = if kept {
        DistilStatus::Kept
    } else {
        DistilStatus::Dropped
    };
    conn.execute(
        "UPDATE distil_proposals SET status = ?2, item_id = ?3, decided_at = ?4 WHERE id = ?1",
        params![
            proposal_id.0,
            status.as_str(),
            item_id.map(|i| i.0.clone()),
            at
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DistilKind;

    fn proposal(chat: &Id, quote: &str) -> DistilProposal {
        DistilProposal {
            id: Id::new(),
            chat_id: Some(chat.clone()),
            kind: DistilKind::Idea,
            title: "a title".into(),
            quote: quote.into(),
            source_item: None,
            start: 0,
            end: quote.len(),
            extractor: "rule:v2".into(),
            created_at: crate::now(),
        }
    }

    #[test]
    fn proposals_are_projected_decided_once_and_replayed() {
        let s = Store::open_in_memory().unwrap();
        let chat = Id::new();
        let (a, b) = (proposal(&chat, "first"), proposal(&chat, "second"));
        s.propose_distilled(vec![a.clone(), b.clone()], Actor::User)
            .unwrap();
        let all = s
            .distil_proposals(&DistilFilter {
                chat_id: Some(chat.clone()),
                status: None,
            })
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].proposal.quote, "first");
        assert!(all.iter().all(|r| r.status == DistilStatus::Pending));

        let item = Id::new();
        let r = s
            .decide_distilled(&a.id, true, Some(item.clone()), Actor::User)
            .unwrap();
        assert_eq!(r.status, DistilStatus::Kept);
        assert_eq!(r.item_id, Some(item));
        assert!(s.decide_distilled(&a.id, false, None, Actor::User).is_err());
        s.decide_distilled(&b.id, false, None, Actor::User).unwrap();
        assert!(
            s.decide_distilled(&Id::new(), true, None, Actor::User)
                .is_err()
        );

        let before = s.distil_proposals(&DistilFilter::default()).unwrap();
        s.rebuild_projections().unwrap();
        assert_eq!(
            s.distil_proposals(&DistilFilter::default()).unwrap(),
            before
        );
        let pending = s
            .distil_proposals(&DistilFilter {
                chat_id: None,
                status: Some(DistilStatus::Pending),
            })
            .unwrap();
        assert!(pending.is_empty());
    }
}
