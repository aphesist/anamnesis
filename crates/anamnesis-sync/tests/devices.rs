//! Two (or three) simulated devices on one sync folder.

use anamnesis_core::{
    Actor, EventKind, Id, ItemKind, ItemPatch, LinkKind, NewNote, Origin, Store, ThreadStatus,
};
use anamnesis_sync::engine::{SyncState, item_version};
use anamnesis_sync::{
    FolderStorage, KdfParams, Key, Resolution, Storage, SyncError, create_or_open, resolve, sync,
    unlink,
};

const PASS: &str = "a long walk at dawn";

struct Device {
    store: Store,
    state: SyncState,
    key: Key,
}

fn device(storage: &dyn Storage, name: &str) -> Device {
    let (vault, key, _) = create_or_open(storage, PASS, KdfParams::insecure_for_tests()).unwrap();
    Device {
        store: Store::open_in_memory().unwrap(),
        state: SyncState::new(&vault, name),
        key,
    }
}

impl Device {
    fn sync(&mut self, storage: &dyn Storage) -> anamnesis_sync::SyncReport {
        sync(&self.store, storage, &self.key, &mut self.state).unwrap()
    }
    fn note(&self, title: &str, body: &str) -> Id {
        self.store
            .create_note(
                NewNote {
                    title: title.into(),
                    body: body.into(),
                    kind: None,
                    tags: vec![],
                    origin: None,
                },
                Actor::User,
            )
            .unwrap()
            .id
    }
    fn edit(&self, id: &Id, body: &str) {
        self.store
            .update_item(
                id,
                ItemPatch {
                    body: Some(body.into()),
                    ..Default::default()
                },
                Actor::User,
            )
            .unwrap();
    }
    fn body(&self, id: &Id) -> Option<String> {
        self.store
            .get_item(id)
            .unwrap()
            .filter(|i| !i.deleted)
            .map(|i| i.body)
    }
}

/// Wall clocks: make sure the next event is at a later millisecond.
fn later() {
    std::thread::sleep(std::time::Duration::from_millis(3));
}

#[test]
fn notes_links_and_edits_travel_both_ways_as_ciphertext_only() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    let x = a.note("on walking", "walking is how the thinking happens");
    let y = a.note("on reading", "reading after walking, never before");
    a.store
        .add_link(&x, &y, LinkKind::Manual, Actor::User)
        .unwrap();
    let r = a.sync(&fs);
    assert!(r.exported > 0);

    // the folder holds a plain vault.json and sealed files only
    let vault = std::fs::read_to_string(dir.path().join("vault.json")).unwrap();
    assert!(vault.contains("argon2id") && !vault.contains("walking"));
    for f in fs.list("").unwrap() {
        let b = fs.get(&f).unwrap().unwrap();
        assert!(
            !b.windows(7).any(|w| w == b"walking"),
            "{f} holds plaintext"
        );
        if f != "vault.json" {
            assert_eq!(&b[..4], b"ANS1", "{f} is sealed");
        }
    }

    // a second device joins with the passphrase and gets everything, with the same ids
    let mut b = device(&fs, "phone");
    assert_eq!(b.state.vault_id, a.state.vault_id);
    let r = b.sync(&fs);
    assert!(r.imported >= 4, "{r:?}");
    assert_eq!(b.body(&x).unwrap(), "walking is how the thinking happens");
    assert!(b.store.links_from(&x).unwrap().iter().any(|l| l.to == y));
    // a's sources are foreign on b (their connectors run on a)
    let src = b.store.get_item(&x).unwrap().unwrap().source_id;
    assert!(b.state.foreign_sources.contains(&src.0));

    // an edit on b reaches a
    later();
    b.edit(&x, "walking is how the thinking happens, slowly");
    b.sync(&fs);
    a.sync(&fs);
    assert_eq!(
        a.body(&x).unwrap(),
        "walking is how the thinking happens, slowly"
    );
    assert!(a.state.conflicts.is_empty() && b.state.conflicts.is_empty());
    // syncing again changes nothing and re-exports nothing received
    let r = a.sync(&fs);
    assert_eq!((r.exported, r.imported), (0, 0), "{r:?}");
    let r = b.sync(&fs);
    assert_eq!((r.exported, r.imported), (0, 0), "{r:?}");

    // a wrong passphrase does not open the vault
    assert!(matches!(
        create_or_open(&fs, "a short walk at dusk", KdfParams::insecure_for_tests()),
        Err(SyncError::WrongPassphrase)
    ));
    // the local logs still verify (received events are chained like local ones)
    assert!(a.store.verify_chain().is_ok() && b.store.verify_chain().is_ok());
}

