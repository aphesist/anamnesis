//! The http api of `docs/contracts.md`, plus `/mcp` and the static ui.

use std::collections::HashMap;
use std::sync::Arc;

use anamnesis_core::{
    Actor, Id, ItemFilter, ItemKind, ItemPatch, NewNote, Origin, Privacy, Source, ThreadFilter,
    ThreadKind, ThreadStatus, now,
};
use anamnesis_embed::EmbedderSpec;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::App;
use crate::chat::{ChatBody, RETRIEVAL_BACKEND, hybrid_search};
use crate::mcp_brain::{StoreBrain, item_summary};

// ---------------------------------------------------------------------------------------
// errors

pub struct ApiError(StatusCode, String);

impl ApiError {
    pub fn message(&self) -> &str {
        &self.1
    }
    pub fn bad(msg: impl Into<String>) -> Self {
        ApiError(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        ApiError(StatusCode::NOT_FOUND, msg.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast_ref::<anamnesis_core::Error>() {
            Some(anamnesis_core::Error::NotFound(m)) => {
                ApiError(StatusCode::NOT_FOUND, format!("not found: {m}"))
            }
            Some(anamnesis_core::Error::Invalid(m)) => ApiError(StatusCode::BAD_REQUEST, m.clone()),
            _ => {
                tracing::warn!("api error: {e:#}");
                ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T = Json<Value>> = Result<T, ApiError>;
type S = State<Arc<App>>;

fn ok(v: impl serde::Serialize) -> ApiResult {
    Ok(Json(
        serde_json::to_value(v).map_err(|e| ApiError::from(anyhow::Error::from(e)))?,
    ))
}

fn parse_enum<T: serde::de::DeserializeOwned>(s: &str, what: &str) -> Result<T, ApiError> {
    serde_json::from_value(Value::String(s.to_string()))
        .map_err(|_| ApiError::bad(format!("unknown {what} {s:?}")))
}

// ---------------------------------------------------------------------------------------
// router

/// The whole app: `/api/*`, `/mcp` (bearer token) and the ui.
pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route("/cloud", get(cloud))
        .route("/items", post(create_item))
        .route("/items/{id}", get(get_item).put(update_item))
        .route("/search", get(search))
        .route("/chat", post(chat))
        .route("/timeline", get(timeline))
        .route("/threads", get(threads))
        .route("/threads/{id}", post(update_thread))
        .route("/review", get(review))
        .route("/review/{id}", post(rate_review))
        .route("/suggestions", get(suggestions))
        .route("/suggestions/{id}", post(decide_suggestion))
        .route("/links", post(add_link))
        .route("/lineage", get(lineage))
        .route("/eval", get(eval))
        .route("/eval/label", post(eval_label))
        .route("/skills", get(skills))
        .route("/sources", get(sources).post(add_source))
        .route("/sources/{id}", delete(remove_source).put(update_source))
        .route("/sources/{id}/sync", post(sync_source))
        .route("/backends", get(backends))
        .route("/backends/refresh", post(refresh_backends))
        .route(
            "/backends/{id}/key",
            axum::routing::put(set_backend_key).delete(delete_backend_key),
        )
        .route("/backends/chatgpt", get(chatgpt_status))
        .route("/backends/chatgpt/sign-in", post(chatgpt_sign_in))
        .route("/backends/chatgpt/sign-out", post(chatgpt_sign_out))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/egress", get(egress))
        .route("/events", get(events))
        .route("/export", get(export))
        .route("/mcp/snippets", get(mcp_snippets))
        .route("/demo", get(demo_status).post(demo))
        .route("/jobs/run", post(run_jobs))
        .route("/status", get(status))
        .layer(middleware::from_fn(loopback_host_only))
        .with_state(app.clone());

    let token = app.brain().mcp_token().ok();
    let mcp = anamnesis_mcp::http_router(
        StoreBrain::new(app.clone()),
        anamnesis_mcp::HttpOptions {
            bearer_token: token,
            fallback_client: "http".into(),
            cancellation_token: app.cancel.child_token(),
            ..Default::default()
        },
    );

    let ui_dir = app.cfg.ui_dir.clone();
    Router::new()
        .nest("/api", api)
        .merge(mcp)
        .fallback(move |uri: axum::http::Uri| crate::ui::serve(ui_dir.clone(), uri))
}

/// DNS-rebinding guard: the api answers only to loopback host names.
async fn loopback_host_only(req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| {
            let h = h.trim();
            if let Some(rest) = h.strip_prefix('[') {
                rest.split(']').next().unwrap_or("").to_string()
            } else {
                h.rsplit_once(':').map(|(a, _)| a).unwrap_or(h).to_string()
            }
        });
    match host.as_deref() {
        None | Some("localhost") | Some("127.0.0.1") | Some("::1") => next.run(req).await,
        Some(_) => (StatusCode::FORBIDDEN, "forbidden host").into_response(),
    }
}

// ---------------------------------------------------------------------------------------
// the brain

fn speaker_index(o: Origin) -> u8 {
    match o.speaker() {
        anamnesis_core::Speaker::You => 0,
        anamnesis_core::Speaker::Assistants => 1,
        anamnesis_core::Speaker::Other => 2,
    }
}

async fn cloud(State(app): S) -> ApiResult {
    let brain = app.brain();
    let views = brain.views();
    let v = brain
        .db(move |s| {
            let items = s.list_item_summaries(&ItemFilter::default())?;
            let placements: HashMap<Id, anamnesis_core::Placement> = s
                .placements()?
                .into_iter()
                .map(|p| (p.item_id.clone(), p))
                .collect();
            let regions: Vec<anamnesis_core::Region> = s
                .current_regions()?
                .into_iter()
                .filter(|r| r.died_at.is_none())
                .collect();
            let region_idx: HashMap<&Id, usize> = regions
                .iter()
                .enumerate()
                .map(|(i, r)| (&r.id, i))
                .collect();
            type PosMap<'a> = HashMap<&'a Id, [f32; 3]>;
            let (anat, sph): (PosMap, PosMap) = match &views {
                Some(lv) => (
                    lv.ids.iter().zip(lv.anatomy.iter().copied()).collect(),
                    lv.ids.iter().zip(lv.spheres.iter().copied()).collect(),
                ),
                None => Default::default(),
            };
            let mut kinds: Vec<String> = Vec::new();
            let n = items.len();
            let (mut ids, mut xyz, mut xa, mut xs) = (
                Vec::with_capacity(n),
                Vec::with_capacity(3 * n),
                Vec::with_capacity(3 * n),
                Vec::with_capacity(3 * n),
            );
            let (mut region, mut kind, mut speaker, mut t, mut titles) = (
                Vec::with_capacity(n),
                Vec::with_capacity(n),
                Vec::with_capacity(n),
                Vec::with_capacity(n),
                Vec::with_capacity(n),
            );
            let mut sums = vec![([0f64; 3], 0usize); regions.len()];
            let mut index: HashMap<Id, usize> = HashMap::new();
            for (i, it) in items.iter().enumerate() {
                let p = placements.get(&it.id);
                let pos = p.map(|p| p.xyz).unwrap_or([0.0; 3]);
                let r = p
                    .and_then(|p| p.region_id.as_ref())
                    .and_then(|r| region_idx.get(r).copied());
                if let Some(r) = r {
                    for (acc, p) in sums[r].0.iter_mut().zip(pos) {
                        *acc += p as f64;
                    }
                    sums[r].1 += 1;
                }
                xyz.extend_from_slice(&pos);
                xa.extend_from_slice(&anat.get(&it.id).copied().unwrap_or(pos));
                xs.extend_from_slice(&sph.get(&it.id).copied().unwrap_or(pos));
                region.push(r.map(|r| r as i32).unwrap_or(-1));
                let ks = enum_str(&it.kind);
                let ki = match kinds.iter().position(|k| *k == ks) {
                    Some(k) => k,
                    None => {
                        kinds.push(ks);
                        kinds.len() - 1
                    }
                };
                kind.push(ki as u8);
                speaker.push(speaker_index(it.origin));
                t.push(it.created_at.timestamp());
                titles.push(it.title.clone());
                ids.push(it.id.clone());
                index.insert(it.id.clone(), i);
            }
            let regions_json: Vec<Value> = regions
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let (s3, c) = sums[i];
                    let centroid = if c > 0 {
                        [s3[0] / c as f64, s3[1] / c as f64, s3[2] / c as f64]
                    } else {
                        [0.0; 3]
                    };
                    json!({
                        "id": r.id, "label": r.label, "anatomy": r.anatomy, "size": r.size,
                        "centroid": centroid, "born_at": r.born_at,
                    })
                })
                .collect();
            let links: Vec<Value> = s
                .all_links()?
                .into_iter()
                .filter_map(|l| {
                    Some(json!([
                        index.get(&l.from)?,
                        index.get(&l.to)?,
                        enum_str(&l.kind)
                    ]))
                })
                .collect();
            Ok(json!({
                "ids": ids, "xyz": xyz, "xyz_anatomy": xa, "xyz_spheres": xs,
                "region": region, "kind": kind, "speaker": speaker, "t": t, "titles": titles,
                "kinds": kinds, "regions": regions_json, "links": links,
            }))
        })
        .await?;
    Ok(Json(v))
}

async fn get_item(State(app): S, Path(id): Path<String>) -> ApiResult {
    let id = Id(id);
    let v = app
        .brain()
        .db(move |s| {
            let Some(item) = s.get_item(&id)? else { return Ok(None) };
            Ok(Some(json!({
                "links_out": s.links_from(&id)?,
                "links_in": s.links_to(&id)?,
                "threads": s.threads(&ThreadFilter { item_id: Some(id.clone()), ..Default::default() })?,
                "region": s.item_region(&id)?,
                "item": item,
            })))
        })
        .await?;
    v.map(Json)
        .ok_or_else(|| ApiError::not_found("no such item"))
}

#[derive(Deserialize)]
struct CreateItem {
    title: String,
    body: String,
    kind: Option<ItemKind>,
    #[serde(default)]
    tags: Vec<String>,
}

async fn create_item(State(app): S, Json(b): Json<CreateItem>) -> ApiResult {
    let item = app
        .brain()
        .db(move |s| {
            Ok(s.create_note(
                NewNote {
                    title: b.title,
                    body: b.body,
                    kind: b.kind,
                    tags: b.tags,
                    origin: None,
                },
                Actor::User,
            )?)
        })
        .await?;
    app.jobs_notify.notify_one();
    ok(item)
}

#[derive(Deserialize)]
struct UpdateItem {
    title: Option<String>,
    body: Option<String>,
    tags: Option<Vec<String>>,
    /// Absent: unchanged. A flag: set by hand (sticks through syncs and source changes).
    /// `null`: back to the note's own or the source's flag.
    #[serde(default, deserialize_with = "present")]
    privacy: Option<Option<Privacy>>,
}

/// Tells an absent field (`None`) from an explicit `null` (`Some(None)`).
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(d)?))
}

