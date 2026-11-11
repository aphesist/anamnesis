//! Per-task routing with hard policy rules:
//!
//! - background tasks (label, extract, summarise) never go to subscription backends
//!   (chatgpt plan, installed claude cli) — not by default, not by explicit choice, not
//!   as a fallback;
//! - defaults: local first; otherwise the cheapest configured metered api backend;
//!   plan backends are only used for chat when the user picks them;
//! - no silent fallback: only fallbacks listed in settings are tried, in order;
//! - every non-local call passes the egress gate first;
//! - cooldowns from rate / usage limits are respected and usage is recorded.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use anamnesis_core::{Id, Privacy};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::backend::Backend;
use crate::egress::{EgressRecord, egress_check_with_consent};
use crate::error::{HarnessError, Result};
use crate::limits::{CooldownState, Limits};
use crate::types::*;
use crate::usage::{BackendUsage, PriceTable, UsageLedger};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingSettings {
    #[serde(default)]
    pub chat: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub extract: Option<String>,
    #[serde(default)]
    pub summarise: Option<String>,
    /// Explicit fallbacks per task, tried in order when the primary is unavailable,
    /// cooling down or rate limited. Nothing else is ever tried.
    #[serde(default)]
    pub fallbacks: BTreeMap<Task, Vec<String>>,
    /// When no local backend is available, may background work go to the cheapest
    /// configured metered api backend by default? Off unless the user opts in: an api
    /// key is spent only on work the user asked for (chat) or routed by name.
    #[serde(default)]
    pub background_to_api_by_default: bool,
}

impl Default for RoutingSettings {
    fn default() -> Self {
        Self {
            chat: None,
            label: None,
            extract: None,
            summarise: None,
            fallbacks: BTreeMap::new(),
            background_to_api_by_default: false,
        }
    }
}

impl RoutingSettings {
    pub fn get(&self, task: Task) -> Option<&str> {
        match task {
            Task::Chat => self.chat.as_deref(),
            Task::Label => self.label.as_deref(),
            Task::Extract => self.extract.as_deref(),
            Task::Summarise => self.summarise.as_deref(),
        }
    }
}

/// What the caller knows about the call: which items are in the prompt, consent for
/// `Ask` items, an explicit backend for this one request (e.g. the chat box's picker).
#[derive(Debug, Clone, Default)]
pub struct CallContext {
    pub items: Vec<(Id, Privacy)>,
    pub approved: Vec<Id>,
    pub backend: Option<String>,
}

/// A started, routed chat.
pub struct RoutedChat {
    pub backend: String,
    pub model: String,
    /// Present for non-local calls: append it to the event log as `EventKind::Egress`.
    pub egress: Option<EgressRecord>,
    /// True when a settings fallback (not the primary) served the call.
    pub fell_back: bool,
    pub stream: ChatStream,
}

/// One row of `GET /api/backends`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendInfo {
    pub id: String,
    pub kind: BackendKind,
    pub label: String,
    pub available: bool,
    pub status: String,
    pub capabilities: Capabilities,
    pub terms: Terms,
    /// Calls skip the egress gate: a loopback server, or one the user marked local.
    pub is_local: bool,
    /// Prompts travel over the network to another machine (a provider, or the user's
    /// own lan server marked local).
    pub leaves_machine: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown: Option<CooldownState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

pub struct Router {
    backends: Vec<Arc<dyn Backend>>,
    settings: RwLock<RoutingSettings>,
    availability: RwLock<HashMap<String, Availability>>,
    limits: Arc<Limits>,
    usage: Arc<UsageLedger>,
    prices: Arc<RwLock<PriceTable>>,
}

fn background_ok(b: &dyn Backend) -> bool {
    let c = b.capabilities();
    c.background_allowed && c.cost_model != CostModel::Subscription
}

impl Router {
    pub fn new(
        backends: Vec<Arc<dyn Backend>>,
        settings: RoutingSettings,
        limits: Arc<Limits>,
        prices: PriceTable,
    ) -> Self {
        Self {
            backends,
            settings: RwLock::new(settings),
            availability: RwLock::new(HashMap::new()),
            limits,
            usage: Arc::new(UsageLedger::new()),
            prices: Arc::new(RwLock::new(prices)),
        }
    }

