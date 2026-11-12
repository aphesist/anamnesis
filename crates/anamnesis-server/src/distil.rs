//! Distil: propose a few atomic notes from the user's own words in a chat, and keep or
//! drop them one by one.
//!
//! - The distiller is the routed `extract` backend when the router has one for
//!   background work (a local model; an api only if the user opted in or named one;
//!   never a subscription: the router enforces that). With a backend that is not
//!   local, only the user's turns flagged `cloud_ok` are sent, and the call goes
//!   through the egress gate (an `Egress` event is appended). With no model, or when
//!   the call fails, the rule-based distiller runs instead.
//! - Proposals are recorded as `DistilProposed`; nothing becomes an item until the
//!   user keeps it. A kept proposal becomes a note of origin `owner_in_chat` whose body
//!   is the user's exact words, linked to the chat turn it came from; a model-written
//!   title is marked as such in `meta.distil.title_by`. Keep and drop are recorded as
//!   `DistilDecided`, which the evaluation reads.

use std::sync::Arc;

use anamnesis_core::{
    Actor, DistilFilter, DistilProposal, DistilRecord, DistilStatus, Document, Id, Item, ItemKind,
    LinkKind, Origin, Privacy, UpsertOutcome, now,
};
use anamnesis_harness::{CallContext, ChatRequest, Message, Task};
use anamnesis_mind::distil::{
    Turn, distil_prompt, distil_rules, title_by_model, validate_distil_str,
};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::json;

use crate::app::App;
use crate::brain::Brain;
use crate::chat::named_source;

/// Longest text of all turns sent to a model, in characters.
const MAX_CHARS: usize = 16_000;
/// Chats distilled per daily pass at most.
pub const DAILY_CHATS: usize = 6;

/// What `POST /api/distil` (and the mcp `distil` tool) returns.
#[derive(Debug, Clone, Serialize)]
pub struct DistilOutcome {
    /// The new proposals of this run (pending).
    pub proposals: Vec<DistilRecord>,
    /// `"rule:v2"`, `"local:<model>"` or `"<provider>:<model>"`.
    pub distiller: String,
    /// A plain-language note when the model was not used (no model routed, it failed,
    /// or no turn was allowed to leave the machine).
    pub note: Option<String>,
}

/// Where the words to distil come from.
pub enum DistilSource {
    /// The app's own chat: its user turns.
    Chat(Id),
    /// Words handed in from outside the brain (the mcp `distil` tool). These are never
    /// sent to a backend that is not local.
    Text(Vec<String>),
}

fn actor_name(a: &Actor) -> String {
    match a {
        Actor::User => "user".into(),
        Actor::Connector(c) | Actor::System(c) | Actor::Mcp(c) | Actor::Model(c) => c.clone(),
    }
}

/// The user's turns of a chat (live items only), with their privacy.
async fn chat_turns(brain: &Arc<Brain>, chat_id: &Id) -> Result<Vec<(Turn, Privacy)>> {
    let cid = chat_id.clone();
    brain
        .db(move |s| {
            let mut out = Vec::new();
            for t in s.chat_turns(&cid)? {
                if t.role != "user" {
                    continue;
                }
                let Some(it) = s.get_item(&t.item_id)?.filter(|i| !i.deleted) else {
                    continue;
                };
                // only the user's own words (a turn could have been re-flagged)
                if !it.origin.is_owner() {
                    continue;
                }
                out.push((
                    Turn {
                        item_id: Some(it.id.clone()),
                        text: it.body,
                    },
                    it.privacy,
                ));
            }
            Ok(out)
        })
        .await
}

