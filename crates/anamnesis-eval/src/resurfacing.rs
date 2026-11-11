//! Resurfacing as a micro-randomised trial: at each slot the item comes either from
//! the algorithm or from a random pick (`ResurfaceShown { arm }`), and the user rates
//! it useful or not (`ResurfaceRated`). Arms are blind in the ui.
//!
//! Each arm's useful-rate gets a Beta-binomial posterior under the Jeffreys prior
//! Beta(½, ½), i.e. Beta(k + ½, n − k + ½). The difference algorithm − random and
//! P(algorithm > random) come from seeded Monte Carlo draws of both posteriors.
//!
//! ## readiness (power calculation)
//! Two-proportion z-test, two-sided α = 0.05, power 0.8:
//! n per arm = (1.96 + 0.84)² · [p₁(1−p₁) + p₂(1−p₂)] / (p₁ − p₂)².
//! * 70% vs 40% (a large, 30-point effect): about 40 ratings per arm (~80 in total).
//! * 60% vs 40% (a 20-point effect): about 95 ratings per arm (~190 in total).
//!
//! The status is "enough data" once the smaller arm reaches the 20-point threshold;
//! `detectable` says which effect size the current sample could detect.
//! The trial is n = 1 over time: ratings are not independent in the strict sense
//! (mood, drift), so intervals are best read as approximate.

use std::collections::HashMap;

