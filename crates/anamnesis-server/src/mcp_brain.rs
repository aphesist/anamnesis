//! `anamnesis_mcp::BrainApi` over the store: what external harnesses see through mcp.

use std::collections::HashMap;
use std::sync::Arc;

use anamnesis_core::{
    Actor, Document, EventKind, Id, Item, ItemFilter, Link, Origin, Region, Thread, ThreadFilter,
    ThreadKind, ThreadStatus, Timestamp, UpsertOutcome, refs::extract_refs,
};
use anamnesis_mcp::{
    BrainApi, BrainError, BrainResult, EgressNote, ItemSummary, ItemView, LinkSuggestion,
    Neighbour, NewCapture, RegionView, Related, SearchHit,
};
use async_trait::async_trait;
use serde_json::json;

use crate::app::App;
use crate::chat::{hybrid_search, named_source};

const SNIPPET: usize = 280;

pub struct StoreBrain {
    pub app: Arc<App>,
}

impl StoreBrain {
    pub fn new(app: Arc<App>) -> Arc<Self> {
        Arc::new(StoreBrain { app })
    }
}

fn internal(e: anyhow::Error) -> BrainError {
    match e.downcast_ref::<anamnesis_core::Error>() {
        Some(anamnesis_core::Error::NotFound(m)) => BrainError::NotFound(m.clone()),
        Some(anamnesis_core::Error::Invalid(m)) => BrainError::Invalid(m.clone()),
        _ => BrainError::Internal(format!("{e:#}")),
    }
}

fn region_map(s: &anamnesis_core::Store) -> anamnesis_core::Result<HashMap<Id, Option<Id>>> {
    Ok(s.placements()?
        .into_iter()
        .map(|p| (p.item_id, p.region_id))
        .collect())
}

fn summarise(items: Vec<Item>, regions: &HashMap<Id, Option<Id>>) -> Vec<ItemSummary> {
    items
        .iter()
        .map(|i| ItemSummary::from_item(i, regions.get(&i.id).cloned().flatten(), SNIPPET))
        .collect()
}

/// Live items, as summaries, by id.
pub(crate) fn item_summary(
    s: &anamnesis_core::Store,
    id: &Id,
) -> anamnesis_core::Result<Option<ItemSummary>> {
    let Some(item) = s.get_item(id)?.filter(|i| !i.deleted) else {
        return Ok(None);
    };
    let region = s.item_region(id)?.map(|r| r.id);
    Ok(Some(ItemSummary::from_item(&item, region, SNIPPET)))
}

#[async_trait]
impl BrainApi for StoreBrain {
    async fn search(&self, query: &str, k: usize) -> BrainResult<Vec<SearchHit>> {
        let brain = self.app.brain();
        let hits = hybrid_search(&self.app, &brain, query, k)
            .await
            .map_err(internal)?;
        brain
            .db(move |s| {
                let mut out = Vec::new();
                for h in hits {
                    if let Some(it) = s.get_item(&h.item_id)? {
                        out.push(SearchHit {
                            kind: it.kind,
                            origin: it.origin,
                            privacy: it.privacy,
                            hit: h,
                        });
                    }
                }
                Ok(out)
            })
            .await
            .map_err(internal)
    }

    async fn get_item(&self, id: &Id) -> BrainResult<Option<ItemView>> {
        let id = id.clone();
        self.app
            .brain()
            .db(move |s| {
                let Some(item) = s.get_item(&id)?.filter(|i| !i.deleted) else {
                    return Ok(None);
                };
                Ok(Some(ItemView {
                    links_out: s.links_from(&id)?,
                    links_in: s.links_to(&id)?,
                    threads: s.threads(&ThreadFilter {
                        item_id: Some(id.clone()),
                        ..Default::default()
                    })?,
                    region: s.item_region(&id)?,
                    item,
                }))
            })
            .await
            .map_err(internal)
    }