/// Propose notes from a chat or from handed-in text, record them and return them.
///
/// `allow_remote: false` (mcp) keeps the distiller on this machine (a local model or the
/// rules) and leaves out the user's turns flagged `local_only` or `ask`, whose text mcp
/// never shows.
pub async fn distil(
    app: &Arc<App>,
    source: DistilSource,
    actor: Actor,
    allow_remote: bool,
) -> Result<DistilOutcome> {
    let brain = app.brain();
    let (chat_id, turns, from_outside) = match source {
        DistilSource::Chat(id) => {
            let mut t = chat_turns(&brain, &id).await?;
            if !allow_remote {
                // mcp: the harness never sees the text of local-only or ask items
                t.retain(|(_, p)| *p == Privacy::CloudOk);
            }
            anyhow::ensure!(
                !t.is_empty(),
                anamnesis_core::Error::NotFound(format!("no messages of yours in chat {id}"))
            );
            (Some(id), t, false)
        }
        DistilSource::Text(texts) => {
            let t: Vec<(Turn, Privacy)> = texts
                .into_iter()
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .map(|text| {
                    (
                        Turn {
                            item_id: None,
                            text,
                        },
                        Privacy::LocalOnly,
                    )
                })
                .collect();
            anyhow::ensure!(
                !t.is_empty(),
                anamnesis_core::Error::Invalid("no messages to distil".into())
            );
            (None, t, true)
        }
    };
    let known: Vec<String> = match &chat_id {
        Some(c) => {
            let c = c.clone();
            brain
                .db(move |s| {
                    Ok(s.distil_proposals(&DistilFilter {
                        chat_id: Some(c),
                        status: None,
                    })?)
                })
                .await?
                .into_iter()
                .map(|r| r.proposal.quote)
                .collect()
        }
        None => Vec::new(),
    };
    let at = now();
    let all_turns: Vec<Turn> = turns.iter().map(|(t, _)| t.clone()).collect();

    let (proposals, distiller, note) = match model_distil(
        app,
        &brain,
        &turns,
        chat_id.as_ref(),
        &known,
        from_outside || !allow_remote,
    )
    .await
    {
        ModelResult::Done(ps, extractor) => (ps, extractor, None),
        ModelResult::Skipped(why) => (
            distil_rules(&all_turns, chat_id.as_ref(), &known, at),
            anamnesis_mind::threads::RULE_EXTRACTOR.to_string(),
            why,
        ),
    };
    let records: Vec<DistilRecord> = proposals
        .iter()
        .cloned()
        .map(|p| DistilRecord {
            proposal: p,
            status: DistilStatus::Pending,
            item_id: None,
            decided_at: None,
        })
        .collect();
    let a = actor.clone();
    brain
        .db(move |s| Ok(s.propose_distilled(proposals, a)?))
        .await?;
    tracing::info!(
        "distil: {} proposal(s) by {distiller} for {}",
        records.len(),
        actor_name(&actor)
    );
    Ok(DistilOutcome {
        proposals: records,
        distiller,
        note,
    })
}

enum ModelResult {
    Done(Vec<DistilProposal>, String),
    /// The rules run instead; the reason, when worth telling the user.
    Skipped(Option<String>),
}

