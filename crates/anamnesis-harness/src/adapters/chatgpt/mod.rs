//! "Continue with ChatGPT": chat on the user's ChatGPT plan through OpenAI's sign-in
//! programme for open-source and locally hosted apps.
//!
//! Inference (https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference):
//! `POST https://api.openai.com/v1/responses` with `Authorization: Bearer <access token>`,
//! `store: false` and `stream: true` on every request; models from `GET /v1/models`
//! (`models[]` with `slug`, `display_name`, `visibility == "list"`).
//! Preview limitations (https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations):
//! leave out the unsupported fields listed in `FORBIDDEN_PARAMS` (temperature, top_p,
//! max_output_tokens, previous_response_id…; history goes in `input`), no system-role
//! input items (the system prompt goes in `instructions`). Hosted tools are unsupported.
//! Errors (https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery):
//! plan-usage errors stop inference; there is never a switch to another billing path.
//!
//! The adapter is marked experimental: it follows the published docs but has not been
//! exercised against the live service from this codebase.

pub mod oauth;
pub mod store;

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::backend::Backend;
use crate::decode::{Decoder, UsageAcc, decode_sse};
use crate::error::{HarnessError, Result};
use crate::http::{self, join};
use crate::limits::BackoffPolicy;
use crate::sse::SseEvent;
use crate::types::*;
use oauth::Discovery;
use store::{StoredCredentials, TokenStore, TokenStoreKind};

/// The sign-in button label OpenAI requires (an exception to the lowercase ui voice).
pub const BUTTON_LABEL: &str = "Continue with ChatGPT";
pub const USAGE_SETTINGS_URL: &str = "https://chatgpt.com/settings/usage";
pub const NOTICE: &str = "uses your chatgpt plan through openai's sign-in for open-source apps (preview). you can cap this app's weekly usage in chatgpt settings → usage. signing out here does not disconnect anamnesis; remove it in chatgpt settings to revoke access.";

