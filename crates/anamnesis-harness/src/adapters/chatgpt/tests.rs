//! "Continue with ChatGPT": sign-in flow, refresh, and Responses streaming against a mock
//! authorization server and api (no network).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

use super::*;
use crate::testutil::{MockServer, Recorded, Reply, collect};

const RESPONSES_SSE: &str = "event: response.created\n\
data: {\"type\":\"response.created\",\"response\":{\"id\":\"r1\",\"status\":\"in_progress\"}}\n\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"item_id\":\"m1\",\"output_index\":0,\"content_index\":0,\"delta\":\"plan \"}\n\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"item_id\":\"m1\",\"output_index\":0,\"content_index\":0,\"delta\":\"words\"}\n\n\
event: response.output_text.done\n\
data: {\"type\":\"response.output_text.done\",\"text\":\"plan words\"}\n\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":20,\"output_tokens\":3,\"total_tokens\":23}}}\n\n";

const RESPONSES_LIMIT_SSE: &str = "event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"par\"}\n\n\
event: response.failed\n\
data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"usage limit\"}}}\n\n";

#[derive(Default)]
struct AuthState {
    nonce: Option<String>,
    refresh_fails: bool,
}

fn form(body: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect()
}

fn jwt(claims: serde_json::Value) -> String {
    format!(
        "{}.{}.{}",
        B64.encode(br#"{"alg":"RS256","kid":"k1"}"#),
        B64.encode(claims.to_string()),
        B64.encode(b"unsigned-test")
    )
}

async fn server(state: Arc<Mutex<AuthState>>) -> MockServer {
    // the handler needs its own base url for discovery; fill it after start
    let base_cell: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let bc = base_cell.clone();
    let srv = MockServer::start(move |r: &Recorded| {
        let base = bc.lock().unwrap().clone();
        let st = state.clone();
        match (r.method.as_str(), r.path.as_str()) {
            ("GET", "/.well-known/openid-configuration") => Some(Reply::json(
                200,
                &serde_json::json!({
                    "issuer": base,
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "revocation_endpoint": format!("{base}/revoke"),
                    "jwks_uri": format!("{base}/jwks"),
                })
                .to_string(),
            )),
            ("POST", "/token") => {
                let f = form(&r.body);
                match f.get("grant_type").map(String::as_str) {
                    Some("authorization_code") => {
                        let nonce = st.lock().unwrap().nonce.clone();
                        let id_token = jwt(serde_json::json!({
                            "iss": base, "sub": "user-1", "aud": "oaiapp_test",
                            "exp": chrono::Utc::now().timestamp() + 3600, "nonce": nonce, "email": "u@example.org"
                        }));
                        Some(Reply::json(
                            200,
                            &serde_json::json!({
                                "access_token": "at1", "refresh_token": "rt1", "id_token": id_token,
                                "token_type": "Bearer", "expires_in": 3600,
                                "scope": "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"
                            })
                            .to_string(),
                        ))
                    }
                    Some("refresh_token") => {
                        if st.lock().unwrap().refresh_fails {
                            Some(Reply::json(400, r#"{"error":"invalid_grant"}"#))
                        } else {
                            Some(Reply::json(
                                200,
                                r#"{"access_token":"at2","refresh_token":"rt2","expires_in":3600,"token_type":"Bearer"}"#,
                            ))
                        }
                    }
                    _ => Some(Reply::json(400, r#"{"error":"unsupported_grant_type"}"#)),
                }
            }
            ("POST", "/revoke") => Some(Reply::json(200, "{}")),
            ("GET", "/v1/models") => Some(Reply::json(
                200,
                r#"{"models":[{"slug":"gpt-test","display_name":"GPT Test","visibility":"list"},{"slug":"hidden","visibility":"hide"}]}"#,
            )),
            ("POST", "/v1/responses") => {
                let body: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
                if body["input"][0]["content"] == "limit" {
                    Some(Reply::json(429, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"x"}}"#))
                } else if r.headers.get("authorization").map(String::as_str) == Some("Bearer stale") {
                    Some(Reply::json(401, r#"{"detail":"unauthorized"}"#))
                } else {
                    Some(Reply::sse(RESPONSES_SSE, 19))
                }
            }
            _ => None,
        }
    })
    .await;
    *base_cell.lock().unwrap() = srv.base.clone();
    srv
}

fn config(srv: &MockServer, dir: &std::path::Path) -> ChatgptConfig {
    let mut c = ChatgptConfig::new(dir.to_path_buf());
    c.enabled = true;
    c.issuer = srv.base.clone();
    c.api_base = format!("{}/v1", srv.base);
    c.redirect_port = 0;
    c.token_store = TokenStoreKind::File;
    c.sign_in_timeout_secs = 10;
    c.skip_id_token_signature = true;
    c
}

fn query(u: &str) -> HashMap<String, String> {
    url::Url::parse(u)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

/// Run the browser side: follow the redirect with a code.
async fn browser(redirect_uri: String, state: String, client_id: Option<&str>) {
    let mut u = url::Url::parse(&redirect_uri).unwrap();
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("code", "code-1");
        q.append_pair("state", &state);
        q.append_pair(
            "scope",
            "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
        );
        if let Some(c) = client_id {
            q.append_pair("client_id", c);
        }
    }
    let r = reqwest::get(u).await.unwrap();
    assert!(r.text().await.unwrap().contains("signed in"));
}

async fn signed_in(srv: &MockServer, st: &Arc<Mutex<AuthState>>, dir: &std::path::Path) -> Chatgpt {
    let b = Chatgpt::new(
        config(srv, dir),
        BackoffPolicy {
            base_ms: 1,
            max_ms: 2,
            max_retries: 0,
            max_inline_wait_ms: 0,
        },
    );
    let p = b.begin_sign_in(false).await.unwrap();
    let q = query(&p.authorize_url);
    st.lock().unwrap().nonce = Some(q["nonce"].clone());
    let h = tokio::spawn(browser(
        q["redirect_uri"].clone(),
        q["state"].clone(),
        Some("oaiapp_test"),
    ));
    let status = b.complete_sign_in(p).await.unwrap();
    h.await.unwrap();
    assert!(status.signed_in && status.plan_usage);
    b
}

#[tokio::test]
async fn sign_in_flow_pkce_registration_and_storage() {
    let st = Arc::new(Mutex::new(AuthState::default()));
    let srv = server(st.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Chatgpt::new(config(&srv, dir.path()), BackoffPolicy::default());
    assert!(!b.status().await.signed_in);

    let p = b.begin_sign_in(false).await.unwrap();
    let q = query(&p.authorize_url);
    assert!(
        p.authorize_url
            .starts_with(&format!("{}/authorize?", srv.base))
    );
    assert_eq!(q["client_id"], "dynamic_agent_client");
    assert_eq!(q["agent_name_hint"], "anamnesis");
    assert!(q["ext_agent_host_id"].starts_with("urn:uuid:"));
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["resource"], "https://api.openai.com/v1");
    assert!(
        q["redirect_uri"].starts_with("http://127.0.0.1:")
            && q["redirect_uri"].ends_with("/auth/callback")
    );
    let host_id = q["ext_agent_host_id"].clone();
    st.lock().unwrap().nonce = Some(q["nonce"].clone());

    let h = tokio::spawn(browser(
        q["redirect_uri"].clone(),
        q["state"].clone(),
        Some("oaiapp_test"),
    ));
    let status = b.complete_sign_in(p).await.unwrap();
    h.await.unwrap();
    assert!(status.signed_in);
    assert!(status.plan_usage);
    assert_eq!(status.email.as_deref(), Some("u@example.org"));

    // the token request carried the verifier matching the challenge
    let tok = srv
        .recorded()
        .into_iter()
        .find(|r| r.path == "/token")
        .unwrap();
    let f = form(&tok.body);
    assert_eq!(f["grant_type"], "authorization_code");
    assert_eq!(f["client_id"], "oaiapp_test");
    assert_eq!(
        oauth::pkce_challenge(&f["code_verifier"]),
        q["code_challenge"]
    );
    assert_eq!(f["redirect_uri"], q["redirect_uri"]);
    assert_eq!(f["resource"], "https://api.openai.com/v1");
    assert!(!f.contains_key("client_secret"));

    // stored in the 0600 file (file store selected), host id stable
    let stored = TokenStore::new(TokenStoreKind::File, dir.path())
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(stored.client_id.as_deref(), Some("oaiapp_test"));
    assert_eq!(stored.access_token.as_deref(), Some("at1"));
    assert_eq!(b.host_id().unwrap(), host_id);

    // re-authorisation reuses the issued client id and does not re-send the name hint
    let p2 = b.begin_sign_in(false).await.unwrap();
    let q2 = query(&p2.authorize_url);
    assert_eq!(q2["client_id"], "oaiapp_test");
    assert!(!q2.contains_key("agent_name_hint"));
    assert_eq!(q2["ext_agent_host_id"], host_id);
    assert_eq!(q2["login_hint"], "u@example.org");
}

#[tokio::test]
async fn state_mismatch_is_rejected() {
    let st = Arc::new(Mutex::new(AuthState::default()));
    let srv = server(st.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Chatgpt::new(config(&srv, dir.path()), BackoffPolicy::default());
    let p = b.begin_sign_in(false).await.unwrap();
    let q = query(&p.authorize_url);
    let h = tokio::spawn(browser(
        q["redirect_uri"].clone(),
        "forged".into(),
        Some("oaiapp_test"),
    ));
    assert!(
        matches!(b.complete_sign_in(p).await, Err(HarnessError::OAuth(m)) if m.contains("state"))
    );
    h.await.unwrap();
    assert!(
        !srv.recorded().iter().any(|r| r.path == "/token"),
        "no code exchange on a state mismatch"
    );
}

#[tokio::test]
async fn chat_streams_with_preview_constraints() {
    let st = Arc::new(Mutex::new(AuthState::default()));
    let srv = server(st.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let b = signed_in(&srv, &st, dir.path()).await;
    let a = b.available().await;
    assert!(a.available);
    assert_eq!(a.models, vec!["gpt-test"], "hidden models are filtered");

    let mut r = ChatRequest::new(vec![
        Message::user("hi"),
        Message::assistant("hello"),
        Message::user("more"),
    ])
    .with_system("be the brain");
    r.temperature = Some(0.7);
    r.max_tokens = Some(50);
    let (text, evs) = collect(b.chat(r).await.unwrap()).await;
    assert_eq!(text, "plan words");
    let u = evs
        .iter()
        .find_map(|e| {
            if let ChatEvent::Usage(u) = e {
                Some(*u)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!((u.input, u.output), (20, 3));
    assert!(u.estimated, "plan usage is marked estimated");

    let rec = srv
        .recorded()
        .into_iter()
        .find(|r| r.path == "/v1/responses")
        .unwrap();
    assert_eq!(rec.headers["authorization"], "Bearer at1");
    let body: serde_json::Value = serde_json::from_str(&rec.body).unwrap();
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "gpt-test");
    assert_eq!(body["instructions"], "be the brain");
    assert_eq!(body["input"].as_array().unwrap().len(), 3);
    for f in FORBIDDEN_PARAMS {
        assert!(body.get(*f).is_none(), "{f} must not be sent");
    }
    assert!(body.get("max_tokens").is_none());
}

#[tokio::test]
async fn usage_limit_http_error_stops_without_fallback() {
    let st = Arc::new(Mutex::new(AuthState::default()));
    let srv = server(st.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let b = signed_in(&srv, &st, dir.path()).await;
    let r = ChatRequest::new(vec![Message::user("limit")]);
    assert!(matches!(b.chat(r).await, Err(HarnessError::UsageLimit(_))));
}

#[tokio::test]
async fn refresh_when_expired_and_on_401() {
    let st = Arc::new(Mutex::new(AuthState::default()));
    let srv = server(st.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let b = signed_in(&srv, &st, dir.path()).await;
    b.creds.lock().await.as_mut().unwrap().expires_at = Some(chrono::Utc::now().timestamp() - 10);
    let (text, _) = collect(
        b.chat(ChatRequest::new(vec![Message::user("x")]))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text, "plan words");
    let refresh = srv
        .recorded()
        .into_iter()
        .filter(|r| r.path == "/token")
        .map(|r| form(&r.body))
        .find(|f| f["grant_type"] == "refresh_token")
        .unwrap();
    assert_eq!(refresh["refresh_token"], "rt1");
    assert_eq!(refresh["client_id"], "oaiapp_test");
    assert_eq!(refresh["resource"], "https://api.openai.com/v1");
    let last = srv
        .recorded()
        .into_iter()
        .rfind(|r| r.path == "/v1/responses")
        .unwrap();
    assert_eq!(last.headers["authorization"], "Bearer at2");
    // rotated refresh token persisted
    let stored = TokenStore::new(TokenStoreKind::File, dir.path())
        .load()
        .unwrap()
        .unwrap();
    assert_eq!(stored.refresh_token.as_deref(), Some("rt2"));

    // a 401 triggers one refresh-and-retry
    b.creds.lock().await.as_mut().unwrap().access_token = Some("stale".into());
    let (text, _) = collect(
        b.chat(ChatRequest::new(vec![Message::user("x")]))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text, "plan words");
}

#[tokio::test]
async fn terminal_refresh_error_clears_tokens_keeps_registration() {
    let st = Arc::new(Mutex::new(AuthState::default()));
    let srv = server(st.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let b = signed_in(&srv, &st, dir.path()).await;
    st.lock().unwrap().refresh_fails = true;
    b.creds.lock().await.as_mut().unwrap().expires_at = Some(0);
    assert!(matches!(
        b.chat(ChatRequest::new(vec![Message::user("x")])).await,
        Err(HarnessError::Auth(_))
    ));
    assert!(!b.status().await.signed_in);
    let stored = TokenStore::new(TokenStoreKind::File, dir.path())
        .load()
        .unwrap()
        .unwrap();
    assert!(stored.access_token.is_none() && stored.refresh_token.is_none());
    assert_eq!(stored.client_id.as_deref(), Some("oaiapp_test"));
}

#[tokio::test]
async fn sign_out_revokes_and_forgets_tokens() {
    let st = Arc::new(Mutex::new(AuthState::default()));
    let srv = server(st.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let b = signed_in(&srv, &st, dir.path()).await;
    b.sign_out().await.unwrap();
    assert!(!b.status().await.signed_in);
    let rev = srv
        .recorded()
        .into_iter()
        .find(|r| r.path == "/revoke")
        .unwrap();
    let f = form(&rev.body);
    assert_eq!(f["token"], "rt1");
    assert_eq!(f["token_type_hint"], "refresh_token");
}

#[tokio::test]
async fn responses_stream_failure_maps_usage_limit() {
    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = RESPONSES_LIMIT_SSE
        .as_bytes()
        .chunks(7)
        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
        .collect();
    let (text, evs) = collect(decode_sse(
        futures_util::stream::iter(chunks),
        ResponsesDecoder::new(UsageAcc::new(0)),
    ))
    .await;
    assert_eq!(text, "par");
    assert!(matches!(
        evs.last(),
        Some(ChatEvent::Error {
            failure: FailureKind::UsageLimit { resets_at: None },
            ..
        })
    ));
}

#[test]
fn capabilities_are_experimental_plan_backend() {
    let b = Chatgpt::new(
        ChatgptConfig {
            token_store: TokenStoreKind::File,
            ..ChatgptConfig::new(std::env::temp_dir().join("anamnesis-none"))
        },
        BackoffPolicy::default(),
    );
    let c = b.capabilities();
    assert_eq!(c.cost_model, CostModel::Subscription);
    assert!(!c.background_allowed);
    assert_eq!(c.maturity, Maturity::Experimental);
    assert!(c.forbidden_params.contains(&"temperature".to_string()));
}
