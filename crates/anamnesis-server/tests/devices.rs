//! Sync across devices through the http api: two apps (two data dirs) on one folder.

use std::path::Path;
use std::sync::Arc;

use anamnesis_server::{App, ServerConfig, api};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Dev {
    app: Arc<App>,
    r: Router,
}

fn dev(root: &Path) -> Dev {
    let app = App::open(ServerConfig::for_tests(root), false).unwrap();
    let r = api::router(app.clone());
    Dev { app, r }
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

fn later() {
    std::thread::sleep(std::time::Duration::from_millis(3));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_devices_sync_resolve_and_unlink_over_the_api() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("Dropbox/anamnesis");
    let a = dev(&tmp.path().join("a/anamnesis"));
    let b = dev(&tmp.path().join("b/anamnesis"));

    let st = ok(&a.r, "GET", "/api/devices", None).await;
    assert_eq!(st["configured"], false);

    let setup = |name: &str, pass: &str| json!({ "kind": "folder", "path": folder.display().to_string(), "passphrase": pass, "device_name": name });
    let v = ok(
        &a.r,
        "POST",
        "/api/devices/setup",
        Some(setup("laptop", "walden at dawn")),
    )
    .await;
    assert_eq!(v["created"], true);
    assert_eq!(v["place"]["kind"], "folder");
    assert!(v["place"]["inbox"].as_str().unwrap().ends_with("inbox"));
    // the key is not a setting and never in a response
    let settings = ok(&a.r, "GET", "/api/settings", None).await;
    assert!(!settings.to_string().contains("sync-key"));

    // a note on the laptop
    let note = ok(
        &a.r,
        "POST",
        "/api/items",
        Some(json!({ "title": "on walking", "body": "walking is how the thinking happens" })),
    )
    .await;
    let id = note["id"].as_str().unwrap().to_string();
    let v = ok(&a.r, "POST", "/api/devices/sync", None).await;
    assert!(v["report"]["exported"].as_u64().unwrap() > 0, "{v}");

    // the phone joins: a wrong passphrase first
    let (s, e) = call(
        &b.r,
        "POST",
        "/api/devices/setup",
        Some(setup("phone", "concord at dusk")),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    assert!(e["error"].as_str().unwrap().contains("passphrase"));
    let v = ok(
        &b.r,
        "POST",
        "/api/devices/setup",
        Some(setup("phone", "walden at dawn")),
    )
    .await;
    assert_eq!(v["created"], false);
    let v = ok(&b.r, "POST", "/api/devices/sync", None).await;
    assert!(v["report"]["imported"].as_u64().unwrap() > 0, "{v}");
    let item = ok(&b.r, "GET", &format!("/api/items/{id}"), None).await;
    assert_eq!(item["item"]["body"], "walking is how the thinking happens");

    // both edit; the phone's edit is later
    later();
    ok(
        &a.r,
        "PUT",
        &format!("/api/items/{id}"),
        Some(json!({ "body": "the laptop's words" })),
    )
    .await;
    later();
    ok(
        &b.r,
        "PUT",
        &format!("/api/items/{id}"),
        Some(json!({ "body": "the phone's words" })),
    )
    .await;
    ok(&a.r, "POST", "/api/devices/sync", None).await;
    let v = ok(&b.r, "POST", "/api/devices/sync", None).await;
    assert_eq!(v["report"]["new_conflicts"], 1, "{v}");
    let v = ok(&a.r, "POST", "/api/devices/sync", None).await;
    let conflicts = v["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    let names: Vec<&str> = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["laptop", "phone"]);
    let item = ok(&a.r, "GET", &format!("/api/items/{id}"), None).await;
    assert_eq!(
        item["item"]["body"], "the phone's words",
        "the later clock is shown"
    );

    // keep the laptop's words, on the laptop
    let c = &conflicts[0];
    let mine = c["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["item"]["body"] == "the laptop's words")
        .unwrap();
    let v = ok(
        &a.r,
        "POST",
        &format!("/api/devices/conflicts/{}", c["id"].as_str().unwrap()),
        Some(json!({ "keep": "one", "version": mine["version"] })),
    )
    .await;
    assert_eq!(v["conflicts"], json!([]));
    assert_eq!(v["item"]["body"], "the laptop's words");
    ok(&a.r, "POST", "/api/devices/sync", None).await;
    let v = ok(&b.r, "POST", "/api/devices/sync", None).await;
    assert_eq!(v["conflicts"], json!([]), "{v}");
    let item = ok(&b.r, "GET", &format!("/api/items/{id}"), None).await;
    assert_eq!(item["item"]["body"], "the laptop's words");

    // a source added on the laptop is not run on the phone
    let notes = tmp.path().join("laptop-notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("x.md"), "a note in a folder on the laptop").unwrap();
    let src = ok(
        &a.r,
        "POST",
        "/api/sources",
        Some(json!({ "connector": "folder", "config": { "path": notes.display().to_string() } })),
    )
    .await;
    anamnesis_server::sync::sync_source(
        &a.app.brain(),
        &a.app.plugins,
        &anamnesis_core::Id(src["id"].as_str().unwrap().into()),
    )
    .await
    .unwrap();
    ok(&a.r, "POST", "/api/devices/sync", None).await;
    ok(&b.r, "POST", "/api/devices/sync", None).await;
    let src_id = src["id"].as_str().unwrap();
    assert!(anamnesis_server::devices::is_foreign(
        &b.app,
        &b.app.brain(),
        src_id
    ));
    let ran = anamnesis_server::sync::sync_all(&b.app).await.unwrap();
    assert!(
        ran.iter().all(|(s, _)| s.id.0 != src_id),
        "the laptop's folder is not synced by the phone"
    );
    let cloud = ok(&b.r, "GET", "/api/cloud", None).await;
    assert!(
        cloud["titles"].as_array().unwrap().iter().any(|t| t == "x"),
        "{}",
        cloud["titles"]
    );

    // the laptop unlinks the phone; the phone stops
    let phone_id = ok(&b.r, "GET", "/api/devices", None).await["this_device"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let v = ok(
        &a.r,
        "POST",
        &format!("/api/devices/{phone_id}/unlink"),
        None,
    )
    .await;
    let phone = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == json!(phone_id))
        .unwrap();
    assert_eq!(phone["unlinked"]["by"], "laptop");
    let (s, e) = call(&b.r, "POST", "/api/devices/sync", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(e["error"].as_str().unwrap().contains("unlinked"), "{e}");

    // stopping sync forgets the place and the key, not the brain
    let v = ok(&b.r, "DELETE", "/api/devices", None).await;
    assert_eq!(v["configured"], false);
    let item = ok(&b.r, "GET", &format!("/api/items/{id}"), None).await;
    assert_eq!(item["item"]["body"], "the laptop's words");
}
