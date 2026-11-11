//! Anthropic Messages API with the user's own api key.
//!
//! `POST {base_url}/v1/messages` with `x-api-key`, `anthropic-version`, `stream: true`.
//! Stream events: `message_start` (input usage), `content_block_delta` / `text_delta`,
//! `message_delta` (stop reason, output usage), `message_stop`, `error`, `ping`.
//! Reference: https://docs.claude.com/en/api/messages-streaming

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::backend::Backend;
use crate::decode::{Decoder, UsageAcc, decode_sse};
use crate::error::{HarnessError, Result};
use crate::http::{self, join, send_with_retry};
use crate::limits::BackoffPolicy;
use crate::sse::SseEvent;
use crate::types::*;

/// The cheap model used for background labelling by default.
pub const DEFAULT_LABEL_MODEL: &str = "claude-haiku-5-5";
pub const DEFAULT_CHAT_MODEL: &str = "claude-opus-5-5";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicConfig {
    #[serde(default = "default_id")]
    pub id: String,
    #[serde(default = "default_base")]
    pub base_url: String,
    /// Never written back out by serde; the server loads it from the os keychain.
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    /// Model for interactive chat.
    #[serde(default = "default_chat_model")]
    pub chat_model: String,
    /// Model for background work (label / extract / summarise).
    #[serde(default = "default_label_model")]
    pub label_model: String,
    #[serde(default = "default_version")]
    pub anthropic_version: String,
    /// Default `max_tokens` (required by the api) when a request sets none.
    #[serde(default = "default_max_tokens")]
    pub default_max_tokens: u32,
    /// Current models reject non-default sampling parameters, so temperature is not
    /// sent unless this is turned on (for older models that still accept it).
    #[serde(default)]
    pub send_temperature: bool,
}

fn default_id() -> String {
    "anthropic".into()
}
fn default_base() -> String {
    "https://api.anthropic.com".into()
}
fn default_chat_model() -> String {
    DEFAULT_CHAT_MODEL.into()
}
fn default_label_model() -> String {
    DEFAULT_LABEL_MODEL.into()
}
fn default_version() -> String {
    "2023-06-01".into()
}
fn default_max_tokens() -> u32 {
    8192
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            id: default_id(),
            base_url: default_base(),
            api_key: None,
            chat_model: default_chat_model(),
            label_model: default_label_model(),
            anthropic_version: default_version(),
            default_max_tokens: default_max_tokens(),
            send_temperature: false,
        }
    }
}

pub struct Anthropic {
    cfg: AnthropicConfig,
    client: reqwest::Client,
    policy: BackoffPolicy,
}

impl Anthropic {
    pub fn new(cfg: AnthropicConfig, policy: BackoffPolicy) -> Self {
        Self {
            cfg,
            client: http::client(None),
            policy,
        }
    }

    pub fn config(&self) -> &AnthropicConfig {
        &self.cfg
    }

    fn headers(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        let key = self
            .cfg
            .api_key
            .as_deref()
            .ok_or_else(|| HarnessError::NotConfigured("anthropic api key".into()))?;
        Ok(rb
            .header("x-api-key", key)
            .header("anthropic-version", &self.cfg.anthropic_version))
    }

    pub(crate) fn body(&self, req: &ChatRequest) -> Value {
        let messages: Vec<Value> = req
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
            "model": req.model.clone().unwrap_or_else(|| self.cfg.chat_model.clone()),
            "max_tokens": req.max_tokens.unwrap_or(self.cfg.default_max_tokens),
            "messages": messages,
            "stream": true,
        });
        if let Some(s) = &req.system {
            body["system"] = json!(s);
        }
        if let (Some(t), true) = (req.temperature, self.cfg.send_temperature) {
            body["temperature"] = json!(t);
        }
        if let Some(schema) = &req.json_schema {
            body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
        }
        body
    }
}