async fn model_distil(
    app: &Arc<App>,
    brain: &Arc<Brain>,
    turns: &[(Turn, Privacy)],
    chat_id: Option<&Id>,
    known: &[String],
    local_only: bool,
) -> ModelResult {
    let Some(router) = app.router() else {
        return ModelResult::Skipped(None);
    };
    let Ok(plan) = router.plan(Task::Extract, None) else {
        return ModelResult::Skipped(None);
    };
    let Some(primary) = plan.first() else {
        return ModelResult::Skipped(None);
    };
    let local = primary.is_local();
    if local_only && !local {
        return ModelResult::Skipped(Some(
            "over mcp, distil runs on this machine only: the rules ran".into(),
        ));
    }
    // a backend that is not local gets only cloud_ok turns; never ask in the background
    let mut sent: Vec<(Turn, Privacy)> = turns
        .iter()
        .filter(|(_, p)| local || *p == Privacy::CloudOk)
        .cloned()
        .collect();
    if sent.is_empty() {
        return ModelResult::Skipped(Some(
            "none of these messages may leave this machine: the rules ran".into(),
        ));
    }
    // newest turns win when the chat is long
    let mut total = 0;
    let mut keep_from = sent.len();
    for (i, (t, _)) in sent.iter().enumerate().rev() {
        total += t.text.chars().count();
        if total > MAX_CHARS && keep_from < sent.len() {
            break;
        }
        keep_from = i;
    }
    sent.drain(..keep_from);
    let sent_turns: Vec<Turn> = sent.iter().map(|(t, _)| t.clone()).collect();
    let prompt = distil_prompt(&sent_turns);
    let mut req = ChatRequest::new(vec![Message::user(prompt.user)]).with_system(prompt.system);
    req.json_schema = Some(prompt.schema);
    req.temperature = Some(0.0);
    let call = CallContext {
        items: sent
            .iter()
            .filter_map(|(t, p)| t.item_id.clone().map(|i| (i, *p)))
            .collect(),
        approved: vec![],
        backend: None,
    };
    let routed = match router.chat(Task::Extract, req, call).await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("distil: extract route failed: {e}");
            return ModelResult::Skipped(Some(
                "the model could not be reached: the rules ran".into(),
            ));
        }
    };
    let backend = routed.backend.clone();
    let extractor = match router.get(&backend) {
        Some(b) if b.is_local() => format!("local:{}", routed.model),
        Some(b) => format!("{}:{}", b.provider(), routed.model),
        None => format!("{backend}:{}", routed.model),
    };
    if let Some(eg) = routed.egress {
        let kind = eg.into_event_kind();
        let b = backend.clone();
        if let Err(e) = brain
            .db(move |s| Ok(s.append(Actor::Model(b), kind)?))
            .await
        {
            tracing::warn!("distil: could not record egress: {e:#}");
        }
    }
    let answer = match crate::jobs::collect_text(routed.stream, &backend).await {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!("distil: {e}");
            return ModelResult::Skipped(Some("the model could not finish: the rules ran".into()));
        }
    };
    match validate_distil_str(&sent_turns, &answer, chat_id, known, &extractor, now()) {
        Ok(r) => {
            if !r.rejected.is_empty() {
                tracing::debug!(
                    "distil: {} model proposal(s) rejected: {:?}",
                    r.rejected.len(),
                    r.rejected.iter().map(|x| &x.reason).collect::<Vec<_>>()
                );
            }
            ModelResult::Done(r.accepted, extractor)
        }
        Err(e) => {
            tracing::debug!("distil: {backend}: {e}");
            ModelResult::Skipped(Some(
                "the model's answer was not usable: the rules ran".into(),
            ))
        }
    }
}