#[test]
fn concurrent_edits_keep_both_versions_and_ask() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    let x = a.note("plan", "first draft");
    a.sync(&fs);
    let mut b = device(&fs, "phone");
    b.sync(&fs);

    // both edit without seeing each other; b's edit is later
    later();
    a.edit(&x, "draft from the laptop");
    later();
    b.edit(&x, "draft from the phone");
    a.sync(&fs);
    let rb = b.sync(&fs);
    assert_eq!(rb.new_conflicts, 1);
    let ra = a.sync(&fs);
    assert_eq!(ra.new_conflicts, 1);

    // both devices show the same version (the later clock) and keep both
    assert_eq!(a.body(&x).unwrap(), "draft from the phone");
    assert_eq!(b.body(&x).unwrap(), "draft from the phone");
    for d in [&a, &b] {
        assert_eq!(d.state.conflicts.len(), 1);
        let c = &d.state.conflicts[0];
        let mut bodies: Vec<String> = c
            .versions
            .iter()
            .map(|v| v.item.as_ref().unwrap().body.clone())
            .collect();
        bodies.sort();
        assert_eq!(
            bodies,
            vec!["draft from the laptop", "draft from the phone"]
        );
        assert_eq!(c.versions.iter().filter(|v| v.shown).count(), 1);
    }

    // the user keeps the laptop's version on the phone; the laptop settles too
    let c = b.state.conflicts[0].clone();
    let laptop = c
        .versions
        .iter()
        .find(|v| v.item.as_ref().unwrap().body == "draft from the laptop")
        .unwrap();
    resolve(
        &b.store,
        &mut b.state,
        &c.id,
        &Resolution::One {
            version: laptop.version.clone(),
        },
    )
    .unwrap();
    assert!(b.state.conflicts.is_empty());
    b.sync(&fs);
    a.sync(&fs);
    assert_eq!(a.body(&x).unwrap(), "draft from the laptop");
    assert_eq!(b.body(&x).unwrap(), "draft from the laptop");
    assert!(a.state.conflicts.is_empty(), "{:?}", a.state.conflicts);
    let va = item_version(&a.store.get_item(&x).unwrap().unwrap());
    let vb = item_version(&b.store.get_item(&x).unwrap().unwrap());
    assert_eq!(va, vb);
}

#[test]
fn keep_both_makes_a_second_note_and_deletes_conflict_too() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    let x = a.note("list", "one");
    let z = a.note("old", "to be deleted or kept");
    a.sync(&fs);
    let mut b = device(&fs, "phone");
    b.sync(&fs);
    later();
    a.edit(&x, "one, two");
    a.store
        .append(Actor::User, EventKind::ItemDeleted { item_id: z.clone() })
        .unwrap();
    later();
    b.edit(&x, "one, three");
    b.edit(&z, "kept and edited on the phone");
    a.sync(&fs);
    b.sync(&fs);
    a.sync(&fs);
    assert_eq!(a.state.conflicts.len(), 2);
    // the edit of z is later than its deletion: z is shown, edited, on both
    assert_eq!(a.body(&z).unwrap(), "kept and edited on the phone");
    let cz = a.state.conflicts.iter().find(|c| c.item_id == z).unwrap();
    assert!(
        cz.versions.iter().any(|v| v.item.is_none()),
        "the deletion is a version"
    );

    let cx = a
        .state
        .conflicts
        .iter()
        .find(|c| c.item_id == x)
        .unwrap()
        .clone();
    resolve(&a.store, &mut a.state, &cx.id, &Resolution::All).unwrap();
    a.sync(&fs);
    b.sync(&fs);
    for d in [&a, &b] {
        assert_eq!(d.body(&x).unwrap(), "one, three");
        let copies: Vec<_> = d
            .store
            .list_items(&Default::default())
            .unwrap()
            .into_iter()
            .filter(|i| i.title.starts_with("list (from laptop)"))
            .collect();
        assert_eq!(copies.len(), 1, "one copy of the other version");
        assert_eq!(copies[0].body, "one, two");
        assert!(d.state.conflicts.iter().all(|c| c.item_id != x));
    }
}

