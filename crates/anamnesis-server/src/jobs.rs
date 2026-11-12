//! Background jobs: embeddings, the mind (regions + layout), link suggestions, threads
//! (rule-based, plus model-based through the `extract` route when a background-capable
//! backend exists) and the daily review. Every job reads what it needs under the store lock,
//! releases it, computes, and writes back under the lock; network calls (region labels)
//! happen with no lock held.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anamnesis_core::{
    Actor, Id, Item, ItemFilter, Placement, Privacy, Region, Store, ThreadFilter, ThreadKind,
    ThreadStatus, Timestamp, now,
};
use anamnesis_harness::{CallContext, ChatEvent, ChatRequest, Message, Task};
use anamnesis_mind::layout::views;
use anamnesis_mind::regions::apply_labels;
use anamnesis_mind::resurface::{ReviewInfo, ReviewParams, Scheduler, SrsCard};
use anamnesis_mind::threads::{RULE_EXTRACTOR, is_rule_extractor};
use anamnesis_mind::{
    ItemMeta, LinkParams, LinkSuggester, MindParams, MindState, daily_review, extract_rules,
    full_run, place_new, split_for_holdout,
};
use anyhow::Result;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;

use crate::app::App;
use crate::brain::{Brain, random_u64};

/// Share of top suggestions held out as the control arm.
pub const HOLDOUT_RATE: f64 = 0.2;
const EMBED_BATCH: usize = 64;

/// What one pass of the jobs did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct JobsReport {
    pub embedded: usize,
    pub mind: Option<MindReport>,
    pub suggestions: usize,
    pub threads: usize,
    pub resurfaced: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MindReport {
    pub full: bool,
    pub items: usize,
    pub placed_new: usize,
    pub regions: usize,
    pub labelled_by: Option<String>,
}

fn sys(job: &str) -> Actor {
    Actor::System(job.to_string())
}

/// A u64 setting created on first use (random), e.g. the hold-out seed.
fn seed_setting(store: &Store, key: &str) -> Result<u64> {
    if let Some(v) = store.get_setting(key)? {
        if let Some(n) = v.as_u64() {
            return Ok(n);
        }
        if let Some(n) = v.as_str().and_then(|s| s.parse().ok()) {
            return Ok(n);
        }
    }
    // stored as a string: json numbers above 2^53 do not survive every reader
    let n = random_u64();
    store.set_setting(key, json!(n.to_string()))?;
    Ok(n)
}

