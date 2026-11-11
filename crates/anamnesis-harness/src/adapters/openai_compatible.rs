//! OpenAI-compatible chat completions: Ollama, llama.cpp `llama-server`, LM Studio,
//! OpenRouter, the OpenAI api with a key.
//!
//! `POST {base_url}/chat/completions` with `stream: true`; SSE chunks carry
//! `choices[0].delta.content`, a final chunk may carry `usage` (requested with
//! `stream_options.include_usage`), and the stream ends with `data: [DONE]`.
//! Reference: https://platform.openai.com/docs/api-reference/chat/create

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::backend::Backend;
use crate::decode::{Decoder, UsageAcc, decode_sse};
use crate::error::{HarnessError, Result};
use crate::http::{self, is_local_url, join, send_with_retry};
use crate::limits::BackoffPolicy;
use crate::sse::SseEvent;
use crate::types::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCompatibleConfig {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Provider name for egress records: "ollama", "llama.cpp", "lm studio", "openrouter", "openai"…
    pub provider: String,
    /// Base url including the version segment, e.g. `http://127.0.0.1:11434/v1`,
    /// `https://openrouter.ai/api/v1`, `https://api.openai.com/v1`.
    pub base_url: String,
    /// Bearer key, if the server needs one. Never written back out by serde.
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    pub model: String,
    /// Force local / remote. Default: loopback hosts are local, everything else is not.
    #[serde(default)]
    pub local: Option<bool>,
    #[serde(default)]
    pub max_context: Option<u32>,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub tools: bool,
    #[serde(default = "yes")]
    pub json_schema: bool,
    /// Ask for a final usage chunk (`stream_options.include_usage`). Some older servers reject it.
    #[serde(default = "yes")]
    pub stream_usage: bool,
    /// Extra headers, e.g. OpenRouter's `HTTP-Referer` / `X-Title`.
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

impl OpenAiCompatibleConfig {
    pub fn ollama(model: impl Into<String>) -> Self {
        Self {
            id: "ollama".into(),
            label: Some("ollama (local)".into()),
            provider: "ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model: model.into(),
            local: Some(true),
            max_context: None,
            vision: false,
            tools: false,
            json_schema: true,
            stream_usage: true,
            extra_headers: BTreeMap::new(),
        }
    }

    pub fn is_local(&self) -> bool {
        self.local.unwrap_or_else(|| is_local_url(&self.base_url))
    }
}

pub struct OpenAiCompatible {
    cfg: OpenAiCompatibleConfig,
    client: reqwest::Client,
    policy: BackoffPolicy,
}

impl OpenAiCompatible {
    pub fn new(cfg: OpenAiCompatibleConfig, policy: BackoffPolicy) -> Self {
        // loopback servers are never reached through a system proxy
        let client = if is_local_url(&cfg.base_url) {
            http::local_client()
        } else {
            http::client(None)
        };
        Self {
            cfg,
            client,
            policy,
        }
    }

    pub fn config(&self) -> &OpenAiCompatibleConfig {
        &self.cfg
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut rb = rb;
        if let Some(k) = &self.cfg.api_key {
            rb = rb.bearer_auth(k);
        }
        for (k, v) in &self.cfg.extra_headers {
            rb = rb.header(k, v);
        }
        rb
    }

    pub(crate) fn body(&self, req: &ChatRequest) -> Value {
        let mut messages = Vec::new();
        if let Some(s) = &req.system {
            messages.push(json!({"role": "system", "content": s}));
        }
        for m in &req.messages {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            messages.push(json!({"role": role, "content": m.content}));
        }
        let mut body = json!({
            "model": req.model.clone().unwrap_or_else(|| self.cfg.model.clone()),
            "messages": messages,
            "stream": true,
        });
        if self.cfg.stream_usage {
            body["stream_options"] = json!({"include_usage": true});
        }
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(n) = req.max_tokens {
            body["max_tokens"] = json!(n);
        }
        if let (Some(schema), true) = (&req.json_schema, self.cfg.json_schema) {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": "output", "schema": schema, "strict": true}
            });
        }
        body
    }

    /// Model ids from `GET {base}/models`.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let resp = self
            .auth(self.client.get(join(&self.cfg.base_url, "models")))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| HarnessError::Unavailable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(http::error_from_response(resp).await);
        }
        let v: Value = resp.json().await?;
        Ok(parse_model_list(&v))
    }
}

