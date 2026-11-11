//! anamnesis-eval: honest, user-facing metrics computed from the event log.
//!
//! Everything here is a pure function over `&[anamnesis_core::Event]` (plus, for
//! retrieval, a caller-supplied search closure) returning serialisable structs.
//! No storage, no network. Random draws (bootstrap, Monte Carlo) are seeded, so
//! the same log always gives the same report.
//!
//! * [`links`] — the latent share (held-out vs shown), hit rates, precision@k /
//!   recall@k / MRR, acceptance (a diagnostic), time to link.
//! * [`retrieval`] — nDCG@10, P@5, MRR on the user's own labels, bootstrap intervals.
//! * [`resurfacing`] — the algorithm-vs-random micro-randomised trial.
//! * [`growth`] — items, links and regions over time.
//! * [`honesty`] — measured / modelled / hypothesis for each headline claim.
//! * [`synthetic`] — a seeded, plausible event log for a simulated user.
//!
//! Every metric is also reported for the feature's first 28 days and after them
//! (`novelty`), because new features are rated more kindly while they are new.

pub mod common;
pub mod growth;
pub mod honesty;
pub mod links;
pub mod resurfacing;
pub mod retrieval;
pub mod stats;
pub mod synthetic;

use std::collections::HashMap;

use anamnesis_core::{Event, Id};
use serde::{Deserialize, Serialize};

pub use common::{Estimate, NOVELTY_DAYS, Novelty, Status, Window};
pub use growth::{GrowthReport, growth_metrics};
pub use honesty::{Claim, ClaimClass, Honesty, honesty};
pub use links::{LinkConfig, LinksReport, link_metrics};
pub use resurfacing::{ResurfacingConfig, ResurfacingReport, resurfacing_metrics};
pub use retrieval::{RetrievalConfig, RetrievalReport, retrieval_metrics};
pub use stats::Interval;

/// The app's search: ranked item ids for a query.
pub type SearchFn<'a> = &'a mut dyn FnMut(&str) -> Vec<Id>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EvalConfig {
    pub links: LinkConfig,
    pub retrieval: RetrievalConfig,
    pub resurfacing: ResurfacingConfig,
}

/// The body of `GET /api/eval`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    pub events: usize,
    pub links: LinksReport,
    /// `None` when no search function was supplied.
    pub retrieval: Option<RetrievalReport>,
    pub resurfacing: ResurfacingReport,
    pub growth: GrowthReport,
    pub honesty: Honesty,
    pub config: EvalConfig,
}

/// Compute every metric. `search` ranks item ids for a query (the app's hybrid
/// search); `cohesion` optionally maps clustering run ids to mean region cohesion.
pub fn evaluate(
    events: &[Event],
    search: Option<SearchFn<'_>>,
    cohesion: Option<&HashMap<Id, f64>>,
    cfg: &EvalConfig,
) -> EvalReport {
    let links = link_metrics(events, &cfg.links);
    let retrieval = search.map(|s| retrieval_metrics(events, s, &cfg.retrieval));
    let resurfacing = resurfacing_metrics(events, &cfg.resurfacing);
    let growth = growth_metrics(events, cohesion);
    let mut report = EvalReport {
        events: events.len(),
        links,
        retrieval,
        resurfacing,
        growth,
        honesty: Honesty { claims: Vec::new() },
        config: cfg.clone(),
    };
    report.honesty = honesty(&report);
    report
}
