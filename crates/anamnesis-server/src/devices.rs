//! Sync across the user's own devices (`anamnesis-sync`), for the user's brain (never the
//! demo). The sync root is a folder the user's own cloud drive syncs, or an s3-compatible
//! bucket they configured; there is no anamnesis account or service.
//!
//! On this device:
//! - `<brain>/sync/config.json`: where the sync root is and whether sync is on;
//! - `<brain>/sync/state.json`: this device's id, chain, clock and open conflicts;
//! - the vault key (derived from the passphrase, never the passphrase itself) and the s3
//!   keys in the os keychain (or `secrets.json`, mode 0600), never in settings.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anamnesis_sync::engine::SyncState;
use anamnesis_sync::{
    FolderStorage, KdfParams, Key, Resolution, S3Config, S3Storage, Storage, SyncError, SyncReport,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::App;
use crate::brain::Brain;

const KEY_ACCOUNT: &str = "sync-key";
const S3_ACCESS_ACCOUNT: &str = "sync-s3-access";
const S3_SECRET_ACCOUNT: &str = "sync-s3-secret";
/// Minutes between passes when nothing asks for one sooner.
pub const DEFAULT_INTERVAL_MINUTES: u64 = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Place {
    Folder { path: String },
    S3(S3Config),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevicesConfig {
    pub place: Place,
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval_minutes: u64,
}

fn default_interval() -> u64 {
    DEFAULT_INTERVAL_MINUTES
}

/// Live status, kept in memory.
#[derive(Default)]
pub struct DevicesRuntime {
    pub running: AtomicBool,
    pub last_report: Mutex<Option<SyncReport>>,
    pub last_error: Mutex<Option<String>>,
    /// Sources whose connectors run on another device.
    pub foreign: RwLock<HashSet<String>>,
    pub notify: tokio::sync::Notify,
    /// One pass at a time.
    pub lock: tokio::sync::Mutex<()>,
}

fn sync_dir(brain: &Brain) -> std::path::PathBuf {
    brain.dir.join("sync")
}

pub fn read_config(brain: &Brain) -> Option<DevicesConfig> {
    let b = std::fs::read(sync_dir(brain).join("config.json")).ok()?;
    serde_json::from_slice(&b).ok()
}

fn write_json<T: Serialize>(brain: &Brain, name: &str, v: &T) -> Result<()> {
    let dir = sync_dir(brain);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, dir.join(name))?;
    Ok(())
}

pub fn read_state(brain: &Brain) -> Option<SyncState> {
    let b = std::fs::read(sync_dir(brain).join("state.json")).ok()?;
    serde_json::from_slice(&b).ok()
}

/// Is this source run by another device (so its connector must not run here)?
pub fn is_foreign(app: &App, brain: &Brain, source_id: &str) -> bool {
    !brain.is_demo && app.devices.foreign.read().unwrap().contains(source_id)
}

fn user_brain(app: &App) -> Result<Arc<Brain>> {
    app.user_brain().context("the user's brain is not open")
}

fn storage_for(app: &App, place: &Place) -> Result<Box<dyn Storage>> {
    Ok(match place {
        Place::Folder { path } => Box::new(FolderStorage::new(
            anamnesis_connectors::connectors::files::expand_home(path),
        )),
        Place::S3(cfg) => {
            let access = app
                .secrets
                .get(S3_ACCESS_ACCOUNT, &[])
                .map(|x| x.0)
                .context("the s3 access key is missing; set sync up again")?;
            let secret = app
                .secrets
                .get(S3_SECRET_ACCOUNT, &[])
                .map(|x| x.0)
                .context("the s3 secret key is missing; set sync up again")?;
            Box::new(S3Storage::new(cfg.clone(), access, secret)?)
        }
    })
}

fn key(app: &App) -> Result<Key> {
    let (hex, _) = app
        .secrets
        .get(KEY_ACCOUNT, &[])
        .context("the sync key is missing on this device; set sync up again")?;
    Ok(Key::from_hex(&hex)?)
}

/// Load what is on disk into the runtime (foreign sources), at start.
pub fn load(app: &App) {
    if let Some(b) = app.user_brain()
        && let Some(st) = read_state(&b)
    {
        *app.devices.foreign.write().unwrap() = st.foreign_sources.iter().cloned().collect();
    }
}

/// The body of `POST /api/devices/setup`.
#[derive(Debug, Clone, Deserialize)]
pub struct Setup {
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub bucket: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub access_key: Option<String>,
    #[serde(default)]
    pub secret_key: Option<String>,
    pub passphrase: String,
    #[serde(default)]
    pub device_name: Option<String>,
}

fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| format!("{} device", std::env::consts::OS))
        .to_lowercase()
}