    async fn related(&self, id: &Id, k: usize) -> BrainResult<Related> {
        let model = self.app.embedder().map(|e| e.id().to_string());
        let id = id.clone();
        self.app
            .brain()
            .db(move |s| {
                if s.get_item(&id)?.is_none_or(|i| i.deleted) {
                    return Err(anamnesis_core::Error::NotFound(format!("item {id}")).into());
                }
                let mut neighbours = Vec::new();
                if let Some(model) = model {
                    let (ids, vecs) = s.item_vectors(&model)?;
                    if let Some(pos) = ids.iter().position(|x| x == &id) {
                        let q = &vecs[pos];
                        let mut scored: Vec<(f32, usize)> = vecs
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| *i != pos)
                            .map(|(i, v)| (anamnesis_embed::dot(q, v), i))
                            .collect();
                        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
                        for (score, i) in scored.into_iter().take(k) {
                            if let Some(sum) = item_summary(s, &ids[i])? {
                                neighbours.push(Neighbour { item: sum, score });
                            }
                        }
                    }
                }
                // only suggestions recorded as shown: the held-out arm never leaves
                let mut suggestions = Vec::new();
                for sg in s
                    .open_suggestions(&id)?
                    .into_iter()
                    .filter(|x| x.shown)
                    .take(k)
                {
                    if let Some(to) = item_summary(s, &sg.to)? {
                        suggestions.push(LinkSuggestion {
                            suggestion_id: sg.id,
                            to,
                            score: sg.score,
                        });
                    }
                }
                Ok(Related {
                    neighbours,
                    suggestions,
                })
            })
            .await
            .map_err(internal)
    }

    async fn list_threads(
        &self,
        kind: Option<ThreadKind>,
        status: Option<ThreadStatus>,
    ) -> BrainResult<Vec<Thread>> {
        self.app
            .brain()
            .db(move |s| {
                let threads = s.threads(&ThreadFilter {
                    kind,
                    status,
                    item_id: None,
                })?;
                // a thread quotes its item: only items the user lets leave may be listed
                let mut out = Vec::new();
                for t in threads {
                    if s.get_item(&t.item_id)?.is_some_and(|i| {
                        !i.deleted && i.privacy == anamnesis_core::Privacy::CloudOk
                    }) {
                        out.push(t);
                    }
                }
                Ok(out)
            })
            .await
            .map_err(internal)
    }

    async fn regions(&self) -> BrainResult<Vec<Region>> {
        self.app
            .brain()
            .db(|s| {
                Ok(s.current_regions()?
                    .into_iter()
                    .filter(|r| r.died_at.is_none())
                    .collect())
            })
            .await
            .map_err(internal)
    }

    async fn get_region(&self, id: &Id) -> BrainResult<Option<RegionView>> {
        let id = id.clone();
        self.app
            .brain()
            .db(move |s| {
                let Some(region) = s.get_region(&id)? else {
                    return Ok(None);
                };
                let members: Vec<Id> = s
                    .placements()?
                    .into_iter()
                    .filter(|p| p.region_id.as_ref() == Some(&id))
                    .map(|p| p.item_id)
                    .collect();
                let mut items: Vec<Item> = members
                    .iter()
                    .filter_map(|m| s.get_item(m).ok().flatten())
                    .filter(|i| !i.deleted)
                    .collect();
                items.sort_by_key(|i| std::cmp::Reverse(i.created_at));
                items.truncate(10);
                let regions = region_map(s)?;
                Ok(Some(RegionView {
                    region,
                    items: summarise(items, &regions),
                }))
            })
            .await
            .map_err(internal)
    }

    async fn timeline(
        &self,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
        limit: usize,
    ) -> BrainResult<Vec<ItemSummary>> {
        self.app
            .brain()
            .db(move |s| {
                let items: Vec<Item> = s
                    .list_items(&ItemFilter {
                        since: from,
                        ..Default::default()
                    })?
                    .into_iter()
                    .filter(|i| to.is_none_or(|t| i.created_at <= t))
                    .take(limit)
                    .collect();
                Ok(summarise(items, &region_map(s)?))
            })
            .await
            .map_err(internal)
    }

    async fn recent(&self, n: usize) -> BrainResult<Vec<ItemSummary>> {
        self.app
            .brain()
            .db(move |s| {
                let mut items = s.list_items(&ItemFilter::default())?;
                items.sort_by(|a, b| b.captured_at.cmp(&a.captured_at).then(b.id.cmp(&a.id)));
                items.truncate(n);
                Ok(summarise(items, &region_map(s)?))
            })
            .await
            .map_err(internal)
    }

    async fn capture(&self, actor: Actor, c: NewCapture) -> BrainResult<Item> {
        if c.body.trim().is_empty() {
            return Err(BrainError::Invalid("the body is empty".into()));
        }
        let item = self
            .app
            .brain()
            .db(move |s| {
                let src = named_source(s, "mcp", "mcp", Origin::AiGenerated)?;
                let doc = Document {
                    external_id: Id::new().0,
                    uri: None,
                    kind: c.kind,
                    title: c.title,
                    refs: extract_refs(&c.body),
                    body: c.body,
                    origin: c.origin,
                    created_at: None,
                    modified_at: None,
                    tags: c.tags,
                    meta: json!({ "client": c.client }),
                };
                let id = match s.upsert_document_as(&src.id, doc, actor)? {
                    UpsertOutcome::Created(i) | UpsertOutcome::Updated(i) => return Ok(i),
                    UpsertOutcome::Unchanged(id) | UpsertOutcome::Duplicate { of: id } => id,
                };
                s.get_item(&id)?
                    .ok_or_else(|| anamnesis_core::Error::NotFound(format!("item {id}")).into())
            })
            .await
            .map_err(internal)?;
        self.app.jobs_notify.notify_one();
        Ok(item)
    }

    async fn link(
        &self,
        actor: Actor,
        from: &Id,
        to: &Id,
        _note: Option<String>,
    ) -> BrainResult<Link> {
        let (from, to) = (from.clone(), to.clone());
        let l = self
            .app
            .brain()
            .db(move |s| Ok(s.add_manual_link(&from, &to, actor)?))
            .await
            .map_err(internal)?;
        self.app.jobs_notify.notify_one();
        Ok(l)
    }

    async fn update_thread(
        &self,
        actor: Actor,
        id: &Id,
        status: ThreadStatus,
        resolved_by: Option<Id>,
    ) -> BrainResult<Thread> {
        let id = id.clone();
        self.app
            .brain()
            .db(move |s| Ok(s.set_thread_status(&id, status, resolved_by, actor)?))
            .await
            .map_err(internal)
    }

    async fn note_egress(&self, actor: Actor, note: EgressNote) -> BrainResult<()> {
        if note.items.is_empty() && note.chars == 0 {
            return Ok(());
        }
        self.app
            .brain()
            .db(move |s| {
                s.append(
                    actor,
                    EventKind::Egress {
                        provider: format!("mcp:{}", note.client),
                        model: "unknown".into(),
                        purpose: note.purpose,
                        items: note.items,
                        chars: note.chars,
                    },
                )?;
                Ok(())
            })
            .await
            .map_err(internal)
    }
}
