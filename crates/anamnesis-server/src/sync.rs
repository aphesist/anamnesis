//! Connector sync: one task per source (poll interval, plus a debounced file watch where
//! the connector supports it), and a one-shot `sync_source` used by the cli and the api.
//!
//! Cursors are stored as `<config hash>:<connector cursor>`, so a source whose config
//! changed starts again from a full sync.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anamnesis_connectors::{
    Batch, Connector, META_ORIGIN_EXPLICIT, RESERVED_CONNECTOR_IDS, validated_list,
};
use anamnesis_core::{Actor, EventKind, Id, Origin, Source, UpsertOutcome, canon::canonical_json};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::brain::Brain;
use crate::plugins::Plugins;

const WATCH_DEBOUNCE: Duration = Duration::from_millis(1500);
const DEFAULT_POLL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub duplicates: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub pages: usize,
}

impl SyncReport {
    pub fn changed(&self) -> bool {
        self.created + self.updated + self.deleted + self.renamed > 0
    }
}

fn config_hash(source: &Source) -> String {
    let mut h = Sha256::new();
    h.update(canonical_json(&source.config).as_bytes());
    hex::encode(&h.finalize()[..8])
}

/// The connector's own cursor, if the stored one belongs to the current config.
fn inner_cursor(source: &Source) -> Option<String> {
    let c = source.cursor.as_deref()?;
    let (h, inner) = c.split_once(':')?;
    (h == config_hash(source) && !inner.is_empty()).then(|| inner.to_string())
}

fn wrap_cursor(source: &Source, inner: Option<String>) -> Option<String> {
    inner.map(|c| format!("{}:{c}", config_hash(source)))
}

/// A connector for a source, or `None` for sources the server writes itself.
pub fn connector_for(plugins: &Plugins, source: &Source) -> Result<Option<Box<dyn Connector>>> {
    if RESERVED_CONNECTOR_IDS.contains(&source.connector.as_str()) {
        return Ok(None);
    }
    Ok(Some(plugins.resolve(&source.connector)?))
}

/// Apply one batch: renames (re-keyed in place, so the item keeps its id, links and
/// history), then upserts, then deletes.
fn apply_batch(
    store: &anamnesis_core::Store,
    source: &Source,
    connector_default_origin: Option<Origin>,
    batch: Batch,
    r: &mut SyncReport,
) -> Result<()> {
    let actor = Actor::Connector(source.connector.clone());
    for rn in &batch.renames {
        if let Some(mut item) = store.get_item_by_external_id(&source.id, &rn.from)? {
            if item.deleted || store.get_item_by_external_id(&source.id, &rn.to)?.is_some() {
                continue;
            }
            item.external_id = rn.to.clone();
            store.append(actor.clone(), EventKind::ItemUpdated { item })?;
            r.renamed += 1;
        }
    }
    for mut doc in batch.upserts {
        if !doc.meta.is_object() {
            doc.meta = if doc.meta.is_null() {
                json!({})
            } else {
                json!({ "connector_meta": doc.meta })
            };
        }
        // keys the server keeps on the item survive a connector update (which replaces meta)
        if let Some(old) = store.get_item_by_external_id(&source.id, &doc.external_id)?
            && crate::privacy::is_explicit(&old)
        {
            doc.meta[crate::privacy::META_PRIVACY_EXPLICIT] = json!(true);
        }
        // the source's default origin, unless the document stated its own
        let explicit_origin = doc.meta.get(META_ORIGIN_EXPLICIT) == Some(&json!(true));
        if !explicit_origin && Some(doc.origin) == connector_default_origin {
            doc.origin = source.default_origin;
        }
        let item = match store.upsert_document(&source.id, doc)? {
            UpsertOutcome::Created(i) => {
                r.created += 1;
                Some(i)
            }
            UpsertOutcome::Updated(i) => {
                r.updated += 1;
                Some(i)
            }
            UpsertOutcome::Unchanged(id) => {
                r.unchanged += 1;
                store.get_item(&id)?
            }
            UpsertOutcome::Duplicate { .. } => {
                r.duplicates += 1;
                None
            }
        };
        if let Some(item) = item {
            crate::privacy::reconcile_item(store, &item, source, actor.clone())?;
        }
    }
    for ext in &batch.deletes {
        if store.delete_by_external_id(&source.id, ext)?.is_some() {
            r.deleted += 1;
        }
    }
    Ok(())
}

/// Sync one source until the connector has nothing more, persisting the cursor after each
/// applied page. No store lock is held while the connector runs.
pub async fn sync_source(
    brain: &Arc<Brain>,
    plugins: &Plugins,
    source_id: &Id,
) -> Result<SyncReport> {
    let sid = source_id.clone();
    let mut source = brain
        .db(move |s| Ok(s.get_source(&sid)?))
        .await?
        .with_context(|| format!("no source {source_id}"))?;
    let Some(conn) = connector_for(plugins, &source)? else {
        return Ok(SyncReport::default());
    };
    let default_origin = Some(conn.manifest().default_origin);
    let mut report = SyncReport::default();
    let mut cursor = inner_cursor(&source);
    for _ in 0..10_000 {
        let batch = validated_list(conn.as_ref(), &source.config, cursor.as_deref())
            .await
            .with_context(|| format!("syncing {}", source.name))?;
        report.pages += 1;
        let has_more = batch.has_more;
        let next = batch.next_cursor.clone();
        let src = source.clone();
        let mut r = std::mem::take(&mut report);
        let (r, updated) = brain
            .db(move |s| {
                apply_batch(s, &src, default_origin, batch, &mut r)?;
                let updated = s.set_source_cursor(&src.id, wrap_cursor(&src, next))?;
                Ok((r, updated))
            })
            .await?;
        report = r;
        source = updated;
        cursor = inner_cursor(&source);
        if !has_more {
            break;
        }
    }
    if report.changed() {
        tracing::info!(
            "synced {}: +{} ~{} -{} renamed {}",
            source.name,
            report.created,
            report.updated,
            report.deleted,
            report.renamed
        );
    }
    Ok(report)
}