async fn update_item(
    State(app): S,
    Path(id): Path<String>,
    Json(b): Json<UpdateItem>,
) -> ApiResult {
    let id = Id(id);
    let content = b.title.is_some() || b.body.is_some() || b.tags.is_some();
    let item = app
        .brain()
        .db(move |s| {
            let mut item = None;
            if content || b.privacy.is_none() {
                item = Some(s.update_item(
                    &id,
                    ItemPatch {
                        title: b.title,
                        body: b.body,
                        tags: b.tags,
                        kind: None,
                    },
                    Actor::User,
                )?);
            }
            if let Some(p) = b.privacy {
                item = Some(crate::privacy::set_item_privacy(s, &id, p, Actor::User)?);
            }
            Ok(item)
        })
        .await?;
    app.jobs_notify.notify_one();
    ok(item)
}

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    k: Option<usize>,
}

async fn search(State(app): S, Query(p): Query<SearchParams>) -> ApiResult {
    let k = p.k.unwrap_or(20).clamp(1, 200);
    let hits = hybrid_search(&app, &app.brain(), &p.q, k).await?;
    ok(json!({ "hits": hits }))
}

async fn chat(State(app): S, Json(b): Json<ChatBody>) -> Response {
    crate::chat::chat(app, b).await.into_response()
}