    pub fn backends(&self) -> &[Arc<dyn Backend>] {
        &self.backends
    }
    pub fn get(&self, id: &str) -> Option<Arc<dyn Backend>> {
        self.backends.iter().find(|b| b.id() == id).cloned()
    }
    pub fn settings(&self) -> RoutingSettings {
        self.settings.read().unwrap().clone()
    }
    pub fn set_settings(&self, s: RoutingSettings) {
        *self.settings.write().unwrap() = s;
    }
    pub fn set_prices(&self, p: PriceTable) {
        *self.prices.write().unwrap() = p;
    }
    pub fn prices(&self) -> PriceTable {
        self.prices.read().unwrap().clone()
    }
    pub fn limits(&self) -> &Arc<Limits> {
        &self.limits
    }
    pub fn usage(&self) -> Vec<BackendUsage> {
        self.usage.snapshot()
    }
    pub fn cooldowns(&self) -> Vec<CooldownState> {
        self.limits.snapshot()
    }

    pub fn set_availability(&self, id: &str, a: Availability) {
        self.availability.write().unwrap().insert(id.to_string(), a);
    }

    /// Re-run every backend's health check.
    pub async fn refresh(&self) {
        let checks = self.backends.iter().map(|b| {
            let b = b.clone();
            async move { (b.id().to_string(), b.available().await) }
        });
        let results = futures_util::future::join_all(checks).await;
        let mut m = self.availability.write().unwrap();
        for (id, a) in results {
            m.insert(id, a);
        }
    }

    fn is_available(&self, id: &str) -> bool {
        self.availability
            .read()
            .unwrap()
            .get(id)
            .map(|a| a.available)
            .unwrap_or(false)
    }

    /// Price used to rank metered backends (unknown price ranks last).
    fn rank_price(&self, b: &dyn Backend, task: Task) -> f64 {
        self.prices
            .read()
            .unwrap()
            .lookup(&b.model_for(task))
            .map(|p| p.blended())
            .unwrap_or(f64::INFINITY)
    }

    fn default_backend(&self, task: Task) -> Option<Arc<dyn Backend>> {
        let avail: Vec<_> = self
            .backends
            .iter()
            .filter(|b| self.is_available(b.id()))
            .cloned()
            .collect();
        if let Some(local) = avail.iter().find(|b| b.is_local()) {
            return Some(local.clone());
        }
        if task.is_background() && !self.settings.read().unwrap().background_to_api_by_default {
            return None;
        }
        avail
            .iter()
            .filter(|b| b.capabilities().cost_model == CostModel::Metered)
            .filter(|b| !task.is_background() || background_ok(b.as_ref()))
            .min_by(|a, b| {
                self.rank_price(a.as_ref(), task)
                    .total_cmp(&self.rank_price(b.as_ref(), task))
            })
            .cloned()
    }

    /// The ordered candidates for a task: primary, then explicit fallbacks.
    pub fn plan(
        &self,
        task: Task,
        override_backend: Option<&str>,
    ) -> Result<Vec<Arc<dyn Backend>>> {
        let settings = self.settings();
        let explicit = override_backend.or(settings.get(task));
        let primary = match explicit {
            Some(id) => {
                let b = self
                    .get(id)
                    .ok_or_else(|| HarnessError::NotConfigured(id.to_string()))?;
                if task.is_background() && !background_ok(b.as_ref()) {
                    return Err(HarnessError::Routing(format!(
                        "{} is a plan backend; background {} never runs on a subscription",
                        b.id(),
                        task.as_str()
                    )));
                }
                b
            }
            None => self.default_backend(task).ok_or_else(|| {
                HarnessError::Routing(format!(
                    "no backend available for {}; configure a local model or an api key",
                    task.as_str()
                ))
            })?,
        };
        let mut out = vec![primary];
        for id in settings.fallbacks.get(&task).into_iter().flatten() {
            let Some(b) = self.get(id) else { continue };
            if task.is_background() && !background_ok(b.as_ref()) {
                tracing::warn!(
                    "ignoring fallback {id} for background {}: plan backend",
                    task.as_str()
                );
                continue;
            }
            if !out.iter().any(|x| x.id() == b.id()) {
                out.push(b);
            }
        }
        Ok(out)
    }