fn mind_params(store: &Store) -> MindParams {
    store
        .get_setting("mind_params")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------
// embeddings

/// Embed every chunk that has no vector for the current embedder yet.
pub async fn run_embeddings(app: &Arc<App>, brain: &Arc<Brain>) -> Result<usize> {
    let Some(emb) = app.embedder() else {
        return Ok(0);
    };
    let model = emb.id().to_string();
    let mut total = 0;
    loop {
        let m = model.clone();
        let chunks = brain
            .db(move |s| Ok(s.chunks_missing_embedding(&m, EMBED_BATCH)?))
            .await?;
        if chunks.is_empty() {
            break;
        }
        let n = chunks.len();
        let e = emb.clone();
        let entries = tokio::task::spawn_blocking(move || -> Result<Vec<(Id, Vec<f32>)>> {
            let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
            let vecs = e.embed(&texts)?;
            Ok(chunks.into_iter().map(|c| c.id).zip(vecs).collect())
        })
        .await??;
        let m = model.clone();
        let stored = brain
            .db(move |s| Ok(s.put_embeddings(&m, &entries)?))
            .await?;
        total += stored;
        if stored == 0 || n < EMBED_BATCH {
            break;
        }
    }
    if total > 0 {
        tracing::info!("embedded {total} chunks with {model}");
    }
    Ok(total)
}

// ---------------------------------------------------------------------------------------
// mind

struct MindInput {
    ids: Vec<Id>,
    vecs: Vec<Vec<f32>>,
    texts: Vec<String>,
    placements: Vec<Placement>,
    params: MindParams,
    privacy: HashMap<Id, Privacy>,
}

/// Recompute regions and layout (full) or place new items into the existing layout.
/// `force_full` runs a full recompute regardless of `min_interval`.
pub async fn run_mind(
    app: &Arc<App>,
    brain: &Arc<Brain>,
    force_full: bool,
    min_interval: Duration,
) -> Result<Option<MindReport>> {
    let Some(emb) = app.embedder() else {
        return Ok(None);
    };
    let model = emb.id().to_string();
    let mind_model = brain.job_state().mind_model;
    // the demo brain ships arranged with a real model; while only the hash embedder is at hand
    // (`--hash-embedder`, or the model could not load) keep that arrangement rather than
    // redraw the whole brain from hashes
    if brain.is_demo
        && model.starts_with("hash@")
        && mind_model.as_deref().is_some_and(|m| m != model)
    {
        return Ok(None);
    }
    let model_changed = mind_model.as_deref() != Some(model.as_str());
    let force_full = force_full || model_changed;
    let m = model.clone();
    let input = brain
        .db(move |s| {
            let (ids, vecs) = s.item_vectors(&m)?;
            let items = s.list_items(&ItemFilter::default())?;
            let by_id: HashMap<&Id, &Item> = items.iter().map(|i| (&i.id, i)).collect();
            let texts = ids
                .iter()
                .map(|id| {
                    by_id
                        .get(id)
                        .map(|i| format!("{}\n{}", i.title, i.body))
                        .unwrap_or_default()
                })
                .collect();
            let privacy = items.iter().map(|i| (i.id.clone(), i.privacy)).collect();
            Ok(MindInput {
                ids,
                vecs,
                texts,
                placements: s.placements()?,
                params: mind_params(s),
                privacy,
            })
        })
        .await?;
    if input.ids.is_empty() {
        return Ok(None);
    }
    let mut state = brain.mind_state();
    let mut jobs = brain.job_state();
    let live: HashSet<&Id> = input.ids.iter().collect();
    let in_layout: HashSet<&Id> = state.layout.ids.iter().collect();
    let new_ids: Vec<Id> = input
        .ids
        .iter()
        .filter(|i| !in_layout.contains(i))
        .cloned()
        .collect();
    let removed = state
        .layout
        .ids
        .iter()
        .filter(|i| !live.contains(i))
        .count();
    let placed: HashSet<&Id> = input.placements.iter().map(|p| &p.item_id).collect();
    let unplaced = input.ids.iter().filter(|i| !placed.contains(i)).count();
    if new_ids.is_empty() && removed == 0 && unplaced == 0 && !force_full {
        return Ok(None);
    }
    let due = jobs
        .last_full_run
        .is_none_or(|t| (now() - t).to_std().unwrap_or_default() >= min_interval);
    // a full run when forced, on first run, or when the corpus changed and enough time passed;
    // also when the layout has grown by half since the last full run
    let grown = new_ids.len() * 2 > state.layout.ids.len().max(1);
    let full = force_full || state.layout.ids.is_empty() || due || grown;

    if input.ids.len() < TINY {
        return tiny_layout(brain, &input.ids, &model).await.map(Some);
    }

    if full {
        let MindInput {
            ids,
            vecs,
            texts,
            params,
            privacy,
            ..
        } = input;
        let prev = state.clone();
        let mut run = tokio::task::spawn_blocking(move || {
            let at = now();
            let mut r = full_run(&ids, &vecs, Some(&texts), &prev, at, &params);
            sanitize(&mut r.state.layout.xyz);
            sanitize(&mut r.views.layout);
            sanitize(&mut r.views.anatomy);
            sanitize(&mut r.views.spheres);
            for p in r.placements.iter_mut() {
                sanitize(std::slice::from_mut(&mut p.xyz));
            }
            (r, ids, texts)
        })
        .await?;
        let (ref mut fr, ref ids, ref texts) = run;
        // region labels from the label route, when a background-capable backend exists
        let labelled_by = label_regions(app, brain, fr, ids, texts, &privacy).await;
        let run_id = Id::new();
        let mut regions: Vec<Region> = fr.regions.regions.clone();
        regions.extend(fr.regions.retired.iter().cloned());
        let transitions = fr.regions.transitions.clone();
        let placements = fr.placements.clone();
        let n_regions = fr.regions.regions.len();
        let rid = run_id.clone();
        brain
            .db(move |s| {
                s.record_regions(rid.clone(), regions, transitions, sys("mind:cluster"))?;
                s.replace_placements(rid, &placements, sys("mind:layout"))?;
                Ok(())
            })
            .await?;
        brain.save_mind_state(&fr.state)?;
        brain.save_views(&fr.views)?;
        jobs = brain.job_state();
        jobs.last_full_run = Some(now());
        jobs.mind_model = Some(model.clone());
        brain.save_job_state(&jobs)?;
        tracing::info!(
            "mind: full run over {} items, {n_regions} regions",
            ids.len()
        );
        return Ok(Some(MindReport {
            full: true,
            items: ids.len(),
            placed_new: 0,
            regions: n_regions,
            labelled_by,
        }));
    }

    // incremental: drop removed items, place new ones, keep everyone else where they are
    let vec_of: HashMap<&Id, &Vec<f32>> = input.ids.iter().zip(&input.vecs).collect();
    let keep: Vec<usize> = (0..state.layout.ids.len())
        .filter(|&i| vec_of.contains_key(&state.layout.ids[i]))
        .collect();
    state.layout.ids = keep.iter().map(|&i| state.layout.ids[i].clone()).collect();
    state.layout.xyz = keep.iter().map(|&i| state.layout.xyz[i]).collect();
    let existing: Vec<Vec<f32>> = state.layout.ids.iter().map(|i| vec_of[i].clone()).collect();
    let new_vecs: Vec<Vec<f32>> = new_ids.iter().map(|i| vec_of[i].clone()).collect();
    let params = input.params;
    let old_region: HashMap<Id, Option<Id>> = input
        .placements
        .iter()
        .map(|p| (p.item_id.clone(), p.region_id.clone()))
        .collect();
    let n_new = new_ids.len();
    let (state, placements, lv) = tokio::task::spawn_blocking(move || {
        let mut state = state;
        let assigned = state.regions.assign(&new_vecs);
        place_new(
            &mut state.layout,
            &existing,
            &new_ids,
            &new_vecs,
            &params.layout,
        );
        let new_region: HashMap<&Id, Option<Id>> = new_ids.iter().zip(assigned).collect();
        let assignment: Vec<Option<Id>> = state
            .layout
            .ids
            .iter()
            .map(|id| {
                new_region
                    .get(id)
                    .cloned()
                    .or_else(|| old_region.get(id).cloned())
                    .flatten()
            })
            .collect();
        sanitize(&mut state.layout.xyz);
        let placements = state.layout.placements(&assignment);
        let mut lv = views(&state.layout, &assignment);
        sanitize(&mut lv.anatomy);
        sanitize(&mut lv.spheres);
        (state, placements, lv)
    })
    .await?;
    let count = placements.len();
    brain
        .db(move |s| {
            s.replace_placements(Id::new(), &placements, sys("mind:place"))?;
            Ok(())
        })
        .await?;
    brain.save_mind_state(&state)?;
    brain.save_views(&lv)?;
    Ok(Some(MindReport {
        full: false,
        items: count,
        placed_new: n_new,
        regions: state.regions.regions.len(),
        labelled_by: None,
    }))
}

/// Below this many items the layout and clustering have too little to work with: items
/// are set on a small ring, without regions, until the brain grows.
const TINY: usize = 5;

fn sanitize(xyz: &mut [[f32; 3]]) {
    for p in xyz.iter_mut() {
        for c in p.iter_mut() {
            if !c.is_finite() {
                *c = 0.0;
            }
        }
    }
}

async fn tiny_layout(brain: &Arc<Brain>, ids: &[Id], model: &str) -> Result<MindReport> {
    let n = ids.len();
    let xyz: Vec<[f32; 3]> = (0..n)
        .map(|i| {
            if n == 1 {
                return [0.0; 3];
            }
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [0.3 * a.cos(), 0.3 * a.sin(), 0.0]
        })
        .collect();
    let layout = anamnesis_mind::Layout {
        ids: ids.to_vec(),
        xyz,
    };
    let assignment = vec![None; n];
    let placements = layout.placements(&assignment);
    let lv = views(&layout, &assignment);
    brain
        .db(move |s| {
            s.replace_placements(Id::new(), &placements, sys("mind:layout"))?;
            Ok(())
        })
        .await?;
    brain.save_views(&anamnesis_mind::LayoutViews {
        ids: layout.ids.clone(),
        layout: layout.xyz.clone(),
        anatomy: if lv.anatomy.iter().flatten().all(|c| c.is_finite()) {
            lv.anatomy
        } else {
            layout.xyz.clone()
        },
        spheres: if lv.spheres.iter().flatten().all(|c| c.is_finite()) {
            lv.spheres
        } else {
            layout.xyz.clone()
        },
    })?;
    // the next real run starts fresh
    brain.save_mind_state(&MindState::default())?;
    let mut js = brain.job_state();
    js.mind_model = Some(model.to_string());
    brain.save_job_state(&js)?;
    Ok(MindReport {
        full: true,
        items: n,
        placed_new: 0,
        regions: 0,
        labelled_by: None,
    })
}

/// Ask the label route to name regions that need a label. Never runs on subscription
/// backends (the router enforces it); falls back to mind's deterministic labels.
async fn label_regions(
    app: &Arc<App>,
    brain: &Arc<Brain>,
    run: &mut anamnesis_mind::FullRun,
    ids: &[Id],
    texts: &[String],
    privacy: &HashMap<Id, Privacy>,
) -> Option<String> {
    if run.regions.needs_label.is_empty() {
        return None;
    }
    let router = app.router()?;
    let inputs: Vec<_> = run
        .regions
        .label_inputs
        .iter()
        .filter(|l| run.regions.needs_label.contains(&l.region_id))
        .cloned()
        .collect();
    if inputs.is_empty() {
        return None;
    }
    let text_of: HashMap<&Id, &String> = ids.iter().zip(texts).collect();
    let snippet = |id: &Id| {
        text_of
            .get(id)
            .map(|t| t.chars().take(400).collect::<String>())
    };
    let prompt = anamnesis_mind::labels::label_prompt(&inputs, &snippet);
    let items: Vec<(Id, Privacy)> = inputs
        .iter()
        .flat_map(|l| l.representatives.iter())
        .map(|id| (id.clone(), privacy.get(id).copied().unwrap_or_default()))
        .collect();
    let mut req = ChatRequest::new(vec![Message::user(prompt.user)]).with_system(prompt.system);
    req.json_schema = Some(prompt.schema);
    req.temperature = Some(0.2);
    let routed = match router
        .chat(
            Task::Label,
            req,
            CallContext {
                items,
                approved: vec![],
                backend: None,
            },
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("region labels: no model ({e}); keeping deterministic labels");
            return None;
        }
    };
    let backend = routed.backend.clone();
    if let Some(eg) = routed.egress {
        let kind = eg.into_event_kind();
        let b = backend.clone();
        let _ = brain
            .db(move |s| Ok(s.append(Actor::Model(b), kind)?))
            .await;
    }
    let mut text = String::new();
    let mut stream = routed.stream;
    let collect = async {
        while let Some(ev) = stream.next().await {
            match ev {
                ChatEvent::Token { text: t } => text.push_str(&t),
                ChatEvent::Error { message, .. } => return Err(message),
                ChatEvent::Done { .. } => break,
                ChatEvent::Usage(_) => {}
            }
        }
        Ok(())
    };
    match tokio::time::timeout(Duration::from_secs(120), collect).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::warn!("region labels from {backend} failed: {e}");
            return None;
        }
        Err(_) => {
            tracing::warn!("region labels from {backend} timed out");
            return None;
        }
    }
    let answer: serde_json::Value = match serde_json::from_str(extract_json(&text)) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("region labels from {backend}: not json ({e})");
            return None;
        }
    };
    let labels = anamnesis_mind::labels::parse_labels(&answer);
    if labels.is_empty() {
        return None;
    }
    apply_labels(&mut run.regions, &labels);
    // keep the persisted state's labels in step with the recorded regions
    for snap in run.state.regions.regions.iter_mut() {
        if let Some(l) = labels.get(&snap.region.id) {
            snap.region.label = l.clone();
        }
    }
    Some(backend)
}

