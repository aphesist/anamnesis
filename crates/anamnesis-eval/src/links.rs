//! Link suggestions scored against the links the user later makes by hand.
//!
//! The headline number is the **latent share**: of the links you made by hand,
//! what share had the system already predicted from what you had written?
//!
//! ## design
//! * Every suggestion is logged as `SuggestionMade` with `shown`. About 20% of
//!   top-k suggestions are randomly held out (`shown = false`) and never displayed.
//! * Links are matched to suggestions as **unordered pairs** (a hand link `b → a`
//!   matches a suggestion `a → b`). Only the first link per pair counts; self links
//!   are ignored.
//! * "Hand links" are `LinkKind::Manual` and `LinkKind::Written`. Accepting a
//!   suggestion (`AcceptedSuggestion`) is exposure-driven by construction, so it
//!   only ever counts in the shown arm.
//! * A pair is *predicted* at time t if some suggestion for it with `rank <= k`
//!   was made strictly before t. It is in the **shown arm** if any such suggestion
//!   was displayed before t, otherwise in the **held-out arm**.
//!
//! ## latent share estimator
//! Let H be hand links whose pair was predicted in the held-out arm, U hand links
//! whose pair was not predicted at all, and π_h the share of predicted pairs that
//! were held out (a design constant, read from the log). Because held-out pairs
//! are a random π_h sample of predicted pairs and were never shown, H/π_h estimates
//! how many predicted pairs you would have linked anyway, so
//!
//! ```text
//! latent share = (H/π_h) / (H/π_h + U)
//! ```
//!
//! The same formula on the shown arm (S, π_s = 1 − π_h) is reported alongside: it
//! is contaminated by exposure, and the gap between the two is the exposure lift.
//! The estimator assumes that showing suggestions does not change how many
//! *unpredicted* links you make, which is why it is classed as *modelled*.
//! Its interval is Monte Carlo over Jeffreys (Gamma(x + ½)) posteriors of the two
//! counts, with π treated as known.

use std::collections::{BTreeMap, HashMap};

use anamnesis_core::{Event, EventKind, Id, LinkKind, Timestamp};
use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::common::{
    Estimate, NOVELTY_DAYS, Novelty, Status, Window, as_of, days_between, first_at,
};
use crate::stats::{self, Rng};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkConfig {
    /// Suggestions with `rank <= k` count as predictions (rank is 1-based).
    pub k: u32,
    /// Outcome window after a suggestion, in days, for hit rates and ranking metrics.
    pub window_days: i64,
    /// Hand links needed before the latent share is read as a result.
    pub min_hand_links: usize,
    /// Held-out predicted pairs (with a closed window) needed before results are read.
    pub min_held_out_pairs: usize,
    pub level: f64,
    pub draws: usize,
    pub bootstrap_resamples: usize,
    pub seed: u64,
    pub novelty_days: i64,
    /// Read the log as of this time (defaults to the last event).
    pub as_of: Option<Timestamp>,
}