#[async_trait]
impl Backend for Anthropic {
    fn id(&self) -> &str {
        &self.cfg.id
    }
    fn kind(&self) -> BackendKind {
        BackendKind::Anthropic
    }
    fn label(&self) -> String {
        "anthropic (api key)".into()
    }
    fn default_model(&self) -> &str {
        &self.cfg.chat_model
    }
    fn model_for(&self, task: Task) -> String {
        if task.is_background() {
            self.cfg.label_model.clone()
        } else {
            self.cfg.chat_model.clone()
        }
    }
    fn provider(&self) -> &str {
        "anthropic"
    }
    fn capabilities(&self) -> Capabilities {
        let mut c = Capabilities::base(CostModel::Metered);
        c.tools = true;
        c.json_schema = true;
        c.vision = true;
        c.max_context = Some(1_000_000);
        if !self.cfg.send_temperature {
            c.forbidden_params = vec!["temperature".into()];
        }
        c
    }
    async fn available(&self) -> Availability {
        let rb = match self.headers(self.client.get(join(&self.cfg.base_url, "v1/models"))) {
            Ok(rb) => rb,
            Err(_) => return Availability::down("no api key"),
        };
        match rb.timeout(std::time::Duration::from_secs(8)).send().await {
            Ok(r) if r.status().is_success() => {
                let models = r
                    .json::<Value>()
                    .await
                    .map(|v| super::openai_compatible::parse_model_list(&v))
                    .unwrap_or_default();
                Availability {
                    available: true,
                    status: "ready".into(),
                    models,
                }
            }
            Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
                Availability::down("key rejected")
            }
            Ok(r) => Availability::down(format!("http {}", r.status().as_u16())),
            Err(e) => Availability::down(format!("unreachable: {e}")),
        }
    }
    async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
        let body = self.body(&req);
        let url = join(&self.cfg.base_url, "v1/messages");
        let key = self
            .cfg
            .api_key
            .clone()
            .ok_or_else(|| HarnessError::NotConfigured("anthropic api key".into()))?;
        let resp = send_with_retry(&self.policy, || {
            self.client
                .post(&url)
                .header("x-api-key", &key)
                .header("anthropic-version", &self.cfg.anthropic_version)
                .json(&body)
        })
        .await?;
        Ok(decode_sse(
            resp.bytes_stream(),
            MessagesDecoder::new(UsageAcc::new(req.estimated_input_tokens())),
        ))
    }
}

pub(crate) struct MessagesDecoder {
    acc: UsageAcc,
    stop: Option<String>,
}

impl MessagesDecoder {
    pub(crate) fn new(acc: UsageAcc) -> Self {
        Self { acc, stop: None }
    }
}

/// Map an anthropic error object (`{"type": "...", "message": "..."}`) to a failure.
pub(crate) fn anthropic_error(err: &Value) -> ChatEvent {
    let ty = err.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let message = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or(ty)
        .to_string();
    let failure = match ty {
        "rate_limit_error" => FailureKind::RateLimited {
            retry_after_secs: None,
        },
        "overloaded_error" | "api_error" => FailureKind::Unavailable,
        "authentication_error" | "permission_error" => FailureKind::Auth,
        "invalid_request_error" | "not_found_error" | "request_too_large" => {
            FailureKind::InvalidRequest
        }
        _ => FailureKind::Other,
    };
    ChatEvent::Error { message, failure }
}

impl Decoder for MessagesDecoder {
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
            "message_start" => {
                if let Some(u) = v.pointer("/message/usage") {
                    let input = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0)
                        + u.get("cache_creation_input_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0)
                        + u.get("cache_read_input_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0);
                    self.acc.input = Some(input);
                    if let Some(o) = u.get("output_tokens").and_then(|x| x.as_u64()) {
                        self.acc.output = Some(o);
                    }
                }
                Vec::new()
            }
            "content_block_delta" => match v.pointer("/delta/text").and_then(|t| t.as_str()) {
                Some(t)
                    if v.pointer("/delta/type").and_then(|t| t.as_str()) == Some("text_delta") =>
                {
                    self.acc.text(t);
                    vec![ChatEvent::Token {
                        text: t.to_string(),
                    }]
                }
                _ => Vec::new(),
            },
            "message_delta" => {
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(|r| r.as_str()) {
                    self.stop = Some(r.to_string());
                }
                if let Some(o) = v.pointer("/usage/output_tokens").and_then(|x| x.as_u64()) {
                    self.acc.output = Some(o);
                }
                Vec::new()
            }
            "message_stop" => self.finish(),
            "error" => vec![anthropic_error(v.get("error").unwrap_or(&Value::Null))],
            _ => Vec::new(),
        }
    }

    fn finish(&mut self) -> Vec<ChatEvent> {
        vec![
            ChatEvent::Usage(self.acc.usage()),
            ChatEvent::Done {
                stop_reason: self.stop.take(),
            },
        ]
    }
}
