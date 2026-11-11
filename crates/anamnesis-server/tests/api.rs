//! End-to-end tests of the http api, the jobs and the mcp brain, in-process, on a temp
//! data dir with the hash embedder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anamnesis_core::{Actor, Id, ItemKind, Origin, Privacy, ThreadStatus};
use anamnesis_mcp::{BrainApi, NewCapture};
use anamnesis_server::{App, ServerConfig, api, importer, jobs};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes")
}

struct Env {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    router: Router,
}

async fn setup() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ServerConfig::for_tests(&dir.path().join("anamnesis"));
    let app = App::open(cfg, false).unwrap();
    // small corpus: let regions form with a handful of items
    let mut p = anamnesis_mind::MindParams::default();
    p.regions.min_items = 4;
    p.regions.min_samples = 2;
    p.regions.min_cluster_size = Some(3);
    p.space.k = 4;
    p.layout.n_epochs = Some(60);
    app.brain()
        .with_store(|s| Ok(s.set_setting("mind_params", serde_json::to_value(p)?)?))
        .unwrap();
    let r = importer::import_path(&app.brain(), &fixture()).await.unwrap();
    match r {
        importer::ImportReport::Folder { sync, .. } => assert_eq!(sync.created, 11, "{sync:?}"),
        other => panic!("{other:?}"),
    }
    let rep = jobs::run_jobs_once(&app).await.unwrap();
    assert!(rep.embedded >= 11, "{rep:?}");
    assert!(rep.mind.as_ref().is_some_and(|m| m.full && m.items == 11), "{rep:?}");
    let router = api::router(app.clone());
    Env { _dir: dir, app, router }
}

async fn call(r: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = r.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, v)
}

async fn get(r: &Router, uri: &str) -> Value {
    let (s, v) = call(r, "GET", uri, None).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {v}");
    v
}

