//! Link suggestion and the randomised hold-out used to evaluate it.
//!
//! Candidates come from cosine kNN over item vectors; each candidate is
//! rescored with
//!
//! ```text
//! score = cos(a, b)
//!       + α · AA(a, b) / (1 + AA(a, b))      (Adamic–Adar over existing links)
//!       + β · [same region]
//!       − near-duplicate penalty              (cos > 0.95: offer "merge?" instead)
//!       − same-session penalty                (same capture session, or same
//!                                              source within a short window)
//! ```
//!
//! Pairs that are already linked (either direction) are never suggested.
//!
//! The hold-out ([`split_for_holdout`]) marks a fixed share of suggestions
//! as *held* (never shown — the control arm). The arm of a pair is a pure
//! function of `(seed, from, to)`, so evaluation is reproducible and a pair
//! keeps its arm if it is suggested again later.

use std::collections::{HashMap, HashSet};

use anamnesis_core::{Id, Link, Timestamp};
use serde::{Deserialize, Serialize};

use crate::knn::top_k_dot;
use crate::linalg::Rows;
use crate::rng::unit_from;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LinkParams {
    /// Cosine neighbours considered before rescoring.
    pub candidates: usize,
    /// Suggestions returned.
    pub top_k: usize,
    pub alpha_adamic_adar: f32,
    pub beta_same_region: f32,
    pub near_duplicate_cos: f32,
    pub near_duplicate_penalty: f32,
    pub same_session_penalty: f32,
    /// Same source and created within this many seconds counts as one session.
    pub session_window_secs: i64,
    /// Candidates below this cosine are dropped.
    pub min_cos: f32,
}

impl Default for LinkParams {
    fn default() -> Self {
        LinkParams {
            candidates: 50,
            top_k: 10,
            alpha_adamic_adar: 0.2,
            beta_same_region: 0.05,
            near_duplicate_cos: 0.95,
            near_duplicate_penalty: 0.5,
            same_session_penalty: 0.15,
            session_window_secs: 30 * 60,
            min_cos: 0.0,
        }
    }
}

/// What link suggestion needs to know about an item besides its vector.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemMeta {
    pub source_id: Option<Id>,
    /// Explicit capture session (e.g. one chat, one import batch), if known.
    pub session: Option<String>,
    pub created_at: Option<Timestamp>,
    pub region: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionFlag {
    NearDuplicate,
    SameSession,
    SameRegion,
    SharesNeighbours,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkSuggestion {
    pub from: Id,
    pub to: Id,
    pub score: f32,
    /// 1-based rank among this item's suggestions.
    pub rank: u32,
    pub cosine: f32,
    pub adamic_adar: f32,
    pub flags: Vec<SuggestionFlag>,
}

/// A suggestion with its hold-out arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmedSuggestion {
    pub suggestion: LinkSuggestion,
    /// `false` = held out (control arm, never shown).
    pub shown: bool,
}

/// Holds normalised vectors, the undirected link graph and item metadata.
pub struct LinkSuggester {
    ids: Vec<Id>,
    index: HashMap<Id, usize>,
    x: Rows,
    adj: Vec<HashSet<u32>>,
    meta: Vec<ItemMeta>,
}

impl LinkSuggester {
    /// `meta` may be empty (no session/region information) or aligned with `ids`.
    pub fn new(ids: &[Id], vectors: &[Vec<f32>], links: &[Link], meta: &[ItemMeta]) -> Self {
        assert_eq!(ids.len(), vectors.len());
        assert!(meta.is_empty() || meta.len() == ids.len());
        let mut x = Rows::from_vecs(vectors);
        x.normalize_rows();
        let index: HashMap<Id, usize> = ids.iter().enumerate().map(|(i, id)| (id.clone(), i)).collect();
        let mut adj = vec![HashSet::new(); ids.len()];
        for l in links {
            if let (Some(&a), Some(&b)) = (index.get(&l.from), index.get(&l.to))
                && a != b
            {
                adj[a].insert(b as u32);
                adj[b].insert(a as u32);
            }
        }
        let meta = if meta.is_empty() { vec![ItemMeta::default(); ids.len()] } else { meta.to_vec() };
        LinkSuggester { ids: ids.to_vec(), index, x, adj, meta }
    }

    /// Record a new link (e.g. one the user just made).
    pub fn add_link(&mut self, from: &Id, to: &Id) {
        if let (Some(&a), Some(&b)) = (self.index.get(from), self.index.get(to))
            && a != b
        {
            self.adj[a].insert(b as u32);
            self.adj[b].insert(a as u32);
        }
    }