/// The outermost `{...}` of a model answer (models sometimes wrap json in prose or fences).
fn extract_json(s: &str) -> &str {
    match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b > a => &s[a..=b],
        _ => s,
    }
}

// ---------------------------------------------------------------------------------------
// link suggestions

/// Suggest links for items that have none computed yet: top suggestions are split into a
/// shown arm and a held-out control arm (seeded, ~20% held out), all recorded as events.
pub async fn run_suggestions(app: &Arc<App>, brain: &Arc<Brain>) -> Result<usize> {
    let Some(emb) = app.embedder() else {
        return Ok(0);
    };
    let model = emb.id().to_string();
    let done = brain.job_state().suggested;
    let (n, newly) = brain
        .db(move |s| {
            let (ids, vecs) = s.item_vectors(&model)?;
            if ids.len() < 2 {
                return Ok((0, Vec::new()));
            }
            let todo: Vec<Id> = ids
                .iter()
                .filter(|i| !done.contains(&i.0))
                .cloned()
                .collect();
            if todo.is_empty() {
                return Ok((0, Vec::new()));
            }
            let seed = seed_setting(s, "holdout_seed")?;
            let items = s.list_item_summaries(&ItemFilter::default())?;
            let meta_of: HashMap<&Id, &anamnesis_core::ItemSummary> =
                items.iter().map(|i| (&i.id, i)).collect();
            let region_of: HashMap<Id, Option<Id>> = s
                .placements()?
                .into_iter()
                .map(|p| (p.item_id, p.region_id))
                .collect();
            let meta: Vec<ItemMeta> = ids
                .iter()
                .map(|id| ItemMeta {
                    source_id: meta_of.get(id).map(|m| m.source_id.clone()),
                    session: None,
                    created_at: meta_of.get(id).map(|m| m.created_at),
                    region: region_of.get(id).cloned().flatten(),
                })
                .collect();
            let links = s.all_links()?;
            let sugg = LinkSuggester::new(&ids, &vecs, &links, &meta);
            let params = LinkParams {
                top_k: 5,
                ..Default::default()
            };
            let tag = format!("links-v1/{model}");
            let mut n = 0;
            for id in &todo {
                let armed = split_for_holdout(sugg.suggest(id, &params), HOLDOUT_RATE, seed);
                for a in armed {
                    let x = &a.suggestion;
                    s.record_suggestion(
                        &x.from,
                        &x.to,
                        x.score,
                        x.rank,
                        a.shown,
                        &tag,
                        sys("mind:links"),
                    )?;
                    n += 1;
                }
            }
            Ok((n, todo))
        })
        .await?;
    if !newly.is_empty() {
        let mut js = brain.job_state();
        js.suggested.extend(newly.into_iter().map(|i| i.0));
        brain.save_job_state(&js)?;
    }
    Ok(n)
}