#[test]
fn tampering_and_gaps_are_detected_not_imported() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    a.note("one", "first");
    a.sync(&fs);
    later();
    a.note("two", "second");
    a.sync(&fs);
    let batches: Vec<String> = fs
        .list("devices/")
        .unwrap()
        .into_iter()
        .filter(|f| f.contains("/batches/"))
        .collect();
    assert_eq!(batches.len(), 2);

    // a flipped byte: the file fails to open, nothing from it is imported
    let mut bytes = fs.get(&batches[0]).unwrap().unwrap();
    let n = bytes.len() / 2;
    bytes[n] ^= 0x40;
    let good = fs.get(&batches[0]).unwrap().unwrap();
    fs.put(&batches[0], &bytes).unwrap();
    let mut b = device(&fs, "phone");
    let r = b.sync(&fs);
    assert_eq!(r.imported, 0);
    assert!(
        r.errors
            .iter()
            .any(|e| e.contains("laptop") && e.contains("modified")),
        "{r:?}"
    );
    assert!(b.store.list_items(&Default::default()).unwrap().is_empty());

    // a missing batch (a gap in the chain): the later one is not imported either
    fs.delete(&batches[0]).unwrap();
    let mut c = device(&fs, "tablet");
    let r = c.sync(&fs);
    assert_eq!(r.imported, 0);
    assert!(r.errors.iter().any(|e| e.contains("follows")), "{r:?}");

    // someone with the key rewrites an event: sealed fine, but the chain breaks
    let mut evs = anamnesis_sync::format::decode_batch(
        &anamnesis_sync::crypto::open(&a.key, &batches[0], &good).unwrap(),
    )
    .unwrap();
    if let EventKind::ItemCreated { item } = &mut evs.last_mut().unwrap().kind {
        item.body = "forged".into();
    } else {
        panic!("expected an item");
    }
    let forged = anamnesis_sync::format::encode_batch(&evs).unwrap();
    fs.put(
        &batches[0],
        &anamnesis_sync::crypto::seal(&a.key, &batches[0], &forged).unwrap(),
    )
    .unwrap();
    let mut d = device(&fs, "desktop");
    let r = d.sync(&fs);
    assert!(
        r.errors.iter().any(|e| e.contains("does not link")),
        "{r:?}"
    );
    assert!(
        d.store
            .list_items(&Default::default())
            .unwrap()
            .iter()
            .all(|i| i.body != "forged")
    );

    // restored: the devices catch up
    fs.put(&batches[0], &good).unwrap();
    let r = b.sync(&fs);
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(b.store.list_items(&Default::default()).unwrap().len(), 2);
}

#[test]
fn inbox_files_become_the_owners_notes_on_every_device() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    let mut b = device(&fs, "phone");
    std::fs::create_dir_all(dir.path().join("inbox")).unwrap();
    std::fs::write(
        dir.path().join("inbox/shopping thoughts.md"),
        "# on errands\nwalk to the market, not drive",
    )
    .unwrap();
    std::fs::write(dir.path().join("inbox/photo.jpg"), b"\xff\xd8").unwrap();
    let r = a.sync(&fs);
    assert_eq!(r.inbox, 1);
    let items = a.store.list_items(&Default::default()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "on errands");
    assert_eq!(items[0].origin, Origin::OwnerWritten);
    assert_eq!(items[0].kind, ItemKind::Note);
    // the file moved out of the inbox; other files are left alone
    let left = fs.list("inbox/").unwrap();
    assert!(left.contains(&"inbox/photo.jpg".to_string()));
    assert!(
        left.iter()
            .any(|f| f.starts_with("inbox/imported/") && f.ends_with("shopping thoughts.md"))
    );
    // b gets it as a note, and does not take the file in again
    let r = b.sync(&fs);
    assert_eq!(r.inbox, 0);
    let bi = b.store.list_items(&Default::default()).unwrap();
    assert_eq!(bi.len(), 1);
    assert_eq!(bi[0].id, items[0].id);
}