pub(crate) fn parse_model_list(v: &Value) -> Vec<String> {
    let arr = v
        .get("data")
        .or_else(|| v.get("models"))
        .and_then(|d| d.as_array());
    arr.map(|a| {
        a.iter()
            .filter_map(|m| {
                m.get("id")
                    .or_else(|| m.get("name"))
                    .or_else(|| m.get("model"))
            })
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect()
    })
    .unwrap_or_default()
}

#[async_trait]
impl Backend for OpenAiCompatible {
    fn id(&self) -> &str {
        &self.cfg.id
    }
    fn kind(&self) -> BackendKind {
        BackendKind::OpenaiCompatible
    }
    fn label(&self) -> String {
        self.cfg
            .label
            .clone()
            .unwrap_or_else(|| self.cfg.provider.clone())
    }
    fn default_model(&self) -> &str {
        &self.cfg.model
    }
    fn provider(&self) -> &str {
        &self.cfg.provider
    }
    fn capabilities(&self) -> Capabilities {
        let mut c = Capabilities::base(if self.cfg.is_local() {
            CostModel::FreeLocal
        } else {
            CostModel::Metered
        });
        c.tools = self.cfg.tools;
        c.json_schema = self.cfg.json_schema;
        c.max_context = self.cfg.max_context;
        c.vision = self.cfg.vision;
        c
    }
    fn is_local(&self) -> bool {
        self.cfg.is_local()
    }
    fn leaves_machine(&self) -> bool {
        // marked remote: assume it forwards elsewhere even on a loopback address
        !self.cfg.is_local() || !is_local_url(&self.cfg.base_url)
    }
    async fn available(&self) -> Availability {
        match self.list_models().await {
            Ok(models) => Availability {
                available: true,
                status: "ready".into(),
                models,
            },
            Err(HarnessError::Auth(_)) => Availability::down("key rejected"),
            Err(e) => Availability::down(format!("unreachable: {e}")),
        }
    }
    async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
        let body = self.body(&req);
        let url = join(&self.cfg.base_url, "chat/completions");
        let resp = send_with_retry(&self.policy, || {
            self.auth(self.client.post(&url)).json(&body)
        })
        .await?;
        let acc = UsageAcc::new(req.estimated_input_tokens());
        Ok(decode_sse(
            resp.bytes_stream(),
            ChatCompletionsDecoder::new(acc),
        ))
    }
}

pub(crate) struct ChatCompletionsDecoder {
    acc: UsageAcc,
    stop: Option<String>,
}

impl ChatCompletionsDecoder {
    pub(crate) fn new(acc: UsageAcc) -> Self {
        Self { acc, stop: None }
    }
}

impl Decoder for ChatCompletionsDecoder {
    fn event(&mut self, ev: &SseEvent) -> Vec<ChatEvent> {
        if ev.data.trim() == "[DONE]" {
            return self.finish();
        }
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            return Vec::new();
        };
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("provider error")
                .to_string();
            let failure = match err
                .get("code")
                .and_then(|c| c.as_i64().or_else(|| c.as_str()?.parse().ok()))
            {
                Some(429) => FailureKind::RateLimited {
                    retry_after_secs: None,
                },
                Some(401 | 403) => FailureKind::Auth,
                _ => FailureKind::Other,
            };
            return vec![ChatEvent::Error {
                message: msg,
                failure,
            }];
        }
        let mut out = Vec::new();
        if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
            if let Some(t) = choice.pointer("/delta/content").and_then(|t| t.as_str())
                && !t.is_empty()
            {
                self.acc.text(t);
                out.push(ChatEvent::Token {
                    text: t.to_string(),
                });
            }
            if let Some(r) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                self.stop = Some(r.to_string());
            }
        }
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            self.acc.input = u.get("prompt_tokens").and_then(|x| x.as_u64());
            self.acc.output = u.get("completion_tokens").and_then(|x| x.as_u64());
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