async fn timeline(State(app): S) -> ApiResult {
    ok(app.brain().db(|s| Ok(s.timeline()?)).await?)
}

#[derive(Deserialize)]
struct ThreadParams {
    kind: Option<String>,
    status: Option<String>,
}

async fn threads(State(app): S, Query(p): Query<ThreadParams>) -> ApiResult {
    let kind: Option<ThreadKind> = p
        .kind
        .filter(|k| !k.is_empty())
        .map(|k| parse_enum(&k, "kind"))
        .transpose()?;
    let status: Option<ThreadStatus> = p
        .status
        .filter(|k| !k.is_empty())
        .map(|k| parse_enum(&k, "status"))
        .transpose()?;
    let t = app
        .brain()
        .db(move |s| {
            Ok(s.threads(&ThreadFilter {
                kind,
                status,
                item_id: None,
            })?)
        })
        .await?;
    ok(json!({ "threads": t }))
}

#[derive(Deserialize)]
struct ThreadUpdate {
    status: ThreadStatus,
    resolved_by: Option<Id>,
}

async fn update_thread(
    State(app): S,
    Path(id): Path<String>,
    Json(b): Json<ThreadUpdate>,
) -> ApiResult {
    let id = Id(id);
    ok(app
        .brain()
        .db(move |s| Ok(s.set_thread_status(&id, b.status, b.resolved_by, Actor::User)?))
        .await?)
}

