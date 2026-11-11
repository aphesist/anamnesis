use std::time::Duration;

use anamnesis_core::Id;

use crate::types::FailureKind;

pub type Result<T, E = HarnessError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("backend not configured: {0}")]
    NotConfigured(String),
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("rate limited{}", .retry_after.map(|d| format!(", retry after {}s", d.as_secs())).unwrap_or_default())]
    RateLimited { retry_after: Option<Duration> },
    #[error("plan usage limit reached: {0}")]
    UsageLimit(String),
    #[error("cooling down until {until}")]
    CoolingDown { until: chrono::DateTime<chrono::Utc> },
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("request rejected: {0}")]
    InvalidRequest(String),
    #[error("http {status}: {body}")]
    Http { status: u16, body: String },
    #[error("transport: {0}")]
    Transport(String),
    #[error("routing refused: {0}")]
    Routing(String),
    #[error("egress refused: {} item(s) are local-only", .0.len())]
    EgressLocalOnly(Vec<Id>),
    #[error("egress needs consent for {} item(s)", .0.len())]
    EgressNeedsConsent(Vec<Id>),
    #[error("disabled by policy: {0}")]
    DisabledByPolicy(String),
    #[error("oauth: {0}")]
    OAuth(String),
    #[error("token store: {0}")]
    TokenStore(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl HarnessError {
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            HarnessError::RateLimited { retry_after } => FailureKind::RateLimited {
                retry_after_secs: retry_after.map(|d| d.as_secs()),
            },
            HarnessError::UsageLimit(_) => FailureKind::UsageLimit { resets_at: None },
            HarnessError::Auth(_) | HarnessError::OAuth(_) => FailureKind::Auth,
            HarnessError::InvalidRequest(_) => FailureKind::InvalidRequest,
            HarnessError::Unavailable(_) | HarnessError::Transport(_) => FailureKind::Unavailable,
            HarnessError::Http { status, .. } if *status >= 500 => FailureKind::Unavailable,
            _ => FailureKind::Other,
        }
    }
}

impl From<reqwest::Error> for HarnessError {
    fn from(e: reqwest::Error) -> Self {
        HarnessError::Transport(e.to_string())
    }
}