fn id_by_title(cloud: &Value, title: &str) -> String {
    let i = cloud["titles"].as_array().unwrap().iter().position(|t| t == title).unwrap_or_else(|| panic!("no {title}"));
    cloud["ids"][i].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloud_items_search_timeline_lineage() {
    let env = setup().await;
    let r = &env.router;

    let cloud = get(r, "/api/cloud").await;
    let n = cloud["ids"].as_array().unwrap().len();
    assert_eq!(n, 11);
    for k in ["xyz", "xyz_anatomy", "xyz_spheres"] {
        assert_eq!(cloud[k].as_array().unwrap().len(), 3 * n, "{k}");
    }
    for k in ["region", "kind", "speaker", "t", "titles"] {
        assert_eq!(cloud[k].as_array().unwrap().len(), n, "{k}");
    }
    assert_eq!(cloud["kinds"], json!(["note"]));
    assert!(cloud["speaker"].as_array().unwrap().iter().all(|s| s == 0));
    // wikilinks became written links
    let links = cloud["links"].as_array().unwrap();
    assert!(links.len() >= 5, "{links:?}");
    assert!(links.iter().all(|l| l[2] == "written"));
    // positions are not all at the origin
    assert!(cloud["xyz"].as_array().unwrap().iter().any(|x| x.as_f64().unwrap().abs() > 1e-3));
    let regions = cloud["regions"].as_array().unwrap();
    assert!(!regions.is_empty(), "regions formed: {cloud}");
    assert!(regions[0]["label"].as_str().is_some_and(|l| !l.is_empty()));
    for (i, reg) in cloud["region"].as_array().unwrap().iter().enumerate() {
        let ri = reg.as_i64().unwrap();
        assert!(ri >= -1 && ri < regions.len() as i64, "item {i}: {ri}");
    }

    // item detail
    let att = id_by_title(&cloud, "attention");
    let item = get(r, &format!("/api/items/{att}")).await;
    assert_eq!(item["item"]["title"], "attention");
    assert!(item["links_out"].as_array().unwrap().len() >= 2);
    assert!(!item["threads"].as_array().unwrap().is_empty(), "rule threads: {item}");
    let (s, _) = call(r, "GET", "/api/items/01NOPE", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // search (hybrid)
    let hits = get(r, "/api/search?q=compost%20garden&k=5").await;
    let hits = hits["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0]["title"], "compost");
    assert!(hits.len() <= 5);

    // timeline
    let tl = get(r, "/api/timeline").await;
    assert!(!tl["months"].as_array().unwrap().is_empty());
    let total: u64 = tl["days"].as_array().unwrap().iter().map(|d| d["count"].as_u64().unwrap()).sum();
    assert_eq!(total, 11);
    assert!(tl["threads_by_month"]["decision"].is_array());

    // lineage: one run so far
    let lin = get(r, "/api/lineage").await;
    assert_eq!(lin["runs"].as_array().unwrap().len(), 1);

    // threads
    let th = get(r, "/api/threads?kind=decision").await;
    let th = th["threads"].as_array().unwrap();
    assert!(!th.is_empty());
    let tid = th[0]["id"].as_str().unwrap();
    let (s, t) = call(r, "POST", &format!("/api/threads/{tid}"), Some(json!({ "status": "resolved" }))).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["status"], "resolved");
    let (s, _) = call(r, "GET", "/api/threads?kind=nonsense", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editor_suggestions_review_eval() {
    let env = setup().await;
    let r = &env.router;

    // editor create + update
    let (s, item) = call(
        r,
        "POST",
        "/api/items",
        Some(json!({ "title": "mulch", "body": "mulch keeps the garden beds moist around the tomatoes. see [[compost]].", "tags": ["garden"] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{item}");
    assert_eq!(item["origin"], "owner_written");
    let mid = item["id"].as_str().unwrap().to_string();
    let (s, upd) = call(r, "PUT", &format!("/api/items/{mid}"), Some(json!({ "title": "mulch and straw" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(upd["title"], "mulch and straw");
    // incremental jobs place the new item without a full rerun
    let rep = jobs::run_jobs_once(&env.app).await.unwrap();
    assert!(rep.embedded >= 1);
    let cloud = get(r, "/api/cloud").await;
    assert_eq!(cloud["ids"].as_array().unwrap().len(), 12);

    // suggestions: only the shown arm is listed; accept one, dismiss one
    let all = env.app.brain().with_store(|s| Ok(s.all_suggestions()?)).unwrap();
    assert!(!all.is_empty());
    let shown = all.iter().filter(|s| s.shown).count();
    assert!(shown > 0 && shown <= all.len());
    let from = all.iter().find(|s| s.shown).unwrap().from.clone();
    let sg = get(r, &format!("/api/suggestions?item={from}")).await;
    let sg = sg["suggestions"].as_array().unwrap().clone();
    assert!(!sg.is_empty());
    let held: Vec<String> = all.iter().filter(|s| !s.shown).map(|s| s.id.0.clone()).collect();
    for x in &sg {
        assert!(!held.contains(&x["suggestion_id"].as_str().unwrap().to_string()));
        assert!(x["to"]["title"].is_string());
    }
    let sid = sg[0]["suggestion_id"].as_str().unwrap();
    let (s, d) = call(r, "POST", &format!("/api/suggestions/{sid}"), Some(json!({ "accept": true }))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["status"], "accepted");
    if let Some(x) = sg.get(1) {
        let sid = x["suggestion_id"].as_str().unwrap();
        let (_, d) = call(r, "POST", &format!("/api/suggestions/{sid}"), Some(json!({ "accept": false }))).await;
        assert_eq!(d["status"], "dismissed");
    }
    if let Some(h) = held.first() {
        let (s, _) = call(r, "POST", &format!("/api/suggestions/{h}"), Some(json!({ "accept": true }))).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "held-out suggestions cannot be decided");
    }

    // a manual link
    let a = id_by_title(&cloud, "seeds");
    let b = id_by_title(&cloud, "watering");
    let (s, l) = call(r, "POST", "/api/links", Some(json!({ "from": a, "to": b }))).await;
    assert_eq!(s, StatusCode::OK, "{l}");
    assert_eq!(l["kind"], "manual");

    // review: items with reasons; rate one
    let rv = get(r, "/api/review").await;
    let items = rv["items"].as_array().unwrap();
    assert!(!items.is_empty(), "the fixture is old enough to resurface: {rv}");
    let rid = items[0]["resurface_id"].as_str().unwrap();
    assert!(items[0]["reason"].is_string());
    let (s, _) = call(r, "POST", &format!("/api/review/{rid}"), Some(json!({ "useful": true }))).await;
    assert_eq!(s, StatusCode::OK);

    // eval: label a query, read the report
    let (s, _) = call(r, "POST", "/api/eval/label", Some(json!({ "query": "compost", "item_id": id_by_title(&cloud, "compost"), "grade": 3 }))).await;
    assert_eq!(s, StatusCode::OK);
    let ev = get(r, "/api/eval").await;
    for k in ["links", "retrieval", "resurfacing", "growth", "honesty"] {
        assert!(ev.get(k).is_some_and(|v| !v.is_null()), "{k} in {ev}");
    }
    assert_eq!(ev["links"]["status"], "measuring");
    assert_eq!(ev["retrieval"]["status"], "measuring");

    // events, egress, settings, backends, snippets, skills
    let evs = get(r, "/api/events?after=0&limit=5").await;
    assert_eq!(evs["events"].as_array().unwrap().len(), 5);
    assert_eq!(get(r, "/api/egress").await["entries"], json!([]));
    let (s, st) = call(r, "PUT", "/api/settings", Some(json!({ "theme": "aphesist", "motion": "reduced" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(st["theme"], "aphesist");
    assert_eq!(get(r, "/api/settings").await["motion"], "reduced");
    let (s, _) = call(r, "PUT", "/api/settings", Some(json!({ "embedder": { "backend": "nope" } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let be = get(r, "/api/backends").await;
    assert!(be["backends"].as_array().unwrap().iter().any(|b| b["id"] == "retrieval"));
    assert_eq!(be["routing"]["chat"], "retrieval");
    assert_eq!(be["embedder"]["status"], "ready");
    let sn = get(r, "/api/mcp/snippets").await;
    assert!(sn["snippets"]["claude_code_command"].as_str().unwrap().contains("mcp"));
    assert!(sn["token"].as_str().unwrap().len() >= 32);
    assert_eq!(get(r, "/api/skills").await["skills"], json!([]));

    // the api refuses foreign host names (dns rebinding)
    let res = r
        .clone()
        .oneshot(Request::get("/api/status").header(header::HOST, "evil.example:7368").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // the chain is intact after all of this
    let rep = env.app.brain().with_store(|s| Ok(s.verify_chain()?)).unwrap();
    assert!(rep.events > 50);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_streams_with_citations_and_stores_turns() {
    let env = setup().await;
    let req = Request::post("/api/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "message": "what do i know about compost?" }).to_string()))
        .unwrap();
    let res = env.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/event-stream"));
    let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    for ev in ["event: token", "event: cited", "event: usage", "event: done"] {
        assert!(text.contains(ev), "{ev} missing in {text}");
    }
    let done = text.lines().skip_while(|l| *l != "event: done").nth(1).unwrap();
    let done: Value = serde_json::from_str(done.trim_start_matches("data: ")).unwrap();
    let chat_id = Id(done["chat_id"].as_str().unwrap().into());
    let turns = env.app.brain().with_store(|s| Ok(s.chat_turns(&chat_id)?)).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, "user");
    assert!(!turns[1].cited.is_empty());
    let (user, reply) = env
        .app
        .brain()
        .with_store(|s| Ok((s.get_item(&turns[0].item_id)?.unwrap(), s.get_item(&turns[1].item_id)?.unwrap())))
        .unwrap();
    assert_eq!(user.origin, Origin::OwnerInChat);
    assert_eq!(reply.origin, Origin::AiGenerated);
    assert_eq!(reply.kind, ItemKind::Chat);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_import_roundtrip_and_verify() {
    let env = setup().await;
    let (s, _) = call(&env.router, "GET", "/api/export", None).await;
    assert_eq!(s, StatusCode::OK);
    let file = env._dir.path().join("brain.jsonl");
    let m = importer::export_to(&env.app.brain(), &file).await.unwrap();
    assert!(m.events > 0);

    let dir2 = tempfile::tempdir().unwrap();
    let app2 = App::open(ServerConfig::for_tests(&dir2.path().join("anamnesis")), false).unwrap();
    match importer::import_path(&app2.brain(), &file).await.unwrap() {
        importer::ImportReport::Bundle { events } => assert_eq!(events, m.events),
        other => panic!("{other:?}"),
    }
    let (a, b) = (
        env.app.brain().with_store(|s| Ok(s.verify_chain()?)).unwrap(),
        app2.brain().with_store(|s| Ok(s.verify_chain()?)).unwrap(),
    );
    assert_eq!(a.head, b.head);
    // importing into a non-empty brain is refused
    assert!(importer::import_path(&app2.brain(), &file).await.is_err());
    // the imported brain serves a cloud after its jobs ran
    jobs::run_jobs_once(&app2).await.unwrap();
    let cloud = get(&api::router(app2.clone()), "/api/cloud").await;
    assert_eq!(cloud["ids"].as_array().unwrap().len(), 11);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_brain_capture_search_link_threads() {
    let env = setup().await;
    let brain = anamnesis_server::mcp_brain::StoreBrain::new(env.app.clone());
    let actor = Actor::Mcp("claude-code".into());
    let item = brain
        .capture(
            actor.clone(),
            NewCapture {
                title: "raised beds".into(),
                body: "raised beds warm up early in spring; fill them with compost.".into(),
                kind: ItemKind::Note,
                tags: vec![],
                origin: Origin::AiGenerated,
                client: "claude-code".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(item.origin, Origin::AiGenerated);
    let src = env.app.brain().with_store(|s| Ok(s.get_source(&item.source_id)?.unwrap())).unwrap();
    assert_eq!(src.connector, "mcp");
    let ev = env.app.brain().with_store(|s| Ok(s.events_of_type(&["item_created"], 1)?)).unwrap();
    assert_eq!(ev[0].actor, actor);

    jobs::run_jobs_once(&env.app).await.unwrap();
    let hits = brain.search("raised beds compost", 5).await.unwrap();
    assert_eq!(hits[0].hit.item_id, item.id);

    let cloud = get(&env.router, "/api/cloud").await;
    let compost = Id(id_by_title(&cloud, "compost"));
    let l = brain.link(actor.clone(), &item.id, &compost, None).await.unwrap();
    assert_eq!(l.kind, anamnesis_core::LinkKind::Manual);

    let rel = brain.related(&compost, 3).await.unwrap();
    assert!(!rel.neighbours.is_empty());
    let all = env.app.brain().with_store(|s| Ok(s.all_suggestions()?)).unwrap();
    for s in &rel.suggestions {
        assert!(all.iter().any(|a| a.id == s.suggestion_id && a.shown));
    }

    // threads from local-only items are not listed
    let threads = brain.list_threads(None, None).await.unwrap();
    assert!(!threads.is_empty());
    let t = threads[0].clone();
    let up = brain.update_thread(actor.clone(), &t.id, ThreadStatus::Abandoned, None).await.unwrap();
    assert_eq!(up.status, ThreadStatus::Abandoned);
    env.app
        .brain()
        .with_store(|s| {
            let mut it = s.get_item(&t.item_id)?.unwrap();
            it.privacy = Privacy::LocalOnly;
            s.append(Actor::User, anamnesis_core::EventKind::ItemUpdated { item: it })?;
            Ok(())
        })
        .unwrap();
    let threads2 = brain.list_threads(None, None).await.unwrap();
    assert!(threads2.iter().all(|x| x.item_id != t.item_id));

    brain
        .note_egress(actor, anamnesis_mcp::EgressNote { client: "claude-code".into(), purpose: "search".into(), items: vec![item.id.clone()], chars: 42 })
        .await
        .unwrap();
    let eg = get(&env.router, "/api/egress").await;
    assert_eq!(eg["entries"][0]["kind"]["provider"], "mcp:claude-code");
    assert!(brain.recent(1).await.unwrap()[0].id == item.id || brain.recent(3).await.unwrap().iter().any(|x| x.id == item.id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sources_and_demo() {
    let env = setup().await;
    let r = &env.router;
    let src = get(r, "/api/sources").await;
    let sources = src["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["connector"], "folder");
    let sid = sources[0]["id"].as_str().unwrap();
    // syncing again changes nothing
    let (s, rep) = call(r, "POST", &format!("/api/sources/{sid}/sync"), None).await;
    assert_eq!(s, StatusCode::OK, "{rep}");
    assert_eq!(rep["created"], 0);
    // a bad config is refused
    let (s, _) = call(r, "POST", "/api/sources", Some(json!({ "connector": "folder", "config": {} }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // a second folder, then removing it deletes its items
    let extra = env._dir.path().join("extra");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(extra.join("bees.md"), "# bees\n\nbees pollinate the tomatoes and beans.").unwrap();
    let (s, added) = call(
        r,
        "POST",
        "/api/sources",
        Some(json!({ "connector": "folder", "name": "extra", "config": { "path": extra.display().to_string() } })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{added}");
    let id2 = added["id"].as_str().unwrap();
    let (_, rep) = call(r, "POST", &format!("/api/sources/{id2}/sync"), None).await;
    assert_eq!(rep["created"], 1);
    // a rename keeps the item id
    let src2 = Id(id2.to_string());
    let bees = env.app.brain().with_store(|s| Ok(s.get_item_by_external_id(&src2, "bees.md")?.unwrap())).unwrap();
    std::fs::rename(extra.join("bees.md"), extra.join("pollinators.md")).unwrap();
    let (_, rep) = call(r, "POST", &format!("/api/sources/{id2}/sync"), None).await;
    assert_eq!(rep["renamed"], 1, "{rep}");
    let moved = env.app.brain().with_store(|s| Ok(s.get_item_by_external_id(&src2, "pollinators.md")?.unwrap())).unwrap();
    assert_eq!(moved.id, bees.id);
    assert!(!moved.deleted);
    let (s, del) = call(r, "DELETE", &format!("/api/sources/{id2}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(del["items_deleted"], 1);

    // demo: no bundle configured → refused, the user's brain stays active
    let st = get(r, "/api/demo").await;
    assert_eq!(st["active"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn demo_brain_is_separate() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = ServerConfig::for_tests(&dir.path().join("anamnesis"));
    cfg.demo_bundle = Some(fixture().join("garden"));
    let app = App::open(cfg, false).unwrap();
    let r = api::router(app.clone());
    let (s, st) = call(&r, "POST", "/api/demo", Some(json!({ "enabled": true }))).await;
    assert_eq!(s, StatusCode::OK, "{st}");
    assert_eq!(st["active"], true);
    assert_eq!(st["items"], 4);
    assert!(dir.path().join("anamnesis/demo/brain.sqlite").is_file());
    let (_, st) = call(&r, "POST", "/api/demo", Some(json!({ "enabled": false }))).await;
    assert_eq!(st["active"], false);
    assert_eq!(st["items"], 0, "the user's brain is untouched");
}
