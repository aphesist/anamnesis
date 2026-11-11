//! Resurfacing: an FSRS-style scheduler for seeds and open questions, and
//! the daily review mix with a micro-randomised trial.
//!
//! ## scheduler (FSRS-6 formulas, fixed default parameters)
//!
//! Each scheduled item has a stability `S` (days until recall probability
//! falls to 90%) and a difficulty `D ∈ [1, 10]`. With `w = w0…w20` the
//! published FSRS-6 defaults and `F = 0.9^(−1/w20) − 1`:
//!
//! - retrievability after `t` days: `R(t, S) = (1 + F·t/S)^(−w20)`
//! - next interval for desired retention `r`: `I = S/F · (r^(−1/w20) − 1)`
//!   (so `I = S` at `r = 0.9`)
//! - first rating `G ∈ {1 again, 2 hard, 3 good, 4 easy}`: `S₀ = w[G−1]`,
//!   `D₀(G) = w4 − e^(w5·(G−1)) + 1`
//! - difficulty: `D' = D − w6·(G−3)·(10−D)/9`, then mean reversion
//!   `D'' = w7·D₀(4) + (1−w7)·D'`, clamped to [1, 10]
//! - success (`G ≥ 2`): `S' = S·(e^w8·(11−D)·S^(−w9)·(e^(w10·(1−R))−1)·h·e + 1)`
//!   with `h = w15` if hard, `e = w16` if easy
//! - lapse (`G = 1`): `S' = min(w11·D^(−w12)·((S+1)^w13 − 1)·e^(w14·(1−R)), S/e^(w17·w18))`
//! - same-day review: `S' = S·e^(w17·(G−3+w18))·S^(−w19)`
//!
//! The parameters were fitted to flashcard memory, not to ideas; here the
//! interval is a *priority* (when to bring a seed back), so fixed defaults and
//! a retention knob are enough. Engagement maps to ratings: extended or
//! linked → easy, opened → good, glanced → hard, "not now" → again,
//! "never" → suspended.
//!
//! ## daily mix
//!
//! `due` (scheduled items whose interval elapsed, most overdue first) +
//! `contextual` (most similar to what the user touched recently) +
//! `serendipity` (old, far from recent attention, rarely shown; sampled with
//! temperature). Each slot of each day is independently assigned to the
//! `algorithm` arm or the `random` arm (a uniformly random eligible item)
//! from a seed — a micro-randomised trial whose ratings the eval package
//! compares.

use std::collections::HashSet;

use anamnesis_core::{Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::linalg::{Rows, dot};
use crate::rng::{Rng, mix64, unit_from};

/// FSRS-6 default parameters.
pub const FSRS6_DEFAULT: [f64; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483,
    0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

/// What the user did with a resurfaced item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engagement {
    /// Edited, extended or linked it.
    Extended,
    Opened,
    Glanced,
    NotNow,
    Never,
}

impl Engagement {
    /// `None` means suspend.
    pub fn rating(self) -> Option<Rating> {
        match self {
            Engagement::Extended => Some(Rating::Easy),
            Engagement::Opened => Some(Rating::Good),
            Engagement::Glanced => Some(Rating::Hard),
            Engagement::NotNow => Some(Rating::Again),
            Engagement::Never => None,
        }
    }

    /// The review panel's "useful / not useful" buttons.
    pub fn from_useful(useful: bool) -> Engagement {
        if useful { Engagement::Opened } else { Engagement::NotNow }
    }
}

/// Scheduling state of one item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SrsCard {
    pub stability: f64,
    pub difficulty: f64,
    pub last_review: Option<Timestamp>,
    pub due: Timestamp,
    pub reps: u32,
    pub lapses: u32,
    pub suspended: bool,
}