async fn review(State(app): S) -> ApiResult {
    let brain = app.brain();
    // draw today's review if the job loop has not yet
    if brain.job_state().review_day.as_deref() != Some(now().date_naive().to_string().as_str()) {
        let _g = brain.jobs_lock.lock().await;
        crate::jobs::run_review(&app, &brain, false).await?;
    }
    let reasons = brain.job_state().review_reasons;
    let items = brain
        .db(move |s| {
            let mut out = Vec::new();
            for r in s.resurfacings(true)? {
                let Some(item) = s.get_item(&r.item_id)?.filter(|i| !i.deleted) else {
                    continue;
                };
                out.push(json!({
                    "resurface_id": r.id,
                    "item": item,
                    "reason": reasons.get(&r.id.0).cloned().unwrap_or_else(|| "resurfaced".into()),
                    "arm": r.arm,
                }));
            }
            Ok(out)
        })
        .await?;
    ok(json!({ "items": items }))
}

#[derive(Deserialize)]
struct Rate {
    useful: bool,
}

async fn rate_review(State(app): S, Path(id): Path<String>, Json(b): Json<Rate>) -> ApiResult {
    let id = Id(id);
    app.brain()
        .db(move |s| Ok(s.rate_resurface(&id, b.useful, Actor::User)?))
        .await?;
    ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct SuggestionParams {
    item: String,
}

async fn suggestions(State(app): S, Query(p): Query<SuggestionParams>) -> ApiResult {
    let id = Id(p.item);
    let v = app
        .brain()
        .db(move |s| {
            let mut out = Vec::new();
            let mut displayed = Vec::new();
            // only the shown arm: held-out suggestions are never displayed
            for sg in s.open_suggestions(&id)?.into_iter().filter(|x| x.shown) {
                if let Some(to) = item_summary(s, &sg.to)? {
                    out.push(json!({ "suggestion_id": sg.id, "to": to, "score": sg.score }));
                    displayed.push(sg.id);
                }
            }
            // the ui asks for these when it opens the item: they are now in front of the
            // user, which is what acceptance and the shown arm of the evaluation count
            s.mark_suggestions_displayed(&displayed, Actor::User)?;
            Ok(out)
        })
        .await?;
    ok(json!({ "suggestions": v }))
}

#[derive(Deserialize)]
struct Decide {
    accept: bool,
}

async fn decide_suggestion(
    State(app): S,
    Path(id): Path<String>,
    Json(b): Json<Decide>,
) -> ApiResult {
    let id = Id(id);
    let s = app
        .brain()
        .db(move |s| {
            let sg = s
                .get_suggestion(&id)?
                .ok_or_else(|| anamnesis_core::Error::NotFound(format!("suggestion {id}")))?;
            if !sg.shown {
                // the control arm cannot be decided from the ui
                return Err(anamnesis_core::Error::NotFound(format!("suggestion {id}")).into());
            }
            Ok(s.decide_suggestion(&id, b.accept, Actor::User)?)
        })
        .await?;
    app.jobs_notify.notify_one();
    ok(s)
}

#[derive(Deserialize)]
struct NewLink {
    from: Id,
    to: Id,
}

async fn add_link(State(app): S, Json(b): Json<NewLink>) -> ApiResult {
    let l = app
        .brain()
        .db(move |s| Ok(s.add_manual_link(&b.from, &b.to, Actor::User)?))
        .await?;
    app.jobs_notify.notify_one();
    ok(l)
}

async fn lineage(State(app): S) -> ApiResult {
    let runs = app.brain().db(|s| Ok(s.region_runs()?)).await?;
    ok(json!({ "runs": runs }))
}

/// The eval report: every metric from the event log, retrieval scored with the app's own
/// hybrid search (the query embedded by the current embedder).
pub async fn eval_report(app: &Arc<App>) -> anyhow::Result<anamnesis_eval::EvalReport> {
    let brain = app.brain();
    let emb = app.embedder();
    brain
        .db(move |s| {
            let events = s.all_events()?;
            let mut search = |q: &str| -> Vec<Id> {
                let mut query = anamnesis_core::SearchQuery::text(q, 20);
                if let Some(e) = &emb
                    && let Ok(v) = anamnesis_embed::embed_one_query(e.as_ref(), q)
                {
                    query.vector = Some(anamnesis_core::QueryVector {
                        model_id: e.id().to_string(),
                        values: v,
                    });
                }
                s.search(&query)
                    .map(|h| h.into_iter().map(|h| h.item_id).collect())
                    .unwrap_or_default()
            };
            Ok(anamnesis_eval::evaluate(
                &events,
                Some(&mut search),
                None,
                &Default::default(),
            ))
        })
        .await
}

async fn eval(State(app): S) -> ApiResult {
    ok(eval_report(&app).await?)
}

#[derive(Deserialize)]
struct Label {
    query: String,
    item_id: Id,
    grade: u8,
}

async fn eval_label(State(app): S, Json(b): Json<Label>) -> ApiResult {
    app.brain()
        .db(move |s| Ok(s.label_retrieval(&b.query, &b.item_id, b.grade, Actor::User)?))
        .await?;
    ok(json!({ "ok": true }))
}

async fn skills(State(app): S) -> ApiResult {
    let v = app
        .brain()
        .db(|s| {
            Ok(s.list_items(&ItemFilter {
                kind: Some(ItemKind::Skill),
                ..Default::default()
            })?
            .into_iter()
            .map(|i| json!({ "id": i.id, "title": i.title, "body": i.body }))
            .collect::<Vec<_>>())
        })
        .await?;
    ok(json!({ "skills": v }))
}

// ---------------------------------------------------------------------------------------
// sources

async fn sources(State(app): S) -> ApiResult {
    let v = app.brain().db(|s| Ok(s.list_sources()?)).await?;
    // pick up plugins added or removed since the last look
    let plugins = app.clone();
    tokio::task::spawn_blocking(move || plugins.plugins.scan())
        .await
        .map_err(|e| ApiError::from(anyhow::Error::from(e)))?;
    let mut connectors: Vec<Value> = anamnesis_connectors::builtin_connectors()
        .into_iter()
        .map(|b| connector_row(&b.manifest, None))
        .collect();
    for p in app.plugins.loaded() {
        connectors.push(connector_row(&p.manifest, Some(&p.manifest_path)));
    }
    ok(json!({
        "sources": v,
        "connectors": connectors,
        "plugins": {
            "dirs": app.plugins.dirs(),
            "errors": app.plugins.errors(),
        },
    }))
}

/// A connector manifest for the ui, with `plugin: bool` (and the manifest's path for a
/// plugin). `network_hosts` / `fs_paths` are part of every manifest.
fn connector_row(m: &anamnesis_connectors::Manifest, plugin: Option<&std::path::Path>) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("plugin".into(), json!(plugin.is_some()));
        if let Some(p) = plugin {
            o.insert("manifest_path".into(), json!(p));
        }
    }
    v
}

