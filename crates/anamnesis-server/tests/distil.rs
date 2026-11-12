//! Distil end to end: proposals from the user's own chat messages, keep / drop, the
//! extract route with privacy and egress, mcp, and the evaluation metric.

use std::sync::{Arc, Mutex};

use anamnesis_core::{Actor, Id, LinkKind, Origin, Privacy};
use anamnesis_mcp::BrainApi;
use anamnesis_server::mcp_brain::StoreBrain;
use anamnesis_server::{App, ServerConfig, api};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Env {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    router: Router,
}

fn setup() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(
        ServerConfig::for_tests(&dir.path().join("anamnesis")),
        false,
    )
    .unwrap();
    let router = api::router(app.clone());
    Env {
        _dir: dir,
        app,
        router,
    }
}

async fn call(r: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = r.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, v)
}

async fn ok(r: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(r, method, uri, body).await;
    assert_eq!(s, StatusCode::OK, "{method} {uri}: {v}");
    v
}

/// Send messages through `/api/chat` (the retrieval responder) and return the chat id.
async fn chat(r: &Router, messages: &[&str]) -> String {
    let mut chat_id: Option<String> = None;
    for m in messages {
        let mut body = json!({ "message": m, "backend": "retrieval" });
        if let Some(c) = &chat_id {
            body["chat_id"] = json!(c);
        }
        let req = Request::post("/api/chat")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = r.clone().oneshot(req).await.unwrap();
        let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec())
            .unwrap();
        let done = text
            .lines()
            .skip_while(|l| *l != "event: done")
            .nth(1)
            .unwrap_or_else(|| panic!("no done in {text}"));
        let done: Value = serde_json::from_str(done.trim_start_matches("data: ")).unwrap();
        chat_id = Some(done["chat_id"].as_str().unwrap().to_string());
    }
    chat_id.unwrap()
}

