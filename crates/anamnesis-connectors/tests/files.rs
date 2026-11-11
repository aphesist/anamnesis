//! Folder and obsidian connectors against on-disk fixtures (offline).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anamnesis_connectors::{Batch, ConnectorError, Rename, builtin, list_all, validated_check, validated_list};
use anamnesis_core::{Document, ItemKind, Origin};
use serde_json::json;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn fixture_vault() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("vault");
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault"), &root);
    (tmp, root)
}

fn doc<'a>(b: &'a Batch, id: &str) -> &'a Document {
    b.upserts.iter().find(|d| d.external_id == id).unwrap_or_else(|| {
        panic!("{id} not in {:?}", b.upserts.iter().map(|d| &d.external_id).collect::<Vec<_>>())
    })
}

#[tokio::test]
async fn obsidian_vault_full_then_incremental_with_rename() {
    let (_tmp, root) = fixture_vault();
    let c = builtin("obsidian").unwrap();
    let config = json!({ "path": root.to_str().unwrap() });

    let check = validated_check(c.as_ref(), &config).await.unwrap();
    assert!(check.ok, "{}", check.message);

    let b = list_all(c.as_ref(), &config, None).await.unwrap();
    let mut ids: Vec<_> = b.upserts.iter().map(|d| d.external_id.as_str()).collect();
    ids.sort();
    // .obsidian/ and .trash/ ignored; attachments are not documents.
    assert_eq!(ids, vec!["2024-03-05.md", "Meno.md", "Recollection.md", "folder/Deep Note.md"]);
    assert!(b.deletes.is_empty() && b.renames.is_empty());

    let r = doc(&b, "Recollection.md");
    assert_eq!(r.title, "On Recollection");
    assert_eq!(r.kind, ItemKind::Note);
    assert_eq!(r.origin, Origin::OwnerWritten);
    assert_eq!(r.tags, vec!["philosophy", "plato", "idea", "memory/long-term"]);
    assert_eq!(r.created_at.unwrap().to_rfc3339(), "2024-03-05T09:30:00+00:00");
    assert!(r.body.starts_with("# Recollection"), "frontmatter stripped");
    assert_eq!(
        r.refs,
        vec![
            "Meno.md",
            "attachments/diagram.svg",
            "Missing Note",
            "folder/Deep Note.md",
            "https://plato.stanford.edu/entries/meno/",
        ]
    );
    assert_eq!(r.meta["aliases"], json!(["anamnesis", "learning as remembering"]));
    let wl = r.meta["wikilinks"].as_array().unwrap();
    assert_eq!(wl[0], json!({"target": "Meno", "heading": "The slave boy", "alias": "the boy", "embed": false, "resolved": "Meno.md"}));
    assert_eq!(wl[1]["embed"], true);
    assert_eq!(wl[2]["resolved"], serde_json::Value::Null);
    assert!(r.meta["obsidian_uri"].as_str().unwrap().starts_with("obsidian://open?vault=vault&file=Recollection"));
    assert!(r.uri.as_deref().unwrap().starts_with("file://"));

    let m = doc(&b, "Meno.md");
    assert_eq!(m.title, "Meno");
    assert_eq!(m.refs, vec!["Recollection.md"]);
    assert_eq!(m.tags, vec!["dialogue"]);

    let d = doc(&b, "folder/Deep Note.md");
    assert_eq!(d.title, "Deep Note");
    assert_eq!(d.kind, ItemKind::Quote, "frontmatter kind");
    assert_eq!(d.tags, vec!["alpha", "beta"]);
    assert_eq!(d.created_at.unwrap().to_rfc3339(), "2023-12-01T00:00:00+00:00");
    assert_eq!(d.refs, vec!["Meno.md"], "relative markdown link resolved");

    assert_eq!(doc(&b, "2024-03-05.md").kind, ItemKind::Journal);

    // Nothing changed → empty batch, same cursor content.
    let cursor = b.next_cursor.clone().unwrap();
    let b2 = validated_list(c.as_ref(), &config, Some(&cursor)).await.unwrap();
    assert!(b2.is_empty(), "{b2:?}");

    // Rewrite a file with identical content: mtime changes, hash does not → still nothing.
    let meno = std::fs::read(root.join("Meno.md")).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(root.join("Meno.md"), &meno).unwrap();
    let b3 = validated_list(c.as_ref(), &config, b2.next_cursor.as_deref()).await.unwrap();
    assert!(b3.is_empty(), "{b3:?}");

    // Move Meno into a folder (same content), edit Deep Note, delete the daily note.
    std::fs::create_dir_all(root.join("Philosophy")).unwrap();
    std::fs::rename(root.join("Meno.md"), root.join("Philosophy/Meno.md")).unwrap();
    std::fs::write(root.join("folder/Deep Note.md"), "---\ntags: [alpha]\n---\nrewritten, longer than before.\n").unwrap();
    std::fs::remove_file(root.join("2024-03-05.md")).unwrap();
    let b4 = validated_list(c.as_ref(), &config, b3.next_cursor.as_deref()).await.unwrap();
    assert_eq!(b4.renames, vec![Rename { from: "Meno.md".into(), to: "Philosophy/Meno.md".into() }]);
    assert_eq!(b4.deletes, vec!["2024-03-05.md"]);
    let moved = doc(&b4, "Philosophy/Meno.md");
    assert_eq!(moved.meta["renamed_from"], "Meno.md");
    assert_eq!(doc(&b4, "folder/Deep Note.md").tags, vec!["alpha"]);
    assert_eq!(b4.upserts.len(), 2);

    // Consumers that cannot re-key items get a delete instead.
    let flat = b4.clone().flatten_renames();
    assert!(flat.renames.is_empty());
    assert!(flat.deletes.contains(&"Meno.md".to_string()));
}

