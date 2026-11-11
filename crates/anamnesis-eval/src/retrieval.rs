//! Retrieval quality on the user's own relevance labels.
//!
//! Labels come from `RetrievalLabeled { query, item_id, grade }` (grade 0..=3; the
//! latest label for a (query, item) pair wins). For each labelled query we call the
//! caller's `search` closure and score its ranked ids:
//!
//! * **nDCG@10** with gain 2^grade − 1 and log2 discount; the ideal ranking uses
//!   all labelled grades for the query. Unlabelled results count as grade 0.
//! * **P@5**: share of the top 5 with grade ≥ `relevant_grade` (denominator is
//!   always 5).
//! * **MRR**: reciprocal rank of the first relevant result (0 if none).
//!
//! Queries without any relevant label are skipped (they cannot be scored) and
//! counted in `skipped_queries`. Intervals are percentile bootstraps over queries.
//!
//! ## readiness
//! IR practice asks for at least 25 topics (queries), with 50 the classic TREC
//! size (Buckley & Voorhees; Voorhees & Buckley, SIGIR 2002, found small topic
//! sets give larger error rates than expected). Below 25 labelled queries the
//! status is "measuring". Even at 25–50 only large differences (≈0.1 nDCG) are
//! reliably detectable.

use std::collections::{BTreeMap, HashMap};

use anamnesis_core::{Event, EventKind, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{Estimate, NOVELTY_DAYS, Novelty, Status, Window, first_at};
use crate::stats;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalConfig {
    pub ndcg_k: usize,
    pub precision_k: usize,
    /// Grade at or above which a result counts as relevant for P@k and MRR.
    pub relevant_grade: u8,
    /// Labelled queries needed for "enough data" (IR convention: 25).
    pub min_queries: usize,
    pub level: f64,
    pub bootstrap_resamples: usize,
    pub seed: u64,
    pub novelty_days: i64,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        RetrievalConfig {
            ndcg_k: 10,
            precision_k: 5,
            relevant_grade: 1,
            min_queries: 25,
            level: 0.95,
            bootstrap_resamples: 2_000,
            seed: 0x2e_7e_1e,
            novelty_days: NOVELTY_DAYS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub query: String,
    pub first_labeled_at: Timestamp,
    pub labels: usize,
    pub ndcg: f64,
    pub precision: f64,
    pub reciprocal_rank: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalMetrics {
    /// Scored queries.
    pub n: usize,
    pub ndcg_at_10: Estimate,
    pub precision_at_5: Estimate,
    pub mrr: Estimate,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalReport {
    pub ndcg_k: usize,
    pub precision_k: usize,
    pub labelled_queries: usize,
    pub skipped_queries: usize,
    pub min_queries: usize,
    /// Why the thresholds are what they are.
    pub convention: String,
    #[serde(flatten)]
    pub all: RetrievalMetrics,
    pub novelty: Novelty<RetrievalMetrics>,
    pub queries: Vec<QueryResult>,
}

/// Graded labels per query: query → (item id → grade), plus first label time.
pub fn collect_labels(events: &[Event]) -> BTreeMap<String, (Timestamp, HashMap<String, u8>)> {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| (e.at, e.seq));
    let mut out: BTreeMap<String, (Timestamp, HashMap<String, u8>)> = BTreeMap::new();
    for e in sorted {
        if let EventKind::RetrievalLabeled {
            query,
            item_id,
            grade,
        } = &e.kind
        {
            let q = normalise_query(query);
            let entry = out.entry(q).or_insert_with(|| (e.at, HashMap::new()));
            entry.1.insert(item_id.0.clone(), (*grade).min(3));
        }
    }
    out
}

/// Queries are matched case- and whitespace-insensitively.
pub fn normalise_query(q: &str) -> String {
    q.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// nDCG@k for a ranking given graded labels (gain 2^g − 1, log2 discount).
pub fn ndcg_at(ranked: &[Id], grades: &HashMap<String, u8>, k: usize) -> f64 {
    let gain = |g: u8| (1u32 << g) as f64 - 1.0;
    let dcg: f64 = ranked
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, id)| gain(*grades.get(&id.0).unwrap_or(&0)) / ((i + 2) as f64).log2())
        .sum();
    let mut ideal: Vec<u8> = grades.values().copied().collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let idcg: f64 = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, g)| gain(*g) / ((i + 2) as f64).log2())
        .sum();
    if idcg == 0.0 { 0.0 } else { dcg / idcg }
}

