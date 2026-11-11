//! Growth of the brain: items over time (by speaker, origin and kind), links by
//! kind, regions over time from `RegionsRecomputed`, and region transitions.
//! These are plain counts from the log, so they are *measured* by construction.

use std::collections::{BTreeMap, HashMap, HashSet};

use anamnesis_core::{Event, EventKind, Id, RegionTransition, Speaker, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{NOVELTY_DAYS, Novelty, Window, days_between};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TransitionCounts {
    pub births: usize,
    pub survivals: usize,
    pub splits: usize,
    pub merges: usize,
    pub deaths: usize,
}

impl TransitionCounts {
    fn add(&mut self, t: &RegionTransition) {
        match t {
            RegionTransition::Birth { .. } => self.births += 1,
            RegionTransition::Survive { .. } => self.survivals += 1,
            RegionTransition::Split { .. } => self.splits += 1,
            RegionTransition::Merge { .. } => self.merges += 1,
            RegionTransition::Death { .. } => self.deaths += 1,
        }
    }
    fn merge_from(&mut self, o: &TransitionCounts) {
        self.births += o.births;
        self.survivals += o.survivals;
        self.splits += o.splits;
        self.merges += o.merges;
        self.deaths += o.deaths;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthCount {
    /// "yyyy-mm"
    pub month: String,
    pub created: usize,
    pub deleted: usize,
    /// Live items at the end of the month.
    pub cumulative: usize,
    pub you: usize,
    pub assistants: usize,
    pub other: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionRun {
    pub run_id: Id,
    pub at: Timestamp,
    /// Regions without `died_at` in this run.
    pub live_regions: usize,
    pub transitions: TransitionCounts,
    /// Mean within-region cohesion for this run, if the caller supplied it.
    pub cohesion: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrowthWindow {
    pub items_created: usize,
    pub links_added: usize,
    pub days: f64,
    pub items_per_day: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrowthReport {
    /// Live items at the end of the log.
    pub n: usize,
    pub items_created: usize,
    pub items_deleted: usize,
    pub by_month: Vec<MonthCount>,
    /// Live items by speaker ("you", "assistants", "other").
    pub by_speaker: BTreeMap<String, usize>,
    /// Live items by origin (snake_case origin names).
    pub by_origin: BTreeMap<String, usize>,
    /// Live items by kind.
    pub by_kind: BTreeMap<String, usize>,
    /// Links by kind (first link per directed pair).
    pub links_by_kind: BTreeMap<String, usize>,
    pub regions_over_time: Vec<RegionRun>,
    pub transitions: TransitionCounts,
    /// Mean of the supplied per-run cohesion values.
    pub mean_cohesion: Option<f64>,
    /// Items in the first 28 days of the log vs after.
    pub novelty: Novelty<GrowthWindow>,
}

fn snake<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Growth metrics. `cohesion` optionally maps a clustering run id to its mean
/// within-region cohesion (computed by the mind package, which has the vectors).
pub fn growth_metrics(events: &[Event], cohesion: Option<&HashMap<Id, f64>>) -> GrowthReport {
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by_key(|e| (e.at, e.seq));

    struct Live {
        speaker: Speaker,
        origin: String,
        kind: String,
    }
    let mut live: HashMap<String, Live> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut created_at: Vec<Timestamp> = Vec::new();
    let mut link_at: Vec<Timestamp> = Vec::new();
    let mut months: BTreeMap<String, MonthCount> = BTreeMap::new();
    let (mut created, mut deleted) = (0, 0);
    let mut links_by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut link_pairs: HashSet<(String, String)> = HashSet::new();
    let mut runs = Vec::new();
    let mut transitions = TransitionCounts::default();

    let month_of = |t: Timestamp| t.format("%Y-%m").to_string();
    let blank = |m: String| MonthCount {
        month: m,
        created: 0,
        deleted: 0,
        cumulative: 0,
        you: 0,
        assistants: 0,
        other: 0,
    };

    for e in &sorted {
        match &e.kind {
            EventKind::ItemCreated { item } => {
                if !seen.insert(item.id.0.clone()) {
                    continue;
                }
                created += 1;
                created_at.push(e.at);
                let m = months
                    .entry(month_of(e.at))
                    .or_insert_with_key(|k| blank(k.clone()));
                m.created += 1;
                match item.origin.speaker() {
                    Speaker::You => m.you += 1,
                    Speaker::Assistants => m.assistants += 1,
                    Speaker::Other => m.other += 1,
                }
                live.insert(
                    item.id.0.clone(),
                    Live {
                        speaker: item.origin.speaker(),
                        origin: snake(&item.origin),
                        kind: snake(&item.kind),
                    },
                );
            }
            EventKind::ItemUpdated { item } => {
                if let Some(l) = live.get_mut(&item.id.0) {
                    l.origin = snake(&item.origin);
                    l.speaker = item.origin.speaker();
                    l.kind = snake(&item.kind);
                }
            }
            EventKind::ItemDeleted { item_id } => {
                if live.remove(&item_id.0).is_some() {
                    deleted += 1;
                    months
                        .entry(month_of(e.at))
                        .or_insert_with_key(|k| blank(k.clone()))
                        .deleted += 1;
                }
            }
            EventKind::LinkAdded { link } => {
                if link_pairs.insert((link.from.0.clone(), link.to.0.clone())) {
                    *links_by_kind.entry(snake(&link.kind)).or_default() += 1;
                    link_at.push(e.at);
                }
            }
            EventKind::RegionsRecomputed {
                run_id,
                regions,
                transitions: ts,
            } => {
                let mut c = TransitionCounts::default();
                ts.iter().for_each(|t| c.add(t));
                transitions.merge_from(&c);
                runs.push(RegionRun {
                    run_id: run_id.clone(),
                    at: e.at,
                    live_regions: regions.iter().filter(|r| r.died_at.is_none()).count(),
                    transitions: c,
                    cohesion: cohesion.and_then(|m| m.get(run_id).copied()),
                });
            }
            _ => {}
        }
    }

    // cumulative counts; fill in empty months so charts have a continuous axis
    let mut by_month = Vec::new();
    if let (Some(first), Some(last)) =
        (months.keys().next().cloned(), months.keys().last().cloned())
    {
        let (mut y, mut mo) = parse_month(&first);
        let mut total: i64 = 0;
        loop {
            let key = format!("{y:04}-{mo:02}");
            let mut m = months.remove(&key).unwrap_or_else(|| blank(key.clone()));
            total += m.created as i64 - m.deleted as i64;
            m.cumulative = total.max(0) as usize;
            by_month.push(m);
            if key == last {
                break;
            }
            mo += 1;
            if mo > 12 {
                mo = 1;
                y += 1;
            }
        }
    }

    let mut by_speaker = BTreeMap::new();
    let mut by_origin = BTreeMap::new();
    let mut by_kind = BTreeMap::new();
    for l in live.values() {
        *by_speaker.entry(snake(&l.speaker)).or_default() += 1;
        *by_origin.entry(l.origin.clone()).or_default() += 1;
        *by_kind.entry(l.kind.clone()).or_default() += 1;
    }
    let cs: Vec<f64> = runs.iter().filter_map(|r| r.cohesion).collect();
    let mean_cohesion = crate::stats::mean(&cs);

    let start = sorted.first().map(|e| e.at);
    let end = sorted.last().map(|e| e.at);
    let novelty = Novelty::compute(start, NOVELTY_DAYS, |w: Window| {
        let lo = w.from.or(start);
        let hi = match (w.to, end) {
            (Some(t), Some(e)) => Some(t.min(e)),
            (t, e) => t.or(e),
        };
        let days = match (lo, hi) {
            (Some(a), Some(b)) if b > a => days_between(a, b),
            _ => 0.0,
        };
        let items = created_at.iter().filter(|t| w.contains(**t)).count();
        GrowthWindow {
            items_created: items,
            links_added: link_at.iter().filter(|t| w.contains(**t)).count(),
            days,
            items_per_day: (days > 0.0).then(|| items as f64 / days),
        }
    });

    GrowthReport {
        n: live.len(),
        items_created: created,
        items_deleted: deleted,
        by_month,
        by_speaker,
        by_origin,
        by_kind,
        links_by_kind,
        regions_over_time: runs,
        transitions,
        mean_cohesion,
        novelty,
    }
}

fn parse_month(s: &str) -> (i32, u32) {
    let (y, m) = s.split_once('-').unwrap_or(("1970", "01"));
    (y.parse().unwrap_or(1970), m.parse().unwrap_or(1))
}
