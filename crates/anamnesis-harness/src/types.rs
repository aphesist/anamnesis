//! Request, event and capability types shared by every backend.

use futures_util::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// Who said a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: text.into(),
        }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: text.into(),
        }
    }
}

/// One chat request. Fields a backend cannot honour are dropped by that backend
/// (e.g. `temperature` on the chatgpt plan flow), never sent and never an error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    /// JSON schema the reply must follow, where the backend supports it.
    pub json_schema: Option<serde_json::Value>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Override the backend's configured model for this request.
    pub model: Option<String>,
}

impl ChatRequest {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            ..Default::default()
        }
    }
    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }
    /// Characters that would leave the machine (system + all message bodies).
    pub fn chars(&self) -> u64 {
        let sys = self
            .system
            .as_deref()
            .map(|s| s.chars().count())
            .unwrap_or(0);
        let msgs: usize = self
            .messages
            .iter()
            .map(|m| m.content.chars().count())
            .sum();
        (sys + msgs) as u64
    }
    /// Very rough token estimate (≈4 chars per token) for backends that report no usage.
    pub fn estimated_input_tokens(&self) -> u64 {
        self.chars().div_ceil(4)
    }
}

/// Token usage for one call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    /// True when the numbers are an estimate (plan / cli usage, or a server that reported nothing).
    pub estimated: bool,
    /// Cost the provider itself reported (the claude cli reports an api-equivalent figure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_cost_usd: Option<f64>,
}

/// Why a call failed, coarse enough for the router and the ui.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureKind {
    /// HTTP 429 or equivalent; `retry_after_secs` when the server said so.
    RateLimited { retry_after_secs: Option<u64> },
    /// A subscription plan's usage limit (claude plan window, chatgpt weekly cap).
    /// `resets_at` only when the provider stated it; never inferred.
    UsageLimit {
        resets_at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Credentials missing, expired or rejected.
    Auth,
    /// The request itself was rejected (bad parameter, unsupported feature).
    InvalidRequest,
    /// Temporary server-side trouble (5xx, overloaded).
    Unavailable,
    /// Anything else.
    Other,
}

/// A streamed chat event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Token {
        text: String,
    },
    Usage(Usage),
    Done {
        stop_reason: Option<String>,
    },
    Error {
        message: String,
        failure: FailureKind,
    },
}

/// The stream a backend returns. It ends after `Done` or `Error`.
pub type ChatStream = Pin<Box<dyn Stream<Item = ChatEvent> + Send>>;

/// Which adapter a backend is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    OpenaiCompatible,
    Anthropic,
    Gemini,
    Chatgpt,
    ClaudeCli,
}

/// How a backend is paid for. Background work is never routed to `Subscription`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostModel {
    /// Runs on this machine (or the user's own server); no money changes hands.
    FreeLocal,
    /// Pay per token with the user's api key.
    Metered,
    /// Draws on a consumer plan (chatgpt plan, claude plan).
    Subscription,
}

/// Provider terms for this route, as shown in the ui.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terms {
    Allowed,
    Grey,
}

/// Maturity of the adapter itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maturity {
    Stable,
    Experimental,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub streaming: bool,
    pub tools: bool,
    pub json_schema: bool,
    pub max_context: Option<u32>,
    pub vision: bool,
    pub cost_model: CostModel,
    /// May the router send background work (label, extract, summarise) here?
    pub background_allowed: bool,
    /// Request fields this backend strips before sending.
    pub forbidden_params: Vec<String>,
    pub terms: Terms,
    pub maturity: Maturity,
    /// Shown next to the backend in the provider list, lowercase.
    pub notice: Option<String>,
}

impl Capabilities {
    pub(crate) fn base(cost_model: CostModel) -> Self {
        Self {
            streaming: true,
            tools: false,
            json_schema: false,
            max_context: None,
            vision: false,
            cost_model,
            background_allowed: cost_model != CostModel::Subscription,
            forbidden_params: Vec::new(),
            terms: Terms::Allowed,
            maturity: Maturity::Stable,
            notice: None,
        }
    }
}

/// Result of a health check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Availability {
    pub available: bool,
    /// Short lowercase status for the ui ("ready", "not signed in", "unreachable: …").
    pub status: String,
    /// Models the backend reports, when it lists them (ollama, openai-compatible, chatgpt).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

impl Availability {
    pub fn ready() -> Self {
        Self {
            available: true,
            status: "ready".into(),
            models: Vec::new(),
        }
    }
    pub fn down(status: impl Into<String>) -> Self {
        Self {
            available: false,
            status: status.into(),
            models: Vec::new(),
        }
    }
}

/// What a model call is for. Everything except `Chat` is background work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    Chat,
    Label,
    Extract,
    Summarise,
}

impl Task {
    pub const ALL: [Task; 4] = [Task::Chat, Task::Label, Task::Extract, Task::Summarise];

    /// Background tasks run without the user asking for this specific call.
    pub fn is_background(self) -> bool {
        !matches!(self, Task::Chat)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Task::Chat => "chat",
            Task::Label => "label",
            Task::Extract => "extract",
            Task::Summarise => "summarise",
        }
    }
}
