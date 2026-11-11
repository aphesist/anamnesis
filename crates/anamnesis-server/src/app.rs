//! The running application: the active brain, the embedder, the model router and the
//! handles of the background jobs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anamnesis_embed::{Embedder, EmbedderSpec};
use anamnesis_harness::Router;
use anyhow::{Context, Result};
use serde::Serialize;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::brain::Brain;

/// How the server was started.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `<data dir>/anamnesis`: the user's brain lives here, the demo brain in `demo/`.
    pub root: PathBuf,
    pub port: u16,
    /// Directory holding `app/` and `theme/` (the repo's `ui/`), if found on disk.
    pub ui_dir: Option<PathBuf>,
    /// A demo bundle: a folder of notes, or a folder / file with a `.jsonl` bundle.
    pub demo_bundle: Option<PathBuf>,
    /// Use this embedder instead of the one in settings (tests use the hash embedder).
    pub embedder_override: Option<EmbedderSpec>,
    /// Detect model backends (probes loopback ports, runs `claude --version`). Off in tests.
    pub detect_backends: bool,
    /// Where local embedding weights are cached.
    pub model_cache: PathBuf,
    /// Extra connector plugin folders (`--plugins-dir`), besides `<root>/plugins`.
    pub plugin_dirs: Vec<PathBuf>,
    /// Where api keys go: the os keychain (with a 0600 file as fallback), or the file only.
    pub key_store: crate::secrets::KeyStoreMode,
}

impl ServerConfig {
    /// A config rooted at `root` with defaults suitable for tests.
    pub fn for_tests(root: &Path) -> Self {
        ServerConfig {
            root: root.to_path_buf(),
            port: 7368,
            ui_dir: None,
            demo_bundle: None,
            embedder_override: Some(EmbedderSpec::hash()),
            detect_backends: false,
            model_cache: root.join("models"),
            plugin_dirs: Vec::new(),
            // tests never touch the os keychain
            key_store: crate::secrets::KeyStoreMode::File,
        }
    }

    /// Every folder scanned for connector plugins: `<root>/plugins`, then `--plugins-dir`.
    pub fn all_plugin_dirs(&self) -> Vec<PathBuf> {
        let mut v = vec![self.root.join("plugins")];
        v.extend(self.plugin_dirs.iter().cloned());
        v
    }

    pub fn user_brain_dir(&self) -> PathBuf {
        self.root.clone()
    }
    pub fn demo_brain_dir(&self) -> PathBuf {
        self.root.join("demo")
    }
}

/// The default data root: `<os data dir>/anamnesis`.
pub fn default_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".anamnesis"))
        .join("anamnesis")
}

/// The embedder in use, and how it got there.
#[derive(Clone)]
pub struct EmbedState {
    pub spec: EmbedderSpec,
    /// `None` while a local model is loading.
    pub embedder: Option<Arc<dyn Embedder>>,
    /// Short lowercase status for `/api/backends`.
    pub status: String,
    /// True when the configured embedder could not load and the hash embedder stands in.
    pub fallback: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbedStatus {
    pub id: Option<String>,
    pub backend: String,
    pub status: String,
    pub fallback: bool,
    pub local: bool,
}

pub struct App {
    pub cfg: ServerConfig,
    brain: RwLock<Arc<Brain>>,
    /// The user's brain, kept open while the demo brain is shown.
    user_brain: RwLock<Option<Arc<Brain>>>,
    embed: RwLock<EmbedState>,
    router: RwLock<Option<Arc<Router>>>,
    /// Wakes the job loop (something changed).
    pub jobs_notify: Notify,
    /// Wakes the sync supervisor (sources changed).
    pub sync_notify: Notify,
    /// Cancelled on shutdown.
    pub cancel: CancellationToken,
    pub sync: crate::sync::SyncManager,
    pub chatgpt: crate::backends::ChatgptSignIn,
    /// Api keys (keychain or `<root>/secrets.json`).
    pub secrets: crate::secrets::Secrets,
    /// Where each backend's api key came from, as of the last detection.
    pub key_sources: RwLock<std::collections::HashMap<String, crate::secrets::KeyStorage>>,
    /// Connector plugins found on disk.
    pub plugins: crate::plugins::Plugins,
    /// Progress of a one-shot jobs pass (`jobs` in `/api/status`).
    pub jobs_progress: RwLock<JobsProgress>,
}

/// Where a one-shot pass of the background jobs is (`run_jobs_once`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct JobsProgress {
    pub running: bool,
    /// `embeddings`, `threads`, `mind`, `suggestions`, `review`; null when idle.
    pub phase: Option<String>,
    /// Phases finished in this pass, of `total`.
    pub done: u32,
    pub total: u32,
    /// The last pass's error, if it failed.
    pub error: Option<String>,
}