#[derive(Deserialize)]
struct NewSource {
    connector: String,
    name: Option<String>,
    #[serde(default)]
    config: Value,
    default_origin: Option<Origin>,
    default_privacy: Option<Privacy>,
}

/// Validate and register a source (used by the api and `anamnesis add-source`).
pub async fn add_source_checked(app: &Arc<App>, b: NewSourceArgs) -> Result<Source, ApiError> {
    let conn = app
        .plugins
        .resolve(&b.connector)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let manifest = conn.manifest();
    let config = if b.config.is_null() {
        json!({})
    } else {
        b.config
    };
    manifest
        .validate_config(&config)
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let check = conn
        .check(&config)
        .await
        .map_err(|e| ApiError::bad(e.to_string()))?;
    if !check.ok {
        return Err(ApiError::bad(check.message));
    }
    // no flag given: the `privacy.default` setting, then the connector's own default
    let setting = app
        .brain()
        .db(|s| Ok(crate::privacy::default_setting(s)))
        .await?;
    let src = Source {
        id: Id::new(),
        connector: b.connector.clone(),
        name: b.name.unwrap_or_else(|| manifest.name.clone()),
        config,
        default_origin: b.default_origin.unwrap_or(manifest.default_origin),
        default_privacy: b
            .default_privacy
            .or(setting)
            .unwrap_or(manifest.default_privacy),
        cursor: None,
        created_at: now(),
    };
    let src = app.brain().db(move |s| Ok(s.add_source(src)?)).await?;
    app.sync_notify.notify_one();
    Ok(src)
}

pub struct NewSourceArgs {
    pub connector: String,
    pub name: Option<String>,
    pub config: Value,
    pub default_origin: Option<Origin>,
    pub default_privacy: Option<Privacy>,
}

async fn add_source(State(app): S, Json(b): Json<NewSource>) -> ApiResult {
    let src = add_source_checked(
        &app,
        NewSourceArgs {
            connector: b.connector,
            name: b.name,
            config: b.config,
            default_origin: b.default_origin,
            default_privacy: b.default_privacy,
        },
    )
    .await?;
    ok(src)
}