#[tokio::test]
async fn folder_incremental_cursor_and_ignores() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("drafts")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.md"), "# Alpha\n\nfirst #one").unwrap();
    std::fs::write(root.join("b.txt"), "plain text, no #parsing").unwrap();
    std::fs::write(root.join("sub/c.markdown"), "see [[a]]").unwrap();
    std::fs::write(root.join("drafts/d.md"), "ignored").unwrap();
    std::fs::write(root.join("x.tmp.md"), "ignored by glob").unwrap();
    std::fs::write(root.join("image.png"), [0u8, 1, 2]).unwrap();
    std::fs::write(root.join(".hidden.md"), "hidden").unwrap();

    let c = builtin("folder").unwrap();
    let config = json!({ "path": root.to_str().unwrap(), "ignore": ["drafts/", "*.tmp.md"], "kind": "journal" });
    let b = validated_list(c.as_ref(), &config, None).await.unwrap();
    let mut ids: Vec<_> = b.upserts.iter().map(|d| d.external_id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["a.md", "b.txt", "sub/c.markdown"]);
    assert_eq!(doc(&b, "a.md").title, "Alpha");
    assert_eq!(doc(&b, "a.md").kind, ItemKind::Journal);
    let txt = doc(&b, "b.txt");
    assert_eq!(txt.title, "b");
    assert!(txt.tags.is_empty(), "plain text is not parsed as markdown");
    assert_eq!(doc(&b, "sub/c.markdown").refs, vec!["a.md"]);

    // Edit one, add one, delete one.
    std::fs::write(root.join("a.md"), "# Alpha\n\nfirst, now edited #two").unwrap();
    std::fs::write(root.join("sub/new.md"), "new note").unwrap();
    std::fs::remove_file(root.join("b.txt")).unwrap();
    let b2 = validated_list(c.as_ref(), &config, b.next_cursor.as_deref()).await.unwrap();
    let mut ids: Vec<_> = b2.upserts.iter().map(|d| d.external_id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["a.md", "sub/new.md"]);
    assert_eq!(b2.deletes, vec!["b.txt"]);
    assert!(b2.renames.is_empty());
    assert_eq!(doc(&b2, "a.md").tags, vec!["two"]);

    // Two files with identical content vanish and appear: ambiguous → delete + create.
    std::fs::write(root.join("p.md"), "same").unwrap();
    std::fs::write(root.join("q.md"), "same").unwrap();
    let b3 = validated_list(c.as_ref(), &config, b2.next_cursor.as_deref()).await.unwrap();
    std::fs::rename(root.join("p.md"), root.join("p2.md")).unwrap();
    std::fs::rename(root.join("q.md"), root.join("q2.md")).unwrap();
    let b4 = validated_list(c.as_ref(), &config, b3.next_cursor.as_deref()).await.unwrap();
    assert!(b4.renames.is_empty());
    assert_eq!(b4.deletes, vec!["p.md", "q.md"]);
    assert_eq!(b4.upserts.len(), 2);
}

#[tokio::test]
async fn config_is_validated_against_schema() {
    let c = builtin("folder").unwrap();
    match validated_list(c.as_ref(), &json!({ "path": 3 }), None).await {
        Err(ConnectorError::InvalidConfig(e)) => assert!(e.iter().any(|m| m.contains("/path")), "{e:?}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        validated_list(c.as_ref(), &json!({ "path": "/x", "bogus": true }), None).await,
        Err(ConnectorError::InvalidConfig(_))
    ));
    let missing = validated_check(c.as_ref(), &json!({ "path": "/definitely/not/here" })).await.unwrap();
    assert!(!missing.ok);
    assert!(matches!(
        validated_list(c.as_ref(), &json!({ "path": "/x" }), Some("not json")).await,
        Err(ConnectorError::BadCursor(_))
    ));
}

#[tokio::test]
async fn folder_watch_reports_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let c = builtin("folder").unwrap();
    let mut w = c.watch(&json!({ "path": tmp.path().to_str().unwrap() })).unwrap().expect("folder can watch");
    std::fs::write(tmp.path().join("new.md"), "hello").unwrap();
    let n = tokio::time::timeout(Duration::from_secs(5), w.notices.recv()).await.expect("a notice").unwrap();
    assert_eq!(n.external_id.as_deref(), Some("new.md"));
}
