//! anamnesis-harness: bring-your-own-model.
//!
//! - [`Backend`]: one streaming chat trait over five adapters — openai-compatible
//!   (ollama, llama.cpp, lm studio, openrouter, openai keys), anthropic api key, gemini
//!   api key, "continue with chatgpt" (the app's own oauth token), and the user's own
//!   installed `claude` cli.
//! - [`Router`]: per-task routing (chat, label, extract, summarise) with hard policy
//!   rules (background work never on a subscription; no silent fallbacks), cooldowns
//!   and usage accounting.
//! - [`egress_check`]: the gate every non-local call passes; its record becomes an
//!   `EventKind::Egress` (counts and ids, never text).
//! - [`detect_all`]: startup capability detection.

pub mod adapters;
pub mod backend;
mod decode;
pub mod detect;
pub mod egress;
pub mod error;
mod http;
pub mod limits;
pub mod router;
pub mod sse;
pub mod types;
pub mod usage;

#[cfg(test)]
mod adapter_tests;
#[cfg(test)]
mod claude_cli_tests;
#[cfg(test)]
mod router_tests;
#[cfg(test)]
pub(crate) mod testutil;

pub use backend::Backend;
pub use detect::{Detection, DetectionReport, HarnessConfig, detect_all, detect_all_with};
pub use egress::{EgressRecord, egress_check, egress_check_with_consent};
pub use error::{HarnessError, Result};
pub use limits::{BackoffPolicy, CooldownState, Limits};
pub use router::{BackendInfo, CallContext, RoutedChat, Router, RoutingSettings};
pub use types::*;
pub use usage::{BackendUsage, Price, PriceTable, UsageLedger};