use anamnesis_core::{Event, EventKind, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{Estimate, NOVELTY_DAYS, Novelty, Status, Window, first_at};
use crate::stats::{self, Interval, Rng};

pub const ARM_ALGORITHM: &str = "algorithm";
pub const ARM_RANDOM: &str = "random";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResurfacingConfig {
    pub level: f64,
    pub draws: usize,
    pub seed: u64,
    pub alpha: f64,
    pub power: f64,
    pub novelty_days: i64,
}

impl Default for ResurfacingConfig {
    fn default() -> Self {
        ResurfacingConfig {
            level: 0.95,
            draws: 20_000,
            seed: 0x7e_5a_7f,
            alpha: 0.05,
            power: 0.8,
            novelty_days: NOVELTY_DAYS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmResult {
    pub arm: String,
    /// Rated slots.
    pub n: usize,
    pub useful: usize,
    /// Shown but not (yet) rated.
    pub unrated: usize,
    /// k / n
    pub raw_rate: Option<f64>,
    /// Posterior mean (k + ½) / (n + 1) and Jeffreys credible interval.
    pub posterior: Estimate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    /// Posterior mean of p_algorithm − p_random.
    pub mean: Option<f64>,
    pub interval: Option<Interval>,
    /// Posterior probability that the algorithm's useful-rate is higher.
    pub p_algorithm_better: Option<f64>,
    pub draws: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Threshold {
    pub label: String,
    pub p_algorithm: f64,
    pub p_random: f64,
    pub n_per_arm: usize,
    pub n_total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Readiness {
    pub n_smaller_arm: usize,
    pub thresholds: Vec<Threshold>,
    /// "none yet" | "large (30 points)" | "moderate (20 points)"
    pub detectable: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResurfacingMetrics {
    pub n: usize,
    pub algorithm: ArmResult,
    pub random: ArmResult,
    pub difference: Difference,
    pub readiness: Readiness,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResurfacingReport {
    /// Shown slots with an arm other than "algorithm"/"random" (ignored).
    pub other_arm: usize,
    #[serde(flatten)]
    pub all: ResurfacingMetrics,
    pub novelty: Novelty<ResurfacingMetrics>,
}

struct Slot {
    arm: String,
    rating: Option<(bool, Timestamp)>,
    shown_at: Timestamp,
}

pub fn resurfacing_metrics(events: &[Event], cfg: &ResurfacingConfig) -> ResurfacingReport {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| (e.at, e.seq));
    let mut slots: HashMap<String, Slot> = HashMap::new();
    let mut order = Vec::new();
    for e in sorted {
        match &e.kind {
            EventKind::ResurfaceShown {
                resurface_id, arm, ..
            } => {
                if !slots.contains_key(&resurface_id.0) {
                    order.push(resurface_id.0.clone());
                    slots.insert(
                        resurface_id.0.clone(),
                        Slot {
                            arm: arm.to_lowercase(),
                            rating: None,
                            shown_at: e.at,
                        },
                    );
                }
            }
            EventKind::ResurfaceRated {
                resurface_id,
                useful,
            } => {
                // latest rating wins
                if let Some(s) = slots.get_mut(&resurface_id.0) {
                    s.rating = Some((*useful, e.at));
                }
            }
            _ => {}
        }
    }
    let slots: Vec<Slot> = order
        .into_iter()
        .filter_map(|id| slots.remove(&id))
        .collect();
    let other_arm = slots
        .iter()
        .filter(|s| s.arm != ARM_ALGORITHM && s.arm != ARM_RANDOM)
        .count();
    let feature_start = first_at(events, |e| {
        matches!(e.kind, EventKind::ResurfaceShown { .. })
    });
    let all = aggregate(&slots, cfg, Window::ALL);
    let novelty = Novelty::compute(feature_start, cfg.novelty_days, |w| {
        aggregate(&slots, cfg, w)
    });
    ResurfacingReport {
        other_arm,
        all,
        novelty,
    }
}

/// Power-calculation thresholds used for readiness.
pub fn thresholds(cfg: &ResurfacingConfig) -> Vec<Threshold> {
    [
        ("large (30 points)", 0.7, 0.4),
        ("moderate (20 points)", 0.6, 0.4),
    ]
    .into_iter()
    .map(|(label, a, b)| {
        let n = stats::two_proportion_n_per_arm(a, b, cfg.alpha, cfg.power);
        Threshold {
            label: label.to_string(),
            p_algorithm: a,
            p_random: b,
            n_per_arm: n,
            n_total: 2 * n,
        }
    })
    .collect()
}

fn arm_result(slots: &[&Slot], arm: &str, cfg: &ResurfacingConfig) -> ArmResult {
    let mine: Vec<&&Slot> = slots.iter().filter(|s| s.arm == arm).collect();
    // a slot belongs to a window by its rating time (unrated: its shown time)
    let n = mine.iter().filter(|s| s.rating.is_some()).count();
    let useful = mine
        .iter()
        .filter(|s| matches!(s.rating, Some((true, _))))
        .count();
    ArmResult {
        arm: arm.to_string(),
        n,
        useful,
        unrated: mine.len() - n,
        raw_rate: (n > 0).then(|| useful as f64 / n as f64),
        posterior: Estimate {
            n,
            value: Some((useful as f64 + 0.5) / (n as f64 + 1.0)),
            interval: Some(stats::jeffreys(useful, n, cfg.level)),
        },
    }
}

fn aggregate(slots: &[Slot], cfg: &ResurfacingConfig, w: Window) -> ResurfacingMetrics {
    let sel: Vec<&Slot> = slots
        .iter()
        .filter(|s| w.contains(s.rating.map(|r| r.1).unwrap_or(s.shown_at)))
        .collect();
    let algorithm = arm_result(&sel, ARM_ALGORITHM, cfg);
    let random = arm_result(&sel, ARM_RANDOM, cfg);

    let difference = if algorithm.n == 0 && random.n == 0 {
        Difference {
            mean: None,
            interval: None,
            p_algorithm_better: None,
            draws: 0,
        }
    } else {
        let mut rng = Rng::new(cfg.seed);
        let (aa, ab) = (
            algorithm.useful as f64 + 0.5,
            (algorithm.n - algorithm.useful) as f64 + 0.5,
        );
        let (ra, rb) = (
            random.useful as f64 + 0.5,
            (random.n - random.useful) as f64 + 0.5,
        );
        let mut diffs: Vec<f64> = (0..cfg.draws)
            .map(|_| rng.beta(aa, ab) - rng.beta(ra, rb))
            .collect();
        let better = diffs.iter().filter(|d| **d > 0.0).count() as f64 / diffs.len() as f64;
        let mean = stats::mean(&diffs);
        Difference {
            mean,
            interval: stats::draws_interval(
                &mut diffs,
                cfg.level,
                "monte carlo (jeffreys posteriors)",
            ),
            p_algorithm_better: Some(better),
            draws: cfg.draws,
        }
    };

    let ths = thresholds(cfg);
    let smaller = algorithm.n.min(random.n);
    let detectable = ths
        .iter()
        .rev()
        .find(|t| smaller >= t.n_per_arm)
        .map(|t| t.label.clone())
        .unwrap_or_else(|| "none yet".to_string());
    let status = Status::from_bool(ths.last().is_some_and(|t| smaller >= t.n_per_arm));
    ResurfacingMetrics {
        n: algorithm.n + random.n,
        algorithm,
        random,
        difference,
        readiness: Readiness {
            n_smaller_arm: smaller,
            thresholds: ths,
            detectable,
        },
        status,
    }
}