#[test]
fn settings_are_last_writer_wins_and_only_the_users_own() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    let mut b = device(&fs, "phone");
    a.store
        .set_setting("theme", serde_json::json!({ "name": "paper" }))
        .unwrap();
    a.store
        .set_setting("routing", serde_json::json!({ "chat": "local" }))
        .unwrap();
    later();
    b.store
        .set_setting("theme", serde_json::json!({ "name": "aphesis" }))
        .unwrap();
    a.sync(&fs);
    b.sync(&fs);
    a.sync(&fs);
    for d in [&a, &b] {
        assert_eq!(
            d.store.get_setting("theme").unwrap().unwrap()["name"],
            "aphesis",
            "{}",
            d.state.device_name
        );
    }
    assert!(
        b.store.get_setting("routing").unwrap().is_none(),
        "routing is per device"
    );
}

#[test]
fn thread_status_follows_the_user() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    let x = a.note("decisions", "I decided to walk every morning.");
    a.sync(&fs);
    let mut b = device(&fs, "phone");
    b.sync(&fs);
    // each device extracts the thread itself; ids agree
    for d in [&a, &b] {
        let it = d.store.get_item(&x).unwrap().unwrap();
        for t in anamnesis_mind_like_extract(&it) {
            d.store
                .upsert_thread(t, Actor::System("mind:threads".into()))
                .unwrap();
        }
    }
    let t = a.store.threads(&Default::default()).unwrap()[0].clone();
    assert_eq!(b.store.threads(&Default::default()).unwrap()[0].id, t.id);
    later();
    a.store
        .set_thread_status(&t.id, ThreadStatus::Resolved, None, Actor::User)
        .unwrap();
    a.sync(&fs);
    b.sync(&fs);
    assert_eq!(
        b.store.get_thread(&t.id).unwrap().unwrap().status,
        ThreadStatus::Resolved
    );
}

/// A stand-in for mind's rule extractor with deterministic ids (this crate does not
/// depend on mind).
fn anamnesis_mind_like_extract(it: &anamnesis_core::Item) -> Vec<anamnesis_core::Thread> {
    let quote = it.body.clone();
    vec![anamnesis_core::Thread {
        id: Id(format!("t-{}", it.id)),
        kind: anamnesis_core::ThreadKind::Decision,
        status: ThreadStatus::Open,
        statement: quote.clone(),
        item_id: it.id.clone(),
        quote,
        resolved_by: None,
        created_at: it.created_at,
        updated_at: it.created_at,
        extractor: "rule:v2".into(),
    }]
}

#[test]
fn an_unlinked_device_stops_and_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FolderStorage::new(dir.path());
    let mut a = device(&fs, "laptop");
    let mut b = device(&fs, "old phone");
    b.note("before", "written before the unlink");
    b.sync(&fs);
    a.sync(&fs);
    assert_eq!(a.store.list_items(&Default::default()).unwrap().len(), 1);
    let bid = b.state.device_id.clone();
    unlink(&fs, &a.key, &mut a.state, &bid).unwrap();
    b.note("after", "written after the unlink");
    match sync(&b.store, &fs, &b.key, &mut b.state) {
        Err(SyncError::Unlinked { by }) => assert_eq!(by, "laptop"),
        other => panic!("{other:?}"),
    }
    a.sync(&fs);
    assert_eq!(a.store.list_items(&Default::default()).unwrap().len(), 1);
    assert!(a.state.remotes[&bid].unlinked.is_some());
    assert!(
        unlink(
            &fs,
            &a.key,
            &mut a.state.clone(),
            &a.state.device_id.clone()
        )
        .is_err()
    );
}