pub fn precision_at(ranked: &[Id], grades: &HashMap<String, u8>, k: usize, min_grade: u8) -> f64 {
    let hits = ranked
        .iter()
        .take(k)
        .filter(|id| grades.get(&id.0).is_some_and(|g| *g >= min_grade))
        .count();
    hits as f64 / k as f64
}

pub fn reciprocal_rank(ranked: &[Id], grades: &HashMap<String, u8>, min_grade: u8) -> f64 {
    ranked
        .iter()
        .position(|id| grades.get(&id.0).is_some_and(|g| *g >= min_grade))
        .map(|i| 1.0 / (i + 1) as f64)
        .unwrap_or(0.0)
}

/// Score `search` on every labelled query in the log.
pub fn retrieval_metrics<F>(
    events: &[Event],
    mut search: F,
    cfg: &RetrievalConfig,
) -> RetrievalReport
where
    F: FnMut(&str) -> Vec<Id>,
{
    let labels = collect_labels(events);
    let mut results = Vec::new();
    let mut skipped = 0;
    for (query, (first, grades)) in &labels {
        if !grades.values().any(|g| *g >= cfg.relevant_grade) {
            skipped += 1;
            continue;
        }
        let ranked = search(query);
        results.push(QueryResult {
            query: query.clone(),
            first_labeled_at: *first,
            labels: grades.len(),
            ndcg: ndcg_at(&ranked, grades, cfg.ndcg_k),
            precision: precision_at(&ranked, grades, cfg.precision_k, cfg.relevant_grade),
            reciprocal_rank: reciprocal_rank(&ranked, grades, cfg.relevant_grade),
        });
    }
    let feature_start = first_at(events, |e| {
        matches!(e.kind, EventKind::RetrievalLabeled { .. })
    });
    let all = aggregate(&results, cfg, Window::ALL);
    let novelty = Novelty::compute(feature_start, cfg.novelty_days, |w| {
        aggregate(&results, cfg, w)
    });
    RetrievalReport {
        ndcg_k: cfg.ndcg_k,
        precision_k: cfg.precision_k,
        labelled_queries: labels.len(),
        skipped_queries: skipped,
        min_queries: cfg.min_queries,
        convention: "ir practice asks for at least 25 queries (buckley & voorhees), 50 being the classic trec size; \
                     below 25 the numbers are too unstable to read. at 25–50 only large differences (about 0.1 ndcg) show."
            .to_string(),
        all,
        novelty,
        queries: results,
    }
}

fn aggregate(results: &[QueryResult], cfg: &RetrievalConfig, w: Window) -> RetrievalMetrics {
    let sel: Vec<&QueryResult> = results
        .iter()
        .filter(|r| w.contains(r.first_labeled_at))
        .collect();
    let est = |f: &dyn Fn(&QueryResult) -> f64, salt: u64| {
        let v: Vec<f64> = sel.iter().map(|r| f(r)).collect();
        Estimate {
            n: v.len(),
            value: stats::mean(&v),
            interval: stats::bootstrap_mean(
                &v,
                cfg.bootstrap_resamples,
                cfg.level,
                cfg.seed ^ salt,
            ),
        }
    };
    RetrievalMetrics {
        n: sel.len(),
        ndcg_at_10: est(&|r| r.ndcg, 1),
        precision_at_5: est(&|r| r.precision, 2),
        mrr: est(&|r| r.reciprocal_rank, 3),
        status: Status::from_bool(sel.len() >= cfg.min_queries),
    }
}
