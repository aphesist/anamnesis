//! Configuration and startup capability detection.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::anthropic::{Anthropic, AnthropicConfig};
use crate::adapters::chatgpt::{Chatgpt, ChatgptConfig};
use crate::adapters::claude_cli::{ClaudeCli, ClaudeCliConfig, McpOffer};
use crate::adapters::gemini::{Gemini, GeminiConfig};
use crate::adapters::openai_compatible::{OpenAiCompatible, OpenAiCompatibleConfig};
use crate::backend::Backend;
use crate::http::{self, join};
use crate::limits::{BackoffPolicy, Limits};
use crate::router::{BackendInfo, Router, RoutingSettings};
use crate::usage::PriceTable;

/// Everything the harness needs from settings. Secrets (api keys) are filled in by the
/// server from the os keychain before calling `detect_all`; serde never writes them back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessConfig {
    #[serde(default)]
    pub openai_compatible: Vec<OpenAiCompatibleConfig>,
    #[serde(default)]
    pub anthropic: Option<AnthropicConfig>,
    #[serde(default)]
    pub gemini: Option<GeminiConfig>,
    #[serde(default)]
    pub chatgpt: Option<ChatgptConfig>,
    #[serde(default)]
    pub claude_cli: ClaudeCliConfig,
    #[serde(default)]
    pub routing: RoutingSettings,
    #[serde(default)]
    pub prices: PriceTable,
    #[serde(default)]
    pub backoff: BackoffPolicy,
    /// Probe the usual local servers (ollama :11434, llama.cpp :8080, lm studio :1234)
    /// and add any that answer and are not configured yet.
    #[serde(default = "yes")]
    pub probe_local: bool,
}

fn yes() -> bool {
    true
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            openai_compatible: Vec::new(),
            anthropic: None,
            gemini: None,
            chatgpt: None,
            claude_cli: ClaudeCliConfig::default(),
            routing: RoutingSettings::default(),
            prices: PriceTable::default(),
            backoff: BackoffPolicy::default(),
            probe_local: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaudeCliDetection {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub enabled: bool,
    pub disabled_by_policy: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionReport {
    pub backends: Vec<BackendInfo>,
    /// Models a reachable ollama server reports (`/api/tags`).
    pub ollama_models: Vec<String>,
    /// Local servers found by probing that were not configured.
    pub discovered_local: Vec<String>,
    pub claude_cli: ClaudeCliDetection,
}

pub struct Detection {
    pub router: Router,
    pub report: DetectionReport,
}

/// Ollama's native model list at `{root}/api/tags`.
pub async fn ollama_models(client: &reqwest::Client, root: &str) -> Option<Vec<String>> {
    let r = client.get(join(root, "api/tags")).timeout(Duration::from_millis(1500)).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: Value = r.json().await.ok()?;
    Some(
        v.get("models")?
            .as_array()?
            .iter()
            .filter_map(|m| m.get("name").or_else(|| m.get("model")).and_then(|n| n.as_str()).map(str::to_string))
            .collect(),
    )
}

async fn probe_models(client: &reqwest::Client, base: &str) -> Option<Vec<String>> {
    let r = client.get(join(base, "models")).timeout(Duration::from_millis(1500)).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: Value = r.json().await.ok()?;
    Some(crate::adapters::openai_compatible::parse_model_list(&v))
}

fn root_of(base_url: &str) -> String {
    base_url.trim_end_matches('/').trim_end_matches("/v1").to_string()
}

/// Build every configured backend, probe local servers, run health checks.
pub async fn detect_all(config: &HarnessConfig) -> Detection {
    detect_all_with(config, None).await
}

/// As `detect_all`, offering the brain's mcp server to the installed claude cli.
pub async fn detect_all_with(config: &HarnessConfig, mcp: Option<McpOffer>) -> Detection {
    let client = http::client(None);
    let policy = config.backoff;
    let mut compat = config.openai_compatible.clone();
    let mut discovered = Vec::new();
    let mut ollama_list = Vec::new();

    if config.probe_local {
        let known = |base: &str| compat.iter().any(|c| root_of(&c.base_url) == root_of(base));
        let candidates = [
            ("ollama", "ollama (local)", "http://127.0.0.1:11434/v1"),
            ("llama.cpp", "llama.cpp (local)", "http://127.0.0.1:8080/v1"),
            ("lm studio", "lm studio (local)", "http://127.0.0.1:1234/v1"),
        ];
        let todo: Vec<_> = candidates.iter().filter(|(_, _, b)| !known(b)).collect();
        let probes = todo.iter().map(|(p, l, b)| {
            let client = client.clone();
            async move { (*p, *l, *b, probe_models(&client, b).await) }
        });
        for (provider, label, base, models) in futures_util::future::join_all(probes).await {
            let Some(models) = models else { continue };
            let Some(first) = models.first().cloned() else { continue };
            discovered.push(provider.to_string());
            let mut c = OpenAiCompatibleConfig::ollama(first);
            c.id = provider.replace(' ', "-").replace('.', "");
            c.provider = provider.to_string();
            c.label = Some(label.to_string());
            c.base_url = base.to_string();
            compat.push(c);
        }
    }
    for c in compat.iter().filter(|c| c.provider == "ollama") {
        if let Some(m) = ollama_models(&client, &root_of(&c.base_url)).await {
            ollama_list = m;
            break;
        }
    }

    let mut backends: Vec<Arc<dyn Backend>> = Vec::new();
    for c in compat {
        backends.push(Arc::new(OpenAiCompatible::new(c, policy)));
    }
    if let Some(a) = &config.anthropic {
        backends.push(Arc::new(Anthropic::new(a.clone(), policy)));
    }
    if let Some(g) = &config.gemini {
        backends.push(Arc::new(Gemini::new(g.clone(), policy)));
    }
    if let Some(c) = &config.chatgpt {
        backends.push(Arc::new(Chatgpt::new(c.clone(), policy)));
    }
    let cli = ClaudeCli::new(config.claude_cli.clone()).with_mcp(mcp);
    let cli_path = cli.locate();
    let cli_version = if cli_path.is_some() && !config.claude_cli.disabled_by_policy { cli.version().await } else { None };
    let claude_cli = ClaudeCliDetection {
        found: cli_path.is_some(),
        path: cli_path.map(|p| p.display().to_string()),
        version: cli_version,
        enabled: config.claude_cli.enabled,
        disabled_by_policy: config.claude_cli.disabled_by_policy,
    };
    if claude_cli.found || config.claude_cli.enabled {
        backends.push(Arc::new(cli));
    }

    let router = Router::new(backends, config.routing.clone(), Arc::new(Limits::new(policy)), config.prices.clone());
    router.refresh().await;
    let report =
        DetectionReport { backends: router.backends_info(), ollama_models: ollama_list, discovered_local: discovered, claude_cli };
    Detection { router, report }
}
