//! Per-item privacy, settings merge, mcp snippets, key storage, plugins and model-based
//! thread extraction, end to end through the http api on a temp data dir.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anamnesis_core::{Id, Privacy};
use anamnesis_server::{App, ServerConfig, api, jobs};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Env {
    dir: tempfile::TempDir,
    app: Arc<App>,
    router: Router,
}

fn env_with(f: impl FnOnce(&mut ServerConfig)) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = ServerConfig::for_tests(&dir.path().join("anamnesis"));
    f(&mut cfg);
    let app = App::open(cfg, false).unwrap();
    let router = api::router(app.clone());
    Env { dir, app, router }
}

fn env() -> Env {
    env_with(|_| {})
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
    let v = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, v)
}

async fn ok(r: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(r, method, uri, body).await;
    assert_eq!(s, StatusCode::OK, "{method} {uri}: {v}");
    v
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), text).unwrap();
}

/// Add a folder source over the api and sync it once; returns the source id.
async fn add_folder(e: &Env, dir: &Path, extra: Value) -> String {
    let mut body =
        json!({ "connector": "folder", "config": { "path": dir.display().to_string() } });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let src = ok(&e.router, "POST", "/api/sources", Some(body)).await;
    let id = src["id"].as_str().unwrap().to_string();
    ok(&e.router, "POST", &format!("/api/sources/{id}/sync"), None).await;
    id
}

