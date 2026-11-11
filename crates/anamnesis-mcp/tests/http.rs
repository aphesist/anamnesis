//! The Streamable HTTP router: bearer token and host checks, driven with tower's `oneshot`.

#[allow(dead_code)]
mod fake;

use std::sync::Arc;

use anamnesis_mcp::{HttpOptions, http_router};
use axum::{body::Body, http::Request};
use fake::FakeBrain;
use http_body_util::BodyExt;
use tower::ServiceExt;

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;

fn init_request(host: &str, auth: Option<&str>) -> Request<Body> {
    let mut b = Request::post("/mcp")
        .header("host", host)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(a) = auth {
        b = b.header("authorization", a);
    }
    b.body(Body::from(INIT)).unwrap()
}

async fn body_text(res: axum::response::Response) -> String {
    // the sse stream stays open; read the first frame only
    let mut body = res.into_body();
    let mut out = String::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.unwrap().into_data() {
            out.push_str(&String::from_utf8_lossy(&data));
            if out.contains("\"result\"") || out.contains("\"error\"") {
                break;
            }
        }
    }
    out
}

#[tokio::test]
async fn bearer_token_is_required_when_configured() {
    let opts = HttpOptions {
        bearer_token: Some("s3cret".into()),
        ..Default::default()
    };
    let app = http_router(Arc::new(FakeBrain::seeded()), opts);

    let res = app
        .clone()
        .oneshot(init_request("127.0.0.1:7368", None))
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    let res = app
        .clone()
        .oneshot(init_request("127.0.0.1:7368", Some("Bearer nope")))
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    let res = app
        .clone()
        .oneshot(init_request("127.0.0.1:7368", Some("Bearer s3cret")))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let text = body_text(res).await;
    assert!(text.contains("anamnesis"), "initialize result: {text}");
}

#[tokio::test]
async fn open_without_token_but_loopback_hosts_only() {
    let app = http_router(Arc::new(FakeBrain::seeded()), HttpOptions::default());
    let res = app
        .clone()
        .oneshot(init_request("localhost:7368", None))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    // dns rebinding: a foreign host name is refused
    let res = app
        .clone()
        .oneshot(init_request("evil.example.com", None))
        .await
        .unwrap();
    assert!(res.status().is_client_error(), "got {}", res.status());
    // other paths are not served
    let res = app
        .oneshot(
            Request::get("/other")
                .header("host", "127.0.0.1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn foreign_browser_origins_are_refused() {
    let app = http_router(Arc::new(FakeBrain::seeded()), HttpOptions::default());
    let mut req = init_request("127.0.0.1:7368", None);
    req.headers_mut()
        .insert("origin", "https://evil.example.com".parse().unwrap());
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(res.status().is_client_error(), "got {}", res.status());

    let mut req = init_request("127.0.0.1:7368", None);
    req.headers_mut()
        .insert("origin", "http://127.0.0.1:7368".parse().unwrap());
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), 200);
}
