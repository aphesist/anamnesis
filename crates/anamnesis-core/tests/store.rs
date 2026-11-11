use anamnesis_core::*;
use serde_json::json;

fn source(store: &Store, connector: &str) -> Source {
    store
        .add_source(Source {
            id: Id::new(),
            connector: connector.into(),
            name: connector.into(),
            config: json!({}),
            default_origin: Origin::OwnerWritten,
            default_privacy: Privacy::CloudOk,
            cursor: None,
            created_at: now(),
        })
        .unwrap()
}

fn doc(eid: &str, title: &str, body: &str, refs: &[&str]) -> Document {
    Document {
        external_id: eid.into(),
        uri: None,
        kind: ItemKind::Note,
        title: title.into(),
        body: body.into(),
        origin: Origin::OwnerWritten,
        created_at: Some("2026-03-04T10:00:00Z".parse().unwrap()),
        modified_at: None,
        tags: vec![],
        refs: refs.iter().map(|s| s.to_string()).collect(),
        meta: json!({}),
    }
}

fn created(o: UpsertOutcome) -> Item {
    match o {
        UpsertOutcome::Created(i) => i,
        other => panic!("expected created, got {other:?}"),
    }
}

fn link_pairs(store: &Store) -> Vec<(String, String, LinkKind)> {
    let mut v: Vec<_> = store
        .all_links()
        .unwrap()
        .into_iter()
        .map(|l| (l.from.0, l.to.0, l.kind))
        .collect();
    v.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    v
}