#[derive(Deserialize)]
struct SourcePatch {
    name: Option<String>,
    config: Option<Value>,
    default_origin: Option<Origin>,
    default_privacy: Option<Privacy>,
}

async fn update_source(
    State(app): S,
    Path(id): Path<String>,
    Json(b): Json<SourcePatch>,
) -> ApiResult {
    let id = Id(id);
    let mut src = app
        .brain()
        .db(move |s| Ok(s.get_source(&id)?))
        .await?
        .ok_or_else(|| ApiError::not_found("no such source"))?;
    if let Some(c) = b.config {
        if let Ok(conn) = app.plugins.resolve(&src.connector) {
            conn.manifest()
                .validate_config(&c)
                .map_err(|e| ApiError::bad(e.to_string()))?;
        }
        // the stored cursor carries a hash of the config, so a changed config resyncs fully
        src.config = c;
    }
    if let Some(n) = b.name {
        src.name = n;
    }
    if let Some(o) = b.default_origin {
        src.default_origin = o;
    }
    let privacy_changed = b.default_privacy.is_some_and(|p| p != src.default_privacy);
    if let Some(p) = b.default_privacy {
        src.default_privacy = p;
    }
    let (src, reflagged) = app
        .brain()
        .db(move |s| {
            let src = s.update_source(src, Actor::User)?;
            // items that took the old default follow the new one; hand-set flags stay
            let n = if privacy_changed {
                crate::privacy::reflag_source(s, &src, Actor::User)?
            } else {
                0
            };
            Ok((src, n))
        })
        .await?;
    app.sync_notify.notify_one();
    let mut v = serde_json::to_value(&src).map_err(|e| ApiError::from(anyhow::Error::from(e)))?;
    v["items_reflagged"] = json!(reflagged);
    Ok(Json(v))
}

async fn remove_source(State(app): S, Path(id): Path<String>) -> ApiResult {
    let id = Id(id);
    let n = app.brain().db(move |s| Ok(s.remove_source(&id)?)).await?;
    app.sync_notify.notify_one();
    app.jobs_notify.notify_one();
    ok(json!({ "ok": true, "items_deleted": n }))
}

async fn sync_source(State(app): S, Path(id): Path<String>) -> ApiResult {
    let id = Id(id);
    let r = crate::sync::sync_source(&app.brain(), &app.plugins, &id).await?;
    if r.changed() {
        app.jobs_notify.notify_one();
    }
    ok(r)
}

// ---------------------------------------------------------------------------------------
// backends

async fn backends(State(app): S) -> ApiResult {
    let mut rows: Vec<Value> = Vec::new();
    let mut routing = json!({});
    let key_sources = app.key_sources.read().unwrap().clone();
    if let Some(r) = app.router() {
        for b in r.backends_info() {
            let mut row = serde_json::to_value(&b).unwrap_or(Value::Null);
            row["key_storage"] = json!(key_sources.get(&b.id));
            rows.push(row);
        }
        for (task, id) in r.effective_routing() {
            routing[task.as_str()] = json!(id);
        }
    }
    rows.push(json!({
        "id": RETRIEVAL_BACKEND, "kind": "retrieval", "label": "retrieval only (no model)",
        "available": true, "status": "ready", "capabilities": { "streaming": true, "cost_model": "free_local" },
        "terms": "allowed", "is_local": true, "leaves_machine": false, "key_storage": null,
        "notice": "answers by listing what your brain holds; no text is generated and nothing leaves the machine",
    }));
    if routing.get("chat").is_none_or(|v| v.is_null()) {
        routing["chat"] = json!(RETRIEVAL_BACKEND);
    }
    for t in ["label", "extract", "summarise"] {
        if routing.get(t).is_none() {
            routing[t] = Value::Null;
        }
    }
    let usage = app
        .router()
        .map(|r| serde_json::to_value(r.usage()).unwrap_or(Value::Null));
    ok(json!({
        "backends": rows,
        "routing": routing,
        "embedder": app.embed_status(),
        "usage": usage,
        "detected": app.router().is_some(),
        // where new api keys are written (os store or the private file)
        "key_store": app.secrets.info(),
        // shown before a backend is enabled (its row may not exist yet)
        "notices": {
            "claude_cli": anamnesis_harness::adapters::claude_cli::NOTICE,
            "claude_cli_relabel": anamnesis_harness::adapters::claude_cli::RELABEL_NOTICE,
            "chatgpt": anamnesis_harness::adapters::chatgpt::NOTICE,
        },
        // false once the project withdraws relabelling; the ui then hides the option
        "claude_cli_relabel_offered": !anamnesis_harness::adapters::claude_cli::RELABEL_WITHDRAWN,
    }))
}