    pub fn linked(&self, a: &Id, b: &Id) -> bool {
        match (self.index.get(a), self.index.get(b)) {
            (Some(&i), Some(&j)) => self.adj[i].contains(&(j as u32)),
            _ => false,
        }
    }

    /// Adamic–Adar index of two items over the link graph.
    pub fn adamic_adar(&self, a: usize, b: usize) -> f32 {
        let (small, large) = if self.adj[a].len() <= self.adj[b].len() { (a, b) } else { (b, a) };
        self.adj[small]
            .iter()
            .filter(|z| self.adj[large].contains(z))
            .map(|&z| {
                let d = self.adj[z as usize].len() as f32;
                if d > 1.0 { 1.0 / d.ln() } else { 0.0 }
            })
            .sum()
    }

    fn same_session(&self, a: usize, b: usize, params: &LinkParams) -> bool {
        let (ma, mb) = (&self.meta[a], &self.meta[b]);
        if let (Some(sa), Some(sb)) = (&ma.session, &mb.session)
            && sa == sb
        {
            return true;
        }
        if let (Some(sa), Some(sb), Some(ta), Some(tb)) = (&ma.source_id, &mb.source_id, ma.created_at, mb.created_at) {
            return sa == sb && (ta - tb).num_seconds().abs() <= params.session_window_secs;
        }
        false
    }

    /// Top suggestions for `item`, best first.
    pub fn suggest(&self, item: &Id, params: &LinkParams) -> Vec<LinkSuggestion> {
        let Some(&a) = self.index.get(item) else { return vec![] };
        let q = self.x.row(a).to_vec();
        let cands = top_k_dot(&self.x, &q, params.candidates, |j| j == a || self.adj[a].contains(&(j as u32)));
        let mut out: Vec<LinkSuggestion> = cands
            .into_iter()
            .filter(|&(_, c)| c >= params.min_cos)
            .map(|(b, cos)| {
                let aa = self.adamic_adar(a, b);
                let mut score = cos + params.alpha_adamic_adar * aa / (1.0 + aa);
                let mut flags = vec![];
                if aa > 0.0 {
                    flags.push(SuggestionFlag::SharesNeighbours);
                }
                if let (Some(ra), Some(rb)) = (&self.meta[a].region, &self.meta[b].region)
                    && ra == rb
                {
                    score += params.beta_same_region;
                    flags.push(SuggestionFlag::SameRegion);
                }
                if cos > params.near_duplicate_cos {
                    score -= params.near_duplicate_penalty;
                    flags.push(SuggestionFlag::NearDuplicate);
                }
                if self.same_session(a, b, params) {
                    score -= params.same_session_penalty;
                    flags.push(SuggestionFlag::SameSession);
                }
                LinkSuggestion { from: item.clone(), to: self.ids[b].clone(), score, rank: 0, cosine: cos, adamic_adar: aa, flags }
            })
            .collect();
        out.sort_by(|x, y| y.score.total_cmp(&x.score).then_with(|| x.to.cmp(&y.to)));
        out.truncate(params.top_k);
        for (r, s) in out.iter_mut().enumerate() {
            s.rank = r as u32 + 1;
        }
        out
    }

    /// Items so similar to `item` (cos > threshold) that they are probably
    /// duplicates — offer "merge?" rather than "link?".
    pub fn near_duplicates(&self, item: &Id, threshold: f32) -> Vec<(Id, f32)> {
        let Some(&a) = self.index.get(item) else { return vec![] };
        let q = self.x.row(a).to_vec();
        top_k_dot(&self.x, &q, 20, |j| j == a).into_iter().filter(|&(_, c)| c > threshold).map(|(j, c)| (self.ids[j].clone(), c)).collect()
    }
}

/// Is the pair `(from, to)` in the held-out arm for this seed and rate?
pub fn is_held_out(from: &Id, to: &Id, rate: f64, seed: u64) -> bool {
    unit_from(seed, &["holdout", from.as_str(), to.as_str()]) < rate
}

/// Mark each suggestion shown or held out (≈ `rate` held). Deterministic in
/// `(seed, from, to)`, independent of order and of the other suggestions.
pub fn split_for_holdout(suggestions: Vec<LinkSuggestion>, rate: f64, seed: u64) -> Vec<ArmedSuggestion> {
    suggestions
        .into_iter()
        .map(|s| {
            let shown = !is_held_out(&s.from, &s.to, rate, seed);
            ArmedSuggestion { suggestion: s, shown }
        })
        .collect()
}
