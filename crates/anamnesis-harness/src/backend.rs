//! The `Backend` trait every adapter implements.

use async_trait::async_trait;

use crate::error::Result;
use crate::types::{
    Availability, BackendKind, Capabilities, ChatRequest, ChatStream, CostModel, Task,
};

#[async_trait]
pub trait Backend: Send + Sync {
    /// Stable id from settings (e.g. "ollama", "anthropic", "claude-cli").
    fn id(&self) -> &str;
    fn kind(&self) -> BackendKind;
    /// Lowercase label for the provider list.
    fn label(&self) -> String;
    /// The model used when a request names none.
    fn default_model(&self) -> &str;
    /// The model this backend uses for a task (e.g. a cheaper model for labelling).
    fn model_for(&self, _task: Task) -> String {
        self.default_model().to_string()
    }
    /// Provider name recorded in egress events (e.g. "anthropic", "openrouter", "ollama").
    fn provider(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    /// True when calls never leave the machine (or the user's own lan server). Local
    /// backends skip the egress gate.
    fn is_local(&self) -> bool {
        self.capabilities().cost_model == CostModel::FreeLocal
    }
    /// Health check: configured, reachable, signed in.
    async fn available(&self) -> Availability;
    /// Start a streamed chat. Errors before the first byte are returned here; errors after
    /// the stream opened arrive as `ChatEvent::Error`.
    async fn chat(&self, req: ChatRequest) -> Result<ChatStream>;
}