async fn refresh_backends(State(app): S) -> ApiResult {
    crate::backends::detect(&app).await;
    backends(State(app)).await
}

#[derive(Deserialize)]
struct KeyBody {
    key: String,
}

async fn set_backend_key(
    State(app): S,
    Path(id): Path<String>,
    Json(b): Json<KeyBody>,
) -> ApiResult {
    let key = b.key.trim().to_string();
    if key.is_empty() {
        return Err(ApiError::bad("empty key"));
    }
    let app2 = app.clone();
    let stored = tokio::task::spawn_blocking(move || {
        crate::backends::set_api_key(&app2.secrets, &id, Some(&key))
    })
    .await
    .map_err(|e| ApiError::from(anyhow::Error::from(e)))??;
    crate::backends::detect_in_background(&app);
    ok(json!({ "ok": true, "key_storage": stored }))
}

async fn delete_backend_key(State(app): S, Path(id): Path<String>) -> ApiResult {
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || crate::backends::set_api_key(&app2.secrets, &id, None))
        .await
        .map_err(|e| ApiError::from(anyhow::Error::from(e)))??;
    crate::backends::detect_in_background(&app);
    ok(json!({ "ok": true }))
}

async fn chatgpt_status(State(app): S) -> ApiResult {
    ok(crate::backends::chatgpt_status(&app).await?)
}

async fn chatgpt_sign_in(State(app): S) -> ApiResult {
    let url = crate::backends::chatgpt_begin(&app).await?;
    ok(
        json!({ "authorize_url": url, "button_label": anamnesis_harness::adapters::chatgpt::BUTTON_LABEL }),
    )
}

async fn chatgpt_sign_out(State(app): S) -> ApiResult {
    crate::backends::chatgpt_sign_out(&app).await?;
    ok(json!({ "ok": true }))
}

// ---------------------------------------------------------------------------------------
// settings

async fn get_settings(State(app): S) -> ApiResult {
    ok(app.brain().db(|s| Ok(s.settings()?)).await?)
}

/// Merge `patch` into `base`: objects merge key by key (recursively), `null` removes a
/// key, anything else (strings, numbers, arrays) replaces.
pub fn merge_json(base: &mut Value, patch: &Value) {
    match (base.as_object_mut(), patch.as_object()) {
        (Some(b), Some(p)) => {
            for (k, v) in p {
                if v.is_null() {
                    b.remove(k);
                } else {
                    match b.get_mut(k) {
                        Some(slot) if slot.is_object() && v.is_object() => merge_json(slot, v),
                        _ => {
                            let mut fresh = Value::Object(Default::default());
                            if v.is_object() {
                                // drop nulls nested in a new object
                                merge_json(&mut fresh, v);
                            } else {
                                fresh = v.clone();
                            }
                            b.insert(k.clone(), fresh);
                        }
                    }
                }
            }
        }
        _ => *base = patch.clone(),
    }
}

async fn put_settings(State(app): S, Json(b): Json<Value>) -> ApiResult {
    let patch = b
        .as_object()
        .ok_or_else(|| ApiError::bad("settings must be a json object"))?
        .clone();
    // deep-merge into what is stored; only the touched top-level keys are written
    let current = app.brain().db(|s| Ok(s.settings()?)).await?;
    let mut merged = current.clone();
    if !merged.is_object() {
        merged = json!({});
    }
    merge_json(&mut merged, &Value::Object(patch.clone()));
    let mut obj = serde_json::Map::new();
    for k in patch.keys() {
        obj.insert(k.clone(), merged.get(k).cloned().unwrap_or(Value::Null));
    }
    // typed sections are validated before anything is stored
    let new_spec = match obj.get("embedder") {
        Some(Value::Null) | None => None,
        Some(v) => Some(
            serde_json::from_value::<EmbedderSpec>(v.clone())
                .map_err(|e| ApiError::bad(format!("embedder: {e}")))?,
        ),
    };
    if let Some(r) = obj.get("routing").filter(|v| !v.is_null()) {
        serde_json::from_value::<anamnesis_harness::RoutingSettings>(r.clone())
            .map_err(|e| ApiError::bad(format!("routing: {e}")))?;
    }
    if let Some(d) = obj
        .get("privacy")
        .filter(|v| !v.is_null())
        .and_then(|p| p.get("default"))
    {
        serde_json::from_value::<Privacy>(d.clone())
            .map_err(|e| ApiError::bad(format!("privacy.default: {e}")))?;
    }
    if let Some(h) = obj.get("harness").filter(|v| !v.is_null()) {
        let mut h = h.clone();
        if let Some(c) = h.get_mut("chatgpt").and_then(|c| c.as_object_mut()) {
            c.entry("data_dir").or_insert(json!("."));
        }
        serde_json::from_value::<anamnesis_harness::HarnessConfig>(h)
            .map_err(|e| ApiError::bad(format!("harness: {e}")))?;
    }
    let touches_harness = obj.contains_key("harness");
    let routing = obj.get("routing").cloned();
    let body = Value::Object(obj);
    let all = app.brain().db(move |s| Ok(s.put_settings(&body)?)).await?;
    if let Some(spec) = new_spec
        && app.cfg.embedder_override.is_none()
    {
        let app2 = app.clone();
        tokio::spawn(async move { app2.set_embedder_spec(spec).await });
    }
    if touches_harness {
        crate::backends::detect_in_background(&app);
    } else if let (Some(r), Some(router)) = (routing, app.router()) {
        let rs = if r.is_null() {
            Default::default()
        } else {
            serde_json::from_value(r).unwrap_or_default()
        };
        router.set_settings(rs);
    }
    ok(all)
}