/// Request fields the plan flow rejects; they are never sent.
pub const FORBIDDEN_PARAMS: &[&str] = &[
    "background",
    "conversation",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "moderation",
    "multi_agent",
    "prompt",
    "prompt_cache_retention",
    "safety_identifier",
    "temperature",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
    "previous_response_id",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatgptConfig {
    #[serde(default = "default_id")]
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    /// Model slug; default: the first listed model.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_issuer")]
    pub issuer: String,
    #[serde(default = "default_api_base")]
    pub api_base: String,
    /// Sent as `agent_name_hint` on first registration.
    #[serde(default = "default_app_name")]
    pub app_name: String,
    /// Preferred loopback port (falls back to any free port).
    #[serde(default = "default_port")]
    pub redirect_port: u16,
    #[serde(default)]
    pub token_store: TokenStoreKind,
    /// Where the host id and (fallback) credential file live.
    pub data_dir: PathBuf,
    #[serde(default = "default_sign_in_timeout")]
    pub sign_in_timeout_secs: u64,
    /// Test hook: skip the jwks signature check (claims are still checked). Not persisted.
    #[serde(skip)]
    pub(crate) skip_id_token_signature: bool,
}

fn default_id() -> String {
    "chatgpt".into()
}
fn default_issuer() -> String {
    oauth::DEFAULT_ISSUER.into()
}
fn default_api_base() -> String {
    "https://api.openai.com/v1".into()
}
fn default_app_name() -> String {
    "anamnesis".into()
}
fn default_port() -> u16 {
    1455
}
fn default_sign_in_timeout() -> u64 {
    300
}

impl ChatgptConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            id: default_id(),
            enabled: false,
            model: None,
            issuer: default_issuer(),
            api_base: default_api_base(),
            app_name: default_app_name(),
            redirect_port: default_port(),
            token_store: TokenStoreKind::default(),
            data_dir,
            sign_in_timeout_secs: default_sign_in_timeout(),
            skip_id_token_signature: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatgptStatus {
    pub signed_in: bool,
    /// The plan-usage scope was granted.
    pub plan_usage: bool,
    pub email: Option<String>,
    pub message: String,
}

/// A started sign-in: open `authorize_url` in the system browser, then `complete`.
pub struct PendingSignIn {
    pub authorize_url: String,
    listener: oauth::LoopbackListener,
    redirect_uri: String,
    state: String,
    nonce: String,
    verifier: String,
    discovery: Discovery,
}

pub struct Chatgpt {
    cfg: ChatgptConfig,
    client: reqwest::Client,
    store: TokenStore,
    creds: Mutex<Option<StoredCredentials>>,
    policy: BackoffPolicy,
}

#[derive(Serialize, Deserialize)]
struct HostFile {
    ext_agent_host_id: String,
}

impl Chatgpt {
    pub fn new(cfg: ChatgptConfig, policy: BackoffPolicy) -> Self {
        let store = TokenStore::new(cfg.token_store, &cfg.data_dir);
        let creds = store.load().ok().flatten();
        Self {
            client: http::client(None),
            store,
            creds: Mutex::new(creds),
            cfg,
            policy,
        }
    }

    /// Opaque per-installation host id, created once and reused across restarts and accounts.
    pub fn host_id(&self) -> Result<String> {
        let path = self.cfg.data_dir.join("chatgpt-host.json");
        if let Ok(s) = std::fs::read_to_string(&path)
            && let Ok(h) = serde_json::from_str::<HostFile>(&s)
        {
            return Ok(h.ext_agent_host_id);
        }
        let h = HostFile {
            ext_agent_host_id: oauth::new_host_id(),
        };
        store::write_private(&path, serde_json::to_string(&h)?.as_bytes())?;
        Ok(h.ext_agent_host_id)
    }

    pub async fn status(&self) -> ChatgptStatus {
        let c = self.creds.lock().await;
        match c.as_ref() {
            Some(c) if c.has_tokens() => ChatgptStatus {
                signed_in: true,
                plan_usage: c.plan_usage_granted(),
                email: c.email.clone(),
                message: if c.plan_usage_granted() {
                    "signed in".into()
                } else {
                    "signed in, but plan usage was not granted".into()
                },
            },
            _ => ChatgptStatus {
                signed_in: false,
                plan_usage: false,
                email: None,
                message: "not signed in".into(),
            },
        }
    }

    /// Step 1: bind the loopback listener and build the authorization url.
    pub async fn begin_sign_in(&self, reconsent: bool) -> Result<PendingSignIn> {
        let discovery = oauth::discover_or_documented(&self.client, &self.cfg.issuer).await;
        let listener = oauth::LoopbackListener::bind(self.cfg.redirect_port).await?;
        let (state, nonce, verifier) = (
            oauth::random_value(),
            oauth::random_value(),
            oauth::random_value(),
        );
        let host_id = self.host_id()?;
        let prev = self.creds.lock().await.clone();
        let client_id = prev.as_ref().and_then(|c| c.client_id.clone());
        let email = prev.as_ref().and_then(|c| c.email.clone());
        let authorize_url = oauth::authorize_url(&oauth::AuthorizeParams {
            authorization_endpoint: &discovery.authorization_endpoint,
            client_id: client_id.as_deref(),
            app_name: &self.cfg.app_name,
            host_id: &host_id,
            redirect_uri: &listener.redirect_uri,
            state: &state,
            nonce: &nonce,
            code_challenge: &oauth::pkce_challenge(&verifier),
            login_hint: email.as_deref(),
            reconsent,
        });
        let redirect_uri = listener.redirect_uri.clone();
        Ok(PendingSignIn {
            authorize_url,
            listener,
            redirect_uri,
            state,
            nonce,
            verifier,
            discovery,
        })
    }

    /// Step 2: wait for the browser, check state, exchange the code, verify the id token,
    /// store the credentials.
    pub async fn complete_sign_in(&self, p: PendingSignIn) -> Result<ChatgptStatus> {
        let cb = p
            .listener
            .wait(Duration::from_secs(self.cfg.sign_in_timeout_secs))
            .await?;
        if cb.state.as_deref() != Some(p.state.as_str()) {
            return Err(HarnessError::OAuth("state mismatch".into()));
        }
        if let Some(e) = cb.error {
            return Err(HarnessError::OAuth(e));
        }
        let code = cb
            .code
            .ok_or_else(|| HarnessError::OAuth("no authorization code".into()))?;
        let host_id = self.host_id()?;
        let mut guard = self.creds.lock().await;
        let mut creds = guard.clone().unwrap_or_default();
        let client_id = cb
            .client_id
            .or_else(|| creds.client_id.clone())
            .ok_or_else(|| HarnessError::OAuth("no issued client id".into()))?;
        // the issued client id is saved before the code exchange
        creds.issuer = p.discovery.issuer.clone();
        creds.client_id = Some(client_id.clone());
        creds.ext_agent_host_id = host_id;
        self.store.save(&creds)?;
        *guard = Some(creds.clone());

        let tokens = oauth::exchange_code(
            &self.client,
            &p.discovery.token_endpoint,
            &client_id,
            &code,
            &p.verifier,
            &p.redirect_uri,
        )
        .await?;
        let id_token = tokens
            .id_token
            .clone()
            .ok_or_else(|| HarnessError::OAuth("invalid_id_token: missing".into()))?;
        if !self.cfg.skip_id_token_signature {
            let jwks_uri = p
                .discovery
                .jwks_uri
                .clone()
                .ok_or_else(|| HarnessError::OAuth("no jwks uri from discovery".into()))?;
            let jwks: Value = self
                .client
                .get(jwks_uri)
                .timeout(Duration::from_secs(15))
                .send()
                .await?
                .json()
                .await?;
            oauth::verify_signature(&id_token, &jwks)?;
        }
        let claims = oauth::decode_claims(&id_token)?;
        let now = Utc::now().timestamp();
        oauth::check_claims(
            &claims,
            &p.discovery.issuer,
            &client_id,
            Some(&p.nonce),
            now,
        )?;
        if let Some(prev_sub) = &creds.subject
            && creds.has_tokens()
            && prev_sub != &claims.sub
        {
            return Err(HarnessError::OAuth(
                "a different chatgpt account signed in; sign out first (one account only)".into(),
            ));
        }
        creds.subject = Some(claims.sub.clone());
        creds.email = claims.email.clone();
        oauth::apply_tokens(&mut creds, &tokens, now);
        if creds.scopes.is_empty()
            && let Some(s) = cb.scope
        {
            creds.scopes = s.split_whitespace().map(str::to_string).collect();
        }
        self.store.save(&creds)?;
        *guard = Some(creds);
        drop(guard);
        Ok(self.status().await)
    }

    /// Revoke the refresh token (best effort) and forget the tokens; the registration is kept.
    pub async fn sign_out(&self) -> Result<()> {
        let mut guard = self.creds.lock().await;
        if let Some(c) = guard.as_mut() {
            if let (Some(rt), Some(cid)) = (c.refresh_token.clone(), c.client_id.clone()) {
                let d = oauth::discover_or_documented(&self.client, &self.cfg.issuer).await;
                if let Some(rev) = d.revocation_endpoint
                    && let Err(e) = oauth::revoke(&self.client, &rev, &cid, &rt).await
                {
                    tracing::warn!("chatgpt revoke failed: {e}");
                }
            }
            c.clear_tokens();
            self.store.save(c)?;
        }
        Ok(())
    }

    /// A fresh access token, refreshing when it expires within a minute (or when forced).
    pub(crate) async fn access_token(&self, force_refresh: bool) -> Result<String> {
        let mut guard = self.creds.lock().await;
        let c = guard
            .as_mut()
            .filter(|c| c.has_tokens())
            .ok_or_else(|| HarnessError::Auth("not signed in".into()))?;
        let now = Utc::now().timestamp();
        let expiring = c.expires_at.is_some_and(|e| e - 60 <= now);
        if !(force_refresh || expiring) {
            return Ok(c.access_token.clone().unwrap_or_default());
        }
        let (Some(rt), Some(cid)) = (c.refresh_token.clone(), c.client_id.clone()) else {
            return Err(HarnessError::Auth("session expired; sign in again".into()));
        };
        let d = oauth::discover_or_documented(&self.client, &self.cfg.issuer).await;
        match oauth::refresh(&self.client, &d.token_endpoint, &cid, &rt).await {
            Ok(t) => {
                oauth::apply_tokens(c, &t, now);
                self.store.save(c)?;
                Ok(t.access_token)
            }
            Err(e) if oauth::is_terminal_refresh_error(&e) => {
                c.clear_tokens();
                self.store.save(c)?;
                Err(HarnessError::Auth(
                    "sign-in expired or was revoked; sign in again".into(),
                ))
            }
            // temporary trouble: keep the credentials
            Err(e) => Err(HarnessError::Unavailable(format!(
                "token refresh failed: {e}"
            ))),
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let tok = self.access_token(false).await?;
        let resp = self
            .client
            .get(join(&self.cfg.api_base, "models"))
            .bearer_auth(tok)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(http::error_from_response(resp).await);
        }
        let v: Value = resp.json().await?;
        Ok(parse_models(&v))
    }

    pub(crate) fn body(&self, req: &ChatRequest, model: &str) -> Value {
        let input: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                json!({"role": role, "content": m.content})
            })
            .collect();
        let mut body = json!({
            "model": model,
            "input": input,
            "store": false,
            "stream": true,
        });
        if let Some(s) = &req.system {
            body["instructions"] = json!(s);
        }
        if let Some(schema) = &req.json_schema {
            body["text"] = json!({"format": {"type": "json_schema", "name": "output", "schema": schema, "strict": true}});
        }
        // temperature / max_tokens are deliberately not sent (preview limitations)
        body
    }

    async fn resolve_model(&self, req: &ChatRequest) -> Result<String> {
        if let Some(m) = req.model.clone().or_else(|| self.cfg.model.clone()) {
            return Ok(m);
        }
        self.list_models()
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| HarnessError::NotConfigured("no chatgpt model available".into()))
    }
}