/// A frozen source keeps its items but is never synced again (e.g. the sources of the
/// prebuilt demo brain, whose folders only existed on the machine that built it).
pub fn is_frozen(src: &Source) -> bool {
    src.config
        .get("frozen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Sync every connector-backed source of the active brain once.
pub async fn sync_all(app: &Arc<App>) -> Result<Vec<(Source, Result<SyncReport>)>> {
    let brain = app.brain();
    let sources = brain.db(|s| Ok(s.list_sources()?)).await?;
    let mut out = Vec::new();
    for src in sources {
        if RESERVED_CONNECTOR_IDS.contains(&src.connector.as_str())
            || src.connector == "editor"
            || is_frozen(&src)
            || crate::devices::is_foreign(app, &brain, &src.id.0)
        {
            continue;
        }
        let r = sync_source(&brain, &app.plugins, &src.id).await;
        out.push((src, r));
    }
    Ok(out)
}

/// Background sync tasks, one per source, reconciled with the store's source list.
#[derive(Default)]
pub struct SyncManager {
    tasks: Mutex<HashMap<String, SourceTask>>,
}

struct SourceTask {
    cancel: CancellationToken,
    trigger: Arc<Notify>,
    config: String,
    brain_dir: std::path::PathBuf,
}

impl SyncManager {
    /// Ask a running source task to sync now. Returns false if there is none.
    pub fn trigger(&self, source_id: &Id) -> bool {
        match self.tasks.lock().unwrap().get(&source_id.0) {
            Some(t) => {
                t.trigger.notify_one();
                true
            }
            None => false,
        }
    }

    fn stop_all(&self) {
        for (_, t) in self.tasks.lock().unwrap().drain() {
            t.cancel.cancel();
        }
    }
}

/// Start / stop source tasks so they match the active brain's sources.
pub async fn reconcile(app: &Arc<App>) -> Result<()> {
    let brain = app.brain();
    let sources = brain.db(|s| Ok(s.list_sources()?)).await?;
    let mut tasks = app.sync.tasks.lock().unwrap();
    // drop tasks for removed sources, changed configs or another brain
    tasks.retain(|id, t| {
        let keep = t.brain_dir == brain.dir
            && sources
                .iter()
                .any(|s| &s.id.0 == id && canonical_json(&s.config) == t.config);
        if !keep {
            t.cancel.cancel();
        }
        keep
    });
    for src in sources {
        if tasks.contains_key(&src.id.0)
            || is_frozen(&src)
            || crate::devices::is_foreign(app, &brain, &src.id.0)
        {
            continue;
        }
        let Ok(Some(conn)) = connector_for(&app.plugins, &src) else {
            continue;
        };
        let poll = conn
            .manifest()
            .poll_interval_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_POLL);
        let cancel = app.cancel.child_token();
        let trigger = Arc::new(Notify::new());
        tasks.insert(
            src.id.0.clone(),
            SourceTask {
                cancel: cancel.clone(),
                trigger: trigger.clone(),
                config: canonical_json(&src.config),
                brain_dir: brain.dir.clone(),
            },
        );
        let app = app.clone();
        let brain = brain.clone();
        tokio::spawn(source_loop(app, brain, src, conn, poll, cancel, trigger));
    }
    Ok(())
}

async fn source_loop(
    app: Arc<App>,
    brain: Arc<Brain>,
    src: Source,
    conn: Box<dyn Connector>,
    poll: Duration,
    cancel: CancellationToken,
    trigger: Arc<Notify>,
) {
    let mut watch = match conn.watch(&src.config) {
        Ok(w) => w,
        Err(e) => {
            tracing::debug!(
                "{}: no watch ({e}); polling every {}s",
                src.name,
                poll.as_secs()
            );
            None
        }
    };
    loop {
        match sync_source(&brain, &app.plugins, &src.id).await {
            Ok(r) if r.changed() => app.jobs_notify.notify_one(),
            Ok(_) => {}
            Err(e) => tracing::warn!("{e:#}"),
        }
        let notice = async {
            match watch.as_mut() {
                Some(w) => w.notices.recv().await.is_some(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(poll) => {}
            _ = trigger.notified() => {}
            alive = notice => {
                if !alive {
                    watch = None;
                    continue;
                }
                // debounce: wait until the burst of file events settles
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(WATCH_DEBOUNCE) => break,
                        more = async { match watch.as_mut() { Some(w) => w.notices.recv().await, None => None } } => {
                            if more.is_none() { break; }
                        }
                    }
                }
            }
        }
    }
}

/// The sync supervisor: reconciles source tasks at start, when notified (sources added,
/// removed, brain switched) and every minute (sources added by another process).
pub async fn sync_supervisor(app: Arc<App>) {
    loop {
        if let Err(e) = reconcile(&app).await {
            tracing::warn!("sync: {e:#}");
        }
        tokio::select! {
            _ = app.cancel.cancelled() => {
                app.sync.stop_all();
                return;
            }
            _ = app.sync_notify.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
        }
    }
}