// ---------------------------------------------------------------------------------------
// threads

/// Setting that records which rule extractor produced the stored rule threads. When it
/// differs from [`RULE_EXTRACTOR`], every item is re-extracted once (`rethread`).
const RULE_VERSION_SETTING: &str = "threads_rule_extractor";

/// Is an item's text the owner's own? Its origin must be an owner origin, and a
/// frontmatter that declares another origin (`origin: external`) wins: an essay or an
/// article imported from a folder holds someone else's questions and decisions.
fn owner_text(it: &Item) -> bool {
    if it.deleted || !it.origin.is_owner() {
        return false;
    }
    let declared = it
        .meta
        .get("frontmatter")
        .and_then(|f| f.get("origin"))
        .and_then(|o| serde_json::from_value::<anamnesis_core::Origin>(o.clone()).ok());
    declared.is_none_or(|o| o.is_owner())
}

/// A thread nobody has touched since it was extracted (no status change, no link).
fn untouched(t: &anamnesis_core::Thread) -> bool {
    t.updated_at == t.created_at && t.resolved_by.is_none()
}

/// Rule-based threads for items created or changed since the last pass (owner-written
/// text only: decisions and questions are the user's, not an article's).
///
/// Re-extracting an item replaces its earlier rule threads: one the rules still find is
/// kept (with its id and status; its `extractor` is brought up to date), one they no longer
/// find is withdrawn (`ThreadRemoved`) unless the user has touched it. When the extractor
/// version changed since the last pass, every item is re-extracted once.
/// Returns the number of threads added.
pub async fn run_threads(brain: &Arc<Brain>) -> Result<usize> {
    let seen = brain.job_state().threads_hash;
    let (n, removed, rethread, processed) = brain
        .db(move |s| {
            let stored = s
                .get_setting(RULE_VERSION_SETTING)?
                .and_then(|v| v.as_str().map(str::to_string));
            let rethread = stored.as_deref() != Some(RULE_EXTRACTOR);
            let items = s.list_items(&ItemFilter::default())?;
            let mut by_item: HashMap<Id, Vec<anamnesis_core::Thread>> = HashMap::new();
            for t in s.threads(&ThreadFilter::default())? {
                by_item.entry(t.item_id.clone()).or_default().push(t);
            }
            let actor = sys("mind:threads");
            let mut n = 0;
            let mut stale = Vec::new();
            let mut processed = Vec::new();
            for it in items {
                if !rethread && seen.get(&it.id.0) == Some(&it.content_hash) {
                    continue;
                }
                let existing = by_item.remove(&it.id).unwrap_or_default();
                let fresh = if owner_text(&it) {
                    extract_rules(&it.id, &it.body, it.created_at)
                } else {
                    Vec::new()
                };
                let mut kept: Vec<&anamnesis_core::Thread> = Vec::new();
                for e in &existing {
                    if !is_rule_extractor(&e.extractor) {
                        kept.push(e);
                        continue;
                    }
                    let found = fresh.iter().any(|t| t.quote == e.quote && t.kind == e.kind);
                    if found && e.extractor != RULE_EXTRACTOR {
                        let mut up = e.clone();
                        up.extractor = RULE_EXTRACTOR.to_string();
                        s.upsert_thread(up, actor.clone())?;
                    }
                    if !found && untouched(e) {
                        stale.push(e.id.clone());
                    } else {
                        kept.push(e);
                    }
                }
                for t in fresh {
                    // the same span may already be a thread (kept above, or from a model)
                    if kept.iter().any(|e| e.quote == t.quote) {
                        continue;
                    }
                    s.upsert_thread(t, actor.clone())?;
                    n += 1;
                }
                processed.push((it.id.0, it.content_hash));
            }
            let removed = s.remove_threads(&stale, actor)?;
            if rethread {
                s.set_setting(RULE_VERSION_SETTING, json!(RULE_EXTRACTOR))?;
            }
            Ok((n, removed, rethread, processed))
        })
        .await?;
    if rethread || removed > 0 {
        tracing::info!(
            "threads: {RULE_EXTRACTOR} {} — {n} added, {removed} withdrawn",
            if rethread {
                "re-read every item"
            } else {
                "pass"
            }
        );
    }
    if !processed.is_empty() {
        let mut js = brain.job_state();
        js.threads_hash.extend(processed);
        brain.save_job_state(&js)?;
    }
    Ok(n)
}

