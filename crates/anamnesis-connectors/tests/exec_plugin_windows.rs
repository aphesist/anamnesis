//! The exec plugin runtime with a tiny batch-file plugin (offline, windows only).
#![cfg(windows)]

use anamnesis_connectors::{load_plugin, validated_check, validated_list};
use serde_json::json;

/// Reads the request line, then answers. `echo` of json is safe in batch as long as
/// the line holds no `&`, `|`, `<`, `>` or `%`.
const PLUGIN: &str = "@echo off\r\n\
set /p req=\r\n\
echo {\"type\":\"check\",\"ok\":true,\"message\":\"batch plugin ready\"}\r\n\
echo {\"type\":\"upsert\",\"document\":{\"external_id\":\"notes/a.md\",\"title\":\"A\",\"body\":\"alpha\"}}\r\n\
echo {\"type\":\"done\",\"next_cursor\":\"c1\"}\r\n";

#[tokio::test]
async fn batch_plugin_check_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let pdir = tmp.path().join("batchy");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(pdir.join("plugin.cmd"), PLUGIN).unwrap();
    let manifest = json!({
        "id": "batchy",
        "name": "batchy",
        "version": "0.1.0",
        "api_version": 1,
        "runtime": "exec",
        "entry": "plugin.cmd",
        "config_schema": {"type": "object"},
        "capabilities": ["check"],
        "default_origin": "owner_written",
        "default_kind": "note"
    });
    std::fs::write(pdir.join("anamnesis-plugin.json"), manifest.to_string()).unwrap();

    let c = load_plugin(&pdir).unwrap();
    let check = validated_check(c.as_ref(), &json!({})).await.unwrap();
    assert!(check.ok, "{}", check.message);
    assert_eq!(check.message, "batch plugin ready");

    let b = validated_list(c.as_ref(), &json!({}), None).await.unwrap();
    assert_eq!(b.upserts.len(), 1);
    assert_eq!(b.upserts[0].external_id, "notes/a.md");
    assert_eq!(b.next_cursor.as_deref(), Some("c1"));
}