// ---------------------------------------------------------------------------------------
// log

#[derive(Deserialize)]
struct LimitParams {
    limit: Option<usize>,
    after: Option<u64>,
}

async fn egress(State(app): S, Query(p): Query<LimitParams>) -> ApiResult {
    let limit = p.limit.unwrap_or(100).min(10_000);
    ok(json!({ "entries": app.brain().db(move |s| Ok(s.egress_entries(limit)?)).await? }))
}

async fn events(State(app): S, Query(p): Query<LimitParams>) -> ApiResult {
    let limit = p.limit.unwrap_or(1000).min(10_000);
    let after = p.after.unwrap_or(0);
    ok(json!({ "events": app.brain().db(move |s| Ok(s.events_after(after, limit)?)).await? }))
}

async fn export(State(app): S) -> Result<Response, ApiError> {
    let bytes = app
        .brain()
        .db(|s| {
            let mut buf = Vec::new();
            s.export_bundle(&mut buf)?;
            Ok(buf)
        })
        .await?;
    let name = format!("anamnesis-{}.jsonl", now().format("%Y%m%d-%H%M%S"));
    let mut res = Response::new(axum::body::Body::from(bytes));
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        res.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}

// ---------------------------------------------------------------------------------------
// mcp, demo, jobs

/// Environment variable the snippets reference for the http bearer token.
pub const MCP_TOKEN_ENV: &str = "ANAMNESIS_MCP_TOKEN";

async fn mcp_snippets(State(app): S) -> ApiResult {
    let token = app.brain().mcp_token()?;
    let command = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "anamnesis".into());
    let url = format!("http://127.0.0.1:{}/mcp", app.cfg.port);
    let args = crate::backends::mcp_stdio_args(&app);
    let snippets = anamnesis_mcp::config_snippets(&anamnesis_mcp::SnippetOptions {
        command,
        args: args.clone(),
        http_url: Some(url.clone()),
        token_env_var: Some(MCP_TOKEN_ENV.into()),
        ..Default::default()
    });
    ok(json!({
        "snippets": snippets,
        "http_url": url,
        "token_env_var": MCP_TOKEN_ENV,
        "token": token,
        "stdio_args": args,
        "tools": anamnesis_mcp::TOOL_NAMES,
    }))
}

async fn demo_status(State(app): S) -> ApiResult {
    ok(crate::importer::demo_status(&app).await?)
}

#[derive(Deserialize)]
struct DemoBody {
    #[serde(default = "yes")]
    enabled: bool,
}
fn yes() -> bool {
    true
}

async fn demo(State(app): S, body: Option<Json<DemoBody>>) -> ApiResult {
    let on = body.map(|b| b.enabled).unwrap_or(true);
    ok(crate::importer::set_demo(&app, on)
        .await
        .map_err(|e| ApiError::bad(format!("{e:#}")))?)
}

async fn run_jobs(State(app): S) -> ApiResult {
    ok(crate::jobs::run_jobs_once(&app).await?)
}

async fn status(State(app): S) -> ApiResult {
    let brain = app.brain();
    let (items, events, head) = brain
        .db(|s| Ok((s.count_items()?, s.event_count()?, s.head()?.map(|h| h.1))))
        .await?;
    ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "demo": brain.is_demo,
        "items": items,
        "events": events,
        "head": head,
        "embedder": app.embed_status(),
        "jobs": app.jobs_progress.read().unwrap().clone(),
    }))
}

/// The serde string of a unit enum (e.g. `ItemKind::Note` → `"note"`).
fn enum_str<T: serde::Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}
