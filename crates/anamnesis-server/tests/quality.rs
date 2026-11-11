//! Rule threads are re-extracted when the extractor changes, and suggestions count as
//! displayed only once the ui has fetched them.

use std::sync::Arc;

use anamnesis_core::{
    Actor, EventKind, Id, NewNote, Thread, ThreadFilter, ThreadKind, ThreadStatus, now,
};
use anamnesis_mind::threads::RULE_EXTRACTOR;
use anamnesis_server::{App, ServerConfig, api, jobs};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ServerConfig::for_tests(&dir.path().join("anamnesis"));
    let app = App::open(cfg, false).unwrap();
    (dir, app)
}

fn v1_thread(item: &Id, kind: ThreadKind, quote: &str) -> Thread {
    let at = now();
    Thread {
        id: Id::new(),
        kind,
        status: ThreadStatus::Open,
        statement: quote.into(),
        item_id: item.clone(),
        quote: quote.into(),
        resolved_by: None,
        created_at: at,
        updated_at: at,
        extractor: "rule".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rule_threads_are_re_extracted_once_per_extractor_version() {
    let (_dir, app) = app();
    let brain = app.brain();
    let body = "What is a river but a road that moves?\n\
                I decided to keep the journal in plain text.\n\
                Is the bridge older than the mill?";
    let (item, stale, touched, kept) = brain
        .with_store(|s| {
            let it = s.create_note(
                NewNote {
                    title: "walk".into(),
                    body: body.into(),
                    kind: None,
                    tags: vec![],
                    origin: None,
                },
                Actor::User,
            )?;
            // threads the first, unversioned rules would have stored
            let sys = Actor::System("mind:threads".into());
            let stale = v1_thread(
                &it.id,
                ThreadKind::OpenQuestion,
                "What is a river but a road that moves?",
            );
            let touched = v1_thread(
                &it.id,
                ThreadKind::OpenQuestion,
                "Is the bridge older than the mill?",
            );
            let kept = v1_thread(
                &it.id,
                ThreadKind::Decision,
                "I decided to keep the journal in plain text.",
            );
            for t in [&stale, &touched, &kept] {
                s.upsert_thread(t.clone(), sys.clone())?;
            }
            // the user resolved one: theirs to keep
            s.set_thread_status(&touched.id, ThreadStatus::Resolved, None, Actor::User)?;
            Ok((it, stale.id, touched.id, kept.id))
        })
        .unwrap();

    jobs::run_threads(&brain).await.unwrap();
    let threads = brain
        .with_store(|s| {
            Ok(s.threads(&ThreadFilter {
                item_id: Some(item.id.clone()),
                ..Default::default()
            })?)
        })
        .unwrap();
    let ids: Vec<&Id> = threads.iter().map(|t| &t.id).collect();
    assert!(
        !ids.contains(&&stale),
        "the rhetorical question is withdrawn"
    );
    assert!(ids.contains(&&touched), "a thread the user touched stays");
    let k = threads.iter().find(|t| t.id == kept).expect("still found");
    assert_eq!(k.extractor, RULE_EXTRACTOR);
    assert_eq!(threads.len(), 2, "{threads:?}");
    let removed = brain
        .with_store(|s| Ok(s.events_of_type(&["thread_removed"], 100)?))
        .unwrap();
    assert_eq!(removed.len(), 1);

    // the same version again: nothing to do
    let before = brain.with_store(|s| Ok(s.event_count()?)).unwrap();
    assert_eq!(jobs::run_threads(&brain).await.unwrap(), 0);
    assert_eq!(brain.with_store(|s| Ok(s.event_count()?)).unwrap(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetching_suggestions_records_them_as_displayed_once() {
    let (_dir, app) = app();
    let brain = app.brain();
    let (a, shown, held) = brain
        .with_store(|s| {
            let mk = |t: &str| {
                s.create_note(
                    NewNote {
                        title: t.into(),
                        body: format!("a note about {t}"),
                        kind: None,
                        tags: vec![],
                        origin: None,
                    },
                    Actor::User,
                )
            };
            let (a, b, c) = (mk("herons")?, mk("marsh")?, mk("reeds")?);
            let sys = Actor::System("mind:links".into());
            let shown = s.record_suggestion(&a.id, &b.id, 0.9, 1, true, "m", sys.clone())?;
            let held = s.record_suggestion(&a.id, &c.id, 0.8, 2, false, "m", sys)?;
            Ok((a, shown, held))
        })
        .unwrap();
    let router = api::router(app.clone());
    let get = |uri: String| {
        let r = router.clone();
        async move {
            let res = r
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }
    };
    let displayed = || {
        brain
            .with_store(|s| Ok(s.events_of_type(&["suggestion_displayed"], 100)?))
            .unwrap()
    };
    assert!(
        displayed().is_empty(),
        "computing a suggestion is not displaying it"
    );
    let v = get(format!("/api/suggestions?item={}", a.id)).await;
    assert_eq!(v["suggestions"].as_array().unwrap().len(), 1);
    let evs = displayed();
    assert_eq!(evs.len(), 1);
    match &evs[0].kind {
        EventKind::SuggestionDisplayed { suggestion_ids } => {
            assert_eq!(suggestion_ids, &vec![shown.clone()]);
            assert!(!suggestion_ids.contains(&held));
        }
        other => panic!("{other:?}"),
    }
    // opening the item again does not count twice
    get(format!("/api/suggestions?item={}", a.id)).await;
    assert_eq!(displayed().len(), 1);
}