impl App {
    /// Open the user's brain (or the demo brain) and resolve the embedder spec. Does not
    /// start any background task and does not load a local model yet.
    pub fn open(cfg: ServerConfig, demo: bool) -> Result<Arc<App>> {
        let user = Arc::new(Brain::open(&cfg.user_brain_dir(), false)?);
        let (active, user_slot) = if demo {
            (
                Arc::new(Brain::open(&cfg.demo_brain_dir(), true)?),
                Some(user),
            )
        } else {
            (user.clone(), Some(user))
        };
        let spec = Self::spec_for(&cfg, &active)?;
        let plugins = crate::plugins::Plugins::new(cfg.all_plugin_dirs());
        plugins.scan();
        let app = Arc::new(App {
            secrets: crate::secrets::Secrets::new(&cfg.root, cfg.key_store),
            key_sources: RwLock::default(),
            plugins,
            jobs_progress: RwLock::default(),
            embed: RwLock::new(EmbedState {
                status: "not loaded".into(),
                spec,
                embedder: None,
                fallback: false,
            }),
            cfg,
            brain: RwLock::new(active),
            user_brain: RwLock::new(user_slot),
            router: RwLock::new(None),
            jobs_notify: Notify::new(),
            sync_notify: Notify::new(),
            cancel: CancellationToken::new(),
            sync: crate::sync::SyncManager::default(),
            chatgpt: crate::backends::ChatgptSignIn::default(),
        });
        Ok(app)
    }

    fn spec_for(cfg: &ServerConfig, brain: &Brain) -> Result<EmbedderSpec> {
        if let Some(s) = &cfg.embedder_override {
            return Ok(s.clone());
        }
        let v = brain.with_store(|s| Ok(s.get_setting("embedder")?))?;
        Ok(match v {
            Some(v) if !v.is_null() => serde_json::from_value(v).context("settings.embedder")?,
            _ => EmbedderSpec::default(),
        })
    }

    /// The brain currently shown (the user's, or the demo brain).
    pub fn brain(&self) -> Arc<Brain> {
        self.brain.read().unwrap().clone()
    }

    pub fn is_demo(&self) -> bool {
        self.brain().is_demo
    }

    /// Switch between the user's brain and the demo brain. Returns the active brain.
    pub fn set_demo(&self, on: bool) -> Result<Arc<Brain>> {
        let cur = self.brain();
        if cur.is_demo == on {
            return Ok(cur);
        }
        let next = if on {
            Arc::new(Brain::open(&self.cfg.demo_brain_dir(), true)?)
        } else {
            match self.user_brain.read().unwrap().clone() {
                Some(b) => b,
                None => Arc::new(Brain::open(&self.cfg.user_brain_dir(), false)?),
            }
        };
        *self.brain.write().unwrap() = next.clone();
        self.sync_notify.notify_one();
        self.jobs_notify.notify_one();
        Ok(next)
    }

    // ---------- embedder ----------

    pub fn embed_state(&self) -> EmbedState {
        self.embed.read().unwrap().clone()
    }

    pub fn embedder(&self) -> Option<Arc<dyn Embedder>> {
        self.embed.read().unwrap().embedder.clone()
    }

    pub fn embed_status(&self) -> EmbedStatus {
        let s = self.embed_state();
        EmbedStatus {
            id: s.embedder.as_ref().map(|e| e.id().to_string()),
            backend: s.spec.backend().to_string(),
            status: s.status,
            fallback: s.fallback,
            local: s.spec.is_local(),
        }
    }

    /// Build the embedder for the current spec (blocking: a local model may load or
    /// download here). Falls back to the hash embedder if it cannot load.
    pub fn load_embedder_blocking(&self) {
        let spec = self.embed_state().spec;
        {
            let mut e = self.embed.write().unwrap();
            e.status = "loading".into();
        }
        let built = anamnesis_embed::build_with_secret(
            &spec,
            &self.cfg.model_cache,
            crate::backends::embed_api_key(&self.secrets, &spec).as_deref(),
        );
        let mut e = self.embed.write().unwrap();
        if e.spec != spec {
            return; // the spec changed while loading; a newer load is responsible
        }
        match built {
            Ok(emb) => {
                tracing::info!("embedder ready: {}", emb.id());
                e.embedder = Some(Arc::from(emb));
                e.status = "ready".into();
                e.fallback = false;
            }
            Err(err) => {
                tracing::warn!(
                    "embedder {} failed to load, using the hash embedder: {err:#}",
                    spec.backend()
                );
                let h = anamnesis_embed::HashEmbedder::new(
                    match anamnesis_embed::EmbedderSpec::hash() {
                        EmbedderSpec::Hash { dim } => dim,
                        _ => 256,
                    },
                );
                e.embedder = Some(Arc::new(h));
                e.status = format!("fallback to hash embedder: {}", short_err(&err));
                e.fallback = true;
            }
        }
    }

    /// Load the embedder on a blocking thread.
    pub async fn load_embedder(self: &Arc<Self>) {
        let app = self.clone();
        let _ = tokio::task::spawn_blocking(move || app.load_embedder_blocking()).await;
        self.jobs_notify.notify_one();
    }

    /// Replace the embedder spec (from settings) and reload.
    pub async fn set_embedder_spec(self: &Arc<Self>, spec: EmbedderSpec) {
        {
            let mut e = self.embed.write().unwrap();
            if e.spec == spec && e.embedder.is_some() {
                return;
            }
            e.spec = spec;
            e.embedder = None;
            e.status = "loading".into();
            e.fallback = false;
        }
        self.load_embedder().await;
    }

    // ---------- model router ----------

    pub fn router(&self) -> Option<Arc<Router>> {
        self.router.read().unwrap().clone()
    }

    pub fn set_router(&self, r: Option<Arc<Router>>) {
        *self.router.write().unwrap() = r;
    }
}

fn short_err(e: &anyhow::Error) -> String {
    let s = e.to_string();
    let s = s.lines().next().unwrap_or("").to_string();
    if s.chars().count() > 160 {
        format!("{}…", s.chars().take(160).collect::<String>())
    } else {
        s
    }
}