impl Default for LinkConfig {
    fn default() -> Self {
        LinkConfig {
            k: 5,
            window_days: 90,
            min_hand_links: 50,
            min_held_out_pairs: 30,
            level: 0.95,
            draws: 20_000,
            bootstrap_resamples: 2_000,
            seed: 0x11_4e_75,
            novelty_days: NOVELTY_DAYS,
            as_of: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    HeldOut,
    Shown,
}

/// One arm's estimate of the latent share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentArm {
    pub arm: Arm,
    /// Links (in the window) whose pair was predicted in this arm before the link.
    pub links_predicted: usize,
    /// Of those, made by accepting a suggestion (always 0 for the held-out arm).
    pub links_accepted: usize,
    /// Hand links whose pair was never predicted before the link.
    pub links_unpredicted: usize,
    /// Share of predicted pairs that fell in this arm (the design propensity).
    pub propensity: Option<f64>,
    /// Latent share estimated from this arm; `n` = links_predicted + links_unpredicted.
    pub estimate: Estimate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentShare {
    /// Manual + written links in the window.
    pub hand_links: usize,
    /// Plain share of hand links that had been predicted before they were made
    /// (either arm). Contaminated by exposure; shown for transparency only.
    pub predicted_before_raw: Estimate,
    /// The unbiased estimate (from randomly held-out suggestions).
    pub held_out: LatentArm,
    /// The same estimator on displayed suggestions (inflated by exposure).
    pub shown: LatentArm,
    /// shown − held-out point estimates: how much showing suggestions shifts linking.
    pub exposure_lift: Option<f64>,
    pub status: Status,
}

/// Share of an arm's predicted pairs that became links within the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitRate {
    pub arm: Arm,
    /// Predicted pairs whose window has closed.
    pub pairs: usize,
    pub linked: usize,
    /// Predicted pairs whose window is still open (excluded: right-censored).
    pub pending: usize,
    pub estimate: Estimate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitRates {
    pub held_out: HitRate,
    pub shown: HitRate,
}

/// precision@k / recall@k / MRR of suggestion lists against later links.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ranking {
    pub k: u32,
    pub window_days: i64,
    /// Items with a suggestion list whose window has closed.
    pub queries: usize,
    /// Of those, items that got at least one qualifying link in the window.
    pub queries_with_links: usize,
    /// Items whose window is still open.
    pub pending: usize,
    /// Over all closed queries.
    pub precision_at_k: Estimate,
    /// Over queries with at least one qualifying link.
    pub recall_at_k: Estimate,
    pub mrr: Estimate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rankings {
    /// All suggestions vs all later links (incl. accepted). Contaminated by exposure.
    pub all: Ranking,
    /// Held-out suggestions only vs hand links to targets never shown for the pair.
    /// Precision is per held-out slot (hits / held-out suggestions in the top k).
    pub held_out: Ranking,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Acceptance {
    pub shown: usize,
    pub accepted: usize,
    pub dismissed: usize,
    pub ignored: usize,
    /// accepted / shown
    pub rate: Estimate,
    /// accepted / (accepted + dismissed)
    pub rate_of_responses: Estimate,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bucket {
    pub label: String,
    pub count: usize,
}

/// Days from the first top-k prediction of a pair to the link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Distribution {
    pub n: usize,
    pub median: Option<f64>,
    pub p25: Option<f64>,
    pub p75: Option<f64>,
    pub mean: Option<f64>,
    pub buckets: Vec<Bucket>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeToLink {
    pub held_out: Distribution,
    pub shown: Distribution,
}

/// All link metrics for one time window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkMetrics {
    pub latent: LatentShare,
    pub hit_rate: HitRates,
    pub ranking: Rankings,
    pub acceptance: Acceptance,
    pub time_to_link: TimeToLink,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinksReport {
    pub k: u32,
    pub window_days: i64,
    pub as_of: Option<Timestamp>,
    /// Predicted pairs over the whole log, by arm.
    pub predicted_pairs: usize,
    pub held_out_pairs: usize,
    pub status: Status,
    #[serde(flatten)]
    pub all: LinkMetrics,
    pub novelty: Novelty<LinkMetrics>,
}

// ---------------------------------------------------------------------------

type Pair = (String, String);

fn pair(a: &Id, b: &Id) -> Option<Pair> {
    use std::cmp::Ordering::*;
    match a.0.cmp(&b.0) {
        Less => Some((a.0.clone(), b.0.clone())),
        Greater => Some((b.0.clone(), a.0.clone())),
        Equal => None,
    }
}

#[derive(Debug, Clone)]
struct Sugg {
    id: Id,
    from: Id,
    to: Id,
    rank: u32,
    shown: bool,
    at: Timestamp,
}

#[derive(Debug, Clone)]
struct PairInfo {
    first_at: Timestamp,
    first_shown_at: Option<Timestamp>,
    first_held_at: Option<Timestamp>,
}

impl PairInfo {
    fn arm_before(&self, t: Timestamp) -> Option<Arm> {
        if self.first_shown_at.is_some_and(|s| s < t) {
            Some(Arm::Shown)
        } else if self.first_held_at.is_some_and(|h| h < t) {
            Some(Arm::HeldOut)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
struct LinkObs {
    kind: LinkKind,
    at: Timestamp,
}

fn is_hand(kind: LinkKind) -> bool {
    matches!(kind, LinkKind::Manual | LinkKind::Written)
}

struct Parsed {
    top: Vec<Sugg>,
    pairs: HashMap<Pair, PairInfo>,
    links: HashMap<Pair, LinkObs>,
    /// links in time order
    link_list: Vec<(Pair, LinkObs)>,
    accepted: HashMap<String, Timestamp>,
    dismissed: HashMap<String, Timestamp>,
    feature_start: Option<Timestamp>,
    as_of: Option<Timestamp>,
}

fn parse(events: &[Event], cfg: &LinkConfig) -> Parsed {
    let mut top = Vec::new();
    let mut pairs: HashMap<Pair, PairInfo> = HashMap::new();
    let mut links: HashMap<Pair, LinkObs> = HashMap::new();
    let mut link_list = Vec::new();
    let mut accepted = HashMap::new();
    let mut dismissed = HashMap::new();
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| (e.at, e.seq));
    for e in sorted {
        match &e.kind {
            EventKind::SuggestionMade {
                suggestion_id,
                from,
                to,
                rank,
                shown,
                ..
            } => {
                if *rank > cfg.k {
                    continue;
                }
                let Some(p) = pair(from, to) else { continue };
                let info = pairs.entry(p).or_insert(PairInfo {
                    first_at: e.at,
                    first_shown_at: None,
                    first_held_at: None,
                });
                if *shown {
                    info.first_shown_at.get_or_insert(e.at);
                } else {
                    info.first_held_at.get_or_insert(e.at);
                }
                top.push(Sugg {
                    id: suggestion_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    rank: *rank,
                    shown: *shown,
                    at: e.at,
                });
            }
            EventKind::SuggestionAccepted { suggestion_id } => {
                accepted.entry(suggestion_id.0.clone()).or_insert(e.at);
            }
            EventKind::SuggestionDismissed { suggestion_id } => {
                dismissed.entry(suggestion_id.0.clone()).or_insert(e.at);
            }
            EventKind::LinkAdded { link } => {
                let Some(p) = pair(&link.from, &link.to) else {
                    continue;
                };
                if links.contains_key(&p) {
                    continue;
                }
                // the event time is when the brain learned of the link
                let obs = LinkObs {
                    kind: link.kind,
                    at: e.at,
                };
                links.insert(p.clone(), obs.clone());
                link_list.push((p, obs));
            }
            _ => {}
        }
    }
    let feature_start = first_at(events, |e| {
        matches!(e.kind, EventKind::SuggestionMade { .. })
    });
    Parsed {
        top,
        pairs,
        links,
        link_list,
        accepted,
        dismissed,
        feature_start,
        as_of: as_of(events, cfg.as_of),
    }
}

/// Compute all link metrics from the event log.
pub fn link_metrics(events: &[Event], cfg: &LinkConfig) -> LinksReport {
    let p = parse(events, cfg);
    let predicted_pairs = p.pairs.len();
    let held_out_pairs = p
        .pairs
        .values()
        .filter(|i| i.first_shown_at.is_none())
        .count();
    let all = metrics_in(&p, cfg, Window::ALL);
    let novelty = Novelty::compute(p.feature_start, cfg.novelty_days, |w| {
        metrics_in(&p, cfg, w)
    });
    let status = all.latent.status;
    LinksReport {
        k: cfg.k,
        window_days: cfg.window_days,
        as_of: p.as_of,
        predicted_pairs,
        held_out_pairs,
        status,
        all,
        novelty,
    }
}

fn metrics_in(p: &Parsed, cfg: &LinkConfig, w: Window) -> LinkMetrics {
    let hit_rate = hit_rates(p, cfg, w);
    let mut latent = latent_share(p, cfg, w);
    latent.status = Status::from_bool(
        latent.hand_links >= cfg.min_hand_links
            && hit_rate.held_out.pairs >= cfg.min_held_out_pairs,
    );
    LinkMetrics {
        latent,
        hit_rate,
        ranking: rankings(p, cfg, w),
        acceptance: acceptance(p, cfg, w),
        time_to_link: time_to_link(p, w),
    }
}

fn latent_share(p: &Parsed, cfg: &LinkConfig, w: Window) -> LatentShare {
    let total = p.pairs.len();
    let held = p
        .pairs
        .values()
        .filter(|i| i.first_shown_at.is_none())
        .count();
    let pi_h = (total > 0).then(|| held as f64 / total as f64);
    let pi_s = pi_h.map(|x| 1.0 - x);

    let (mut h, mut s, mut s_acc, mut s_hand, mut u, mut hand) = (0, 0, 0, 0, 0, 0);
    for (pr, l) in &p.link_list {
        if !w.contains(l.at) {
            continue;
        }
        let arm = p.pairs.get(pr).and_then(|i| i.arm_before(l.at));
        let hand_link = is_hand(l.kind);
        if hand_link {
            hand += 1;
        }
        match (arm, hand_link) {
            (None, true) => u += 1,
            (None, false) => {} // accepted link without a prior top-k suggestion: ignore
            (Some(Arm::HeldOut), true) => h += 1,
            (Some(Arm::HeldOut), false) => {} // cannot accept an unseen suggestion
            (Some(Arm::Shown), true) => {
                s += 1;
                s_hand += 1;
            }
            (Some(Arm::Shown), false) => {
                s += 1;
                s_acc += 1;
            }
        }
    }

    let held_out = latent_arm(Arm::HeldOut, h, 0, u, pi_h, cfg, cfg.seed ^ 0xa1);
    let shown = latent_arm(Arm::Shown, s, s_acc, u, pi_s, cfg, cfg.seed ^ 0xb2);
    let exposure_lift = match (held_out.estimate.value, shown.estimate.value) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    let raw_k = h + s_hand;
    LatentShare {
        hand_links: hand,
        predicted_before_raw: Estimate {
            n: hand,
            value: (hand > 0).then(|| raw_k as f64 / hand as f64),
            interval: stats::wilson(raw_k, hand, cfg.level),
        },
        held_out,
        shown,
        exposure_lift,
        status: Status::Measuring, // set by the caller
    }
}

fn latent_arm(
    arm: Arm,
    x: usize,
    accepted: usize,
    u: usize,
    pi: Option<f64>,
    cfg: &LinkConfig,
    seed: u64,
) -> LatentArm {
    let n = x + u;
    let estimate = match pi {
        Some(pi) if pi > 0.0 && n > 0 => {
            let latent = x as f64 / pi;
            let value = latent / (latent + u as f64);
            let mut rng = Rng::new(seed);
            let mut draws: Vec<f64> = (0..cfg.draws)
                .map(|_| {
                    let l = rng.gamma(x as f64 + 0.5) / pi;
                    let un = rng.gamma(u as f64 + 0.5);
                    l / (l + un)
                })
                .collect();
            Estimate {
                n,
                value: Some(value),
                interval: stats::draws_interval(
                    &mut draws,
                    cfg.level,
                    "monte carlo (jeffreys counts)",
                ),
            }
        }
        _ => Estimate {
            n,
            value: None,
            interval: None,
        },
    };
    LatentArm {
        arm,
        links_predicted: x,
        links_accepted: accepted,
        links_unpredicted: u,
        propensity: pi,
        estimate,
    }
}

fn hit_rates(p: &Parsed, cfg: &LinkConfig, w: Window) -> HitRates {
    let win = Duration::days(cfg.window_days);
    let mut counts: BTreeMap<u8, (usize, usize, usize)> = BTreeMap::new(); // arm -> (pairs, linked, pending)
    for (pr, info) in &p.pairs {
        if !w.contains(info.first_at) {
            continue;
        }
        let end = info.first_at + win;
        let link = p
            .links
            .get(pr)
            .filter(|l| l.at > info.first_at && l.at <= end);
        let exposed_by = link.map(|l| l.at).unwrap_or(end);
        let arm = if info.first_shown_at.is_some_and(|s| s < exposed_by) {
            Arm::Shown
        } else {
            Arm::HeldOut
        };
        let linked = match (arm, link) {
            (Arm::HeldOut, Some(l)) => is_hand(l.kind),
            (Arm::Shown, Some(_)) => true,
            (_, None) => false,
        };
        let c = counts.entry(arm as u8).or_default();
        if p.as_of.is_none_or(|a| end > a) {
            // window still open: counting only the pairs already linked would bias the rate up
            c.2 += 1;
            continue;
        }
        c.0 += 1;
        if linked {
            c.1 += 1;
        }
    }
    let mk = |arm: Arm| {
        let (pairs, linked, pending) = counts.get(&(arm as u8)).copied().unwrap_or_default();
        HitRate {
            arm,
            pairs,
            linked,
            pending,
            estimate: Estimate {
                n: pairs,
                value: (pairs > 0).then(|| linked as f64 / pairs as f64),
                interval: stats::wilson(linked, pairs, cfg.level),
            },
        }
    };
    HitRates {
        held_out: mk(Arm::HeldOut),
        shown: mk(Arm::Shown),
    }
}

#[derive(Clone, Copy)]
struct QueryScore {
    precision: Option<f64>,
    recall: Option<f64>,
    rr: Option<f64>,
}

fn rankings(p: &Parsed, cfg: &LinkConfig, w: Window) -> Rankings {
    let win = Duration::days(cfg.window_days);
    // first list per source item: top-k suggestions made within a day of the first one
    let mut by_from: BTreeMap<String, Vec<&Sugg>> = BTreeMap::new();
    for s in &p.top {
        by_from.entry(s.from.0.clone()).or_default().push(s);
    }
    // all links touching an item, for recall denominators
    let mut links_of: HashMap<&str, Vec<(&str, &LinkObs)>> = HashMap::new();
    for (pr, l) in &p.link_list {
        links_of
            .entry(pr.0.as_str())
            .or_default()
            .push((pr.1.as_str(), l));
        links_of
            .entry(pr.1.as_str())
            .or_default()
            .push((pr.0.as_str(), l));
    }

    let mut all_scores = Vec::new();
    let mut held_scores = Vec::new();
    let (mut pending, mut queries) = (0, 0);
    for (from, list) in &by_from {
        let t0 = list.iter().map(|s| s.at).min().unwrap();
        if !w.contains(t0) {
            continue;
        }
        let end = t0 + win;
        if p.as_of.is_none_or(|a| end > a) {
            pending += 1;
            continue;
        }
        queries += 1;
        let mut first: Vec<&Sugg> = list
            .iter()
            .copied()
            .filter(|s| s.at <= t0 + Duration::days(1))
            .collect();
        first.sort_by_key(|s| (s.rank, s.at));
        let mut seen = std::collections::HashSet::new();
        first.retain(|s| seen.insert(s.to.0.clone()));
        first.truncate(cfg.k as usize);

        let in_window: Vec<(&str, &LinkObs)> = links_of
            .get(from.as_str())
            .map(|v| {
                v.iter()
                    .copied()
                    .filter(|(_, l)| l.at > t0 && l.at <= end)
                    .collect()
            })
            .unwrap_or_default();

        // all suggestions vs any link
        let relevant: std::collections::HashSet<&str> = in_window.iter().map(|(t, _)| *t).collect();
        let ranked: Vec<&str> = first.iter().map(|s| s.to.0.as_str()).collect();
        all_scores.push(score(&ranked, &relevant, cfg.k as usize, true));

        // held-out suggestions vs hand links to targets not shown before the link
        let held: Vec<&str> = first
            .iter()
            .filter(|s| !s.shown)
            .map(|s| s.to.0.as_str())
            .collect();
        let relevant_h: std::collections::HashSet<&str> = in_window
            .iter()
            .filter(|(t, l)| {
                is_hand(l.kind)
                    && pair(&Id(from.clone()), &Id(t.to_string()))
                        .and_then(|pr| p.pairs.get(&pr))
                        .is_none_or(|i| i.first_shown_at.is_none_or(|s| s >= l.at))
            })
            .map(|(t, _)| *t)
            .collect();
        let mut sc = score(&held, &relevant_h, cfg.k as usize, false);
        if held.is_empty() {
            sc.precision = None;
        }
        held_scores.push(sc);
    }

    let mk = |scores: &[QueryScore]| {
        let prec: Vec<f64> = scores.iter().filter_map(|s| s.precision).collect();
        let rec: Vec<f64> = scores.iter().filter_map(|s| s.recall).collect();
        let rr: Vec<f64> = scores.iter().filter_map(|s| s.rr).collect();
        let est = |v: &[f64], salt: u64| Estimate {
            n: v.len(),
            value: stats::mean(v),
            interval: stats::bootstrap_mean(v, cfg.bootstrap_resamples, cfg.level, cfg.seed ^ salt),
        };
        Ranking {
            k: cfg.k,
            window_days: cfg.window_days,
            queries,
            queries_with_links: rec.len(),
            pending,
            precision_at_k: est(&prec, 1),
            recall_at_k: est(&rec, 2),
            mrr: est(&rr, 3),
        }
    };
    Rankings {
        all: mk(&all_scores),
        held_out: mk(&held_scores),
    }
}

fn score(
    ranked: &[&str],
    relevant: &std::collections::HashSet<&str>,
    k: usize,
    fixed_k: bool,
) -> QueryScore {
    let top = &ranked[..ranked.len().min(k)];
    let hits = top.iter().filter(|t| relevant.contains(*t)).count();
    let denom = if fixed_k { k } else { top.len() };
    let precision = (denom > 0).then(|| hits as f64 / denom as f64);
    if relevant.is_empty() {
        return QueryScore {
            precision,
            recall: None,
            rr: None,
        };
    }
    let rr = ranked
        .iter()
        .position(|t| relevant.contains(*t))
        .map(|i| 1.0 / (i + 1) as f64)
        .unwrap_or(0.0);
    QueryScore {
        precision,
        recall: Some(hits as f64 / relevant.len() as f64),
        rr: Some(rr),
    }
}

fn acceptance(p: &Parsed, cfg: &LinkConfig, w: Window) -> Acceptance {
    let (mut shown, mut acc, mut dis) = (0, 0, 0);
    for s in p.top.iter().filter(|s| s.shown && w.contains(s.at)) {
        shown += 1;
        if p.accepted.contains_key(&s.id.0) {
            acc += 1;
        } else if p.dismissed.contains_key(&s.id.0) {
            dis += 1;
        }
    }
    let responses = acc + dis;
    Acceptance {
        shown,
        accepted: acc,
        dismissed: dis,
        ignored: shown - responses,
        rate: Estimate {
            n: shown,
            value: (shown > 0).then(|| acc as f64 / shown as f64),
            interval: stats::wilson(acc, shown, cfg.level),
        },
        rate_of_responses: Estimate {
            n: responses,
            value: (responses > 0).then(|| acc as f64 / responses as f64),
            interval: stats::wilson(acc, responses, cfg.level),
        },
        note: "acceptance is usefulness under influence, not a quality measure: showing a suggestion changes what you link. \
               read the held-out latent share for quality."
            .to_string(),
    }
}

fn time_to_link(p: &Parsed, w: Window) -> TimeToLink {
    let mut held = Vec::new();
    let mut shown = Vec::new();
    for (pr, l) in &p.link_list {
        if !w.contains(l.at) {
            continue;
        }
        let Some(info) = p.pairs.get(pr) else {
            continue;
        };
        let d = days_between(info.first_at, l.at);
        match (info.arm_before(l.at), is_hand(l.kind)) {
            (Some(Arm::HeldOut), true) => held.push(d),
            (Some(Arm::Shown), _) => shown.push(d),
            _ => {}
        }
    }
    TimeToLink {
        held_out: distribution(held),
        shown: distribution(shown),
    }
}

/// Summary of a set of day counts with fixed buckets (<1, 1–7, 7–30, 30–90, ≥90 days).
pub fn distribution(mut days: Vec<f64>) -> Distribution {
    stats::sort_f64(&mut days);
    let edges = [
        (0.0, 1.0, "< 1 day"),
        (1.0, 7.0, "1–7 days"),
        (7.0, 30.0, "7–30 days"),
        (30.0, 90.0, "30–90 days"),
        (90.0, f64::INFINITY, "≥ 90 days"),
    ];
    let buckets = edges
        .iter()
        .map(|(lo, hi, label)| Bucket {
            label: label.to_string(),
            count: days.iter().filter(|d| **d >= *lo && **d < *hi).count(),
        })
        .collect();
    let q = |x: f64| (!days.is_empty()).then(|| stats::quantile_sorted(&days, x));
    Distribution {
        n: days.len(),
        median: q(0.5),
        p25: q(0.25),
        p75: q(0.75),
        mean: stats::mean(&days),
        buckets,
    }
}
