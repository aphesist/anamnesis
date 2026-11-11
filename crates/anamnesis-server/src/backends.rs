//! Model backends: harness configuration from settings + the os keychain, detection,
//! and the "continue with chatgpt" sign-in flow.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anamnesis_embed::EmbedderSpec;
use anamnesis_harness::adapters::chatgpt::{Chatgpt, ChatgptConfig, ChatgptStatus};
use anamnesis_harness::adapters::claude_cli::McpOffer;
use anamnesis_harness::{HarnessConfig, RoutingSettings, detect_all_with};
use serde::Serialize;
use serde_json::{Value, json};

use crate::app::App;

pub use crate::secrets::KEYRING_SERVICE;
use crate::secrets::{KeyStorage, Secrets};

/// Read an api key: the os keychain entry `anamnesis/<account>`, then the 0600 file in
/// the data dir, then `env`.
pub fn api_key(secrets: &Secrets, account: &str, env: &[&str]) -> Option<String> {
    secrets.get(account, env).map(|(k, _)| k)
}

/// Store (or with `None`, delete) an api key: the os keychain, or the 0600 file in the
/// data dir when there is no usable keychain. Returns where it was stored.
pub fn set_api_key(
    secrets: &Secrets,
    account: &str,
    key: Option<&str>,
) -> anyhow::Result<Option<KeyStorage>> {
    match key {
        Some(k) => Ok(Some(secrets.set(account, k)?)),
        None => {
            secrets.delete(account)?;
            Ok(None)
        }
    }
}

/// Api key for a remote embedding endpoint.
pub fn embed_api_key(secrets: &Secrets, spec: &EmbedderSpec) -> Option<String> {
    match spec {
        EmbedderSpec::OpenaiCompatible { .. } if !spec.is_local() => {
            api_key(secrets, "embed", &["ANAMNESIS_EMBED_API_KEY"])
        }
        _ => None,
    }
}

fn env_for_provider(provider: &str) -> &'static [&'static str] {
    match provider {
        "openai" => &["OPENAI_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY"],
        _ => &[],
    }
}

/// The harness configuration from settings (`harness`, `routing`), with api keys filled
/// in from the keychain, the secrets file or the usual environment variables.
pub fn harness_config(app: &App, settings: &Value) -> HarnessConfig {
    harness_config_with_sources(app, settings).0
}

/// [`harness_config`], plus where each backend's key came from (by backend id).
pub fn harness_config_with_sources(
    app: &App,
    settings: &Value,
) -> (HarnessConfig, HashMap<String, KeyStorage>) {
    let mut sources = HashMap::new();
    let mut fill = |id: &str, env: &[&str]| -> Option<String> {
        let (k, from) = app.secrets.get(id, env)?;
        sources.insert(id.to_string(), from);
        Some(k)
    };
    let mut v = settings
        .get("harness")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !v.is_object() {
        v = json!({});
    }
    let data_dir = app.cfg.root.display().to_string();
    if let Some(c) = v.get_mut("chatgpt")
        && let Some(o) = c.as_object_mut()
    {
        o.entry("data_dir").or_insert(json!(data_dir));
    }
    let mut cfg: HarnessConfig = serde_json::from_value(v).unwrap_or_else(|e| {
        tracing::warn!("settings.harness is not valid, using defaults: {e}");
        HarnessConfig::default()
    });
    if let Some(r) = settings.get("routing").filter(|r| r.is_object()) {
        match serde_json::from_value::<RoutingSettings>(r.clone()) {
            Ok(r) => cfg.routing = r,
            Err(e) => tracing::warn!("settings.routing is not valid: {e}"),
        }
    }
    if let Some(a) = cfg.anthropic.as_mut()
        && a.api_key.is_none()
    {
        a.api_key = fill(&a.id, &["ANTHROPIC_API_KEY"]);
    }
    if let Some(g) = cfg.gemini.as_mut()
        && g.api_key.is_none()
    {
        g.api_key = fill(&g.id, &["GEMINI_API_KEY", "GOOGLE_API_KEY"]);
    }
    for c in cfg.openai_compatible.iter_mut() {
        if c.api_key.is_none() && !c.is_local() {
            c.api_key = fill(&c.id, env_for_provider(&c.provider));
        }
    }
    if cfg.claude_cli.work_dir.is_none() {
        let wd = app.cfg.root.join("claude-cli-work");
        let _ = std::fs::create_dir_all(&wd);
        cfg.claude_cli.work_dir = Some(wd);
    }
    (cfg, sources)
}

/// Arguments after the binary that start `anamnesis mcp` on the brain this server shows:
/// `--brain-dir <demo dir>` while the demo is active, `--data-dir <dir>` when the server
/// runs on a non-default data dir, and `--hash-embedder` when the server uses it (so
/// query vectors match the stored ones).
pub fn mcp_stdio_args(app: &App) -> Vec<String> {
    let mut args = vec!["mcp".to_string()];
    let root = &app.cfg.root;
    if app.is_demo() {
        args.push("--brain-dir".into());
        args.push(app.cfg.demo_brain_dir().display().to_string());
    } else if *root != crate::app::default_root() {
        match root.parent() {
            // the root is `<data dir>/anamnesis`
            Some(parent) if root.file_name().is_some_and(|n| n == "anamnesis") => {
                args.push("--data-dir".into());
                args.push(parent.display().to_string());
            }
            _ => {
                args.push("--brain-dir".into());
                args.push(root.display().to_string());
            }
        }
    }
    if matches!(app.cfg.embedder_override, Some(EmbedderSpec::Hash { .. })) {
        args.push("--hash-embedder".into());
    }
    args
}

