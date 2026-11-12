//! Distil: are the notes proposed from a conversation worth keeping?
//!
//! Each `DistilProposed` proposal the user answers (`DistilDecided`, keep or drop) is
//! one observation. The headline is **kept / proposed** over the proposals the user
//! answered, with a Wilson interval; proposals still waiting for an answer are counted
//! (`pending`) but are not evidence either way. It is a rate of the user's own
//! judgement, not a comparison: there is no control arm, so it says how often the
//! distiller is useful, not whether it beats anything.
//!
//! ## readiness
//! The Wilson interval's half-width at p = ½ is about ±15 points at n = 40 answers,
//! ±11 at 80. The status is "enough data" from 40 answered proposals.

use std::collections::{BTreeMap, HashMap};

use anamnesis_core::{Event, EventKind, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{Estimate, NOVELTY_DAYS, Novelty, Status, Window, first_at};
use crate::stats;

/// Answered proposals needed before the rate is read as a result.
pub const ENOUGH: usize = 40;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistilConfig {
    pub level: f64,
    pub enough: usize,
    pub novelty_days: i64,
}

impl Default for DistilConfig {
    fn default() -> Self {
        DistilConfig {
            level: 0.95,
            enough: ENOUGH,
            novelty_days: NOVELTY_DAYS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistilMetrics {
    /// Answered proposals.
    pub n: usize,
    pub kept: usize,
    pub dropped: usize,
    /// kept / n with a Wilson interval.
    pub keep_rate: Estimate,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistillerResult {
    /// `"rule"` for every rule version, else the extractor as recorded (`local:<model>`, …).
    pub distiller: String,
    pub proposed: usize,
    pub n: usize,
    pub kept: usize,
    pub keep_rate: Estimate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistilReport {
    /// Every proposal ever made.
    pub proposed: usize,
    /// Proposals not answered yet.
    pub pending: usize,
    #[serde(flatten)]
    pub all: DistilMetrics,
    pub by_distiller: Vec<DistillerResult>,
    pub novelty: Novelty<DistilMetrics>,
}

struct Proposal {
    distiller: String,
    decision: Option<(bool, Timestamp)>,
}

fn distiller_key(extractor: &str) -> String {
    if extractor == "rule" || extractor.starts_with("rule:") {
        "rule".into()
    } else {
        extractor.to_string()
    }
}

pub fn distil_metrics(events: &[Event], cfg: &DistilConfig) -> DistilReport {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| (e.at, e.seq));
    let mut props: HashMap<String, Proposal> = HashMap::new();
    for e in sorted {
        match &e.kind {
            EventKind::DistilProposed { proposals } => {
                for p in proposals {
                    props.entry(p.id.0.clone()).or_insert(Proposal {
                        distiller: distiller_key(&p.extractor),
                        decision: None,
                    });
                }
            }
            EventKind::DistilDecided {
                proposal_id, kept, ..
            } => {
                // the first answer counts (the store refuses a second one anyway)
                if let Some(p) = props.get_mut(&proposal_id.0)
                    && p.decision.is_none()
                {
                    p.decision = Some((*kept, e.at));
                }
            }
            _ => {}
        }
    }
    let list: Vec<&Proposal> = props.values().collect();
    let all = aggregate(&list, cfg, Window::ALL);
    let feature_start = first_at(events, |e| {
        matches!(e.kind, EventKind::DistilProposed { .. })
    });
    let novelty = Novelty::compute(feature_start, cfg.novelty_days, |w| {
        aggregate(&list, cfg, w)
    });
    let mut by: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for p in &list {
        let e = by.entry(p.distiller.clone()).or_default();
        e.0 += 1;
        if let Some((kept, _)) = p.decision {
            e.1 += 1;
            if kept {
                e.2 += 1;
            }
        }
    }
    let by_distiller = by
        .into_iter()
        .map(|(distiller, (proposed, n, kept))| DistillerResult {
            distiller,
            proposed,
            n,
            kept,
            keep_rate: estimate(kept, n, cfg.level),
        })
        .collect();
    DistilReport {
        proposed: list.len(),
        pending: list.iter().filter(|p| p.decision.is_none()).count(),
        all,
        by_distiller,
        novelty,
    }
}

fn estimate(kept: usize, n: usize, level: f64) -> Estimate {
    Estimate {
        n,
        value: (n > 0).then(|| kept as f64 / n as f64),
        interval: stats::wilson(kept, n, level),
    }
}

fn aggregate(list: &[&Proposal], cfg: &DistilConfig, w: Window) -> DistilMetrics {
    let (mut n, mut kept) = (0, 0);
    for p in list {
        if let Some((k, at)) = p.decision
            && w.contains(at)
        {
            n += 1;
            if k {
                kept += 1;
            }
        }
    }
    DistilMetrics {
        n,
        kept,
        dropped: n - kept,
        keep_rate: estimate(kept, n, cfg.level),
        status: Status::from_bool(n >= cfg.enough),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anamnesis_core::{Actor, DistilKind, DistilProposal, Id};
    use chrono::{Duration, TimeZone, Utc};

    fn ev(seq: u64, day: i64, kind: EventKind) -> Event {
        Event {
            seq,
            at: Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).unwrap() + Duration::days(day),
            actor: Actor::User,
            kind,
            prev_hash: String::new(),
            hash: String::new(),
        }
    }

    fn prop(id: &str, extractor: &str) -> DistilProposal {
        DistilProposal {
            id: Id(id.into()),
            chat_id: None,
            kind: DistilKind::Idea,
            title: "t".into(),
            quote: "q".into(),
            source_item: None,
            start: 0,
            end: 1,
            extractor: extractor.into(),
            created_at: Utc::now(),
        }
    }

    fn decided(seq: u64, day: i64, id: &str, kept: bool) -> Event {
        ev(
            seq,
            day,
            EventKind::DistilDecided {
                proposal_id: Id(id.into()),
                kept,
                item_id: None,
            },
        )
    }

    #[test]
    fn empty_log_is_measuring() {
        let r = distil_metrics(&[], &DistilConfig::default());
        assert_eq!(r.proposed, 0);
        assert_eq!(r.all.n, 0);
        assert_eq!(r.all.keep_rate.value, None);
        assert_eq!(r.all.status, Status::Measuring);
        assert_eq!(r.novelty.feature_start, None);
    }

    #[test]
    fn counts_answers_not_pending_and_splits_by_distiller_and_time() {
        let mut events = vec![ev(
            1,
            0,
            EventKind::DistilProposed {
                proposals: vec![
                    prop("a", "rule:v2"),
                    prop("b", "rule:v2"),
                    prop("c", "local:qwen"),
                    prop("d", "local:qwen"),
                ],
            },
        )];
        events.push(decided(2, 1, "a", true));
        events.push(decided(3, 1, "b", false));
        events.push(decided(4, 40, "c", true));
        events.push(decided(5, 41, "c", false)); // a second answer is ignored
        events.push(decided(6, 41, "zzz", true)); // unknown proposal
        let r = distil_metrics(&events, &DistilConfig::default());
        assert_eq!(r.proposed, 4);
        assert_eq!(r.pending, 1);
        assert_eq!((r.all.n, r.all.kept, r.all.dropped), (3, 2, 1));
        assert!((r.all.keep_rate.value.unwrap() - 2.0 / 3.0).abs() < 1e-9);
        let iv = r.all.keep_rate.interval.clone().unwrap();
        assert!(iv.lo < 2.0 / 3.0 && iv.hi > 2.0 / 3.0);
        assert_eq!(r.all.status, Status::Measuring);
        assert_eq!(r.by_distiller.len(), 2);
        let local = r
            .by_distiller
            .iter()
            .find(|d| d.distiller == "local:qwen")
            .unwrap();
        assert_eq!((local.proposed, local.n, local.kept), (2, 1, 1));
        assert_eq!(r.novelty.first_days.n, 2);
        assert_eq!(r.novelty.after.n, 1);
    }

    #[test]
    fn enough_data_after_forty_answers() {
        let ids: Vec<String> = (0..40).map(|i| format!("p{i}")).collect();
        let mut events = vec![ev(
            0,
            0,
            EventKind::DistilProposed {
                proposals: ids.iter().map(|i| prop(i, "rule:v2")).collect(),
            },
        )];
        for (k, id) in ids.iter().enumerate() {
            events.push(decided(k as u64 + 1, 1, id, k % 4 != 0));
        }
        let r = distil_metrics(&events, &DistilConfig::default());
        assert_eq!(r.all.n, 40);
        assert_eq!(r.all.kept, 30);
        assert_eq!(r.all.status, Status::EnoughData);
    }
}