fn item(e: &Env, source: &str, ext: &str) -> anamnesis_core::Item {
    let sid = Id(source.to_string());
    e.app
        .brain()
        .with_store(|s| Ok(s.get_item_by_external_id(&sid, ext)?.unwrap()))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_item_privacy_end_to_end() {
    let e = env();
    let notes = e.dir.path().join("notes");
    write(
        &notes,
        "plain.md",
        "# plain\n\nwinter squash keeps till march.",
    );
    write(
        &notes,
        "hand.md",
        "# hand\n\nthe cellar stays at ten degrees.",
    );
    write(
        &notes,
        "secret.md",
        "---\nprivacy: local_only\n---\n# secret\n\nthe spare key is under the stone.",
    );
    let sid = add_folder(&e, &notes, json!({})).await;
    assert_eq!(item(&e, &sid, "plain.md").privacy, Privacy::CloudOk);
    assert_eq!(item(&e, &sid, "secret.md").privacy, Privacy::LocalOnly);

    // set one item by hand
    let hand = item(&e, &sid, "hand.md");
    let v = ok(
        &e.router,
        "PUT",
        &format!("/api/items/{}", hand.id),
        Some(json!({ "privacy": "ask" })),
    )
    .await;
    assert_eq!(v["privacy"], "ask");
    assert_eq!(v["meta"]["privacy_explicit"], true);
    assert_eq!(v["title"], "hand", "nothing else changed");
    let (s, _) = call(
        &e.router,
        "PUT",
        &format!("/api/items/{}", hand.id),
        Some(json!({ "privacy": "nope" })),
    )
    .await;
    assert!(s.is_client_error());

    // the source's default changes: items that took the default follow, the rest stay
    let v = ok(
        &e.router,
        "PUT",
        &format!("/api/sources/{sid}"),
        Some(json!({ "default_privacy": "local_only" })),
    )
    .await;
    assert_eq!(v["default_privacy"], "local_only");
    assert_eq!(v["items_reflagged"], 1);
    assert_eq!(item(&e, &sid, "plain.md").privacy, Privacy::LocalOnly);
    assert_eq!(item(&e, &sid, "hand.md").privacy, Privacy::Ask);
    assert_eq!(item(&e, &sid, "secret.md").privacy, Privacy::LocalOnly);

    // an edit on disk resyncs the item; the hand-set flag survives
    write(
        &notes,
        "hand.md",
        "# hand\n\nthe cellar stays at ten degrees, even in july.",
    );
    ok(&e.router, "POST", &format!("/api/sources/{sid}/sync"), None).await;
    let h = item(&e, &sid, "hand.md");
    assert!(h.body.contains("july"));
    assert_eq!(h.privacy, Privacy::Ask);
    assert_eq!(h.meta["privacy_explicit"], true);

    // the frontmatter flag can be relaxed by the note itself
    write(
        &notes,
        "secret.md",
        "---\nprivacy: cloud_ok\n---\n# secret\n\nthe spare key is under the stone.",
    );
    ok(&e.router, "POST", &format!("/api/sources/{sid}/sync"), None).await;
    assert_eq!(item(&e, &sid, "secret.md").privacy, Privacy::CloudOk);

    // null clears the hand-set flag: back to the source's default
    let v = ok(
        &e.router,
        "PUT",
        &format!("/api/items/{}", hand.id),
        Some(json!({ "privacy": null })),
    )
    .await;
    assert_eq!(v["privacy"], "local_only");
    assert!(v["meta"].get("privacy_explicit").is_none());

    // every change is an event
    let n = e
        .app
        .brain()
        .with_store(|s| Ok(s.events_of_type(&["item_updated"], 100)?.len()))
        .unwrap();
    assert!(n >= 5, "{n}");

    // the mcp brain sees the flag (the mcp layer withholds text that is not cloud_ok)
    use anamnesis_mcp::BrainApi;
    let mb = anamnesis_server::mcp_brain::StoreBrain::new(e.app.clone());
    let view = mb.get_item(&hand.id).await.unwrap().unwrap();
    assert_eq!(view.item.privacy, Privacy::LocalOnly);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn privacy_default_setting_applies_to_new_sources() {
    let e = env();
    ok(
        &e.router,
        "PUT",
        "/api/settings",
        Some(json!({ "privacy": { "default": "ask" } })),
    )
    .await;
    let a = e.dir.path().join("a");
    write(&a, "x.md", "# x\n\nfirst frost tonight.");
    let sid = add_folder(&e, &a, json!({})).await;
    assert_eq!(item(&e, &sid, "x.md").privacy, Privacy::Ask);
    // an explicit flag on the request still wins
    let b = e.dir.path().join("b");
    write(&b, "y.md", "# y\n\nthe pond froze.");
    let sid = add_folder(&e, &b, json!({ "default_privacy": "cloud_ok" })).await;
    assert_eq!(item(&e, &sid, "y.md").privacy, Privacy::CloudOk);
    // and `anamnesis import` of a folder uses the setting too
    let c = e.dir.path().join("c");
    write(&c, "z.md", "# z\n\nsnow on the beds.");
    anamnesis_server::importer::import_path(&e.app.brain(), &c)
        .await
        .unwrap();
    let z = e
        .app
        .brain()
        .with_store(|s| {
            Ok(s.list_items(&Default::default())?
                .into_iter()
                .find(|i| i.external_id == "z.md")
                .unwrap())
        })
        .unwrap();
    assert_eq!(z.privacy, Privacy::Ask);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_default_origin_unless_the_note_says() {
    let e = env();
    let d = e.dir.path().join("clips");
    write(&d, "clip.md", "# clip\n\nsomeone else's words.");
    write(
        &d,
        "mine.md",
        "---\norigin: owner_annotation\n---\n# mine\n\nmy note on it.",
    );
    let sid = add_folder(&e, &d, json!({ "default_origin": "external" })).await;
    assert_eq!(
        item(&e, &sid, "clip.md").origin,
        anamnesis_core::Origin::External
    );
    assert_eq!(
        item(&e, &sid, "mine.md").origin,
        anamnesis_core::Origin::OwnerAnnotation
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_deep_merge() {
    let e = env();
    let r = &e.router;
    ok(
        r,
        "PUT",
        "/api/settings",
        Some(json!({
            "harness": { "claude_cli": { "enabled": false, "model": "sonnet" }, "probe_local": false },
            "theme": { "name": "aphesis", "tokens": { "--bg": "#000", "--ink": "#fff" } }
        })),
    )
    .await;
    let all = ok(
        r,
        "PUT",
        "/api/settings",
        Some(json!({
            "harness": { "claude_cli": { "enabled": true } },
            "theme": { "tokens": { "--ink": null, "--accent": "#c00" } }
        })),
    )
    .await;
    assert_eq!(
        all["harness"],
        json!({ "claude_cli": { "enabled": true, "model": "sonnet" }, "probe_local": false })
    );
    assert_eq!(
        all["theme"],
        json!({ "name": "aphesis", "tokens": { "--bg": "#000", "--accent": "#c00" } })
    );
    // null at the top removes the key; arrays replace
    let all = ok(
        r,
        "PUT",
        "/api/settings",
        Some(json!({ "theme": null, "list": [1, 2] })),
    )
    .await;
    assert!(all.get("theme").is_none());
    let all = ok(r, "PUT", "/api/settings", Some(json!({ "list": [3] }))).await;
    assert_eq!(all["list"], json!([3]));
    // the merged result is validated: a bad nested value is refused, nothing stored
    let (s, _) = call(
        r,
        "PUT",
        "/api/settings",
        Some(json!({ "routing": { "background_to_api_by_default": "yes" } })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(
        r,
        "PUT",
        "/api/settings",
        Some(json!({ "privacy": { "default": "public" } })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_snippets_name_the_data_dir_and_the_demo() {
    let e = env_with(|c| c.demo_bundle = Some(Path::new("/nonexistent").to_path_buf()));
    let v = ok(&e.router, "GET", "/api/mcp/snippets", None).await;
    let data_dir = e.dir.path().display().to_string();
    let args: Vec<String> = serde_json::from_value(v["stdio_args"].clone()).unwrap();
    assert_eq!(args[0], "mcp");
    assert_eq!(args[1], "--data-dir");
    assert_eq!(args[2], data_dir);
    assert!(args.contains(&"--hash-embedder".to_string()));
    let cmd = v["snippets"]["claude_code_command"].as_str().unwrap();
    assert!(cmd.contains("--data-dir"), "{cmd}");
    assert!(cmd.contains(&data_dir), "{cmd}");
    let desktop: Value =
        serde_json::from_str(v["snippets"]["claude_desktop_json"].as_str().unwrap()).unwrap();
    assert_eq!(desktop["mcpServers"]["anamnesis"]["args"][2], data_dir);
    assert!(
        v["snippets"]["codex_toml"]
            .as_str()
            .unwrap()
            .contains("--data-dir")
    );

    // with the demo active the snippets point at the demo brain
    e.app.set_demo(true).unwrap();
    let v = ok(&e.router, "GET", "/api/mcp/snippets", None).await;
    let args: Vec<String> = serde_json::from_value(v["stdio_args"].clone()).unwrap();
    let demo = e.dir.path().join("anamnesis/demo").display().to_string();
    assert_eq!(&args[1..3], &["--brain-dir".to_string(), demo]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn api_keys_fall_back_to_a_0600_file() {
    let e = env();
    let r = &e.router;
    // an anthropic backend pointed at a closed loopback port: detection stays offline
    ok(
        r,
        "PUT",
        "/api/settings",
        Some(json!({ "harness": {
            "anthropic": { "base_url": "http://127.0.0.1:9" },
            "probe_local": false,
            "claude_cli": { "enabled": false, "binary": "/nonexistent/claude" }
        } })),
    )
    .await;
    let v = ok(
        r,
        "PUT",
        "/api/backends/anthropic/key",
        Some(json!({ "key": "sk-ant-test-0000" })),
    )
    .await;
    assert_eq!(v["key_storage"], "file");
    let file = e.dir.path().join("anamnesis/secrets.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    anamnesis_server::backends::detect(&e.app).await;
    let be = ok(r, "GET", "/api/backends", None).await;
    let rows = be["backends"].as_array().unwrap();
    let a = rows.iter().find(|b| b["id"] == "anthropic").unwrap();
    assert_eq!(a["kind"], "anthropic");
    assert_eq!(a["key_storage"], "file");
    // where new keys go, and which store that is
    assert_eq!(be["key_store"]["storage"], "file");
    assert_eq!(be["key_store"]["backend"], "file");
    assert_eq!(a["is_local"], false);
    assert_eq!(a["leaves_machine"], true);
    let ret = rows.iter().find(|b| b["id"] == "retrieval").unwrap();
    assert_eq!(ret["is_local"], true);
    assert_eq!(ret["leaves_machine"], false);
    assert_eq!(
        be["notices"]["claude_cli"],
        anamnesis_harness::adapters::claude_cli::NOTICE
    );
    // the key never comes back, and never travels in an export
    let all = serde_json::to_string(&be).unwrap();
    assert!(!all.contains("sk-ant-test-0000"));
    let res = r
        .clone()
        .oneshot(Request::get("/api/export").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("sk-ant-test-0000"));
    // deleting removes it from the file
    ok(r, "DELETE", "/api/backends/anthropic/key", None).await;
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("sk-ant-test-0000")
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugins_are_listed_from_the_data_dir_and_plugins_dir() {
    use std::os::unix::fs::PermissionsExt;
    let extra = tempfile::tempdir().unwrap();
    let extra_path = extra.path().to_path_buf();
    let e = env_with(|c| c.plugin_dirs = vec![extra_path]);
    let plugin = |dir: &Path, id: &str, file: &str| {
        let p = dir.join(id);
        std::fs::create_dir_all(&p).unwrap();
        let exe = p.join("run.sh");
        std::fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let m = json!({
            "id": id, "name": id, "version": "0.1.0", "api_version": 1,
            "runtime": "exec", "entry": "run.sh",
            "config_schema": { "type": "object" },
            "network_hosts": ["api.example.com"], "fs_paths": ["~/things"],
            "default_origin": "external"
        });
        std::fs::write(p.join(file), m.to_string()).unwrap();
    };
    plugin(
        &e.dir.path().join("anamnesis/plugins"),
        "toy",
        "manifest.json",
    );
    plugin(extra.path(), "other", "anamnesis-plugin.json");
    // a plugin may not take a built-in id
    plugin(extra.path(), "folder", "manifest.json");
    let v = ok(&e.router, "GET", "/api/sources", None).await;
    let conns = v["connectors"].as_array().unwrap();
    let toy = conns.iter().find(|c| c["id"] == "toy").expect("toy listed");
    assert_eq!(toy["plugin"], true);
    assert_eq!(toy["network_hosts"], json!(["api.example.com"]));
    assert_eq!(toy["fs_paths"], json!(["~/things"]));
    assert!(
        conns
            .iter()
            .any(|c| c["id"] == "other" && c["plugin"] == true)
    );
    let folder: Vec<_> = conns.iter().filter(|c| c["id"] == "folder").collect();
    assert_eq!(folder.len(), 1);
    assert_eq!(folder[0]["plugin"], false);
    assert!(!v["plugins"]["errors"].as_array().unwrap().is_empty());
    assert!(e.app.plugins.resolve("toy").is_ok());
}

mod fake {
    use anamnesis_harness::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// An extraction model: calls sentences with "from now on" decisions, answers schema json.
    pub struct Extractor {
        pub id: &'static str,
        pub cost: CostModel,
        pub prompts: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Backend for Extractor {
        fn id(&self) -> &str {
            self.id
        }
        fn kind(&self) -> BackendKind {
            BackendKind::OpenaiCompatible
        }
        fn label(&self) -> String {
            self.id.into()
        }
        fn default_model(&self) -> &str {
            "tiny"
        }
        fn provider(&self) -> &str {
            "fakeco"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                streaming: true,
                tools: false,
                json_schema: true,
                max_context: None,
                vision: false,
                cost_model: self.cost,
                background_allowed: true,
                forbidden_params: vec![],
                terms: Terms::Allowed,
                maturity: Maturity::Stable,
                notice: None,
            }
        }
        async fn available(&self) -> Availability {
            Availability::ready()
        }
        async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
            assert!(req.json_schema.is_some(), "extraction asks for the schema");
            let user = req.messages[0].content.clone();
            self.prompts.lock().unwrap().push(user.clone());
            let note = user
                .rsplit_once("note:\n\"\"\"\n")
                .and_then(|(_, t)| t.strip_suffix("\n\"\"\""))
                .unwrap_or("")
                .to_string();
            let mut items = Vec::new();
            // a phrasing the rule-based extractor does not catch
            for sentence in note.split_inclusive('.') {
                let s = sentence.trim();
                if s.contains("from now on") {
                    items.push(serde_json::json!({
                        "type": "decision", "statement": s, "quote": s,
                        "status": "open", "confidence": 0.9
                    }));
                }
            }
            // and one invented quote, which validation must drop
            items.push(serde_json::json!({
                "type": "plan", "statement": "x", "quote": "words that are not in the note",
                "status": "open", "confidence": 0.9
            }));
            let text = serde_json::json!({ "items": items }).to_string();
            let evs = vec![
                ChatEvent::Token { text },
                ChatEvent::Done { stop_reason: None },
            ];
            Ok(Box::pin(futures_util::stream::iter(evs)))
        }
    }
}

fn router_with(
    b: fake::Extractor,
    routing: anamnesis_harness::RoutingSettings,
) -> Arc<anamnesis_harness::Router> {
    let id = b.id;
    let r = anamnesis_harness::Router::new(
        vec![Arc::new(b)],
        routing,
        Arc::new(anamnesis_harness::Limits::new(Default::default())),
        Default::default(),
    );
    r.set_availability(id, anamnesis_harness::Availability::ready());
    Arc::new(r)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_threads_from_a_local_extractor() {
    let e = env();
    let d = e.dir.path().join("journal");
    write(
        &d,
        "a.md",
        "# porch\n\nlong talk about the porch. the porch steps are oak from now on.",
    );
    write(&d, "b.md", "# rain\n\nrain all day, nothing to report.");
    write(
        &d,
        "c.md",
        "---\norigin: external\n---\n# quote\n\nwe sail west from now on, wrote the captain.",
    );
    let sid = add_folder(&e, &d, json!({})).await;
    let prompts = Arc::new(Mutex::new(Vec::new()));
    e.app.set_router(Some(router_with(
        fake::Extractor {
            id: "local-x",
            cost: anamnesis_harness::CostModel::FreeLocal,
            prompts: prompts.clone(),
        },
        Default::default(),
    )));
    jobs::run_jobs_once(&e.app).await.unwrap();
    let a = item(&e, &sid, "a.md");
    let threads = ok(&e.router, "GET", "/api/threads", None).await;
    let model: Vec<&Value> = threads["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["extractor"] == "local:tiny")
        .collect();
    assert_eq!(model.len(), 1, "{threads}");
    assert_eq!(model[0]["item_id"], json!(a.id));
    assert_eq!(model[0]["kind"], "decision");
    assert!(
        model[0]["quote"]
            .as_str()
            .unwrap()
            .contains("the porch steps are oak")
    );
    // owner-written items only: the external note was never sent
    let sent = prompts.lock().unwrap().clone();
    assert_eq!(sent.len(), 2, "a and b");
    assert!(!sent.iter().any(|p| p.contains("sail west")));
    // local: no egress recorded
    let eg = ok(&e.router, "GET", "/api/egress", None).await;
    assert!(eg["entries"].as_array().unwrap().is_empty());

    // nothing new: no further calls; a changed note is read again
    jobs::run_jobs_once(&e.app).await.unwrap();
    assert_eq!(prompts.lock().unwrap().len(), 2);
    write(
        &d,
        "b.md",
        "# rain\n\nrain all day. the gutters get cleared each spring from now on.",
    );
    ok(&e.router, "POST", &format!("/api/sources/{sid}/sync"), None).await;
    jobs::run_jobs_once(&e.app).await.unwrap();
    assert_eq!(prompts.lock().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_threads_skip_metered_keys_unless_opted_in() {
    let e = env();
    let d = e.dir.path().join("journal");
    write(
        &d,
        "a.md",
        "# a\n\ngarlic goes in each october from now on.",
    );
    write(
        &d,
        "s.md",
        "---\nprivacy: local_only\n---\n# s\n\nthe safe stays in the attic from now on.",
    );
    add_folder(&e, &d, json!({})).await;
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let metered = |p: &Arc<Mutex<Vec<String>>>| fake::Extractor {
        id: "paid",
        cost: anamnesis_harness::CostModel::Metered,
        prompts: p.clone(),
    };
    // default routing: background work never goes to an api key
    e.app
        .set_router(Some(router_with(metered(&prompts), Default::default())));
    jobs::run_jobs_once(&e.app).await.unwrap();
    assert!(prompts.lock().unwrap().is_empty());
    let be = ok(&e.router, "GET", "/api/backends", None).await;
    assert!(be["routing"]["extract"].is_null(), "{be}");
    assert_eq!(be["routing"]["chat"], "paid");

    // opted in: it runs, cloud_ok items only, and every call is in the egress ledger
    e.app.set_router(Some(router_with(
        metered(&prompts),
        anamnesis_harness::RoutingSettings {
            background_to_api_by_default: true,
            ..Default::default()
        },
    )));
    jobs::run_jobs_once(&e.app).await.unwrap();
    let sent = prompts.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(!sent[0].contains("the safe stays"));
    let eg = ok(&e.router, "GET", "/api/egress", None).await;
    let entries = eg["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"]["purpose"], "extract");
    let threads = ok(&e.router, "GET", "/api/threads", None).await;
    assert!(
        threads["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["extractor"] == "fakeco:tiny")
    );
}

mod remote {
    use anamnesis_harness::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// A metered chat backend that records the system prompt (where context goes).
    pub struct Remote {
        pub prompts: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Backend for Remote {
        fn id(&self) -> &str {
            "remote"
        }
        fn kind(&self) -> BackendKind {
            BackendKind::Anthropic
        }
        fn label(&self) -> String {
            "remote".into()
        }
        fn default_model(&self) -> &str {
            "m"
        }
        fn provider(&self) -> &str {
            "example"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                streaming: true,
                tools: false,
                json_schema: false,
                max_context: None,
                vision: false,
                cost_model: CostModel::Metered,
                background_allowed: true,
                forbidden_params: vec![],
                terms: Terms::Allowed,
                maturity: Maturity::Stable,
                notice: None,
            }
        }
        async fn available(&self) -> Availability {
            Availability::ready()
        }
        async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
            self.prompts
                .lock()
                .unwrap()
                .push(req.system.clone().unwrap_or_default());
            let evs = vec![
                ChatEvent::Token { text: "ok".into() },
                ChatEvent::Done { stop_reason: None },
            ];
            Ok(Box::pin(futures_util::stream::iter(evs)))
        }
    }
}

async fn chat_text(r: &Router, body: Value) -> String {
    let req = Request::post("/api/chat")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_partial_consent_with_denied() {
    let e = env();
    let d = e.dir.path().join("cellar");
    write(
        &d,
        "a.md",
        "# apples\n\napples keep in the cellar in straw.",
    );
    write(
        &d,
        "b.md",
        "# pears\n\npears keep in the cellar wrapped in paper.",
    );
    let sid = add_folder(&e, &d, json!({})).await;
    let (a, b) = (item(&e, &sid, "a.md"), item(&e, &sid, "b.md"));
    for it in [&a, &b] {
        ok(
            &e.router,
            "PUT",
            &format!("/api/items/{}", it.id),
            Some(json!({ "privacy": "ask" })),
        )
        .await;
    }
    jobs::run_jobs_once(&e.app).await.unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let r = anamnesis_harness::Router::new(
        vec![Arc::new(remote::Remote {
            prompts: prompts.clone(),
        })],
        anamnesis_harness::RoutingSettings {
            chat: Some("remote".into()),
            ..Default::default()
        },
        Arc::new(anamnesis_harness::Limits::new(Default::default())),
        Default::default(),
    );
    r.set_availability("remote", anamnesis_harness::Availability::ready());
    e.app.set_router(Some(Arc::new(r)));
    let q = "what keeps in the cellar, apples or pears";
    let t = chat_text(&e.router, json!({ "message": q })).await;
    assert!(t.contains("event: consent_required"), "{t}");
    assert!(t.contains(&a.id.0) && t.contains(&b.id.0), "{t}");
    // approve one, deny the other: the call goes out with only the approved item
    let t = chat_text(
        &e.router,
        json!({ "message": q, "approved": [a.id], "denied": [b.id] }),
    )
    .await;
    assert!(t.contains("event: done"), "{t}");
    assert!(!t.contains("consent_required"), "{t}");
    let sent = prompts.lock().unwrap().last().cloned().unwrap();
    assert!(sent.contains("straw"), "{sent}");
    assert!(!sent.contains("wrapped in paper"), "{sent}");
    // deny both: still answered, with neither
    let t = chat_text(&e.router, json!({ "message": q, "denied": [a.id, b.id] })).await;
    assert!(t.contains("event: done"), "{t}");
    let sent = prompts.lock().unwrap().last().cloned().unwrap();
    assert!(!sent.contains("straw") && !sent.contains("wrapped in paper"));
    // a denial wins over an approval of the same id
    let t = chat_text(
        &e.router,
        json!({ "message": q, "approved": [a.id, b.id], "denied": [a.id] }),
    )
    .await;
    assert!(t.contains("event: done"), "{t}");
    let sent = prompts.lock().unwrap().last().cloned().unwrap();
    assert!(!sent.contains("straw") && sent.contains("wrapped in paper"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn demo_switch_runs_the_jobs_in_the_background() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes");
    let e = env_with(|c| c.demo_bundle = Some(fixture));
    let st = ok(
        &e.router,
        "POST",
        "/api/demo",
        Some(json!({ "enabled": true })),
    )
    .await;
    assert_eq!(st["active"], true);
    let mut status = Value::Null;
    for _ in 0..600 {
        status = ok(&e.router, "GET", "/api/status", None).await;
        if status["jobs"]["running"] == false && status["jobs"]["done"] == 5 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(status["jobs"]["running"], false, "{status}");
    assert_eq!(status["jobs"]["total"], 5);
    assert!(status["jobs"]["phase"].is_null());
    assert!(status["jobs"]["error"].is_null(), "{status}");
    let cloud = ok(&e.router, "GET", "/api/cloud", None).await;
    assert_eq!(cloud["ids"].as_array().unwrap().len(), 11);
    assert!(e.app.brain().views().is_some(), "a layout right away");
}