pub(crate) fn parse_models(v: &Value) -> Vec<String> {
    if let Some(a) = v.get("models").and_then(|m| m.as_array()) {
        return a
            .iter()
            .filter(|m| {
                m.get("visibility")
                    .and_then(|x| x.as_str())
                    .is_none_or(|x| x == "list")
            })
            .filter_map(|m| {
                m.get("slug")
                    .or_else(|| m.get("id"))
                    .and_then(|s| s.as_str())
                    .map(str::to_string)
            })
            .collect();
    }
    super::openai_compatible::parse_model_list(v)
}

#[async_trait]
impl Backend for Chatgpt {
    fn id(&self) -> &str {
        &self.cfg.id
    }
    fn kind(&self) -> BackendKind {
        BackendKind::Chatgpt
    }
    fn label(&self) -> String {
        "chatgpt plan".into()
    }
    fn default_model(&self) -> &str {
        self.cfg.model.as_deref().unwrap_or("default")
    }
    fn provider(&self) -> &str {
        "openai (chatgpt plan)"
    }
    fn capabilities(&self) -> Capabilities {
        let mut c = Capabilities::base(CostModel::Subscription);
        c.background_allowed = false;
        c.json_schema = true;
        c.vision = true;
        c.forbidden_params = FORBIDDEN_PARAMS.iter().map(|s| s.to_string()).collect();
        c.terms = Terms::Allowed;
        c.maturity = Maturity::Experimental;
        c.notice = Some(NOTICE.into());
        c
    }
    fn is_local(&self) -> bool {
        false
    }
    async fn available(&self) -> Availability {
        if !self.cfg.enabled {
            return Availability::down("not enabled");
        }
        let st = self.status().await;
        if !st.signed_in {
            return Availability::down("not signed in");
        }
        if !st.plan_usage {
            return Availability::down("plan usage not granted");
        }
        match self.list_models().await {
            Ok(models) => Availability {
                available: true,
                status: "ready".into(),
                models,
            },
            Err(e) => Availability::down(format!("{e}")),
        }
    }
    async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
        if !self.cfg.enabled {
            return Err(HarnessError::NotConfigured(
                "chatgpt sign-in is not enabled".into(),
            ));
        }
        if !self.status().await.plan_usage {
            return Err(HarnessError::Auth(
                "plan usage not granted; sign in again to allow it".into(),
            ));
        }
        let model = self.resolve_model(&req).await?;
        let body = self.body(&req, &model);
        let url = join(&self.cfg.api_base, "responses");
        let mut forced = false;
        loop {
            let tok = self.access_token(forced).await?;
            let resp = http::send_with_retry(&self.policy, || {
                self.client.post(&url).bearer_auth(&tok).json(&body)
            })
            .await;
            match resp {
                Ok(r) => {
                    let mut acc = UsageAcc::new(req.estimated_input_tokens());
                    acc.force_estimated = true;
                    return Ok(decode_sse(r.bytes_stream(), ResponsesDecoder::new(acc)));
                }
                // one refresh-and-retry on an auth failure
                Err(HarnessError::Auth(code))
                    if !forced && !code.starts_with("subscription_sharing") =>
                {
                    forced = true
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Map a plan-flow error code to a failure.
pub(crate) fn siwc_failure(code: &str) -> FailureKind {
    match code {
        "subscription_sharing_usage_limit_exceeded" => FailureKind::UsageLimit { resets_at: None },
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            FailureKind::Unavailable
        }
        "subscription_sharing_user_not_eligible"
        | "subscription_sharing_invalid_user"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => FailureKind::Auth,
        "subscription_sharing_unsupported_capability"
        | "subscription_sharing_route_not_supported" => FailureKind::InvalidRequest,
        "rate_limit_exceeded" => FailureKind::RateLimited {
            retry_after_secs: None,
        },
        _ => FailureKind::Other,
    }
}

/// Responses api stream decoder.
pub(crate) struct ResponsesDecoder {
    acc: UsageAcc,
}

impl ResponsesDecoder {
    pub(crate) fn new(acc: UsageAcc) -> Self {
        Self { acc }
    }
    fn take_usage(&mut self, resp: &Value) {
        if let Some(u) = resp.get("usage").filter(|u| !u.is_null()) {
            self.acc.input = u.get("input_tokens").and_then(|x| x.as_u64());
            self.acc.output = u.get("output_tokens").and_then(|x| x.as_u64());
        }
    }
}

impl Decoder for ResponsesDecoder {
    fn event(&mut self, ev: &SseEvent) -> Vec<ChatEvent> {
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            return Vec::new();
        };
        let ty = v
            .get("type")
            .and_then(|t| t.as_str())
            .or(ev.event.as_deref())
            .unwrap_or("");
        match ty {
            "response.output_text.delta" => match v.get("delta").and_then(|d| d.as_str()) {
                Some(t) if !t.is_empty() => {
                    self.acc.text(t);
                    vec![ChatEvent::Token {
                        text: t.to_string(),
                    }]
                }
                _ => Vec::new(),
            },
            "response.completed" => {
                self.take_usage(v.get("response").unwrap_or(&Value::Null));
                vec![
                    ChatEvent::Usage(self.acc.usage()),
                    ChatEvent::Done {
                        stop_reason: Some("completed".into()),
                    },
                ]
            }
            "response.incomplete" => {
                let r = v.get("response").unwrap_or(&Value::Null);
                self.take_usage(r);
                let reason = r
                    .pointer("/incomplete_details/reason")
                    .and_then(|x| x.as_str())
                    .unwrap_or("incomplete")
                    .to_string();
                vec![
                    ChatEvent::Usage(self.acc.usage()),
                    ChatEvent::Done {
                        stop_reason: Some(reason),
                    },
                ]
            }
            "response.failed" | "error" => {
                let err = v
                    .pointer("/response/error")
                    .or_else(|| v.get("error"))
                    .unwrap_or(&v);
                let code = err.get("code").and_then(|c| c.as_str()).unwrap_or("");
                let message = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or(code)
                    .to_string();
                vec![ChatEvent::Error {
                    message: if message.is_empty() {
                        "chatgpt request failed".into()
                    } else {
                        message
                    },
                    failure: siwc_failure(code),
                }]
            }
            _ => Vec::new(),
        }
    }

    fn finish(&mut self) -> Vec<ChatEvent> {
        vec![
            ChatEvent::Usage(self.acc.usage()),
            ChatEvent::Done { stop_reason: None },
        ]
    }
}

#[cfg(test)]
mod tests;
