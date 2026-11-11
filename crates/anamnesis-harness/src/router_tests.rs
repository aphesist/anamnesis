//! Router policy tests with fake backends.

use std::sync::{Arc, Mutex};

use anamnesis_core::{EventKind, Id, Privacy};
use async_trait::async_trait;

use crate::backend::Backend;
use crate::error::{HarnessError, Result};
use crate::limits::{BackoffPolicy, Limits};
use crate::router::{CallContext, Router, RoutingSettings};
use crate::testutil::collect;
use crate::types::*;
use crate::usage::PriceTable;

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Ok,
    RateLimited,
    StreamUsageLimit,
}

struct Fake {
    id: String,
    kind: BackendKind,
    cost: CostModel,
    model: String,
    behaviour: Behaviour,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Backend for Fake {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> BackendKind {
        self.kind
    }
    fn label(&self) -> String {
        self.id.clone()
    }
    fn default_model(&self) -> &str {
        &self.model
    }
    fn provider(&self) -> &str {
        "fake"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::base(self.cost)
    }
    async fn available(&self) -> Availability {
        Availability::ready()
    }
    async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:{}", self.id, req.model.unwrap_or_default()));
        match self.behaviour {
            Behaviour::RateLimited => Err(HarnessError::RateLimited {
                retry_after: Some(std::time::Duration::from_secs(30)),
            }),
            Behaviour::Ok => Ok(Box::pin(futures_util::stream::iter(vec![
                ChatEvent::Token { text: "ok".into() },
                ChatEvent::Usage(Usage {
                    input: 1_000_000,
                    output: 1_000_000,
                    estimated: false,
                    reported_cost_usd: None,
                }),
                ChatEvent::Done { stop_reason: None },
            ]))),
            Behaviour::StreamUsageLimit => Ok(Box::pin(futures_util::stream::iter(vec![
                ChatEvent::Error {
                    message: "limit".into(),
                    failure: FailureKind::UsageLimit { resets_at: None },
                },
            ]))),
        }
    }
}

struct World {
    calls: Arc<Mutex<Vec<String>>>,
    backends: Vec<Arc<dyn Backend>>,
}

