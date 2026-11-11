//! The exec plugin runtime with tiny shell-script plugins (offline, unix only).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use anamnesis_connectors::connectors::exec::ExecConnector;
use anamnesis_connectors::{Connector, ConnectorError, Rename, load_plugin, validated_check, validated_list};
use anamnesis_core::{ItemKind, Origin};
use serde_json::json;

const PLUGIN: &str = r#"#!/bin/sh
# A toy connector: one document on a full sync, a delete and a rename afterwards.
read -r req
case "$req" in
  *'"op":"check"'*)
    echo '{"type":"check","ok":true,"message":"toy plugin ready"}' ;;
  *'"cursor":"c1"'*)
    echo '{"type":"delete","external_id":"gone"}'
    echo '{"type":"rename","from":"a","to":"a2"}'
    echo '{"type":"done","next_cursor":"c2"}' ;;
  *'"op":"list"'*)
    echo "plugin diagnostics go to stderr" >&2
    echo '{"type":"log","level":"info","message":"listing"}'
    echo ''
    echo '{"type":"upsert","document":{"external_id":"a","title":"A","body":"alpha","tags":["x"],"meta":{"n":1}}}'
    echo '{"type":"upsert","document":{"external_id":"b","title":"B","body":"beta","kind":"quote","origin":"external","created_at":"2024-01-01T00:00:00Z"}}'
    echo '{"type":"done","next_cursor":"c1","has_more":false}' ;;
esac
"#;

fn write_plugin(dir: &Path, id: &str, script: &str) -> std::path::PathBuf {
    let pdir = dir.join(id);
    std::fs::create_dir_all(&pdir).unwrap();
    let exe = pdir.join("plugin.sh");
    std::fs::write(&exe, script).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let manifest = json!({
        "id": id,
        "name": "toy",
        "version": "0.1.0",
        "api_version": 1,
        "runtime": "exec",
        "entry": "plugin.sh",
        "config_schema": {"type": "object", "properties": {"greeting": {"type": "string"}}, "additionalProperties": false},
        "capabilities": ["check", "incremental", "deletes", "renames"],
        "network_hosts": ["api.example.com"],
        "fs_paths": ["~/toy"],
        "default_origin": "owner_written",
        "default_kind": "note"
    });
    std::fs::write(pdir.join("anamnesis-plugin.json"), manifest.to_string()).unwrap();
    pdir
}

#[tokio::test]
async fn exec_plugin_check_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_plugin(tmp.path(), "toy", PLUGIN);
    let c = load_plugin(&dir).unwrap();
    let m = c.manifest();
    assert_eq!(m.network_hosts, vec!["api.example.com"]);
    assert_eq!(m.fs_paths, vec!["~/toy"]);

    let check = validated_check(c.as_ref(), &json!({})).await.unwrap();
    assert!(check.ok);
    assert_eq!(check.message, "toy plugin ready");

    let b = validated_list(c.as_ref(), &json!({"greeting": "hi"}), None).await.unwrap();
    assert_eq!(b.upserts.len(), 2);
    let a = &b.upserts[0];
    assert_eq!((a.external_id.as_str(), a.title.as_str(), a.body.as_str()), ("a", "A", "alpha"));
    assert_eq!(a.kind, ItemKind::Note, "manifest default kind");
    assert_eq!(a.origin, Origin::OwnerWritten, "manifest default origin");
    assert_eq!(a.meta["n"], 1);
    assert_eq!(a.meta["connector"], "toy");
    let bdoc = &b.upserts[1];
    assert_eq!(bdoc.kind, ItemKind::Quote);
    assert_eq!(bdoc.origin, Origin::External);
    assert!(bdoc.created_at.is_some());
    assert_eq!(b.next_cursor.as_deref(), Some("c1"));

    let b2 = validated_list(c.as_ref(), &json!({}), Some("c1")).await.unwrap();
    assert_eq!(b2.deletes, vec!["gone"]);
    assert_eq!(b2.renames, vec![Rename { from: "a".into(), to: "a2".into() }]);
    assert_eq!(b2.next_cursor.as_deref(), Some("c2"));

    // Config is validated against the plugin's schema before the process runs.
    assert!(matches!(
        validated_list(c.as_ref(), &json!({"greeting": 1}), None).await,
        Err(ConnectorError::InvalidConfig(_))
    ));
}

#[tokio::test]
async fn exec_plugin_failures() {
    let tmp = tempfile::tempdir().unwrap();

    let slow = write_plugin(tmp.path(), "slow", "#!/bin/sh\nread -r req\nexec sleep 10\n");
    let c = ExecConnector::load(&slow).unwrap().with_timeout(Duration::from_millis(300));
    let t0 = std::time::Instant::now();
    assert!(matches!(c.list(&json!({}), None).await, Err(ConnectorError::Timeout(_))));
    assert!(t0.elapsed() < Duration::from_secs(5));
    let check = c.check(&json!({})).await.unwrap();
    assert!(!check.ok, "a timed-out check is a failed check, not an error");

    let err = write_plugin(tmp.path(), "err", "#!/bin/sh\necho '{\"type\":\"error\",\"message\":\"token expired\"}'\n");
    let c = ExecConnector::load(&err).unwrap();
    match c.list(&json!({}), None).await {
        Err(ConnectorError::Plugin(m)) => assert!(m.contains("token expired"), "{m}"),
        other => panic!("{other:?}"),
    }

    let crash = write_plugin(tmp.path(), "crash", "#!/bin/sh\necho boom >&2\nexit 3\n");
    match ExecConnector::load(&crash).unwrap().list(&json!({}), None).await {
        Err(ConnectorError::Plugin(m)) => assert!(m.contains("boom"), "{m}"),
        other => panic!("{other:?}"),
    }

    let nodone = write_plugin(tmp.path(), "nodone", "#!/bin/sh\necho '{\"type\":\"delete\",\"external_id\":\"x\"}'\n");
    assert!(matches!(ExecConnector::load(&nodone).unwrap().list(&json!({}), None).await, Err(ConnectorError::Plugin(_))));

    let garbage = write_plugin(tmp.path(), "garbage", "#!/bin/sh\necho 'not json'\n");
    assert!(matches!(ExecConnector::load(&garbage).unwrap().list(&json!({}), None).await, Err(ConnectorError::Plugin(_))));

    // Reserved and built-in ids cannot be claimed by plugins.
    let reserved = write_plugin(tmp.path(), "obsidian", PLUGIN);
    assert!(matches!(load_plugin(&reserved), Err(ConnectorError::InvalidManifest(_))));
}