/// Items sent to the extract route per pass (the job loop runs at most every few seconds
/// after a change, else every 30 s): a slow trickle, newest first.
pub const EXTRACT_BATCH: usize = 8;
/// Longest text sent for extraction, in characters (quotes are validated against it).
const EXTRACT_MAX_CHARS: usize = 12_000;
/// Model answers below this confidence are dropped.
const EXTRACT_MIN_CONFIDENCE: f32 = 0.5;

/// Model-based threads through the `extract` route, for owner-written items that are new
/// or changed since a model last read them. Runs only when the router has a backend for
/// background extraction (local by default; an api key only if the user opted in or
/// named one). Without one, or when a call fails, the rule-based threads stand alone.
/// Returns the number of threads added.
pub async fn run_model_threads(app: &Arc<App>, brain: &Arc<Brain>) -> Result<usize> {
    let Some(router) = app.router() else {
        return Ok(0);
    };
    let Ok(plan) = router.plan(Task::Extract, None) else {
        return Ok(0);
    };
    let Some(primary) = plan.first() else {
        return Ok(0);
    };
    // only cloud_ok items may reach a backend that is not local; never ask in the background
    let local = primary.is_local();
    let done = brain.job_state().extracted_hash;
    let (todo, pending) = brain
        .db(move |s| {
            let mut items: Vec<Item> = s
                .list_items(&ItemFilter::default())?
                .into_iter()
                .filter(|i| !i.deleted && i.origin.is_owner())
                .filter(|i| done.get(&i.id.0) != Some(&i.content_hash))
                .filter(|i| !i.body.trim().is_empty())
                .filter(|i| local || i.privacy == Privacy::CloudOk)
                .collect();
            items.sort_by_key(|i| std::cmp::Reverse(i.modified_at));
            let pending = items.len() > EXTRACT_BATCH;
            items.truncate(EXTRACT_BATCH);
            Ok((items, pending))
        })
        .await?;
    let mut added = 0;
    let mut finished = Vec::new();
    let mut failed = false;
    for item in todo {
        match extract_one(brain, &router, &item).await {
            Ok(n) => {
                added += n;
                finished.push((item.id.0.clone(), item.content_hash.clone()));
            }
            Err(e) => {
                // the backend is down or refusing: leave the rest for a later pass
                tracing::debug!("thread extraction: {e}; rules only for now");
                failed = true;
                break;
            }
        }
    }
    let mut js = brain.job_state();
    js.extracted_hash.extend(finished);
    js.extract_pending = pending && !failed;
    brain.save_job_state(&js)?;
    if added > 0 {
        tracing::info!("threads: {added} found by the extract model");
    }
    Ok(added)
}