/// A brain with a bit of everything.
fn populated() -> Store {
    let s = Store::open_in_memory().unwrap();
    let vault = source(&s, "obsidian");
    let feed = source(&s, "rss");
    let a = created(
        s.upsert_document(
            &vault.id,
            doc(
                "notes/a.md",
                "Alpha",
                "Sobre a decisão de mudar.\n\nSee [[Beta]] and [[Gamma]].",
                &["Beta", "Gamma"],
            ),
        )
        .unwrap(),
    );
    let b = created(
        s.upsert_document(
            &vault.id,
            doc(
                "notes/b.md",
                "Beta",
                "beta body, links back to [[Alpha]]",
                &["Alpha"],
            ),
        )
        .unwrap(),
    );
    let _ = s
        .upsert_document(
            &feed.id,
            doc(
                "https://e.org/1",
                "Article",
                "an external article about attention",
                &[],
            ),
        )
        .unwrap();
    // Gamma appears later: pending ref resolves
    let g = created(
        s.upsert_document(&vault.id, doc("notes/gamma.md", "Gamma", "gamma text", &[]))
            .unwrap(),
    );
    // update a, delete article
    s.upsert_document(
        &vault.id,
        doc(
            "notes/a.md",
            "Alpha",
            "Sobre a decisão de mudar. Edited.\n\nSee [[Beta]].",
            &["Beta"],
        ),
    )
    .unwrap();
    s.delete_by_external_id(&feed.id, "https://e.org/1")
        .unwrap();
    let note = s
        .create_note(
            NewNote {
                title: "editor note".into(),
                body: "mentions [[Gamma]]".into(),
                kind: None,
                tags: vec!["x".into()],
                origin: None,
            },
            Actor::User,
        )
        .unwrap();
    s.update_item(
        &note.id,
        ItemPatch {
            body: Some("mentions [[Beta]] now".into()),
            ..Default::default()
        },
        Actor::User,
    )
    .unwrap();
    s.add_manual_link(&b.id, &g.id, Actor::User).unwrap();
    let sug1 = s
        .record_suggestion(
            &a.id,
            &g.id,
            0.9,
            1,
            true,
            "mind:v1",
            Actor::System("mind:links".into()),
        )
        .unwrap();
    let sug2 = s
        .record_suggestion(
            &g.id,
            &b.id,
            0.7,
            1,
            false,
            "mind:v1",
            Actor::System("mind:links".into()),
        )
        .unwrap();
    let sug3 = s
        .record_suggestion(
            &b.id,
            &a.id,
            0.5,
            2,
            true,
            "mind:v1",
            Actor::System("mind:links".into()),
        )
        .unwrap();
    s.decide_suggestion(&sug1, true, Actor::User).unwrap();
    s.decide_suggestion(&sug3, false, Actor::User).unwrap();
    let _ = sug2;
    let t = Thread {
        id: Id::new(),
        kind: ThreadKind::Decision,
        status: ThreadStatus::Open,
        statement: "mudar".into(),
        item_id: a.id.clone(),
        quote: "decisão de mudar".into(),
        resolved_by: None,
        created_at: now(),
        updated_at: now(),
        extractor: "rule".into(),
    };
    s.upsert_thread(t.clone(), Actor::System("mind:threads".into()))
        .unwrap();
    s.set_thread_status(
        &t.id,
        ThreadStatus::Resolved,
        Some(b.id.clone()),
        Actor::User,
    )
    .unwrap();
    let r1 = Region {
        id: Id::new(),
        label: "moving".into(),
        anatomy: None,
        size: 2,
        born_at: now(),
        died_at: None,
        parent_ids: vec![],
    };
    let run = Id::new();
    s.record_regions(
        run.clone(),
        vec![r1.clone()],
        vec![RegionTransition::Birth {
            region: r1.id.clone(),
        }],
        Actor::System("mind:cluster".into()),
    )
    .unwrap();
    s.replace_placements(
        run,
        &[
            Placement {
                item_id: a.id.clone(),
                xyz: [0.0, 1.0, 2.0],
                region_id: Some(r1.id.clone()),
            },
            Placement {
                item_id: b.id.clone(),
                xyz: [1.0, 1.0, 2.0],
                region_id: Some(r1.id.clone()),
            },
        ],
        Actor::System("mind:layout".into()),
    )
    .unwrap();
    let rs = s
        .record_resurface(&b.id, "random", Actor::System("mind:resurface".into()))
        .unwrap();
    s.rate_resurface(&rs, true, Actor::User).unwrap();
    s.label_retrieval("decisão", &a.id, 3, Actor::User).unwrap();
    s.put_settings(&json!({"theme": "aphesist", "motion": "reduced"}))
        .unwrap();
    s.set_setting("motion", serde_json::Value::Null).unwrap();
    s.append(
        Actor::Model("model:x".into()),
        EventKind::Egress {
            provider: "anthropic".into(),
            model: "m".into(),
            purpose: "chat".into(),
            items: vec![a.id.clone()],
            chars: 120,
        },
    )
    .unwrap();
    s.append(
        Actor::User,
        EventKind::ChatTurn {
            chat_id: Id::new(),
            role: "user".into(),
            item_id: note.id.clone(),
            cited: vec![a.id.clone()],
        },
    )
    .unwrap();
    s.set_source_cursor(&vault.id, Some("c1".into())).unwrap();
    s
}

