//! Gemini api with an AI Studio key.
//!
//! `POST {base_url}/v1beta/models/{model}:streamGenerateContent?alt=sse` with
//! `x-goog-api-key`. Each SSE `data:` is a `GenerateContentResponse` with
//! `candidates[0].content.parts[].text`, `finishReason`, and `usageMetadata`.
//! Reference: https://ai.google.dev/api/generate-content#method:-models.streamgeneratecontent
//!
//! Only api keys: consumer google logins (gemini cli / antigravity oauth) are never used.

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeminiConfig {
    #[serde(default = "default_id")]
    pub id: String,
    #[serde(default = "default_base")]
    pub base_url: String,
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    /// Model id, e.g. "gemini-flash-latest". Configurable; no default is assumed to exist forever.
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub label_model: Option<String>,
}

fn default_id() -> String {
    "gemini".into()
}
fn default_base() -> String {
    "https://generativelanguage.googleapis.com".into()
}
fn default_model() -> String {
    "gemini-flash-latest".into()
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            id: default_id(),
            base_url: default_base(),
            api_key: None,
            model: default_model(),
            label_model: None,
        }
    }
}

pub struct Gemini {
    cfg: GeminiConfig,
    client: reqwest::Client,
    policy: BackoffPolicy,
}

impl Gemini {
    pub fn new(cfg: GeminiConfig, policy: BackoffPolicy) -> Self {
        Self {
            cfg,
            client: http::client(None),
            policy,
        }
    }

    fn key(&self) -> Result<&str> {
        self.cfg
            .api_key
            .as_deref()
            .ok_or_else(|| HarnessError::NotConfigured("gemini api key".into()))
    }

    pub(crate) fn body(&self, req: &ChatRequest) -> Value {
        let contents: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "model",
                };
                json!({"role": role, "parts": [{"text": m.content}]})
            })
            .collect();
        let mut body = json!({ "contents": contents });
        if let Some(s) = &req.system {
            body["systemInstruction"] = json!({"parts": [{"text": s}]});
        }
        let mut gen_cfg = serde_json::Map::new();
        if let Some(t) = req.temperature {
            gen_cfg.insert("temperature".into(), json!(t));
        }
        if let Some(n) = req.max_tokens {
            gen_cfg.insert("maxOutputTokens".into(), json!(n));
        }
        if let Some(schema) = &req.json_schema {
            gen_cfg.insert("responseMimeType".into(), json!("application/json"));
            gen_cfg.insert("responseJsonSchema".into(), schema.clone());
        }
        if !gen_cfg.is_empty() {
            body["generationConfig"] = Value::Object(gen_cfg);
        }
        body
    }
}

#[async_trait]
impl Backend for Gemini {
    fn id(&self) -> &str {
        &self.cfg.id
    }
    fn kind(&self) -> BackendKind {
        BackendKind::Gemini
    }
    fn label(&self) -> String {
        "gemini (api key)".into()
    }
    fn default_model(&self) -> &str {
        &self.cfg.model
    }
    fn model_for(&self, task: Task) -> String {
        match (&self.cfg.label_model, task.is_background()) {
            (Some(m), true) => m.clone(),
            _ => self.cfg.model.clone(),
        }
    }
    fn provider(&self) -> &str {
        "google"
    }
    fn capabilities(&self) -> Capabilities {
        let mut c = Capabilities::base(CostModel::Metered);
        c.tools = true;
        c.json_schema = true;
        c.vision = true;
        c.notice = Some(
            "unpaid ai studio keys fall under google's unpaid-service terms, where prompts may be used to improve products."
                .into(),
        );
        c
    }
    async fn available(&self) -> Availability {
        let Ok(key) = self.key() else {
            return Availability::down("no api key");
        };
        let url = join(&self.cfg.base_url, "v1beta/models");
        match self
            .client
            .get(url)
            .header("x-goog-api-key", key)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                let models = r
                    .json::<Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("models").and_then(|m| m.as_array()).cloned())
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                            .map(|n| n.trim_start_matches("models/").to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                Availability {
                    available: true,
                    status: "ready".into(),
                    models,
                }
            }
            Ok(r) if matches!(r.status().as_u16(), 400 | 401 | 403) => {
                Availability::down("key rejected")
            }
            Ok(r) => Availability::down(format!("http {}", r.status().as_u16())),
            Err(e) => Availability::down(format!("unreachable: {e}")),
        }
    }
    async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
        let key = self.key()?.to_string();
        let model = req.model.clone().unwrap_or_else(|| self.cfg.model.clone());
        let url = join(
            &self.cfg.base_url,
            &format!("v1beta/models/{model}:streamGenerateContent?alt=sse"),
        );
        let body = self.body(&req);
        let resp = send_with_retry(&self.policy, || {
            self.client
                .post(&url)
                .header("x-goog-api-key", &key)
                .json(&body)
        })
        .await?;
        Ok(decode_sse(
            resp.bytes_stream(),
            GeminiDecoder::new(UsageAcc::new(req.estimated_input_tokens())),
        ))
    }
}

pub(crate) struct GeminiDecoder {
    acc: UsageAcc,
    stop: Option<String>,
}

impl GeminiDecoder {
    pub(crate) fn new(acc: UsageAcc) -> Self {
        Self { acc, stop: None }
    }
}

impl Decoder for GeminiDecoder {
    fn event(&mut self, ev: &SseEvent) -> Vec<ChatEvent> {
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            return Vec::new();
        };
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
            let failure = match code {
                429 => FailureKind::RateLimited {
                    retry_after_secs: None,
                },
                401 | 403 => FailureKind::Auth,
                500..=599 => FailureKind::Unavailable,
                _ => FailureKind::Other,
            };
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("gemini error")
                .to_string();
            return vec![ChatEvent::Error { message, failure }];
        }
        let mut out = Vec::new();
        if let Some(c) = v.pointer("/candidates/0") {
            if let Some(parts) = c.pointer("/content/parts").and_then(|p| p.as_array()) {
                for p in parts {
                    // skip thought summaries; only answer text is streamed
                    if p.get("thought").and_then(|t| t.as_bool()) == Some(true) {
                        continue;
                    }
                    if let Some(t) = p.get("text").and_then(|t| t.as_str())
                        && !t.is_empty()
                    {
                        self.acc.text(t);
                        out.push(ChatEvent::Token {
                            text: t.to_string(),
                        });
                    }
                }
            }
            if let Some(r) = c.get("finishReason").and_then(|r| r.as_str()) {
                self.stop = Some(r.to_string());
            }
        }
        if let Some(u) = v.get("usageMetadata") {
            if let Some(i) = u.get("promptTokenCount").and_then(|x| x.as_u64()) {
                self.acc.input = Some(i);
            }
            let cand = u.get("candidatesTokenCount").and_then(|x| x.as_u64());
            let thoughts = u
                .get("thoughtsTokenCount")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            if let Some(o) = cand {
                self.acc.output = Some(o + thoughts);
            }
        }
        out
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