async fn extract_one(
    brain: &Arc<Brain>,
    router: &anamnesis_harness::Router,
    item: &Item,
) -> std::result::Result<usize, String> {
    let text: String = item.body.chars().take(EXTRACT_MAX_CHARS).collect();
    let prompt = anamnesis_mind::extraction_prompt(&item.title, &text);
    let mut req = ChatRequest::new(vec![Message::user(prompt.user)]).with_system(prompt.system);
    req.json_schema = Some(prompt.schema);
    req.temperature = Some(0.0);
    let routed = router
        .chat(
            Task::Extract,
            req,
            CallContext {
                items: vec![(item.id.clone(), item.privacy)],
                approved: vec![],
                backend: None,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let backend = routed.backend.clone();
    let extractor = match router.get(&backend) {
        Some(b) if b.is_local() => format!("local:{}", routed.model),
        Some(b) => format!("{}:{}", b.provider(), routed.model),
        None => format!("{backend}:{}", routed.model),
    };
    if let Some(eg) = routed.egress {
        let kind = eg.into_event_kind();
        let b = backend.clone();
        brain
            .db(move |s| Ok(s.append(Actor::Model(b), kind)?))
            .await
            .map_err(|e| e.to_string())?;
    }
    let answer = collect_text(routed.stream, &backend).await?;
    let report = anamnesis_mind::threads::validate_extraction_str(
        &text,
        &answer,
        &item.id,
        item.created_at,
        &extractor,
        EXTRACT_MIN_CONFIDENCE,
    )
    .map_err(|e| format!("{backend}: {e}"))?;
    let item_id = item.id.clone();
    let actor = sys(&format!("mind:threads:{extractor}"));
    brain
        .db(move |s| {
            let existing = s.threads(&ThreadFilter {
                item_id: Some(item_id),
                ..Default::default()
            })?;
            let mut n = 0;
            for t in report.accepted {
                // the rules (or an earlier model pass) may have found the same span
                if existing
                    .iter()
                    .any(|e| e.quote == t.quote && e.kind == t.kind)
                {
                    continue;
                }
                s.upsert_thread(t, actor.clone())?;
                n += 1;
            }
            Ok(n)
        })
        .await
        .map_err(|e| e.to_string())
}

/// Collect a routed stream's text (two minutes at most).
async fn collect_text(
    mut stream: anamnesis_harness::ChatStream,
    backend: &str,
) -> std::result::Result<String, String> {
    let mut text = String::new();
    let collect = async {
        while let Some(ev) = stream.next().await {
            match ev {
                ChatEvent::Token { text: t } => text.push_str(&t),
                ChatEvent::Error { message, .. } => return Err(message),
                ChatEvent::Done { .. } => break,
                ChatEvent::Usage(_) => {}
            }
        }
        Ok(())
    };
    match tokio::time::timeout(Duration::from_secs(120), collect).await {
        Ok(Ok(())) => Ok(text),
        Ok(Err(e)) => Err(format!("{backend}: {e}")),
        Err(_) => Err(format!("{backend}: timed out")),
    }
}

// ---------------------------------------------------------------------------------------
// daily review

/// Draw today's review once per day (micro-randomised: each slot goes to the algorithm or
/// to a random pick) and record each shown item with its arm.
pub async fn run_review(app: &Arc<App>, brain: &Arc<Brain>, force: bool) -> Result<usize> {
    let Some(emb) = app.embedder() else {
        return Ok(0);
    };
    let today = now().date_naive().to_string();
    let js = brain.job_state();
    if !force && js.review_day.as_deref() == Some(today.as_str()) {
        return Ok(0);
    }
    let model = emb.id().to_string();
    let picks = brain
        .db(move |s| {
            let (ids, vecs) = s.item_vectors(&model)?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let seed = seed_setting(s, "review_seed")?;
            let items = s.list_item_summaries(&ItemFilter::default())?;
            let created: HashMap<&Id, Timestamp> =
                items.iter().map(|i| (&i.id, i.created_at)).collect();
            let mut last_seen: HashMap<Id, (Timestamp, u32)> = HashMap::new();
            for r in s.resurfacings(false)? {
                let e = last_seen.entry(r.item_id).or_insert((r.shown_at, 0));
                e.0 = e.0.max(r.shown_at);
                e.1 += 1;
            }
            let open_q: HashSet<Id> = s
                .threads(&ThreadFilter {
                    kind: Some(ThreadKind::OpenQuestion),
                    status: Some(ThreadStatus::Open),
                    ..Default::default()
                })?
                .into_iter()
                .map(|t| t.item_id)
                .collect();
            let t = now();
            let info: Vec<ReviewInfo> = ids
                .iter()
                .map(|id| {
                    let c = created.get(id).copied().unwrap_or(t);
                    let ls = last_seen.get(id);
                    let is_q = open_q.contains(id);
                    ReviewInfo {
                        created_at: c,
                        last_seen: ls.map(|x| x.0),
                        times_shown: ls.map(|x| x.1).unwrap_or(0),
                        card: is_q.then(|| SrsCard::new(c)),
                        is_open_question: is_q,
                        centrality: None,
                    }
                })
                .collect();
            // recent attention: items written in the last two days
            let recent_cut = t - chrono::Duration::days(2);
            let mut recent = Vec::new();
            let mut recent_ids = Vec::new();
            for (i, id) in ids.iter().enumerate() {
                if created.get(id).is_some_and(|c| *c >= recent_cut) {
                    recent.push(vecs[i].clone());
                    recent_ids.push(id.clone());
                }
            }
            let picks = daily_review(
                &ids,
                &vecs,
                &info,
                &recent,
                &recent_ids,
                t,
                &ReviewParams::default(),
                &Scheduler::default(),
                seed,
            );
            let mut out = Vec::new();
            for p in picks {
                let rid = s.record_resurface(&p.item_id, p.arm.as_str(), sys("mind:review"))?;
                let reason = serde_json::to_value(p.reason)?
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                out.push((rid.0, reason));
            }
            Ok(out)
        })
        .await?;
    let mut js = brain.job_state();
    js.review_day = Some(today);
    let n = picks.len();
    js.review_reasons.extend(picks);
    brain.save_job_state(&js)?;
    Ok(n)
}

// ---------------------------------------------------------------------------------------
// the loop

/// One pass of every job, synchronously (cli commands and tests). Runs a full mind
/// recompute and draws the daily review if it was not drawn today.
pub async fn run_jobs_once(app: &Arc<App>) -> Result<JobsReport> {
    let r = run_jobs_once_inner(app).await;
    let mut p = app.jobs_progress.write().unwrap();
    p.running = false;
    p.phase = None;
    p.error = r.as_ref().err().map(|e| format!("{e:#}"));
    r
}

/// Phases of [`run_jobs_once`], in order (progress in `/api/status`).
pub const PHASES: [&str; 5] = ["embeddings", "threads", "mind", "suggestions", "review"];

fn phase(app: &App, i: usize) {
    let mut p = app.jobs_progress.write().unwrap();
    p.running = true;
    p.total = PHASES.len() as u32;
    p.done = i as u32;
    p.phase = PHASES.get(i).map(|s| s.to_string());
    p.error = None;
}

async fn run_jobs_once_inner(app: &Arc<App>) -> Result<JobsReport> {
    phase(app, 0);
    if app.embedder().is_none() {
        app.load_embedder().await;
    }
    let brain = app.brain();
    let _g = brain.jobs_lock.lock().await;
    let mut r = JobsReport {
        embedded: run_embeddings(app, &brain).await?,
        ..Default::default()
    };
    phase(app, 1);
    r.threads = run_threads(&brain).await?;
    r.threads += run_model_threads(app, &brain).await?;
    phase(app, 2);
    r.mind = run_mind(app, &brain, true, Duration::ZERO).await?;
    phase(app, 3);
    r.suggestions = run_suggestions(app, &brain).await?;
    phase(app, 4);
    r.resurfaced = run_review(app, &brain, false).await?;
    mark_seen(&brain).await?;
    app.jobs_progress.write().unwrap().done = PHASES.len() as u32;
    Ok(r)
}

async fn mark_seen(brain: &Arc<Brain>) -> Result<()> {
    let head = brain
        .db(|s| Ok(s.head()?.map(|h| h.0).unwrap_or(0)))
        .await?;
    let mut js = brain.job_state();
    if js.seen_head != head {
        js.seen_head = head;
        brain.save_job_state(&js)?;
    }
    Ok(())
}

/// Minutes between full mind recomputes (setting `mind_interval_minutes`, default 10).
async fn mind_interval(brain: &Arc<Brain>) -> Duration {
    let m = brain
        .db(|s| {
            Ok(s.get_setting("mind_interval_minutes")?
                .and_then(|v| v.as_u64()))
        })
        .await
        .ok()
        .flatten()
        .unwrap_or(10);
    Duration::from_secs(m * 60)
}

/// The background job loop: wakes on change notices (debounced) or every 30 s (to notice
/// writes from other processes, e.g. `anamnesis mcp`), until cancelled.
pub async fn jobs_loop(app: Arc<App>) {
    loop {
        tokio::select! {
            _ = app.cancel.cancelled() => return,
            _ = app.jobs_notify.notified() => {
                // debounce bursts of changes (a sync writes many items)
                tokio::select! {
                    _ = app.cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
        if let Err(e) = jobs_pass(&app).await {
            tracing::warn!("background jobs: {e:#}");
        }
    }
}

async fn jobs_pass(app: &Arc<App>) -> Result<()> {
    let brain = app.brain();
    let _g = brain.jobs_lock.lock().await;
    let head = brain
        .db(|s| Ok(s.head()?.map(|h| h.0).unwrap_or(0)))
        .await?;
    let today = now().date_naive().to_string();
    let js = brain.job_state();
    let changed = js.seen_head != head;
    let review_due = js.review_day.as_deref() != Some(today.as_str());
    let embedded = run_embeddings(app, &brain).await?;
    if !changed && embedded == 0 && !review_due && !js.extract_pending {
        return Ok(());
    }
    run_threads(&brain).await?;
    run_model_threads(app, &brain).await?;
    let interval = mind_interval(&brain).await;
    run_mind(app, &brain, false, interval).await?;
    run_suggestions(app, &brain).await?;
    run_review(app, &brain, false).await?;
    mark_seen(&brain).await?;
    Ok(())
}
