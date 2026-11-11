//! One brain on disk: the sqlite store plus the job caches that live next to it.
//!
//! Layout of a brain directory:
//! - `brain.sqlite` — the event log and its projections (core).
//! - `mind_state.json` — what the next clustering / layout run continues from.
//! - `views.json` — the anatomy and spheres positions of the last layout (the
//!   `layout` view is in the placements table).
//! - `jobs.json` — bookkeeping of the background jobs (which items were already
//!   processed for threads and suggestions, today's review reasons...).
//! - `mcp_token` — the bearer token for the streamable http mcp endpoint (0600). It is
//!   deliberately not a setting: settings are events and travel in exported bundles.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anamnesis_core::{Store, Timestamp};
use anamnesis_mind::{LayoutViews, MindState};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;

/// Bookkeeping for the background jobs. A cache: losing it only means some work is redone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobState {
    /// item id → content hash that threads were last extracted from.
    #[serde(default)]
    pub threads_hash: HashMap<String, String>,
    /// Items whose link suggestions were computed.
    #[serde(default)]
    pub suggested: HashSet<String>,
    /// When the last full clustering / layout run happened.
    #[serde(default)]
    pub last_full_run: Option<Timestamp>,
    /// Day (`YYYY-MM-DD`) the last daily review was drawn.
    #[serde(default)]
    pub review_day: Option<String>,
    /// resurface id → reason ("due", "contextual", "serendipity", "random").
    #[serde(default)]
    pub review_reasons: HashMap<String, String>,
    /// Event head (seq) the jobs last looked at, to notice writes from other processes.
    #[serde(default)]
    pub seen_head: u64,
}

pub struct Brain {
    pub dir: PathBuf,
    pub store: Arc<Mutex<Store>>,
    pub is_demo: bool,
    /// Serialises the background jobs on this brain (the http api never takes it).
    pub jobs_lock: tokio::sync::Mutex<()>,
}

impl Brain {
    /// Open (or create) the brain in `dir`.
    pub fn open(dir: &Path, is_demo: bool) -> Result<Brain> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let store = Store::open(dir.join("brain.sqlite"))
            .with_context(|| format!("opening {}", dir.join("brain.sqlite").display()))?;
        Ok(Brain {
            dir: dir.to_path_buf(),
            store: Arc::new(Mutex::new(store)),
            is_demo,
            jobs_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// Run `f` on the store in a blocking thread. The lock is held only for the duration of
    /// `f`, which is synchronous, so it can never be held across an await.
    pub async fn db<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Store) -> Result<T> + Send + 'static,
    {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let guard = store.lock().unwrap_or_else(|e| e.into_inner());
            f(&guard)
        })
        .await
        .context("store task panicked")?
    }

    /// Synchronous access (cli commands, tests, code already on a blocking thread).
    pub fn with_store<T>(&self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        f(&guard)
    }

    fn read_json<T: DeserializeOwned + Default>(&self, name: &str) -> T {
        let p = self.dir.join(name);
        match std::fs::read(&p) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable {}: {e}", p.display());
                T::default()
            }),
            Err(_) => T::default(),
        }
    }

    fn write_json<T: Serialize>(&self, name: &str, v: &T) -> Result<()> {
        let p = self.dir.join(name);
        let tmp = self.dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec(v)?)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }

    pub fn job_state(&self) -> JobState {
        self.read_json("jobs.json")
    }
    pub fn save_job_state(&self, s: &JobState) -> Result<()> {
        self.write_json("jobs.json", s)
    }
    pub fn mind_state(&self) -> MindState {
        self.read_json("mind_state.json")
    }
    pub fn save_mind_state(&self, s: &MindState) -> Result<()> {
        self.write_json("mind_state.json", s)
    }
    pub fn views(&self) -> Option<LayoutViews> {
        let p = self.dir.join("views.json");
        std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }
    pub fn save_views(&self, v: &LayoutViews) -> Result<()> {
        self.write_json("views.json", v)
    }

    /// The mcp bearer token, created on first use (random 256 bits, hex, file mode 0600).
    pub fn mcp_token(&self) -> Result<String> {
        let p = self.dir.join("mcp_token");
        if let Ok(t) = std::fs::read_to_string(&p) {
            let t = t.trim().to_string();
            if t.len() >= 32 {
                return Ok(t);
            }
        }
        let token = random_hex(32);
        write_private(&p, token.as_bytes())?;
        Ok(token)
    }
}

/// `n` random bytes from the os, hex encoded.
pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("os randomness");
    hex::encode(buf)
}

/// A random u64 from the os.
pub fn random_u64() -> u64 {
    let mut buf = [0u8; 8];
    getrandom::fill(&mut buf).expect("os randomness");
    u64::from_le_bytes(buf)
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(data)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
        Ok(())
    }
}
