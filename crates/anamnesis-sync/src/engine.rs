//! One sync pass between this device's store and a sync root.
//!
//! 1. **unlinked?** if another device unlinked this one, stop.
//! 2. **inbox:** plain `.md` / `.txt` files in `inbox/` become notes (origin owner
//!    written), then move to `inbox/imported/<date>/`.
//! 3. **export:** this device's new events (the brain's content: sources, items,
//!    references, links, chat turns, the user's thread changes, a few settings; never
//!    derived state such as regions, layouts, suggestions, reviews or the egress ledger)
//!    get a hybrid logical clock timestamp, extend the device's own hash chain and are
//!    sealed into one or more batch files under `devices/<this device>/batches/`.
//! 4. **import:** other devices' new batches are opened, their chains verified, and
//!    their events merged in (hlc, device) order. Item edits carry the versions they
//!    replace: an edit made without seeing ours is a **conflict**: both versions are
//!    kept, the one with the later clock is shown, and the user is asked. Settings,
//!    sources and thread statuses are last-writer-wins by clock.
//! 5. **device record:** this device's name and last sync time, sealed.
//!
//! State that must survive between passes ([`SyncState`]) lives on the device, next to
//! the brain, never in the sync root.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anamnesis_core::canon::canonical_json;
use anamnesis_core::{
    Actor, Document, Event, EventKind, Id, Item, ItemKind, Origin, Privacy, Source, Store,
    Timestamp, UpsertOutcome,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::{Key, Vault, open, seal};
use crate::error::{Result, SyncError};
use crate::format::{
    DEVICES_DIR, DeviceRecord, GENESIS, INBOX_DIR, INBOX_IMPORTED, SyncEvent, UnlinkRecord,
    VAULT_FILE, batch_file, decode_batch, device_file, encode_batch, parse_batch, unlinked_file,
    verify_chain,
};
use crate::hlc::{Clock, Hlc};
use crate::storage::Storage;

/// Events per batch file at most.
pub const BATCH_MAX: usize = 2000;
/// Settings that follow the user across devices. Everything else (models, routing,
/// embedder, the mind's parameters) is about one machine.
pub const SYNCED_SETTINGS: &[&str] = &["privacy", "theme", "motion", "distil"];

/// This device's sync bookkeeping (a json file next to the brain).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncState {
    pub vault_id: String,
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub created_at: Timestamp,
    pub clock: Clock,
    /// Local store events up to this seq have been looked at by export.
    pub exported_seq: u64,
    /// This device's own chain.
    pub dseq: u64,
    pub head: String,
    /// Local seq ranges appended by import (never exported again).
    #[serde(default)]
    pub received: Vec<(u64, u64)>,
    #[serde(default)]
    pub remotes: BTreeMap<String, RemoteState>,
    /// item id → its current versions (more than one while a conflict is open).
    #[serde(default)]
    pub items: HashMap<String, Vec<Head>>,
    /// "setting:<key>" / "source:<id>" / "thread:<id>" → clock of the last applied write.
    #[serde(default)]
    pub lww: HashMap<String, Hlc>,
    /// Sources that came from another device: their connectors run there, not here.
    #[serde(default)]
    pub foreign_sources: BTreeSet<String>,
    #[serde(default)]
    pub conflicts: Vec<Conflict>,
    #[serde(default)]
    pub last_sync_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteState {
    pub dseq: u64,
    pub head: String,
    pub name: String,
    pub platform: String,
    pub last_sync_at: Option<Timestamp>,
    /// Set when this device was unlinked (by any device).
    pub unlinked: Option<UnlinkRecord>,
    /// Why its batches could not be read (a broken chain, a file that fails to open).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub version: String,
    pub hlc: Hlc,
    pub device: String,
}

/// Two (or more) versions of one item, made on different devices without seeing each
/// other. The one with the later clock is shown until the user picks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub item_id: Id,
    pub detected_at: Timestamp,
    pub versions: Vec<ConflictVersion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictVersion {
    pub version: String,
    pub hlc: Hlc,
    pub device: String,
    pub device_name: String,
    /// `None`: this version deleted the item.
    pub item: Option<Item>,
    /// Is this the version the brain shows now?
    pub shown: bool,
}

/// How to settle a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "keep")]
pub enum Resolution {
    /// Keep one version (by its `version`).
    One { version: String },
    /// Keep the shown version as the item and every other one as a new note.
    All,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncReport {
    pub exported: usize,
    pub imported: usize,
    pub inbox: usize,
    pub new_conflicts: usize,
    pub devices: usize,
    /// Devices whose batches could not be read, and why.
    pub errors: Vec<String>,
}

fn platform() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

impl SyncState {
    /// A fresh state for this device joining `vault`.
    pub fn new(vault: &Vault, device_name: &str) -> SyncState {
        SyncState {
            vault_id: vault.vault_id.clone(),
            device_id: ulid::Ulid::new().to_string(),
            device_name: device_name.trim().to_string(),
            platform: platform(),
            created_at: chrono::Utc::now(),
            clock: Clock::default(),
            exported_seq: 0,
            dseq: 0,
            head: GENESIS.to_string(),
            received: vec![],
            remotes: BTreeMap::new(),
            items: HashMap::new(),
            lww: HashMap::new(),
            foreign_sources: BTreeSet::new(),
            conflicts: vec![],
            last_sync_at: None,
        }
    }

    fn was_received(&self, seq: u64) -> bool {
        self.received.iter().any(|&(a, b)| seq >= a && seq <= b)
    }

    fn add_received(&mut self, (a, b): (u64, u64)) {
        if let Some(last) = self.received.last_mut()
            && last.1 + 1 == a
        {
            last.1 = b;
            return;
        }
        self.received.push((a, b));
    }

    /// The name another device goes by (or a short id).
    pub fn device_label(&self, id: &str) -> String {
        if id == self.device_id {
            return self.device_name.clone();
        }
        self.remotes
            .get(id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| {
                id.chars()
                    .rev()
                    .take(6)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect()
            })
    }
}

/// A version id for an item's content (its time of modification left out, so the same
/// edit made twice is one version).
pub fn item_version(item: &Item) -> String {
    let mut v = serde_json::to_value(item).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.remove("modified_at");
    }
    let mut h = Sha256::new();
    h.update(canonical_json(&v).as_bytes());
    hex::encode(&h.finalize()[..16])
}

fn deleted_version(id: &Id) -> String {
    format!("deleted:{id}")
}

/// Which local events travel to the other devices.
fn exportable(e: &Event) -> bool {
    match &e.kind {
        EventKind::SourceAdded { .. }
        | EventKind::SourceUpdated { .. }
        | EventKind::SourceRemoved { .. }
        | EventKind::ItemCreated { .. }
        | EventKind::ItemUpdated { .. }
        | EventKind::ItemDeleted { .. }
        | EventKind::RefsSet { .. }
        | EventKind::LinkAdded { .. }
        | EventKind::LinkRemoved { .. }
        | EventKind::ChatTurn { .. } => true,
        // the user's own lifecycle changes; extraction is redone on every device
        EventKind::ThreadUpdated { .. } => matches!(e.actor, Actor::User | Actor::Mcp(_)),
        EventKind::SettingSet { key, .. } => SYNCED_SETTINGS.contains(&key.as_str()),
        _ => false,
    }
}

fn item_event(kind: &EventKind) -> Option<(Id, String, Option<&Item>)> {
    match kind {
        EventKind::ItemCreated { item } | EventKind::ItemUpdated { item } => {
            let v = if item.deleted {
                deleted_version(&item.id)
            } else {
                item_version(item)
            };
            Some((item.id.clone(), v, Some(item)))
        }
        EventKind::ItemDeleted { item_id } => {
            Some((item_id.clone(), deleted_version(item_id), None))
        }
        _ => None,
    }
}

fn lww_key(kind: &EventKind) -> Option<String> {
    match kind {
        EventKind::SettingSet { key, .. } => Some(format!("setting:{key}")),
        EventKind::SourceUpdated { source } => Some(format!("source:{}", source.id)),
        EventKind::ThreadUpdated { thread } => Some(format!("thread:{}", thread.id)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------

/// Read `vault.json` (None if the root has none yet).
pub fn read_vault(storage: &dyn Storage) -> Result<Option<Vault>> {
    match storage.get(VAULT_FILE)? {
        None => Ok(None),
        Some(b) => {
            Ok(Some(serde_json::from_slice(&b).map_err(|e| {
                SyncError::Format(format!("vault.json: {e}"))
            })?))
        }
    }
}

/// Create a vault in an empty root, or open the existing one with `passphrase`.
/// Returns the vault, the key, and whether it was created.
pub fn create_or_open(
    storage: &dyn Storage,
    passphrase: &str,
    kdf: crate::crypto::KdfParams,
) -> Result<(Vault, Key, bool)> {
    if let Some(v) = read_vault(storage)? {
        let key = v.unlock(passphrase)?;
        return Ok((v, key, false));
    }
    let (v, key) = Vault::create(passphrase, kdf)?;
    storage.put(VAULT_FILE, serde_json::to_string_pretty(&v)?.as_bytes())?;
    Ok((v, key, true))
}

fn put_sealed<T: Serialize>(storage: &dyn Storage, key: &Key, path: &str, v: &T) -> Result<()> {
    storage.put(path, &seal(key, path, &serde_json::to_vec(v)?)?)
}

fn get_sealed<T: for<'de> Deserialize<'de>>(
    storage: &dyn Storage,
    key: &Key,
    path: &str,
) -> Result<Option<T>> {
    match storage.get(path)? {
        None => Ok(None),
        Some(b) => Ok(Some(serde_json::from_slice(&open(key, path, &b)?)?)),
    }
}

/// One full pass. `state` is updated only when the pass succeeds as a whole (a failed
/// pass is retried from the same point).
pub fn sync(
    store: &Store,
    storage: &dyn Storage,
    key: &Key,
    state: &mut SyncState,
) -> Result<SyncReport> {
    let vault = read_vault(storage)?
        .ok_or_else(|| SyncError::Invalid("the sync folder has no vault.json".into()))?;
    if vault.vault_id != state.vault_id {
        return Err(SyncError::Invalid(
            "the sync folder holds another vault than the one this device joined".into(),
        ));
    }
    vault.verify(key)?;
    let mut st = state.clone();
    if let Some(u) = get_sealed::<UnlinkRecord>(storage, key, &unlinked_file(&st.device_id))? {
        return Err(SyncError::Unlinked { by: u.by_name });
    }
    let mut report = SyncReport {
        inbox: import_inbox(store, storage, &st)?,
        ..Default::default()
    };
    report.exported = export(store, storage, key, &mut st)?;
    let (imported, conflicts, errors, devices) = import(store, storage, key, &mut st)?;
    report.imported = imported;
    report.new_conflicts = conflicts;
    report.errors = errors;
    report.devices = devices;
    let now = chrono::Utc::now();
    st.last_sync_at = Some(now);
    put_sealed(
        storage,
        key,
        &device_file(&st.device_id),
        &DeviceRecord {
            id: st.device_id.clone(),
            name: st.device_name.clone(),
            platform: st.platform.clone(),
            created_at: st.created_at,
            last_sync_at: now,
            dseq: st.dseq,
        },
    )?;
    *state = st;
    Ok(report)
}

/// Export this device's new events. Returns how many were published.
fn export(store: &Store, storage: &dyn Storage, key: &Key, st: &mut SyncState) -> Result<usize> {
    let mut out: Vec<SyncEvent> = Vec::new();
    loop {
        let evs = store.events_after(st.exported_seq, 5000)?;
        if evs.is_empty() {
            break;
        }
        for e in evs {
            st.exported_seq = e.seq;
            if st.was_received(e.seq) || !exportable(&e) {
                continue;
            }
            let hlc = st.clock.tick(e.at.timestamp_millis());
            let mut parents = Vec::new();
            if let Some((id, version, _)) = item_event(&e.kind) {
                let heads = st.items.remove(&id.0).unwrap_or_default();
                parents = heads.into_iter().map(|h| h.version).collect();
                st.items.insert(
                    id.0.clone(),
                    vec![Head {
                        version,
                        hlc,
                        device: st.device_id.clone(),
                    }],
                );
                // an edit here settles any open conflict on the item
                st.conflicts.retain(|c| c.item_id != id);
            }
            if let Some(k) = lww_key(&e.kind) {
                st.lww.insert(k, hlc);
            }
            let mut se = SyncEvent {
                device: st.device_id.clone(),
                dseq: st.dseq + 1,
                hlc,
                at: e.at,
                actor: e.actor,
                kind: e.kind,
                parents,
                prev: st.head.clone(),
                hash: String::new(),
            };
            se.hash = se.compute_hash()?;
            st.dseq = se.dseq;
            st.head = se.hash.clone();
            out.push(se);
        }
    }
    for chunk in out.chunks(BATCH_MAX) {
        let (first, last) = (chunk[0].dseq, chunk[chunk.len() - 1].dseq);
        let path = batch_file(&st.device_id, first, last);
        storage.put(&path, &seal(key, &path, &encode_batch(chunk)?)?)?;
    }
    Ok(out.len())
}

/// What the local store holds, as import decides (overlaid with decisions not yet
/// written, since everything is appended in one transaction at the end).
struct View<'a> {
    store: &'a Store,
    sources: HashMap<String, bool>,
    items: HashMap<String, Option<Item>>,
    links: HashSet<(String, String, String)>,
}

impl View<'_> {
    fn has_source(&mut self, id: &Id) -> Result<bool> {
        if let Some(b) = self.sources.get(&id.0) {
            return Ok(*b);
        }
        let b = self.store.get_source(id)?.is_some();
        self.sources.insert(id.0.clone(), b);
        Ok(b)
    }
    fn item(&mut self, id: &Id) -> Result<Option<Item>> {
        if let Some(i) = self.items.get(&id.0) {
            return Ok(i.clone());
        }
        let i = self.store.get_item(id)?;
        self.items.insert(id.0.clone(), i.clone());
        Ok(i)
    }
    fn has_link(&self, l: &anamnesis_core::Link) -> Result<bool> {
        let k = (l.from.0.clone(), l.to.0.clone(), format!("{:?}", l.kind));
        if self.links.contains(&k) {
            return Ok(true);
        }
        Ok(self
            .store
            .links_from(&l.from)?
            .iter()
            .any(|x| x.to == l.to && x.kind == l.kind))
    }
}

type Imported = (usize, usize, Vec<String>, usize);

fn import(store: &Store, storage: &dyn Storage, key: &Key, st: &mut SyncState) -> Result<Imported> {
    let files = storage.list(&format!("{DEVICES_DIR}/"))?;
    let mut by_device: BTreeMap<String, Vec<(u64, u64, String)>> = BTreeMap::new();
    let mut device_ids: BTreeSet<String> = BTreeSet::new();
    for f in &files {
        if let Some(rest) = f.strip_prefix(&format!("{DEVICES_DIR}/"))
            && let Some((dev, _)) = rest.split_once('/')
        {
            device_ids.insert(dev.to_string());
        }
        if let Some((dev, a, b)) = parse_batch(f) {
            by_device.entry(dev).or_default().push((a, b, f.clone()));
        }
    }
    let mut errors = Vec::new();
    let mut incoming: Vec<SyncEvent> = Vec::new();
    for dev in device_ids.iter().filter(|d| **d != st.device_id) {
        let rec: Option<DeviceRecord> = get_sealed(storage, key, &device_file(dev)).unwrap_or(None);
        let unl: Option<UnlinkRecord> =
            get_sealed(storage, key, &unlinked_file(dev)).unwrap_or(None);
        let r = st
            .remotes
            .entry(dev.clone())
            .or_insert_with(|| RemoteState {
                dseq: 0,
                head: GENESIS.to_string(),
                name: String::new(),
                platform: String::new(),
                last_sync_at: None,
                unlinked: None,
                error: None,
            });
        if let Some(rec) = &rec {
            r.name = rec.name.clone();
            r.platform = rec.platform.clone();
            r.last_sync_at = Some(rec.last_sync_at);
        }
        if unl.is_some() {
            r.unlinked = unl;
        }
        let limit = r
            .unlinked
            .as_ref()
            .map(|u| u.up_to_dseq)
            .unwrap_or(u64::MAX);
        let mut batches = by_device.remove(dev).unwrap_or_default();
        batches.sort();
        let mut dseq = r.dseq;
        let mut head = r.head.clone();
        let mut got = Vec::new();
        let mut err = None;
        for (_, last, path) in batches {
            if last <= dseq || dseq >= limit {
                continue;
            }
            let res = storage
                .get(&path)?
                .ok_or_else(|| SyncError::Io(format!("{path} vanished")))
                .and_then(|b| open(key, &path, &b))
                .and_then(|p| decode_batch(&p));
            let evs = match res {
                Ok(e) => e,
                Err(e) => {
                    err = Some(e.to_string());
                    break;
                }
            };
            let fresh: Vec<SyncEvent> = evs
                .into_iter()
                .filter(|e| e.dseq > dseq && e.dseq <= limit)
                .collect();
            match verify_chain(dev, dseq, head.clone(), &fresh) {
                Ok((d, h)) => {
                    dseq = d;
                    head = h;
                    got.extend(fresh);
                }
                Err(e) => {
                    err = Some(e.to_string());
                    break;
                }
            }
        }
        r.error = err.clone();
        if let Some(e) = err {
            let name = if r.name.is_empty() {
                dev.clone()
            } else {
                r.name.clone()
            };
            errors.push(format!("{name}: {e}"));
        }
        r.dseq = dseq;
        r.head = head;
        incoming.extend(got);
    }
    let devices = device_ids.len();
    incoming.sort_by(|a, b| (a.hlc, &a.device, a.dseq).cmp(&(b.hlc, &b.device, b.dseq)));
    let n = incoming.len();
    let mut view = View {
        store,
        sources: HashMap::new(),
        items: HashMap::new(),
        links: HashSet::new(),
    };
    let mut to_append: Vec<(Actor, EventKind, Timestamp)> = Vec::new();
    let mut new_conflicts = 0;
    for ev in incoming {
        st.clock.observe(ev.hlc);
        if let Some(c) = apply_one(&mut view, st, ev, &mut to_append)? {
            new_conflicts += c;
        }
    }
    if let Some(range) = store.append_received(to_append)? {
        st.add_received(range);
    }
    Ok((n, new_conflicts, errors, devices))
}

/// Decide what one received event does here. Returns `Some(1)` when it opened a conflict.
fn apply_one(
    view: &mut View<'_>,
    st: &mut SyncState,
    ev: SyncEvent,
    out: &mut Vec<(Actor, EventKind, Timestamp)>,
) -> Result<Option<usize>> {
    let SyncEvent {
        device,
        hlc,
        at,
        actor,
        kind,
        parents,
        ..
    } = ev;
    if let Some(k) = lww_key(&kind) {
        if st.lww.get(&k).is_some_and(|h| *h >= hlc) {
            return Ok(None); // an older write than one already applied
        }
        st.lww.insert(k, hlc);
    }
    match &kind {
        EventKind::SourceAdded { source } => {
            if view.has_source(&source.id)? {
                return Ok(None);
            }
            view.sources.insert(source.id.0.clone(), true);
            st.foreign_sources.insert(source.id.0.clone());
        }
        EventKind::SourceUpdated { source } => {
            if !view.has_source(&source.id)? {
                return Ok(None);
            }
        }
        EventKind::SourceRemoved { source_id } => {
            if !view.has_source(source_id)? {
                return Ok(None);
            }
            view.sources.insert(source_id.0.clone(), false);
        }
        EventKind::LinkAdded { link } => {
            if view.has_link(link)? {
                return Ok(None);
            }
            view.links.insert((
                link.from.0.clone(),
                link.to.0.clone(),
                format!("{:?}", link.kind),
            ));
        }
        _ => {}
    }
    if let Some((id, version, new_item)) = item_event(&kind) {
        let new_item = new_item.cloned();
        let current = view.item(&id)?;
        let mut heads = match st.items.remove(&id.0) {
            Some(h) => h,
            None => match &current {
                // known here but never tracked (a device that joined with a brain): its
                // present version is the one head
                Some(it) => vec![Head {
                    version: if it.deleted {
                        deleted_version(&id)
                    } else {
                        item_version(it)
                    },
                    hlc: Hlc::default(),
                    device: st.device_id.clone(),
                }],
                None => vec![],
            },
        };
        if heads.iter().any(|h| h.version == version) {
            // the same version is already here: drop what it replaces, and show it if it
            // now stands alone (a conflict settled on another device in its favour)
            heads.retain(|h| h.version == version || !parents.contains(&h.version));
            let shown = current.as_ref().map(|c| {
                if c.deleted {
                    deleted_version(&id)
                } else {
                    item_version(c)
                }
            });
            if heads.len() == 1 && shown.as_deref() != Some(version.as_str()) {
                view.items
                    .insert(id.0.clone(), materialise(&kind, current.as_ref(), new_item));
                out.push((actor, kind, at));
            }
            settle(st, &id, heads);
            return Ok(None);
        }
        let fast_forward = heads.iter().all(|h| parents.contains(&h.version));
        if fast_forward {
            st.items.insert(
                id.0.clone(),
                vec![Head {
                    version,
                    hlc,
                    device: device.clone(),
                }],
            );
            st.conflicts.retain(|c| c.item_id != id);
            view.items
                .insert(id.0.clone(), materialise(&kind, current.as_ref(), new_item));
            out.push((actor, kind, at));
            return Ok(None);
        }
        // concurrent: keep every version that this one does not replace
        let mut contents: HashMap<String, (Option<Item>, String)> = HashMap::new();
        if let Some(c) = st.conflicts.iter().find(|c| c.item_id == id) {
            for v in &c.versions {
                contents.insert(v.version.clone(), (v.item.clone(), v.device.clone()));
            }
        }
        let shown_before = heads
            .iter()
            .max_by_key(|h| h.hlc)
            .map(|h| h.version.clone());
        if let (Some(sv), Some(cur)) = (&shown_before, &current) {
            contents
                .entry(sv.clone())
                .or_insert_with(|| ((!cur.deleted).then(|| cur.clone()), st.device_id.clone()));
        }
        let incoming_item = materialise(&kind, current.as_ref(), new_item);
        contents.insert(
            version.clone(),
            (incoming_item.clone().filter(|i| !i.deleted), device.clone()),
        );
        heads.retain(|h| !parents.contains(&h.version));
        heads.push(Head {
            version: version.clone(),
            hlc,
            device: device.clone(),
        });
        let winner = heads
            .iter()
            .max_by_key(|h| (h.hlc, h.device.clone()))
            .cloned()
            .expect("a head");
        if winner.version == version {
            view.items.insert(id.0.clone(), incoming_item);
            out.push((actor, kind, at));
        }
        let versions: Vec<ConflictVersion> = heads
            .iter()
            .map(|h| ConflictVersion {
                version: h.version.clone(),
                hlc: h.hlc,
                device: h.device.clone(),
                device_name: st.device_label(&h.device),
                item: contents.get(&h.version).and_then(|c| c.0.clone()),
                shown: h.version == winner.version,
            })
            .collect();
        let was_open = st.conflicts.iter().any(|c| c.item_id == id);
        st.conflicts.retain(|c| c.item_id != id);
        if versions.len() > 1 {
            st.conflicts.push(Conflict {
                id: format!("{}-{}", id, &winner.version[..winner.version.len().min(8)]),
                item_id: id.clone(),
                detected_at: chrono::Utc::now(),
                versions,
            });
        }
        st.items.insert(id.0.clone(), heads);
        return Ok(Some(usize::from(!was_open)));
    }
    out.push((actor, kind, at));
    Ok(None)
}

fn settle(st: &mut SyncState, id: &Id, heads: Vec<Head>) {
    if heads.len() <= 1 {
        st.conflicts.retain(|c| &c.item_id != id);
    }
    st.items.insert(id.0.clone(), heads);
}

/// The item as it stands after `kind` (None when deleted or unknown).
fn materialise(kind: &EventKind, current: Option<&Item>, new_item: Option<Item>) -> Option<Item> {
    match kind {
        EventKind::ItemDeleted { .. } => current.map(|c| Item {
            deleted: true,
            ..c.clone()
        }),
        _ => new_item,
    }
}

/// Settle a conflict on this device: the chosen version becomes the item (an
/// `ItemUpdated` that replaces every version, so the other devices settle too).
pub fn resolve(
    store: &Store,
    st: &mut SyncState,
    conflict_id: &str,
    how: &Resolution,
) -> Result<Option<Item>> {
    let c = st
        .conflicts
        .iter()
        .find(|c| c.id == conflict_id)
        .cloned()
        .ok_or_else(|| SyncError::Invalid(format!("no conflict {conflict_id}")))?;
    let current = store.get_item(&c.item_id)?;
    let (keep, others): (Option<&ConflictVersion>, Vec<&ConflictVersion>) = match how {
        Resolution::One { version } => {
            let k = c
                .versions
                .iter()
                .find(|v| &v.version == version)
                .ok_or_else(|| SyncError::Invalid(format!("no version {version}")))?;
            (Some(k), vec![])
        }
        Resolution::All => {
            let k = c.versions.iter().find(|v| v.shown);
            (k, c.versions.iter().filter(|v| !v.shown).collect())
        }
    };
    let now = chrono::Utc::now();
    let mut result = None;
    match keep.and_then(|k| k.item.clone()) {
        Some(mut it) => {
            it.modified_at = now;
            it.deleted = false;
            store.append(Actor::User, EventKind::ItemUpdated { item: it.clone() })?;
            result = Some(it);
        }
        None => {
            if current.as_ref().is_some_and(|i| !i.deleted) {
                store.append(
                    Actor::User,
                    EventKind::ItemDeleted {
                        item_id: c.item_id.clone(),
                    },
                )?;
            } else if let Some(mut it) = current.clone() {
                // already deleted here: record the choice so the other devices settle
                it.modified_at = now;
                store.append(Actor::User, EventKind::ItemUpdated { item: it })?;
            }
        }
    }
    for o in others {
        if let Some(it) = &o.item {
            let copy = Item {
                id: Id::new(),
                external_id: format!("{}#from-{}", it.external_id, o.device),
                title: format!("{} (from {})", it.title, o.device_name),
                modified_at: now,
                ..it.clone()
            };
            store.append(Actor::User, EventKind::ItemCreated { item: copy })?;
        }
    }
    st.conflicts.retain(|x| x.id != conflict_id);
    Ok(result)
}

/// Mark another device as unlinked: its later batches are ignored by every device, and
/// it stops syncing when it next sees the mark. (It still holds the key: to lock a lost
/// device out for good, change the passphrase.)
pub fn unlink(storage: &dyn Storage, key: &Key, st: &mut SyncState, device_id: &str) -> Result<()> {
    if device_id == st.device_id {
        return Err(SyncError::Invalid(
            "this is the device itself: stop sync here instead".into(),
        ));
    }
    let r = st
        .remotes
        .get_mut(device_id)
        .ok_or_else(|| SyncError::Invalid(format!("no device {device_id}")))?;
    let rec = UnlinkRecord {
        by_device: st.device_id.clone(),
        by_name: st.device_name.clone(),
        at: chrono::Utc::now(),
        up_to_dseq: r.dseq,
    };
    put_sealed(storage, key, &unlinked_file(device_id), &rec)?;
    r.unlinked = Some(rec);
    Ok(())
}

// ---------------------------------------------------------------------------------------
// inbox

/// The source inbox notes belong to: one id per vault, the same on every device.
pub fn inbox_source_id(vault_id: &str) -> Id {
    Id(format!("inbox-{}", &vault_id[..vault_id.len().min(16)]))
}

fn is_text_note(path: &str) -> bool {
    let lower = path.to_lowercase();
    [".md", ".markdown", ".txt"]
        .iter()
        .any(|e| lower.ends_with(e))
}

/// Title from frontmatter `title:`, a first `# heading`, the first line, or the file name.
fn title_and_body(name: &str, text: &str) -> (String, String) {
    let text = text.trim_start_matches('\u{feff}');
    let mut body = text;
    let mut title = None;
    if let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        && let Some(end) = rest.find("\n---")
    {
        for line in rest[..end].lines() {
            if let Some(t) = line.strip_prefix("title:") {
                title = Some(t.trim().trim_matches(['"', '\'']).to_string());
            }
        }
        body = rest[end + 4..].trim_start_matches(['-', '\r', '\n']);
    }
    let first = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let title = title.filter(|t| !t.is_empty()).unwrap_or_else(|| {
        if let Some(h) = first.strip_prefix('#') {
            h.trim_start_matches('#').trim().to_string()
        } else if !first.is_empty() {
            first.chars().take(80).collect()
        } else {
            name.rsplit_once('.')
                .map(|x| x.0)
                .unwrap_or(name)
                .to_string()
        }
    });
    (title, body.to_string())
}

fn import_inbox(store: &Store, storage: &dyn Storage, st: &SyncState) -> Result<usize> {
    let files: Vec<String> = storage
        .list(&format!("{INBOX_DIR}/"))?
        .into_iter()
        .filter(|f| !f.starts_with(&format!("{INBOX_IMPORTED}/")))
        .filter(|f| is_text_note(f))
        .collect();
    if files.is_empty() {
        return Ok(0);
    }
    let src_id = inbox_source_id(&st.vault_id);
    if store.get_source(&src_id)?.is_none() {
        store.add_source(Source {
            id: src_id.clone(),
            connector: "inbox".into(),
            name: "sync inbox".into(),
            config: serde_json::json!({}),
            default_origin: Origin::OwnerWritten,
            default_privacy: Privacy::CloudOk,
            cursor: None,
            created_at: chrono::Utc::now(),
        })?;
    }
    let day = chrono::Utc::now().date_naive().to_string();
    let mut n = 0;
    for f in files {
        let Some(bytes) = storage.get(&f)? else {
            continue;
        };
        let name = f.rsplit('/').next().unwrap_or(&f).to_string();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        // move it out first, so another device syncing now does not take it in too
        let mut dest = format!("{INBOX_IMPORTED}/{day}/{name}");
        let mut k = 1;
        while storage.get(&dest)?.is_some() {
            k += 1;
            dest = format!("{INBOX_IMPORTED}/{day}/{k}-{name}");
        }
        if storage.rename(&f, &dest).is_err() {
            continue; // another device took it
        }
        if text.trim().is_empty() {
            continue;
        }
        let (title, body) = title_and_body(&name, &text);
        let doc = Document {
            external_id: dest.clone(),
            uri: None,
            kind: ItemKind::Note,
            title,
            refs: anamnesis_core::refs::extract_refs(&body),
            body,
            origin: Origin::OwnerWritten,
            created_at: None,
            modified_at: None,
            tags: vec![],
            meta: serde_json::json!({ "sync_inbox": { "file": name, "device": st.device_id } }),
        };
        if let UpsertOutcome::Created(_) =
            store.upsert_document_as(&src_id, doc, Actor::Connector("sync:inbox".into()))?
        {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_titles() {
        assert_eq!(title_and_body("a.md", "# Groceries\n- milk").0, "Groceries");
        assert_eq!(
            title_and_body("a.md", "---\ntitle: \"On walks\"\n---\nbody").0,
            "On walks"
        );
        assert_eq!(title_and_body("a.md", "---\ntitle: x\n---\nbody").1, "body");
        assert_eq!(title_and_body("walk notes.txt", "  \n").0, "walk notes");
        assert_eq!(
            title_and_body("a.txt", "first line\nsecond").0,
            "first line"
        );
    }
}