/// The brain's own mcp server offered to the installed claude cli: stdio, so no token
/// ever appears on a command line.
pub fn mcp_offer(app: &App) -> Option<McpOffer> {
    let exe = std::env::current_exe().ok()?;
    let config = json!({
        "mcpServers": {
            "anamnesis": {
                "type": "stdio",
                "command": exe.display().to_string(),
                "args": mcp_stdio_args(app),
            }
        }
    });
    Some(McpOffer {
        config: config.to_string(),
        server_name: "anamnesis".into(),
    })
}

/// (Re)build the router from settings and run capability detection.
pub async fn detect(app: &Arc<App>) {
    let brain = app.brain();
    let settings = match brain.db(|s| Ok(s.settings()?)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("reading settings for backend detection: {e:#}");
            return;
        }
    };
    let cfg = {
        let app = app.clone();
        let settings = settings.clone();
        // keychain access can block
        tokio::task::spawn_blocking(move || harness_config_with_sources(&app, &settings))
            .await
            .unwrap_or_default()
    };
    let (cfg, key_sources) = cfg;
    *app.key_sources.write().unwrap() = key_sources;
    let det = detect_all_with(&cfg, mcp_offer(app)).await;
    tracing::info!(
        "model backends: {}",
        det.report
            .backends
            .iter()
            .map(|b| format!("{} ({})", b.id, b.status))
            .collect::<Vec<_>>()
            .join(", ")
    );
    app.set_router(Some(Arc::new(det.router)));
}

/// State of the "continue with chatgpt" sign-in.
#[derive(Default)]
pub struct ChatgptSignIn {
    state: Mutex<SignInState>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SignInState {
    pub in_progress: bool,
    pub last_error: Option<String>,
}

impl ChatgptSignIn {
    pub fn state(&self) -> SignInState {
        self.state.lock().unwrap().clone()
    }
    fn set(&self, s: SignInState) {
        *self.state.lock().unwrap() = s;
    }
}

/// The chatgpt adapter configured from settings (a fresh instance: it shares the token
/// store with the router's instance).
pub fn chatgpt_client(app: &App, settings: &Value) -> Chatgpt {
    let cfg = harness_config(app, settings)
        .chatgpt
        .unwrap_or_else(|| ChatgptConfig::new(app.cfg.root.clone()));
    Chatgpt::new(cfg, Default::default())
}

#[derive(Debug, Serialize)]
pub struct ChatgptInfo {
    pub button_label: &'static str,
    pub notice: &'static str,
    pub usage_settings_url: &'static str,
    pub status: ChatgptStatus,
    pub sign_in: SignInState,
}

pub async fn chatgpt_status(app: &Arc<App>) -> anyhow::Result<ChatgptInfo> {
    let settings = app.brain().db(|s| Ok(s.settings()?)).await?;
    let c = chatgpt_client(app, &settings);
    Ok(ChatgptInfo {
        button_label: anamnesis_harness::adapters::chatgpt::BUTTON_LABEL,
        notice: anamnesis_harness::adapters::chatgpt::NOTICE,
        usage_settings_url: anamnesis_harness::adapters::chatgpt::USAGE_SETTINGS_URL,
        status: c.status().await,
        sign_in: app.chatgpt.state(),
    })
}

/// Start the sign-in: returns the url the ui opens; completion runs in the background
/// (the loopback listener waits for the browser), then backends are re-detected.
pub async fn chatgpt_begin(app: &Arc<App>) -> anyhow::Result<String> {
    if app.chatgpt.state().in_progress {
        anyhow::bail!("a sign-in is already waiting for the browser");
    }
    let settings = app.brain().db(|s| Ok(s.settings()?)).await?;
    let client = chatgpt_client(app, &settings);
    let pending = client.begin_sign_in(false).await?;
    let url = pending.authorize_url.clone();
    app.chatgpt.set(SignInState {
        in_progress: true,
        last_error: None,
    });
    let app2 = app.clone();
    tokio::spawn(async move {
        let res = client.complete_sign_in(pending).await;
        match res {
            Ok(_) => {
                app2.chatgpt.set(SignInState::default());
                // enable the backend if it was not configured yet
                let brain = app2.brain();
                let _ = brain
                    .db(|s| {
                        let mut h = s.get_setting("harness")?.unwrap_or_else(|| json!({}));
                        if !h.is_object() {
                            h = json!({});
                        }
                        let c = h
                            .as_object_mut()
                            .unwrap()
                            .entry("chatgpt")
                            .or_insert_with(|| json!({}));
                        if !c.is_object() {
                            *c = json!({});
                        }
                        c["enabled"] = json!(true);
                        s.set_setting("harness", h)?;
                        Ok(())
                    })
                    .await;
                detect(&app2).await;
            }
            Err(e) => app2.chatgpt.set(SignInState {
                in_progress: false,
                last_error: Some(e.to_string()),
            }),
        }
    });
    Ok(url)
}

pub async fn chatgpt_sign_out(app: &Arc<App>) -> anyhow::Result<()> {
    let settings = app.brain().db(|s| Ok(s.settings()?)).await?;
    chatgpt_client(app, &settings).sign_out().await?;
    detect(app).await;
    Ok(())
}

/// Re-run detection on a background task (when detection is enabled).
pub fn detect_in_background(app: &Arc<App>) {
    if !app.cfg.detect_backends {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move { detect(&app).await });
}
