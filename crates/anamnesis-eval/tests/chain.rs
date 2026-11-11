//! Synthetic logs use core's canonical chain format, so they import as a bundle and
//! pass `Store::verify_chain`.

use anamnesis_core::Store;
use anamnesis_core::{BUNDLE_FORMAT, Manifest};
use anamnesis_eval::synthetic::{self, SyntheticConfig};

fn bundle_of(events: &[anamnesis_core::Event]) -> Vec<u8> {
    let manifest = Manifest {
        format: BUNDLE_FORMAT.into(),
        version: 1,
        schema_version: anamnesis_core::SCHEMA_VERSION,
        exported_at: events.last().map(|e| e.at).unwrap_or_default(),
        events: events.len() as u64,
        head_hash: events.last().map(|e| e.hash.clone()),
    };
    let mut out = serde_json::to_vec(&manifest).unwrap();
    out.push(b'\n');
    for ev in events {
        serde_json::to_writer(&mut out, ev).unwrap();
        out.push(b'\n');
    }
    out
}

#[test]
fn synthetic_log_imports_and_verifies() {
    let cfg = SyntheticConfig {
        days: 60,
        ..SyntheticConfig::default()
    };
    let events = synthetic::generate(&cfg);
    assert!(events.len() > 100);

    let store = Store::open_in_memory().unwrap();
    let manifest = store
        .import_bundle(std::io::Cursor::new(bundle_of(&events)))
        .expect("synthetic bundle imports");
    assert_eq!(manifest.events, events.len() as u64);

    let report = store.verify_chain().expect("chain verifies");
    assert_eq!(report.events, events.len() as u64);
    assert_eq!(
        report.head.as_deref(),
        Some(events.last().unwrap().hash.as_str())
    );

    // And the store's own export round-trips back to the same events.
    assert_eq!(store.all_events().unwrap(), events);
}

#[test]
fn tampered_synthetic_log_is_rejected() {
    let cfg = SyntheticConfig {
        days: 20,
        ..SyntheticConfig::default()
    };
    let mut events = synthetic::generate(&cfg);
    let mid = events.len() / 2;
    events[mid].at += chrono::Duration::seconds(1);
    let store = Store::open_in_memory().unwrap();
    assert!(
        store
            .import_bundle(std::io::Cursor::new(bundle_of(&events)))
            .is_err()
    );
    assert_eq!(store.event_count().unwrap(), 0);
}