#[test]
fn chain_verifies_and_detects_tampering() {
    let s = populated();
    let report = s.verify_chain().unwrap();
    assert_eq!(report.events, s.event_count().unwrap());
    assert!(report.events > 20);

    // edit a payload
    let conn = s.connection();
    let k2: String = conn
        .query_row(
            "SELECT kind FROM events WHERE kind_type = 'item_created' ORDER BY seq LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let seq: i64 = conn
        .query_row(
            "SELECT seq FROM events WHERE kind_type = 'item_created' ORDER BY seq LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let edited = k2.replace("Alpha", "Omega");
    assert_ne!(edited, k2);
    conn.execute(
        "UPDATE events SET kind = ?1 WHERE seq = ?2",
        rusqlite::params![edited, seq],
    )
    .unwrap();
    match s.verify_chain() {
        Err(Error::Chain { seq: at, .. }) => assert_eq!(at as i64, seq),
        other => panic!("tamper not detected: {other:?}"),
    }
    conn.execute(
        "UPDATE events SET kind = ?1 WHERE seq = ?2",
        rusqlite::params![k2, seq],
    )
    .unwrap();
    s.verify_chain().unwrap();

    // recomputing the hash of an edited event breaks the next link
    conn.execute("UPDATE events SET hash = 'tampered' WHERE seq = 3", [])
        .unwrap();
    assert!(matches!(s.verify_chain(), Err(Error::Chain { seq: 3, .. })));
    conn.execute(
        "UPDATE events SET hash = (SELECT prev_hash FROM events WHERE seq = 4) WHERE seq = 3",
        [],
    )
    .unwrap();
    s.verify_chain().unwrap();

    // a deleted event leaves a gap
    conn.execute("DELETE FROM events WHERE seq = 7", [])
        .unwrap();
    assert!(matches!(s.verify_chain(), Err(Error::Chain { seq: 8, .. })));
}

#[test]
fn replay_reproduces_live_state() {
    let s = populated();
    let live = s.projection_snapshot().unwrap();
    let hits_before = s.search(&SearchQuery::text("decisao", 5)).unwrap();
    s.rebuild_projections().unwrap();
    let replayed = s.projection_snapshot().unwrap();
    assert_eq!(live, replayed);
    assert_eq!(
        hits_before,
        s.search(&SearchQuery::text("decisao", 5)).unwrap()
    );
    assert!(live.contains("## items (5)"), "{live}");
}

#[test]
fn events_paging() {
    let s = populated();
    let n = s.event_count().unwrap();
    let first = s.events_after(0, 5).unwrap();
    assert_eq!(
        first.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    let rest = s.events_after(5, 10_000).unwrap();
    assert_eq!(rest.len() as u64, n - 5);
    assert_eq!(rest[0].prev_hash, first[4].hash);
    assert_eq!(s.egress_entries(10).unwrap().len(), 1);
}

#[test]
fn dedup_and_update_semantics() {
    let s = Store::open_in_memory().unwrap();
    let v = source(&s, "folder");
    let w = source(&s, "obsidian");
    let a = created(
        s.upsert_document(&v.id, doc("a.md", "A", "same text\n\nhere", &[]))
            .unwrap(),
    );
    // exact duplicate in another source, modulo whitespace
    assert_eq!(
        s.upsert_document(
            &w.id,
            doc("copy.md", "A copy", "same text  \r\n\r\n\r\nhere\n", &[])
        )
        .unwrap(),
        UpsertOutcome::Duplicate { of: a.id.clone() }
    );
    // empty bodies are never duplicates
    created(
        s.upsert_document(&v.id, doc("e1.md", "E1", "", &[]))
            .unwrap(),
    );
    created(
        s.upsert_document(&v.id, doc("e2.md", "E2", "  ", &[]))
            .unwrap(),
    );
    // re-sync unchanged
    let mut again = doc("a.md", "A", "same text\n\nhere", &[]);
    again.modified_at = Some(now());
    assert_eq!(
        s.upsert_document(&v.id, again).unwrap(),
        UpsertOutcome::Unchanged(a.id.clone())
    );
    // change
    match s
        .upsert_document(&v.id, doc("a.md", "A", "new text", &[]))
        .unwrap()
    {
        UpsertOutcome::Updated(i) => {
            assert_eq!(i.id, a.id);
            assert_eq!(i.captured_at, a.captured_at);
            assert_ne!(i.content_hash, a.content_hash);
        }
        o => panic!("{o:?}"),
    }
    // delete then re-add revives the same item
    assert_eq!(
        s.delete_by_external_id(&v.id, "a.md").unwrap(),
        Some(a.id.clone())
    );
    assert!(s.get_item(&a.id).unwrap().unwrap().deleted);
    assert!(
        s.search(&SearchQuery::text("new text", 5))
            .unwrap()
            .is_empty()
    );
    match s
        .upsert_document(&v.id, doc("a.md", "A", "new text", &[]))
        .unwrap()
    {
        UpsertOutcome::Updated(i) => assert!(!i.deleted && i.id == a.id),
        o => panic!("{o:?}"),
    }
    // after deletion of the original, the copy is no longer a duplicate
    s.delete_item(&a.id, Actor::User).unwrap();
    created(
        s.upsert_document(&w.id, doc("copy2.md", "copy", "new text", &[]))
            .unwrap(),
    );
}

#[test]
fn wikilinks_resolve_including_pending_aliases_and_paths() {
    let s = Store::open_in_memory().unwrap();
    let v = source(&s, "obsidian");
    let a = created(
        s.upsert_document(
            &v.id,
            doc(
                "daily/a.md",
                "A",
                "x",
                &[
                    "Later Note",
                    "[[Beta#heading|b]]",
                    "../topics/c.md",
                    "the alias",
                ],
            ),
        )
        .unwrap(),
    );
    let pending: Vec<String> = s
        .pending_refs()
        .unwrap()
        .into_iter()
        .map(|p| p.reference)
        .collect();
    assert_eq!(pending.len(), 4);

    let b = created(
        s.upsert_document(&v.id, doc("beta.md", "Beta", "b", &[]))
            .unwrap(),
    );
    let c = created(
        s.upsert_document(&v.id, doc("topics/c.md", "Something else", "c", &[]))
            .unwrap(),
    );
    let mut d_doc = doc("d.md", "D", "d", &[]);
    d_doc.meta = json!({"frontmatter": {"aliases": ["The Alias"]}});
    let d = created(s.upsert_document(&v.id, d_doc).unwrap());
    let targets: Vec<Id> = s
        .links_from(&a.id)
        .unwrap()
        .into_iter()
        .map(|l| l.to)
        .collect();
    for t in [&b.id, &c.id, &d.id] {
        assert!(targets.contains(t), "missing link to {t}");
    }
    assert!(
        s.links_from(&a.id)
            .unwrap()
            .iter()
            .all(|l| l.kind == LinkKind::Written)
    );
    assert_eq!(s.pending_refs().unwrap().len(), 1);

    // a file named like the pending ref resolves by basename
    let later = created(
        s.upsert_document(&v.id, doc("inbox/Later Note.md", "", "l", &[]))
            .unwrap(),
    );
    assert!(
        s.links_to(&later.id)
            .unwrap()
            .iter()
            .any(|l| l.from == a.id)
    );
    assert!(s.pending_refs().unwrap().is_empty());

    // removing the reference removes the written link; a manual link on the same pair stays
    s.add_manual_link(&a.id, &c.id, Actor::User).unwrap();
    s.upsert_document(
        &v.id,
        doc("daily/a.md", "A", "x2", &["Later Note", "../topics/c.md"]),
    )
    .unwrap();
    let after = link_pairs(&s);
    assert!(!after.iter().any(|(f, t, _)| *f == a.id.0 && *t == b.id.0));
    assert!(
        after
            .iter()
            .any(|(f, t, k)| *f == a.id.0 && *t == c.id.0 && *k == LinkKind::Manual)
    );

    // editor notes extract their own wikilinks
    let n = s
        .create_note(
            NewNote {
                title: "n".into(),
                body: "see [[Beta]]".into(),
                kind: None,
                tags: vec![],
                origin: None,
            },
            Actor::User,
        )
        .unwrap();
    assert_eq!(s.links_from(&n.id).unwrap()[0].to, b.id);
    assert_eq!(s.resolve_reference("[[beta]]").unwrap(), Some(b.id.clone()));
}

#[test]
fn fts_matches_without_accents() {
    let s = Store::open_in_memory().unwrap();
    let v = source(&s, "folder");
    let a = created(
        s.upsert_document(
            &v.id,
            doc("a.md", "Reunião", "A decisão foi adiada para março.", &[]),
        )
        .unwrap(),
    );
    created(
        s.upsert_document(&v.id, doc("b.md", "Other", "nothing relevant here", &[]))
            .unwrap(),
    );
    for q in ["decisao", "decisão", "DECISAO", "reuniao", "marco adiada"] {
        let hits = s.search(&SearchQuery::text(q, 5)).unwrap();
        assert_eq!(hits.len(), 1, "query {q}");
        assert_eq!(hits[0].item_id, a.id);
        let body = &s.get_item(&a.id).unwrap().unwrap().body;
        assert!(body[hits[0].start..hits[0].end].contains("decisão"));
    }
    // OR fallback when not every word matches; operators in input are inert
    assert_eq!(
        s.search(&SearchQuery::text("decisao zebra", 5))
            .unwrap()
            .len(),
        1
    );
    assert!(
        s.search(&SearchQuery::text("NOT AND (", 5))
            .unwrap()
            .is_empty()
    );
}

fn unit(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

#[test]
fn knn_matches_naive_and_feeds_hybrid_search() {
    let s = Store::open_in_memory().unwrap();
    let v = source(&s, "folder");
    let mut items = Vec::new();
    for i in 0..40 {
        items.push(created(
            s.upsert_document(
                &v.id,
                doc(
                    &format!("{i}.md"),
                    &format!("n{i}"),
                    &format!("note number {i} word{i}"),
                    &[],
                ),
            )
            .unwrap(),
        ));
    }
    let missing = s.chunks_missing_embedding("toy", 1000).unwrap();
    assert_eq!(missing.len(), 40);
    // deterministic pseudo-random 8-d vectors
    let mut seed = 7u64;
    let mut rnd = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f32 / u32::MAX as f32) - 0.25
    };
    let entries: Vec<(Id, Vec<f32>)> = missing
        .iter()
        .map(|c| (c.id.clone(), (0..8).map(|_| rnd()).collect()))
        .collect();
    assert_eq!(s.put_embeddings("toy", &entries).unwrap(), 40);
    assert!(s.chunks_missing_embedding("toy", 10).unwrap().is_empty());
    assert!(
        s.put_embeddings("toy", &[(entries[0].0.clone(), vec![1.0; 3])])
            .is_err()
    );

    let q: Vec<f32> = (0..8).map(|_| rnd()).collect();
    let got = s.knn("toy", &q, 5).unwrap();
    let qn = unit(&q);
    let mut naive: Vec<(Id, f32)> = entries
        .iter()
        .map(|(id, v)| {
            (
                id.clone(),
                unit(v).iter().zip(&qn).map(|(a, b)| a * b).sum(),
            )
        })
        .collect();
    naive.sort_by(|a, b| b.1.total_cmp(&a.1));
    assert_eq!(
        got.iter().map(|h| h.chunk_id.clone()).collect::<Vec<_>>(),
        naive[..5].iter().map(|n| n.0.clone()).collect::<Vec<_>>()
    );
    for (g, n) in got.iter().zip(&naive) {
        assert!((g.score - n.1).abs() < 1e-5);
    }

    // exact vector of one chunk ranks it first, also through hybrid search
    let target = &entries[13];
    let item13 = s.get_chunk(&target.0).unwrap().unwrap().item_id;
    let hits = s
        .search(&SearchQuery {
            text: "word13".into(),
            vector: Some(QueryVector {
                model_id: "toy".into(),
                values: target.1.clone(),
            }),
            k: 3,
            granularity: Granularity::Item,
        })
        .unwrap();
    assert_eq!(hits[0].item_id, item13);
    let (ids, vecs) = s.item_vectors("toy").unwrap();
    assert_eq!(ids.len(), 40);
    assert!((vecs[0].iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4);

    // deleting an item drops it from the index
    s.delete_item(&item13, Actor::User).unwrap();
    assert!(
        s.knn("toy", &target.1, 40)
            .unwrap()
            .iter()
            .all(|h| h.item_id != item13)
    );
    // vectors survive a rebuild
    s.rebuild_projections().unwrap();
    assert_eq!(s.knn("toy", &q, 40).unwrap().len(), 39);
    let _ = items;
}

#[test]
fn export_import_roundtrip() {
    let s = populated();
    let mut buf = Vec::new();
    let m = s.export_bundle(&mut buf).unwrap();
    assert_eq!(m.events, s.event_count().unwrap());

    let t = Store::open_in_memory().unwrap();
    t.import_bundle(&buf[..]).unwrap();
    t.verify_chain().unwrap();
    assert_eq!(
        s.projection_snapshot().unwrap(),
        t.projection_snapshot().unwrap()
    );
    assert_eq!(s.head().unwrap(), t.head().unwrap());
    // importing twice is refused
    assert!(t.import_bundle(&buf[..]).is_err());

    // a tampered bundle is rejected and leaves the store empty
    let text = String::from_utf8(buf.clone()).unwrap();
    let bad = text.replacen("gamma text", "gamma TEXT", 1);
    assert_ne!(bad, text);
    let u = Store::open_in_memory().unwrap();
    assert!(matches!(
        u.import_bundle(bad.as_bytes()),
        Err(Error::Chain { .. })
    ));
    assert_eq!(u.event_count().unwrap(), 0);
    // a truncated bundle is rejected
    let truncated: String = text.lines().take(5).map(|l| format!("{l}\n")).collect();
    assert!(
        Store::open_in_memory()
            .unwrap()
            .import_bundle(truncated.as_bytes())
            .is_err()
    );
}

#[test]
fn file_store_persists_and_uses_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("brain.db");
    let id;
    {
        let s = Store::open(&path).unwrap();
        let mode: String = s
            .connection()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let v = source(&s, "folder");
        id = created(
            s.upsert_document(&v.id, doc("a.md", "A", "persisted", &[]))
                .unwrap(),
        )
        .id;
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(s.get_item(&id).unwrap().unwrap().body, "persisted");
    s.verify_chain().unwrap();
}

#[test]
fn projections_for_the_api() {
    let s = populated();
    // suggestions: accepted one created a link, held-out one is not "open"
    let all = s.all_suggestions().unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(
        all.iter()
            .filter(|x| x.status == SuggestionStatus::Accepted)
            .count(),
        1
    );
    assert!(
        s.all_links()
            .unwrap()
            .iter()
            .any(|l| l.kind == LinkKind::AcceptedSuggestion)
    );
    for x in &all {
        assert!(s.open_suggestions(&x.from).unwrap().is_empty());
    }
    // threads
    let th = s
        .threads(&ThreadFilter {
            status: Some(ThreadStatus::Resolved),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(th.len(), 1);
    assert!(th[0].resolved_by.is_some());
    // regions, placements, timeline
    let regions = s.current_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(s.placements().unwrap().len(), 2);
    assert_eq!(s.region_runs().unwrap().len(), 1);
    let tl = s.timeline().unwrap();
    assert!(!tl.months.is_empty());
    assert_eq!(tl.by_region.len(), 1);
    assert_eq!(tl.by_region[0].counts.iter().sum::<u32>(), 2);
    assert_eq!(tl.by_speaker.you.len(), tl.months.len());
    assert_eq!(tl.threads_by_month.decision.iter().sum::<u32>(), 1);
    let total: u32 = tl.days.iter().map(|d| d.count).sum();
    assert_eq!(total as u64, s.count_items().unwrap());
    // settings, resurfacing, labels
    assert_eq!(s.settings().unwrap(), json!({"theme": "aphesist"}));
    assert_eq!(s.resurfacings(false).unwrap()[0].useful, Some(true));
    assert!(s.resurfacings(true).unwrap().is_empty());
    assert_eq!(s.retrieval_labels().unwrap()[0].grade, 3);
    // sources
    let srcs = s.list_sources().unwrap();
    assert!(srcs.iter().any(|x| x.cursor.as_deref() == Some("c1")));
    let rss = srcs.iter().find(|x| x.connector == "rss").unwrap();
    s.remove_source(&rss.id).unwrap();
    assert!(s.get_source(&rss.id).unwrap().is_none());
    // item listing
    let notes = s
        .list_items(&ItemFilter {
            kind: Some(ItemKind::Note),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(notes.len() as u64, s.count_items().unwrap());
    let summaries = s
        .list_item_summaries(&ItemFilter {
            limit: Some(2),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(summaries.len(), 2);
}