/// Keep a pending proposal: a note of origin `owner_in_chat` with the user's exact words,
/// linked to the chat turn it came from. `title` (the user's own edit) replaces the
/// proposed title.
pub async fn keep(
    app: &Arc<App>,
    proposal_id: &Id,
    title: Option<String>,
    actor: Actor,
) -> Result<(DistilRecord, Item)> {
    let brain = app.brain();
    let pid = proposal_id.clone();
    let res = brain
        .db(move |s| {
            let rec = s
                .get_distil_proposal(&pid)?
                .ok_or_else(|| anamnesis_core::Error::NotFound(format!("proposal {pid}")))?;
            if rec.status != DistilStatus::Pending {
                return Err(anamnesis_core::Error::Invalid(format!(
                    "proposal {pid} was already {}",
                    if rec.status == DistilStatus::Kept {
                        "kept"
                    } else {
                        "dropped"
                    }
                ))
                .into());
            }
            let p = &rec.proposal;
            let edited = title
                .map(|t| t.trim().chars().take(200).collect::<String>())
                .filter(|t| !t.is_empty() && *t != p.title);
            let title_by = match &edited {
                Some(_) => "user".to_string(),
                None if title_by_model(p) => format!("model:{}", p.extractor),
                None => "quote".to_string(),
            };
            let source_item = match &p.source_item {
                Some(i) => s.get_item(i)?.filter(|i| !i.deleted),
                None => None,
            };
            let src = named_source(s, "distil", "distilled notes", Origin::OwnerInChat)?;
            let doc = Document {
                external_id: p.id.0.clone(),
                uri: None,
                kind: ItemKind::Note,
                title: edited.unwrap_or_else(|| p.title.clone()),
                body: p.quote.clone(),
                origin: Origin::OwnerInChat,
                created_at: source_item.as_ref().map(|i| i.created_at),
                modified_at: None,
                tags: vec![],
                refs: vec![],
                meta: json!({
                    "distil": {
                        "proposal_id": p.id,
                        "kind": p.kind,
                        "chat_id": p.chat_id,
                        "source_item": p.source_item,
                        "span": [p.start, p.end],
                        "extractor": p.extractor,
                        "title_by": title_by,
                    }
                }),
            };
            let item = match s.upsert_document_explicit(&src.id, doc, actor.clone())? {
                UpsertOutcome::Created(i) | UpsertOutcome::Updated(i) => i,
                UpsertOutcome::Unchanged(id) | UpsertOutcome::Duplicate { of: id } => s
                    .get_item(&id)?
                    .ok_or_else(|| anamnesis_core::Error::NotFound(format!("item {id}")))?,
            };
            // the note is as private as the words it came from
            let item = match &source_item {
                Some(si) if si.privacy != item.privacy => {
                    crate::privacy::set_item_privacy(s, &item.id, Some(si.privacy), actor.clone())?
                }
                _ => item,
            };
            if let Some(si) = &source_item
                && si.id != item.id
            {
                s.add_link(&item.id, &si.id, LinkKind::Manual, actor.clone())?;
            }
            let rec = s.decide_distilled(&pid, true, Some(item.id.clone()), actor)?;
            Ok((rec, item))
        })
        .await
        .context("keeping a distilled note")?;
    app.jobs_notify.notify_one();
    Ok(res)
}

/// Drop a pending proposal (recorded; nothing is stored).
pub async fn drop_proposal(app: &Arc<App>, proposal_id: &Id, actor: Actor) -> Result<DistilRecord> {
    let pid = proposal_id.clone();
    app.brain()
        .db(move |s| Ok(s.decide_distilled(&pid, false, None, actor)?))
        .await
}

/// The `distil` setting: `{ daily: bool }` (off by default).
pub fn daily_enabled(store: &anamnesis_core::Store) -> bool {
    store
        .get_setting("distil")
        .ok()
        .flatten()
        .and_then(|v| v.get("daily").and_then(|d| d.as_bool()))
        .unwrap_or(false)
}

/// The daily pass: once a day, when the user turned it on, distil the chats that got new
/// messages of the user's since the last pass (a few at most). Returns proposals made.
pub async fn run_daily(app: &Arc<App>, brain: &Arc<Brain>) -> Result<usize> {
    let today = now().date_naive().to_string();
    let js = brain.job_state();
    if js.distil_day.as_deref() == Some(today.as_str()) {
        return Ok(0);
    }
    let since = js.distil_since;
    let (enabled, chats) = brain
        .db(move |s| {
            if !daily_enabled(s) {
                return Ok((false, vec![]));
            }
            Ok((true, s.chats_with_user_turns_since(since)?))
        })
        .await?;
    if !enabled {
        // checked again tomorrow; chats since the last real pass stay due
        let mut js = brain.job_state();
        js.distil_day = Some(today);
        brain.save_job_state(&js)?;
        return Ok(0);
    }
    let started = now();
    let mut n = 0;
    for chat in chats.into_iter().take(DAILY_CHATS) {
        match distil(
            app,
            DistilSource::Chat(chat.clone()),
            Actor::System("mind:distil".into()),
            true,
        )
        .await
        {
            Ok(o) => n += o.proposals.len(),
            Err(e) => tracing::debug!("distil: daily pass, chat {chat}: {e:#}"),
        }
    }
    let mut js = brain.job_state();
    js.distil_day = Some(today);
    js.distil_since = Some(started);
    brain.save_job_state(&js)?;
    Ok(n)
}