const MESSAGES: &[&str] = &[
    "hello! can you remind me what i wrote about bread?",
    "I decided to bake on sundays only. Should I keep a starter in the fridge?",
    "Idea: a bread journal that only records failures.",
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rules_propose_keep_and_drop() {
    let env = setup();
    let r = &env.router;
    let chat_id = chat(r, MESSAGES).await;

    let out = ok(
        r,
        "POST",
        "/api/distil",
        Some(json!({ "chat_id": chat_id })),
    )
    .await;
    assert_eq!(out["distiller"], "rule:v2");
    let ps = out["proposals"].as_array().unwrap().clone();
    let kinds: Vec<&str> = ps.iter().map(|p| p["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["decision", "open_question", "idea"], "{out}");
    // every quote is the user's exact words, inside the turn it names
    for p in &ps {
        let src = Id(p["source_item"].as_str().unwrap().into());
        let item = env
            .app
            .brain()
            .with_store(|s| Ok(s.get_item(&src)?.unwrap()))
            .unwrap();
        assert_eq!(item.origin, Origin::OwnerInChat);
        let (a, b) = (
            p["start"].as_u64().unwrap() as usize,
            p["end"].as_u64().unwrap() as usize,
        );
        assert_eq!(&item.body[a..b], p["quote"].as_str().unwrap());
        assert_eq!(p["status"], "pending");
    }

    // a second run proposes nothing new
    let again = ok(
        r,
        "POST",
        "/api/distil",
        Some(json!({ "chat_id": chat_id })),
    )
    .await;
    assert_eq!(again["proposals"], json!([]));

    // keep the idea, with the user's own title
    let idea = ps.iter().find(|p| p["kind"] == "idea").unwrap();
    let kept = ok(
        r,
        "POST",
        &format!("/api/distil/{}", idea["id"].as_str().unwrap()),
        Some(json!({ "keep": true })),
    )
    .await;
    assert_eq!(kept["proposal"]["status"], "kept");
    let item = &kept["item"];
    assert_eq!(item["origin"], "owner_in_chat");
    assert_eq!(item["kind"], "note");
    assert_eq!(item["body"], idea["quote"]);
    assert_eq!(item["title"], "a bread journal that only records failures");
    assert_eq!(item["meta"]["distil"]["title_by"], "quote");
    assert_eq!(item["meta"]["distil"]["chat_id"], json!(chat_id));
    let item_id = Id(item["id"].as_str().unwrap().into());
    let links = env
        .app
        .brain()
        .with_store(|s| Ok(s.links_from(&item_id)?))
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].to.0, idea["source_item"].as_str().unwrap());
    assert_eq!(links[0].kind, LinkKind::Manual);

    // answering twice is refused; unknown ids are 404
    let (s, _) = call(
        r,
        "POST",
        &format!("/api/distil/{}", idea["id"].as_str().unwrap()),
        Some(json!({ "keep": false })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(r, "POST", "/api/distil/nope", Some(json!({ "keep": true }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // drop the question
    let q = ps.iter().find(|p| p["kind"] == "open_question").unwrap();
    let dropped = ok(
        r,
        "POST",
        &format!("/api/distil/{}", q["id"].as_str().unwrap()),
        Some(json!({ "keep": false })),
    )
    .await;
    assert_eq!(dropped["proposal"]["status"], "dropped");
    assert_eq!(dropped["item"], Value::Null);

    let pending = ok(
        r,
        "GET",
        &format!("/api/distil?chat_id={chat_id}&status=pending"),
        None,
    )
    .await;
    assert_eq!(pending["proposals"].as_array().unwrap().len(), 1);
    let (s, _) = call(r, "GET", "/api/distil?status=bogus", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // the evaluation counts the two answers (one kept), not the pending one
    let eval = ok(r, "GET", "/api/eval", None).await;
    assert_eq!(eval["distil"]["proposed"], 3);
    assert_eq!(eval["distil"]["pending"], 1);
    assert_eq!(eval["distil"]["n"], 2);
    assert_eq!(eval["distil"]["kept"], 1);
    assert_eq!(eval["distil"]["status"], "measuring");
    assert!(
        eval["honesty"]["claims"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["key"] == "distil_worth_keeping" && c["class"] == "hypothesis")
    );

    // an unknown chat is a 404
    let (s, _) = call(r, "POST", "/api/distil", Some(json!({ "chat_id": "nope" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

mod fake {
    use anamnesis_harness::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// A metered backend allowed for background work that answers with `answer` and
    /// records each prompt it was sent.
    pub struct Extractor {
        pub prompts: Arc<Mutex<Vec<String>>>,
        pub answer: String,
    }

    #[async_trait]
    impl Backend for Extractor {
        fn id(&self) -> &str {
            "remote"
        }
        fn kind(&self) -> BackendKind {
            BackendKind::OpenaiCompatible
        }
        fn label(&self) -> String {
            "remote".into()
        }
        fn default_model(&self) -> &str {
            "m1"
        }
        fn provider(&self) -> &str {
            "example"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                streaming: true,
                tools: false,
                json_schema: true,
                max_context: None,
                vision: false,
                cost_model: CostModel::Metered,
                background_allowed: true,
                forbidden_params: vec![],
                terms: Terms::Allowed,
                maturity: Maturity::Stable,
                notice: None,
            }
        }
        async fn available(&self) -> Availability {
            Availability::ready()
        }
        async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
            let all: Vec<String> = req.messages.iter().map(|m| m.content.clone()).collect();
            self.prompts.lock().unwrap().push(all.join("\n"));
            let evs = vec![
                ChatEvent::Token {
                    text: self.answer.clone(),
                },
                ChatEvent::Done { stop_reason: None },
            ];
            Ok(Box::pin(futures_util::stream::iter(evs)))
        }
    }
}

async fn route_extract_to(env: &Env, answer: Value) -> Arc<Mutex<Vec<String>>> {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let router = anamnesis_harness::Router::new(
        vec![Arc::new(fake::Extractor {
            prompts: prompts.clone(),
            answer: answer.to_string(),
        })],
        anamnesis_harness::RoutingSettings {
            extract: Some("remote".into()),
            ..Default::default()
        },
        Arc::new(anamnesis_harness::Limits::new(Default::default())),
        Default::default(),
    );
    router.refresh().await;
    env.app.set_router(Some(Arc::new(router)));
    prompts
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extract_route_sends_only_cloud_ok_turns_and_logs_egress() {
    let env = setup();
    let r = &env.router;
    let chat_id = chat(
        r,
        &[
            "I decided to bake on sundays only.",
            "my neighbour's address is 12 rue secrète, and I keep forgetting it.",
            "What if we sold the failed loaves as croutons?",
        ],
    )
    .await;
    // the second message is marked local-only
    let turns = env
        .app
        .brain()
        .with_store(|s| Ok(s.chat_turns(&Id(chat_id.clone()))?))
        .unwrap();
    let user_turns: Vec<Id> = turns
        .iter()
        .filter(|t| t.role == "user")
        .map(|t| t.item_id.clone())
        .collect();
    let (s, _) = call(
        r,
        "PUT",
        &format!("/api/items/{}", user_turns[1]),
        Some(json!({ "privacy": "local_only" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let prompts = route_extract_to(
        &env,
        json!({ "items": [
            { "type": "idea", "title": "croutons from failed loaves", "quote": "What if we sold the failed loaves as croutons?", "turn": 2 },
            { "type": "decision", "title": "bake on sundays", "quote": "I will only bake on Sundays.", "turn": 1 }
        ]}),
    )
    .await;
    let out = ok(
        r,
        "POST",
        "/api/distil",
        Some(json!({ "chat_id": chat_id })),
    )
    .await;
    assert_eq!(out["distiller"], "example:m1", "{out}");
    let ps = out["proposals"].as_array().unwrap();
    assert_eq!(ps.len(), 1, "the paraphrased quote is rejected: {out}");
    assert_eq!(ps[0]["title"], "croutons from failed loaves");
    assert_eq!(ps[0]["source_item"], json!(user_turns[2]));

    let sent = prompts.lock().unwrap().join("\n");
    assert!(sent.contains("bake on sundays"));
    assert!(
        !sent.contains("rue secrète"),
        "local-only text left the machine"
    );
    assert!(
        !sent.contains("remind me"),
        "assistant replies are never sent"
    );

    let eg = ok(r, "GET", "/api/egress", None).await;
    let e = &eg["entries"][0]["kind"];
    assert_eq!(e["provider"], "example");
    assert_eq!(e["purpose"], "extract");
    assert!(
        !e["items"]
            .as_array()
            .unwrap()
            .contains(&json!(user_turns[1]))
    );

    // the kept note says its title was written by a model
    let kept = ok(
        r,
        "POST",
        &format!("/api/distil/{}", ps[0]["id"].as_str().unwrap()),
        Some(json!({ "keep": true })),
    )
    .await;
    assert_eq!(
        kept["item"]["meta"]["distil"]["title_by"],
        "model:example:m1"
    );
    assert_eq!(kept["item"]["origin"], "owner_in_chat");

    // quotes are checked against this chat's own turns
    let chat2 = chat(r, &["I decided to stop buying sliced bread."]).await;
    let out = ok(r, "POST", "/api/distil", Some(json!({ "chat_id": chat2 }))).await;
    assert_eq!(out["proposals"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_or_absent_model_falls_back_to_the_rules() {
    let env = setup();
    let r = &env.router;
    let chat_id = chat(r, MESSAGES).await;
    route_extract_to(&env, json!("not the schema at all")).await;
    let out = ok(
        r,
        "POST",
        "/api/distil",
        Some(json!({ "chat_id": chat_id })),
    )
    .await;
    assert_eq!(out["distiller"], "rule:v2");
    assert_eq!(out["proposals"].as_array().unwrap().len(), 3);
    assert!(out["note"].as_str().unwrap().contains("rules"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_distils_on_this_machine_and_keeps_by_proposal_id() {
    let env = setup();
    let prompts = route_extract_to(&env, json!({ "items": [] })).await;
    let brain = StoreBrain::new(env.app.clone());
    let actor = Actor::Mcp("claude-code".into());
    let r = brain
        .distil(
            actor.clone(),
            None,
            vec![
                "can you check this function?".into(),
                "I decided to rewrite the parser in rust. Idea: fuzz it with old notes.".into(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(r.distiller, "rule:v2");
    assert!(r.note.as_deref().unwrap().contains("this machine"));
    assert!(
        prompts.lock().unwrap().is_empty(),
        "mcp never uses a remote model"
    );
    assert_eq!(r.proposals.len(), 2);
    assert!(r.proposals.iter().all(|p| p.source_item.is_none()));

    let item = brain
        .keep_distilled(
            actor.clone(),
            &r.proposals[1].id,
            Some("fuzzing the parser".into()),
        )
        .await
        .unwrap();
    assert_eq!(item.origin, Origin::OwnerInChat);
    assert_eq!(item.body, "Idea: fuzz it with old notes.");
    assert_eq!(item.title, "fuzzing the parser");
    assert_eq!(item.meta["distil"]["title_by"], "user");
    assert!(
        brain
            .keep_distilled(actor.clone(), &r.proposals[1].id, None)
            .await
            .is_err()
    );

    // over mcp, local-only (and ask) chat turns are left out
    let chat_id = chat(&env.router, &["I decided to tell nobody about the novel."]).await;
    let turn = env
        .app
        .brain()
        .with_store(|s| Ok(s.chat_turns(&Id(chat_id.clone()))?[0].item_id.clone()))
        .unwrap();
    env.app
        .brain()
        .with_store(|s| {
            let mut it = s.get_item(&turn)?.unwrap();
            it.privacy = Privacy::LocalOnly;
            s.append(
                Actor::User,
                anamnesis_core::EventKind::ItemUpdated { item: it },
            )?;
            Ok(())
        })
        .unwrap();
    let err = brain.distil(actor, Some(Id(chat_id)), vec![]).await;
    assert!(err.is_err(), "no turn of this chat may be shown over mcp");
}