impl World {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            backends: Vec::new(),
        }
    }
    fn add(
        mut self,
        id: &str,
        kind: BackendKind,
        cost: CostModel,
        model: &str,
        behaviour: Behaviour,
    ) -> Self {
        self.backends.push(Arc::new(Fake {
            id: id.into(),
            kind,
            cost,
            model: model.into(),
            behaviour,
            calls: self.calls.clone(),
        }));
        self
    }
    async fn router(&self, settings: RoutingSettings) -> Router {
        let mut prices = PriceTable::default();
        prices.set(
            "pricey",
            crate::usage::Price {
                input_per_m: 5.0,
                output_per_m: 25.0,
            },
        );
        let r = Router::new(
            self.backends.clone(),
            settings,
            Arc::new(Limits::new(BackoffPolicy::default())),
            prices,
        );
        r.refresh().await;
        r
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn subscriptions(w: World) -> World {
    w.add(
        "chatgpt",
        BackendKind::Chatgpt,
        CostModel::Subscription,
        "gpt",
        Behaviour::Ok,
    )
    .add(
        "claude-cli",
        BackendKind::ClaudeCli,
        CostModel::Subscription,
        "default",
        Behaviour::Ok,
    )
}

fn req() -> ChatRequest {
    ChatRequest::new(vec![Message::user("x")])
}

#[tokio::test]
async fn background_never_defaults_to_subscription() {
    // only plan backends configured: background has nowhere to go
    let w = subscriptions(World::new());
    let r = w.router(RoutingSettings::default()).await;
    for t in [Task::Label, Task::Extract, Task::Summarise] {
        assert!(matches!(
            r.chat(t, req(), CallContext::default()).await,
            Err(HarnessError::Routing(_))
        ));
    }
    // and chat is not auto-routed to a plan either
    assert!(
        r.chat(Task::Chat, req(), CallContext::default())
            .await
            .is_err()
    );
    assert!(w.calls().is_empty());
}

#[tokio::test]
async fn background_refuses_explicit_subscription_choice_and_fallbacks() {
    let w = subscriptions(World::new()).add(
        "anthropic",
        BackendKind::Anthropic,
        CostModel::Metered,
        "claude-haiku-5-5",
        Behaviour::Ok,
    );
    let mut s = RoutingSettings {
        label: Some("claude-cli".into()),
        extract: Some("chatgpt".into()),
        ..Default::default()
    };
    s.fallbacks
        .insert(Task::Summarise, vec!["claude-cli".into(), "chatgpt".into()]);
    let r = w.router(s).await;
    assert!(matches!(
        r.chat(Task::Label, req(), CallContext::default()).await,
        Err(HarnessError::Routing(_))
    ));
    assert!(matches!(
        r.chat(Task::Extract, req(), CallContext::default()).await,
        Err(HarnessError::Routing(_))
    ));
    // per-request override cannot sneak a background task onto a plan either
    let ctx = CallContext {
        backend: Some("chatgpt".into()),
        ..Default::default()
    };
    assert!(matches!(
        r.chat(Task::Summarise, req(), ctx).await,
        Err(HarnessError::Routing(_))
    ));
    // summarise default goes to the metered backend; the plan fallbacks are dropped
    let plan = r.plan(Task::Summarise, None).unwrap();
    assert_eq!(
        plan.iter().map(|b| b.id().to_string()).collect::<Vec<_>>(),
        vec!["anthropic"]
    );
    let rc = r
        .chat(Task::Summarise, req(), CallContext::default())
        .await
        .unwrap();
    assert_eq!(rc.backend, "anthropic");
    collect(rc.stream).await;
    assert!(
        w.calls()
            .iter()
            .all(|c| !c.starts_with("chatgpt") && !c.starts_with("claude-cli"))
    );
}

#[tokio::test]
async fn chat_may_use_a_plan_when_chosen() {
    let w = subscriptions(World::new());
    let r = w
        .router(RoutingSettings {
            chat: Some("claude-cli".into()),
            ..Default::default()
        })
        .await;
    let rc = r
        .chat(Task::Chat, req(), CallContext::default())
        .await
        .unwrap();
    assert_eq!(rc.backend, "claude-cli");
    assert!(rc.egress.is_some(), "plan backends are non-local");
}

#[tokio::test]
async fn local_first_for_background_then_cheapest_api() {
    let w = World::new()
        .add(
            "pricey-api",
            BackendKind::Anthropic,
            CostModel::Metered,
            "pricey",
            Behaviour::Ok,
        )
        .add(
            "haiku-api",
            BackendKind::Anthropic,
            CostModel::Metered,
            "claude-haiku-5-5",
            Behaviour::Ok,
        )
        .add(
            "unpriced-api",
            BackendKind::OpenaiCompatible,
            CostModel::Metered,
            "mystery",
            Behaviour::Ok,
        )
        .add(
            "ollama",
            BackendKind::OpenaiCompatible,
            CostModel::FreeLocal,
            "llama",
            Behaviour::Ok,
        );
    let r = w.router(RoutingSettings::default()).await;
    assert_eq!(r.plan(Task::Label, None).unwrap()[0].id(), "ollama");
    let rc = r
        .chat(Task::Extract, req(), CallContext::default())
        .await
        .unwrap();
    assert!(rc.egress.is_none(), "local calls skip the gate");

    let w2 = World::new()
        .add(
            "pricey-api",
            BackendKind::Anthropic,
            CostModel::Metered,
            "pricey",
            Behaviour::Ok,
        )
        .add(
            "unpriced-api",
            BackendKind::OpenaiCompatible,
            CostModel::Metered,
            "mystery",
            Behaviour::Ok,
        )
        .add(
            "haiku-api",
            BackendKind::Anthropic,
            CostModel::Metered,
            "claude-haiku-5-5",
            Behaviour::Ok,
        );
    // by default background work never goes to a metered key: no local model, no label
    let r2 = w2.router(RoutingSettings::default()).await;
    assert!(r2.plan(Task::Label, None).is_err());
    assert!(r2.plan(Task::Extract, None).is_err());
    // chat still may (the user is asking for that call)
    assert!(r2.plan(Task::Chat, None).is_ok());
    // opted in: the cheapest priced api backend
    let r3 = w2
        .router(RoutingSettings {
            background_to_api_by_default: true,
            ..Default::default()
        })
        .await;
    assert_eq!(r3.plan(Task::Label, None).unwrap()[0].id(), "haiku-api");
    // naming a metered backend for a task is an explicit choice and works without opting in
    let r4 = w2
        .router(RoutingSettings {
            label: Some("pricey-api".into()),
            ..Default::default()
        })
        .await;
    assert_eq!(r4.plan(Task::Label, None).unwrap()[0].id(), "pricey-api");
    // the setting deserialises to off when absent
    let s: RoutingSettings = serde_json::from_str("{}").unwrap();
    assert!(!s.background_to_api_by_default);
}

#[tokio::test]
async fn backends_info_says_local_and_leaving() {
    let w = World::new()
        .add(
            "ollama",
            BackendKind::OpenaiCompatible,
            CostModel::FreeLocal,
            "llama",
            Behaviour::Ok,
        )
        .add(
            "anthropic",
            BackendKind::Anthropic,
            CostModel::Metered,
            "claude-haiku-5-5",
            Behaviour::Ok,
        )
        .add(
            "claude-cli",
            BackendKind::ClaudeCli,
            CostModel::Subscription,
            "default",
            Behaviour::Ok,
        );
    let r = w.router(RoutingSettings::default()).await;
    let info = r.backends_info();
    let row = |id: &str| info.iter().find(|b| b.id == id).unwrap().clone();
    assert!(row("ollama").is_local && !row("ollama").leaves_machine);
    assert!(!row("anthropic").is_local && row("anthropic").leaves_machine);
    assert!(!row("claude-cli").is_local && row("claude-cli").leaves_machine);
    let v = serde_json::to_value(row("claude-cli")).unwrap();
    assert_eq!(v["kind"], "claude_cli");
    assert_eq!(v["is_local"], false);
    assert_eq!(v["leaves_machine"], true);
}

#[tokio::test]
async fn no_silent_fallback_from_plan_to_paid() {
    let w = World::new()
        .add(
            "claude-cli",
            BackendKind::ClaudeCli,
            CostModel::Subscription,
            "default",
            Behaviour::RateLimited,
        )
        .add(
            "anthropic",
            BackendKind::Anthropic,
            CostModel::Metered,
            "claude-haiku-5-5",
            Behaviour::Ok,
        );
    let r = w
        .router(RoutingSettings {
            chat: Some("claude-cli".into()),
            ..Default::default()
        })
        .await;
    let e = r
        .chat(Task::Chat, req(), CallContext::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(e, HarnessError::RateLimited { .. }));
    assert_eq!(w.calls(), vec!["claude-cli:default"]);
    // cooldown is visible for the ui, and the next call does not even try
    let cd = r.cooldowns();
    assert_eq!(cd[0].backend, "claude-cli");
    assert!(cd[0].reset_known);
    assert!(matches!(
        r.chat(Task::Chat, req(), CallContext::default()).await,
        Err(HarnessError::CoolingDown { .. })
    ));
    assert_eq!(w.calls().len(), 1);

    // an explicit fallback in settings is honoured
    let mut s = RoutingSettings {
        chat: Some("claude-cli".into()),
        ..Default::default()
    };
    s.fallbacks.insert(Task::Chat, vec!["anthropic".into()]);
    let r = w.router(s).await;
    let rc = r
        .chat(Task::Chat, req(), CallContext::default())
        .await
        .unwrap();
    assert_eq!(rc.backend, "anthropic");
    assert!(rc.fell_back);
}

#[tokio::test]
async fn mid_stream_usage_limit_sets_cooldown() {
    let w = World::new().add(
        "chatgpt",
        BackendKind::Chatgpt,
        CostModel::Subscription,
        "gpt",
        Behaviour::StreamUsageLimit,
    );
    let r = w
        .router(RoutingSettings {
            chat: Some("chatgpt".into()),
            ..Default::default()
        })
        .await;
    let rc = r
        .chat(Task::Chat, req(), CallContext::default())
        .await
        .unwrap();
    collect(rc.stream).await;
    let cd = r.cooldowns();
    assert_eq!(cd[0].reason.as_deref(), Some("plan usage limit reached"));
    assert!(!cd[0].reset_known);
}

#[tokio::test]
async fn egress_gate_refuses_local_only_items() {
    let w = World::new().add(
        "anthropic",
        BackendKind::Anthropic,
        CostModel::Metered,
        "claude-haiku-5-5",
        Behaviour::Ok,
    );
    let r = w.router(RoutingSettings::default()).await;
    let ctx = CallContext {
        items: vec![
            (Id("a".into()), Privacy::CloudOk),
            (Id("secret".into()), Privacy::LocalOnly),
        ],
        ..Default::default()
    };
    match r.chat(Task::Label, req(), ctx).await {
        Err(HarnessError::EgressLocalOnly(ids)) => assert_eq!(ids, vec![Id("secret".into())]),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("expected refusal"),
    }
    assert!(w.calls().is_empty(), "nothing left the machine");

    // cloud-ok items pass and produce an egress record with counts only
    let ctx = CallContext {
        items: vec![(Id("a".into()), Privacy::CloudOk)],
        ..Default::default()
    };
    let rc = r
        .chat(
            Task::Label,
            ChatRequest::new(vec![Message::user("abcd")]),
            ctx,
        )
        .await
        .unwrap();
    let ev: EventKind = rc.egress.unwrap().into();
    assert_eq!(
        ev,
        EventKind::Egress {
            provider: "fake".into(),
            model: "claude-haiku-5-5".into(),
            purpose: "label".into(),
            items: vec![Id("a".into())],
            chars: 4
        }
    );
}

#[tokio::test]
async fn local_only_items_may_go_to_local_backends() {
    let w = World::new().add(
        "ollama",
        BackendKind::OpenaiCompatible,
        CostModel::FreeLocal,
        "llama",
        Behaviour::Ok,
    );
    let r = w.router(RoutingSettings::default()).await;
    let ctx = CallContext {
        items: vec![(Id("secret".into()), Privacy::LocalOnly)],
        ..Default::default()
    };
    assert!(
        r.chat(Task::Label, req(), ctx)
            .await
            .unwrap()
            .egress
            .is_none()
    );
}

#[tokio::test]
async fn usage_is_recorded_with_prices() {
    let w = World::new().add(
        "anthropic",
        BackendKind::Anthropic,
        CostModel::Metered,
        "claude-haiku-5-5",
        Behaviour::Ok,
    );
    let r = w.router(RoutingSettings::default()).await;
    let rc = r
        .chat(Task::Label, req(), CallContext::default())
        .await
        .unwrap();
    collect(rc.stream).await;
    let u = r.usage();
    assert_eq!(u[0].requests, 1);
    assert!((u[0].cost_usd - 0.60).abs() < 1e-9);
    let info = r.backends_info();
    assert_eq!(info[0].id, "anthropic");
    assert!(info[0].available);
}