/// Create a vault in an empty sync root or join the one there; remember the key and the
/// place; keep this device's id if it had joined this vault before. Returns whether the
/// vault was created.
pub async fn setup(app: &Arc<App>, s: Setup, kdf: KdfParams) -> Result<bool> {
    let brain = user_brain(app)?;
    let place = match s.kind.as_str() {
        "folder" => {
            let p = s.path.clone().unwrap_or_default();
            anyhow::ensure!(
                !p.trim().is_empty(),
                anamnesis_core::Error::Invalid("choose a folder".into())
            );
            let abs = anamnesis_connectors::connectors::files::expand_home(p.trim());
            std::fs::create_dir_all(&abs).with_context(|| format!("creating {}", abs.display()))?;
            Place::Folder {
                path: abs.display().to_string(),
            }
        }
        "s3" => {
            let need = |v: &Option<String>, what: &str| -> Result<String> {
                v.clone()
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .ok_or_else(|| {
                        anamnesis_core::Error::Invalid(format!("{what} is missing")).into()
                    })
            };
            let cfg = S3Config {
                endpoint: need(&s.endpoint, "the endpoint")?,
                bucket: need(&s.bucket, "the bucket")?,
                region: s
                    .region
                    .clone()
                    .filter(|r| !r.trim().is_empty())
                    .unwrap_or_else(|| "auto".into()),
                prefix: s.prefix.clone().unwrap_or_default(),
            };
            let access = need(&s.access_key, "the access key")?;
            let secret = need(&s.secret_key, "the secret key")?;
            app.secrets.set(S3_ACCESS_ACCOUNT, &access)?;
            app.secrets.set(S3_SECRET_ACCOUNT, &secret)?;
            Place::S3(cfg)
        }
        other => {
            return Err(
                anamnesis_core::Error::Invalid(format!("unknown sync place {other:?}")).into(),
            );
        }
    };
    let storage = storage_for(app, &place)?;
    let pass = s.passphrase.clone();
    let (vault, key, created) = tokio::task::spawn_blocking(move || {
        anamnesis_sync::create_or_open(storage.as_ref(), &pass, kdf)
    })
    .await?
    .map_err(user_error)?;
    app.secrets.set(KEY_ACCOUNT, &key.to_hex())?;
    let name = s
        .device_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(host_name);
    let state = match read_state(&brain) {
        Some(mut st) if st.vault_id == vault.vault_id => {
            st.device_name = name;
            st
        }
        _ => SyncState::new(&vault, &name),
    };
    write_json(&brain, "state.json", &state)?;
    write_json(
        &brain,
        "config.json",
        &DevicesConfig {
            place,
            enabled: true,
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
        },
    )?;
    *app.devices.last_error.lock().unwrap() = None;
    app.devices.notify.notify_one();
    Ok(created)
}

/// Turn the sync engine's errors into messages for the user.
fn user_error(e: SyncError) -> anyhow::Error {
    match e {
        SyncError::WrongPassphrase | SyncError::Invalid(_) | SyncError::Unlinked { .. } => {
            anamnesis_core::Error::Invalid(e.to_string()).into()
        }
        other => anyhow::Error::new(other),
    }
}

