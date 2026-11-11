//! The honesty table: every headline claim the app makes, classified as
//!
//! * **measured** — a randomised comparison or a labelled test set, with an interval
//!   and enough data;
//! * **modelled** — an estimate that rests on a modelling assumption (e.g. the
//!   latent share's inverse-propensity correction);
//! * **hypothesis** — not yet supported: too little data, or not measured at all.
//!
//! A claim whose metric is still "measuring" is always a hypothesis, whatever its method.
//!
//! `n` always counts observations of the user: hand links, suggestions actually
//! displayed, labels, ratings. Things the app did on its own (suggestions it
//! computed, pairs it held out, clustering runs) are never an `n` by themselves:
//! a brain nobody has used yet reads "measuring" everywhere.

use serde::{Deserialize, Serialize};

use crate::EvalReport;
use crate::common::Status;
use crate::stats::Interval;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimClass {
    Measured,
    Modelled,
    Hypothesis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub key: String,
    /// The claim in ui voice (lowercase).
    pub claim: String,
    pub class: ClaimClass,
    pub n: usize,
    pub value: Option<f64>,
    pub interval: Option<Interval>,
    pub status: Status,
    /// How the number is obtained.
    pub basis: String,
    /// What could make it wrong.
    pub caveat: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Honesty {
    pub claims: Vec<Claim>,
}

fn class_for(method: ClaimClass, status: Status, n: usize) -> ClaimClass {
    if n == 0 || status == Status::Measuring {
        ClaimClass::Hypothesis
    } else {
        method
    }
}

#[allow(clippy::too_many_arguments)]
fn claim(
    key: &str,
    text: &str,
    method: ClaimClass,
    n: usize,
    value: Option<f64>,
    interval: Option<Interval>,
    status: Status,
    basis: &str,
    caveat: &str,
) -> Claim {
    Claim {
        key: key.into(),
        claim: text.into(),
        class: class_for(method, status, n),
        n,
        value,
        interval,
        status,
        basis: basis.into(),
        caveat: caveat.into(),
    }
}

/// Build the honesty table from a full report.
pub fn honesty(r: &EvalReport) -> Honesty {
    let mut claims = Vec::new();
    let l = &r.links.all;

    let ho = &l.latent.held_out;
    claims.push(claim(
        "latent_share",
        "a share of the links you make by hand was already latent in what you had written",
        ClaimClass::Modelled,
        ho.estimate.n,
        ho.estimate.value,
        ho.estimate.interval.clone(),
        l.latent.status,
        "hand links matching randomly held-out suggestions, reweighted by the holdout share, against hand links nobody predicted",
        "assumes showing suggestions does not change how many unpredicted links you make; held-out pairs are a small sample",
    ));

    let hr = &l.hit_rate.held_out;
    // held-out pairs accumulate without the user doing anything; a hit rate means
    // something only once the user has been making links by hand
    let hand_enough = l.latent.hand_links >= r.config.links.min_hand_links;
    let hr_status = Status::from_bool(hr.pairs >= r.config.links.min_held_out_pairs && hand_enough);
    claims.push(claim(
        "held_out_hit_rate",
        "suggestions you never saw still became links you made yourself",
        ClaimClass::Measured,
        hr.pairs,
        hr.estimate.value,
        hr.estimate.interval.clone(),
        hr_status,
        &format!("randomised holdout: share of never-shown top-{} suggestions linked by hand within {} days", r.links.k, r.links.window_days),
        &format!(
            "pairs whose window is still open are left out; long-delayed links are missed; read only after {} hand links",
            r.config.links.min_hand_links
        ),
    ));

    let acc = &l.acceptance;
    let displayed_enough = acc.displayed >= 30;

    // the shown arm exists only once suggestions were actually displayed
    let lift_n = if r.links.displayed_pairs > 0 {
        l.latent.shown.estimate.n
    } else {
        0
    };
    let lift_status = if l.latent.status == Status::EnoughData && displayed_enough {
        Status::EnoughData
    } else {
        Status::Measuring
    };
    claims.push(claim(
        "exposure_lift",
        "seeing a suggestion changes what you link",
        ClaimClass::Modelled,
        lift_n,
        l.latent.exposure_lift,
        None,
        lift_status,
        "latent share estimated from suggestions actually displayed to you, minus the held-out estimate",
        "no interval yet; both arms share the unpredicted-link count; which suggestions you open is not random",
    ));

    claims.push(claim(
        "acceptance_rate",
        "you accept some of the suggestions you are shown (a diagnostic, not a quality measure)",
        ClaimClass::Measured,
        acc.displayed,
        acc.rate.value,
        acc.rate.interval.clone(),
        Status::from_bool(displayed_enough),
        &format!(
            "accepted / suggestions actually displayed to you ({} of {} eligible were displayed)",
            acc.displayed, acc.eligible
        ),
        "acceptance happens under influence; optimising it would reward persuasion, not insight",
    ));

    match &r.retrieval {
        Some(rt) => claims.push(claim(
            "retrieval_ndcg",
            "search finds what you consider relevant",
            ClaimClass::Measured,
            rt.all.n,
            rt.all.ndcg_at_10.value,
            rt.all.ndcg_at_10.interval.clone(),
            rt.all.status,
            "ndcg@10 on your own graded labels, bootstrap over queries",
            "labels go stale as the corpus grows; queries you chose to label may be easier than typical ones",
        )),
        None => claims.push(claim(
            "retrieval_ndcg",
            "search finds what you consider relevant",
            ClaimClass::Measured,
            0,
            None,
            None,
            Status::Measuring,
            "needs a search function to score labelled queries",
            "not computed in this report",
        )),
    }

    let rs = &r.resurfacing.all;
    claims.push(claim(
        "resurfacing_vs_random",
        "resurfaced notes are more useful than a random pick",
        ClaimClass::Measured,
        rs.n,
        rs.difference.mean,
        rs.difference.interval.clone(),
        rs.status,
        "micro-randomised trial, blind arms; difference of jeffreys posteriors",
        "one person over time: ratings drift with mood and season, so intervals are approximate",
    ));

    let after = &r.resurfacing.novelty.after;
    claims.push(claim(
        "resurfacing_after_novelty",
        "the resurfacing effect lasts past the first four weeks",
        ClaimClass::Measured,
        after.n,
        after.difference.mean,
        after.difference.interval.clone(),
        after.status,
        "the same trial restricted to ratings after the feature's first 28 days",
        "early enthusiasm inflates new features; this half excludes it",
    ));

    let g = &r.growth;
    // identity over time is only observed across clustering runs on different days
    // (one import followed by two quick recomputes says nothing about time)
    let run_days: std::collections::BTreeSet<chrono::NaiveDate> = g
        .regions_over_time
        .iter()
        .map(|run| run.at.date_naive())
        .collect();
    claims.push(claim(
        "growth",
        "the brain grows and its regions keep their identity over time",
        ClaimClass::Measured,
        g.n,
        Some(g.n as f64),
        None,
        Status::from_bool(g.n > 0 && run_days.len() >= 2),
        "counts from the event log; region transitions from clustering runs on different days",
        "region identity depends on the clustering parameters",
    ));

    claims.push(Claim {
        key: "thinks_like_me".into(),
        claim: "the brain thinks like you".into(),
        class: ClaimClass::Hypothesis,
        n: 0,
        value: None,
        interval: None,
        status: Status::Measuring,
        basis: "not measured yet: needs held-out decisions scored against your own test-retest consistency".into(),
        caveat: "until then this is a hope, not a result".into(),
    });

    Honesty { claims }
}

impl Honesty {
    /// A markdown table for the readme or an export.
    pub fn to_markdown(&self) -> String {
        let mut s = String::from(
            "| claim | class | n | estimate | 95% interval | status |\n|---|---|---|---|---|---|\n",
        );
        for c in &self.claims {
            let class = match c.class {
                ClaimClass::Measured => "measured",
                ClaimClass::Modelled => "modelled",
                ClaimClass::Hypothesis => "hypothesis",
            };
            let v = c
                .value
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into());
            let i = c
                .interval
                .as_ref()
                .map(|i| format!("{:.2} – {:.2}", i.lo, i.hi))
                .unwrap_or_else(|| "—".into());
            let st = match c.status {
                Status::Measuring => "measuring",
                Status::EnoughData => "enough data",
            };
            s.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                c.claim, class, c.n, v, i, st
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measuring_is_always_hypothesis() {
        assert_eq!(
            class_for(ClaimClass::Measured, Status::Measuring, 50),
            ClaimClass::Hypothesis
        );
        assert_eq!(
            class_for(ClaimClass::Modelled, Status::EnoughData, 0),
            ClaimClass::Hypothesis
        );
        assert_eq!(
            class_for(ClaimClass::Measured, Status::EnoughData, 5),
            ClaimClass::Measured
        );
    }
}
