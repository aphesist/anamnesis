//! Model backends: harness configuration from settings + the os keychain, detection,
//! and the "continue with chatgpt" sign-in flow.

use std::sync::{Arc, Mutex};

use anamnesis_embed::EmbedderSpec;
use anamnesis_harness::adapters::chatgpt::{Chatgpt, ChatgptConfig, ChatgptStatus};
use anamnesis_harness::adapters::claude_cli::McpOffer;
use anamnesis_harness::{HarnessConfig, RoutingSettings, detect_all_with};
use serde::Serialize;
use serde_json::{Value, json};

use crate::app::App;

/// Keychain service name for api keys.
pub const KEYRING_SERVICE: &str = "anamnesis";

/// Read an api key: the os keychain entry `anamnesis/<account>` first, then `env`.
pub fn api_key(account: &str, env: &[&str]) -> Option<String> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, account)
        && let Ok(k) = entry.get_password()
        && !k.trim().is_empty()
    {
        return Some(k.trim().to_string());
    }
    env.iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|k| !k.trim().is_empty())
}

/// Store (or with `None`, delete) an api key in the os keychain.
pub fn set_api_key(account: &str, key: Option<&str>) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)?;
    match key {
        Some(k) => entry.set_password(k)?,
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}

/// Api key for a remote embedding endpoint.
pub fn embed_api_key(spec: &EmbedderSpec) -> Option<String> {
    match spec {
        EmbedderSpec::OpenaiCompatible { .. } if !spec.is_local() => {
            api_key("embed", &["ANAMNESIS_EMBED_API_KEY"])
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
/// in from the keychain (or the usual environment variables).
pub fn harness_config(app: &App, settings: &Value) -> HarnessConfig {
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
        a.api_key = api_key(&a.id, &["ANTHROPIC_API_KEY"]);
    }
    if let Some(g) = cfg.gemini.as_mut()
        && g.api_key.is_none()
    {
        g.api_key = api_key(&g.id, &["GEMINI_API_KEY", "GOOGLE_API_KEY"]);
    }
    for c in cfg.openai_compatible.iter_mut() {
        if c.api_key.is_none() && !c.is_local() {
            c.api_key = api_key(&c.id, env_for_provider(&c.provider));
        }
    }
    if cfg.claude_cli.work_dir.is_none() {
        let wd = app.cfg.root.join("claude-cli-work");
        let _ = std::fs::create_dir_all(&wd);
        cfg.claude_cli.work_dir = Some(wd);
    }
    cfg
}

/// The brain's own mcp server offered to the installed claude cli: stdio, so no token
/// ever appears on a command line.
pub fn mcp_offer(app: &App) -> Option<McpOffer> {
    let exe = std::env::current_exe().ok()?;
    let dir = if app.is_demo() {
        app.cfg.demo_brain_dir()
    } else {
        app.cfg.user_brain_dir()
    };
    let config = json!({
        "mcpServers": {
            "anamnesis": {
                "type": "stdio",
                "command": exe.display().to_string(),
                "args": ["mcp", "--brain-dir", dir.display().to_string()],
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
        tokio::task::spawn_blocking(move || harness_config(&app, &settings))
            .await
            .unwrap_or_default()
    };
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