/// Stop syncing on this device: forget the place and the key (the device's state stays,
/// so joining the same vault again keeps its identity). Nothing in the sync root changes.
pub fn stop(app: &App) -> Result<()> {
    let brain = user_brain(app)?;
    let _ = std::fs::remove_file(sync_dir(&brain).join("config.json"));
    app.secrets.delete(KEY_ACCOUNT)?;
    app.secrets.delete(S3_ACCESS_ACCOUNT)?;
    app.secrets.delete(S3_SECRET_ACCOUNT)?;
    *app.devices.last_report.lock().unwrap() = None;
    *app.devices.last_error.lock().unwrap() = None;
    Ok(())
}

/// One pass now. `Ok(None)` when sync is not set up or is off.
pub async fn sync_now(app: &Arc<App>) -> Result<Option<SyncReport>> {
    let brain = user_brain(app)?;
    let Some(cfg) = read_config(&brain) else {
        return Ok(None);
    };
    if !cfg.enabled {
        return Ok(None);
    }
    let _g = app.devices.lock.lock().await;
    app.devices.running.store(true, Ordering::SeqCst);
    let res = run_pass(app, &brain, &cfg).await;
    app.devices.running.store(false, Ordering::SeqCst);
    match res {
        Ok(r) => {
            *app.devices.last_error.lock().unwrap() = None;
            *app.devices.last_report.lock().unwrap() = Some(r.clone());
            if r.imported > 0 || r.inbox > 0 {
                app.jobs_notify.notify_one();
                app.sync_notify.notify_one();
            }
            Ok(Some(r))
        }
        Err(e) => {
            *app.devices.last_error.lock().unwrap() = Some(format!("{e:#}"));
            Err(e)
        }
    }
}

async fn run_pass(app: &Arc<App>, brain: &Arc<Brain>, cfg: &DevicesConfig) -> Result<SyncReport> {
    let storage = storage_for(app, &cfg.place)?;
    let key = key(app)?;
    let mut state =
        read_state(brain).context("this device's sync state is missing; set sync up again")?;
    let b = brain.clone();
    let (report, state) = tokio::task::spawn_blocking(move || {
        b.with_store(|s| {
            let r =
                anamnesis_sync::sync(s, storage.as_ref(), &key, &mut state).map_err(user_error)?;
            Ok((r, state))
        })
    })
    .await??;
    write_json(brain, "state.json", &state)?;
    *app.devices.foreign.write().unwrap() = state.foreign_sources.iter().cloned().collect();
    if report.imported > 0 || report.exported > 0 || report.inbox > 0 {
        tracing::info!(
            "devices: {} out, {} in, {} from the inbox, {} new conflict(s)",
            report.exported,
            report.imported,
            report.inbox,
            report.new_conflicts
        );
    }
    Ok(report)
}