impl SrsCard {
    /// A new card, due now.
    pub fn new(now: Timestamp) -> Self {
        SrsCard { stability: 0.0, difficulty: 0.0, last_review: None, due: now, reps: 0, lapses: 0, suspended: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scheduler {
    pub w: [f64; 21],
    pub desired_retention: f64,
    pub max_interval_days: f64,
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler { w: FSRS6_DEFAULT, desired_retention: 0.9, max_interval_days: 3650.0 }
    }
}

impl Scheduler {
    fn factor(&self) -> f64 {
        0.9f64.powf(-1.0 / self.w[20]) - 1.0
    }

    /// Recall probability after `t` days at stability `s`.
    pub fn retrievability(&self, t_days: f64, s: f64) -> f64 {
        if s <= 0.0 {
            return 0.0;
        }
        (1.0 + self.factor() * t_days.max(0.0) / s).powf(-self.w[20])
    }

    /// Days until retrievability falls to the desired retention.
    pub fn interval_days(&self, s: f64) -> f64 {
        let r = self.desired_retention;
        (s / self.factor() * (r.powf(-1.0 / self.w[20]) - 1.0)).clamp(1.0, self.max_interval_days)
    }

    fn d0(&self, g: f64) -> f64 {
        (self.w[4] - (self.w[5] * (g - 1.0)).exp() + 1.0).clamp(1.0, 10.0)
    }

    /// Apply a review at `now`.
    pub fn review(&self, card: &SrsCard, rating: Rating, now: Timestamp) -> SrsCard {
        let w = &self.w;
        let g = rating as u8 as f64;
        let mut c = card.clone();
        match card.last_review {
            None => {
                c.stability = w[(g as usize) - 1].max(0.01);
                c.difficulty = self.d0(g);
            }
            Some(last) => {
                let elapsed = (now - last).num_seconds() as f64 / 86_400.0;
                let (s, d) = (card.stability.max(0.01), card.difficulty);
                let dd = -w[6] * (g - 3.0);
                let d1 = d + dd * (10.0 - d) / 9.0;
                c.difficulty = (w[7] * self.d0(4.0) + (1.0 - w[7]) * d1).clamp(1.0, 10.0);
                if elapsed < 1.0 {
                    let mut s2 = s * (w[17] * (g - 3.0 + w[18])).exp() * s.powf(-w[19]);
                    if g >= 3.0 {
                        s2 = s2.max(s);
                    }
                    c.stability = s2;
                } else {
                    let r = self.retrievability(elapsed, s);
                    if rating == Rating::Again {
                        let sf = w[11] * d.powf(-w[12]) * ((s + 1.0).powf(w[13]) - 1.0) * (w[14] * (1.0 - r)).exp();
                        c.stability = sf.min(s / (w[17] * w[18]).exp());
                    } else {
                        let hard = if rating == Rating::Hard { w[15] } else { 1.0 };
                        let easy = if rating == Rating::Easy { w[16] } else { 1.0 };
                        c.stability = s * (w[8].exp() * (11.0 - d) * s.powf(-w[9]) * ((w[10] * (1.0 - r)).exp() - 1.0) * hard * easy + 1.0);
                    }
                }
                c.stability = c.stability.max(0.01);
            }
        }
        if rating == Rating::Again {
            c.lapses += 1;
        }
        c.reps += 1;
        c.last_review = Some(now);
        let days = if rating == Rating::Again { 1.0 } else { self.interval_days(c.stability) };
        c.due = now + chrono::Duration::seconds((days * 86_400.0).round() as i64);
        c
    }

    /// Apply an engagement (suspends on `Never`).
    pub fn engage(&self, card: &SrsCard, e: Engagement, now: Timestamp) -> SrsCard {
        match e.rating() {
            Some(r) => self.review(card, r, now),
            None => SrsCard { suspended: true, ..card.clone() },
        }
    }
}

// --- daily review mix -------------------------------------------------------------

/// Per-item facts for the review mix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewInfo {
    pub created_at: Timestamp,
    pub last_seen: Option<Timestamp>,
    pub times_shown: u32,
    /// Present for seeds / open questions (the SRS queue).
    pub card: Option<SrsCard>,
    pub is_open_question: bool,
    /// Cosine to the item's region centroid, if it has a region.
    pub centrality: Option<f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReviewParams {
    pub due: usize,
    pub contextual: usize,
    pub serendipity: usize,
    /// Probability that a slot goes to the random arm.
    pub random_arm_probability: f64,
    /// Items shown within this many days are not eligible.
    pub cooldown_days: i64,
    /// Serendipity only considers items at least this old.
    pub serendipity_min_age_days: i64,
    /// Sampling temperature for serendipity (0 → argmax).
    pub serendipity_temperature: f64,
    /// Contextual picks this similar to a recent item are near-duplicates and skipped.
    pub contextual_max_cos: f32,
}

impl Default for ReviewParams {
    fn default() -> Self {
        ReviewParams {
            due: 3,
            contextual: 2,
            serendipity: 2,
            random_arm_probability: 0.5,
            cooldown_days: 1,
            serendipity_min_age_days: 30,
            serendipity_temperature: 0.5,
            contextual_max_cos: 0.95,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewReason {
    Due,
    Contextual,
    Serendipity,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    Algorithm,
    Random,
}

impl Arm {
    /// The string stored in `EventKind::ResurfaceShown.arm`.
    pub fn as_str(self) -> &'static str {
        match self {
            Arm::Algorithm => "algorithm",
            Arm::Random => "random",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewPick {
    pub item_id: Id,
    pub reason: ReviewReason,
    /// The queue this slot belonged to (what the algorithm would have used).
    pub slot: ReviewReason,
    pub arm: Arm,
    pub score: f32,
}

/// Build today's review. `vectors` and `info` are aligned with `ids`;
/// `recent` are the vectors of items the user touched recently, and
/// `recent_ids` are excluded from the mix.
#[allow(clippy::too_many_arguments)]
pub fn daily_review(
    ids: &[Id],
    vectors: &[Vec<f32>],
    info: &[ReviewInfo],
    recent: &[Vec<f32>],
    recent_ids: &[Id],
    now: Timestamp,
    params: &ReviewParams,
    sched: &Scheduler,
    seed: u64,
) -> Vec<ReviewPick> {
    assert_eq!(ids.len(), info.len());
    assert_eq!(ids.len(), vectors.len());
    let n = ids.len();
    let day = now.date_naive().to_string();
    let excluded: HashSet<&Id> = recent_ids.iter().collect();
    let eligible: Vec<bool> = (0..n)
        .map(|i| {
            let inf = &info[i];
            !excluded.contains(&ids[i])
                && !inf.card.as_ref().is_some_and(|c| c.suspended)
                && inf.last_seen.is_none_or(|t| (now - t).num_days() >= params.cooldown_days)
        })
        .collect();

    // max cosine to recent attention
    let mut x = Rows::from_vecs(vectors);
    x.normalize_rows();
    let mut rv = Rows::from_vecs(recent);
    rv.normalize_rows();
    let near: Vec<f32> = (0..n).map(|i| (0..rv.n).map(|r| dot(x.row(i), rv.row(r))).fold(f32::MIN, f32::max)).map(|v| if v == f32::MIN { 0.0 } else { v }).collect();

    // due queue: most overdue (lowest retrievability) first
    let mut due: Vec<(usize, f32)> = (0..n)
        .filter(|&i| eligible[i])
        .filter_map(|i| {
            let c = info[i].card.as_ref()?;
            if c.due > now {
                return None;
            }
            let r = match c.last_review {
                Some(l) => sched.retrievability((now - l).num_seconds() as f64 / 86_400.0, c.stability),
                None => 0.0,
            };
            Some((i, (1.0 - r) as f32))
        })
        .collect();
    due.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    // contextual: closest to recent attention, not a near-duplicate, not brand new
    let mut contextual: Vec<(usize, f32)> = if rv.n == 0 {
        vec![]
    } else {
        (0..n)
            .filter(|&i| eligible[i] && near[i] < params.contextual_max_cos && (now - info[i].created_at).num_days() >= 2)
            .map(|i| (i, near[i]))
            .collect()
    };
    contextual.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

    // serendipity: old, far, rarely seen — Gumbel-top-k sampling with temperature
    let mut srng = Rng::new(mix64(seed ^ crate::rng::hash_bytes(day.as_bytes()) ^ 0x5e4e));
    let mut seren: Vec<(usize, f32, f64)> = (0..n)
        .filter(|&i| eligible[i] && (now - info[i].created_at).num_days() >= params.serendipity_min_age_days)
        .map(|i| {
            let inf = &info[i];
            let since = (now - inf.last_seen.unwrap_or(inf.created_at)).num_seconds().max(0) as f64 / 86_400.0;
            let far = (1.0 - near[i].max(0.0)) as f64;
            let cent = inf.centrality.unwrap_or(1.0).max(0.05) as f64;
            let oq = if inf.is_open_question { 1.5 } else { 1.0 };
            let score = since.sqrt() * far * cent * oq / (1.0 + inf.times_shown as f64).sqrt();
            let key = if params.serendipity_temperature > 0.0 {
                let u = srng.f64().clamp(1e-12, 1.0 - 1e-12);
                score.max(1e-12).ln() / params.serendipity_temperature - (-u.ln()).ln()
            } else {
                score
            };
            (i, score as f32, key)
        })
        .collect();
    seren.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));

    let slots: Vec<ReviewReason> = std::iter::repeat_n(ReviewReason::Due, params.due)
        .chain(std::iter::repeat_n(ReviewReason::Contextual, params.contextual))
        .chain(std::iter::repeat_n(ReviewReason::Serendipity, params.serendipity))
        .collect();
    let mut picked: HashSet<usize> = HashSet::new();
    let mut out = vec![];
    let mut rrng = Rng::new(mix64(seed ^ crate::rng::hash_bytes(day.as_bytes()) ^ 0x4a4d));
    let pool: Vec<usize> = (0..n).filter(|&i| eligible[i]).collect();
    let (mut qd, mut qc, mut qs) = (0usize, 0usize, 0usize);
    let seren2: Vec<(usize, f32)> = seren.iter().map(|&(i, s, _)| (i, s)).collect();
    for (k, slot) in slots.iter().enumerate() {
        let arm = if unit_from(seed, &["resurface-arm", &day, &k.to_string()]) < params.random_arm_probability { Arm::Random } else { Arm::Algorithm };
        let choice = match arm {
            Arm::Random => {
                let left: Vec<usize> = pool.iter().copied().filter(|i| !picked.contains(i)).collect();
                if left.is_empty() {
                    None
                } else {
                    Some((left[rrng.below(left.len())], 0.0, ReviewReason::Random))
                }
            }
            Arm::Algorithm => {
                let next = |q: &[(usize, f32)], pos: &mut usize, picked: &HashSet<usize>| -> Option<(usize, f32)> {
                    while *pos < q.len() {
                        let c = q[*pos];
                        *pos += 1;
                        if !picked.contains(&c.0) {
                            return Some(c);
                        }
                    }
                    None
                };
                // fall back through the other queues when one runs dry
                let order: [ReviewReason; 3] = match slot {
                    ReviewReason::Due => [ReviewReason::Due, ReviewReason::Contextual, ReviewReason::Serendipity],
                    ReviewReason::Contextual => [ReviewReason::Contextual, ReviewReason::Serendipity, ReviewReason::Due],
                    _ => [ReviewReason::Serendipity, ReviewReason::Contextual, ReviewReason::Due],
                };
                let mut found = None;
                for r in order {
                    let got = match r {
                        ReviewReason::Due => next(&due, &mut qd, &picked),
                        ReviewReason::Contextual => next(&contextual, &mut qc, &picked),
                        _ => next(&seren2, &mut qs, &picked),
                    };
                    if let Some((i, s)) = got {
                        found = Some((i, s, r));
                        break;
                    }
                }
                found
            }
        };
        if let Some((i, score, reason)) = choice {
            picked.insert(i);
            out.push(ReviewPick { item_id: ids[i].clone(), reason, slot: *slot, arm, score });
        }
    }
    out
}