    /// Route and start a chat for `task`.
    pub async fn chat(&self, task: Task, req: ChatRequest, ctx: CallContext) -> Result<RoutedChat> {
        let candidates = self.plan(task, ctx.backend.as_deref())?;
        let mut last_err: Option<HarnessError> = None;
        for (i, b) in candidates.iter().enumerate() {
            // belt and braces: the policy holds even if `plan` were bypassed
            if task.is_background() && !background_ok(b.as_ref()) {
                continue;
            }
            let now = Utc::now();
            if let Some(until) = self.limits.cooling_until(b.id(), now) {
                last_err = Some(HarnessError::CoolingDown { until });
                continue;
            }
            let model = req.model.clone().unwrap_or_else(|| b.model_for(task));
            let egress = if b.is_local() {
                None
            } else {
                // refusals are final: they do not fall through to another backend
                Some(
                    egress_check_with_consent(&ctx.items, &ctx.approved)?.for_call(
                        b.provider(),
                        &model,
                        task.as_str(),
                        req.chars(),
                    ),
                )
            };
            let mut r = req.clone();
            r.model = Some(model.clone());
            match b.chat(r).await {
                Ok(stream) => {
                    let stream = self.instrument(b.clone(), model.clone(), stream);
                    return Ok(RoutedChat {
                        backend: b.id().to_string(),
                        model,
                        egress,
                        fell_back: i > 0,
                        stream,
                    });
                }
                Err(e) => {
                    let failure = e.failure_kind();
                    let next = matches!(
                        failure,
                        FailureKind::RateLimited { .. }
                            | FailureKind::UsageLimit { .. }
                            | FailureKind::Unavailable
                    );
                    if next {
                        self.limits.record_failure(b.id(), &failure, now);
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| HarnessError::Routing("no usable backend".into())))
    }

    /// Record usage and limit state as events flow past.
    fn instrument(&self, b: Arc<dyn Backend>, model: String, stream: ChatStream) -> ChatStream {
        let limits = self.limits.clone();
        let usage = self.usage.clone();
        let prices = self.prices.clone();
        let id = b.id().to_string();
        let cost_model = b.capabilities().cost_model;
        Box::pin(stream.inspect(move |ev| match ev {
            ChatEvent::Usage(u) => {
                let p = prices.read().unwrap().clone();
                usage.record(&id, &model, cost_model, u, &p);
            }
            ChatEvent::Done { .. } => limits.record_success(&id),
            ChatEvent::Error { failure, .. } => {
                limits.record_failure(&id, failure, Utc::now());
            }
            ChatEvent::Token { .. } => {}
        }))
    }

    /// Rows for `GET /api/backends`.
    pub fn backends_info(&self) -> Vec<BackendInfo> {
        let avail = self.availability.read().unwrap();
        let now = Utc::now();
        let cds = self.limits.snapshot();
        self.backends
            .iter()
            .map(|b| {
                let caps = b.capabilities();
                let a = avail.get(b.id());
                let cooldown = cds
                    .iter()
                    .find(|c| c.backend == b.id() && c.until.is_some_and(|u| u > now))
                    .cloned();
                BackendInfo {
                    id: b.id().to_string(),
                    kind: b.kind(),
                    label: b.label(),
                    available: a.map(|a| a.available).unwrap_or(false),
                    status: a
                        .map(|a| a.status.clone())
                        .unwrap_or_else(|| "not checked".into()),
                    terms: caps.terms,
                    is_local: b.is_local(),
                    leaves_machine: b.leaves_machine(),
                    notice: caps.notice.clone(),
                    capabilities: caps,
                    cooldown,
                    models: a.map(|a| a.models.clone()).unwrap_or_default(),
                }
            })
            .collect()
    }

    /// The effective backend per task for the ui (`routing: {chat, label, extract, summarise}`).
    pub fn effective_routing(&self) -> BTreeMap<Task, Option<String>> {
        Task::ALL
            .iter()
            .map(|t| {
                (
                    *t,
                    self.plan(*t, None)
                        .ok()
                        .and_then(|v| v.first().map(|b| b.id().to_string())),
                )
            })
            .collect()
    }
}