/// The background loop: a pass every `interval_minutes`, and whenever asked.
pub async fn devices_loop(app: Arc<App>) {
    // a first pass shortly after start
    let mut wait = Duration::from_secs(5);
    loop {
        tokio::select! {
            _ = app.cancel.cancelled() => return,
            _ = app.devices.notify.notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
        if let Err(e) = sync_now(&app).await {
            tracing::warn!("devices: {e:#}");
        }
        wait = app
            .user_brain()
            .and_then(|b| read_config(&b))
            .map(|c| Duration::from_secs(c.interval_minutes.max(1) * 60))
            .unwrap_or(Duration::from_secs(DEFAULT_INTERVAL_MINUTES * 60));
    }
}

fn place_json(place: &Place) -> Value {
    match place {
        Place::Folder { path } => json!({
            "kind": "folder",
            "location": path,
            "inbox": std::path::Path::new(path).join("inbox").display().to_string(),
        }),
        Place::S3(c) => {
            let p = c.prefix.trim_matches('/');
            let base = if p.is_empty() {
                c.bucket.clone()
            } else {
                format!("{}/{p}", c.bucket)
            };
            json!({
                "kind": "s3",
                "location": format!("{} · {base}", c.endpoint),
                "endpoint": c.endpoint, "bucket": c.bucket, "region": c.region, "prefix": c.prefix,
                "inbox": format!("{base}/inbox/"),
            })
        }
    }
}

/// `GET /api/devices`.
pub fn status(app: &App) -> Result<Value> {
    let brain = user_brain(app)?;
    let cfg = read_config(&brain);
    let st = read_state(&brain);
    let mut devices = Vec::new();
    if let Some(st) = &st {
        devices.push(json!({
            "id": st.device_id, "name": st.device_name, "platform": st.platform,
            "last_sync_at": st.last_sync_at, "this": true, "unlinked": null, "error": null,
        }));
        for (id, r) in &st.remotes {
            devices.push(json!({
                "id": id,
                "name": if r.name.is_empty() { id.clone() } else { r.name.clone() },
                "platform": r.platform,
                "last_sync_at": r.last_sync_at,
                "this": false,
                "unlinked": r.unlinked.as_ref().map(|u| json!({ "by": u.by_name, "at": u.at })),
                "error": r.error,
            }));
        }
    }
    let conflicts: Vec<Value> = st
        .as_ref()
        .map(|st| {
            st.conflicts
                .iter()
                .map(|c| serde_json::to_value(c).unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "configured": cfg.is_some(),
        "enabled": cfg.as_ref().is_some_and(|c| c.enabled),
        "place": cfg.as_ref().map(|c| place_json(&c.place)),
        "interval_minutes": cfg.as_ref().map(|c| c.interval_minutes),
        "this_device": st.as_ref().map(|s| json!({ "id": s.device_id, "name": s.device_name })),
        "devices": devices,
        "conflicts": conflicts,
        "last_sync_at": st.as_ref().and_then(|s| s.last_sync_at),
        "last_report": *app.devices.last_report.lock().unwrap(),
        "last_error": *app.devices.last_error.lock().unwrap(),
        "running": app.devices.running.load(Ordering::SeqCst),
        "demo_active": app.is_demo(),
    }))
}

/// Unlink another device (see `anamnesis_sync::unlink`).
pub async fn unlink(app: &Arc<App>, device_id: &str) -> Result<()> {
    let brain = user_brain(app)?;
    let cfg = read_config(&brain).context("sync is not set up")?;
    let storage = storage_for(app, &cfg.place)?;
    let key = key(app)?;
    let _g = app.devices.lock.lock().await;
    let mut st = read_state(&brain).context("no sync state")?;
    let id = device_id.to_string();
    let st = tokio::task::spawn_blocking(move || {
        anamnesis_sync::unlink(storage.as_ref(), &key, &mut st, &id).map(|_| st)
    })
    .await?
    .map_err(user_error)?;
    write_json(&brain, "state.json", &st)?;
    Ok(())
}

/// Settle a conflict here (the choice travels to the other devices on the next pass).
pub async fn resolve(
    app: &Arc<App>,
    conflict_id: &str,
    how: Resolution,
) -> Result<Option<anamnesis_core::Item>> {
    let brain = user_brain(app)?;
    let _g = app.devices.lock.lock().await;
    let mut st = read_state(&brain).context("no sync state")?;
    let cid = conflict_id.to_string();
    let b = brain.clone();
    let (item, st) = tokio::task::spawn_blocking(move || {
        b.with_store(|s| {
            let it = anamnesis_sync::resolve(s, &mut st, &cid, &how).map_err(user_error)?;
            Ok((it, st))
        })
    })
    .await??;
    write_json(&brain, "state.json", &st)?;
    drop(_g);
    app.devices.notify.notify_one();
    app.jobs_notify.notify_one();
    Ok(item)
}
