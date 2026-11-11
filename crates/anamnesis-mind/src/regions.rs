//! Regions with stable identity.
//!
//! A full run clusters the item vectors (HDBSCAN on the shared kNN graph),
//! then matches the new clusters to the previous run's regions by member
//! overlap and classifies MONIC-style transitions (Spiliopoulou et al.,
//! KDD 2006):
//!
//! - **merge**: ≥ 2 old regions each have ≥ `merge_containment` of their
//!   (still existing) members inside one new cluster → a new region with
//!   both as parents;
//! - **split**: one old region has ≥ 2 new clusters each holding
//!   ≥ `split_containment` of its members and each made mostly
//!   (≥ `child_purity`) of its members → new regions with it as parent;
//! - **survive**: the remaining old/new pairs are matched by the Hungarian
//!   algorithm on Jaccard overlap; pairs with Jaccard ≥ `survive_jaccard`
//!   keep the old id (and its label, anatomy name and birth time);
//! - **death**: old regions left unmatched;
//! - **birth**: new clusters left unmatched.
//!
//! Overlaps are computed only over items that were members of some old
//! region, so a region that absorbs many new items still survives.
//!
//! Below `min_items` items nothing is clustered: the brain is one
//! "unsorted" state. Between runs, [`RegionState::assign`] puts new vectors
//! in the nearest region (by centroid cosine) when they are close enough.

use std::collections::{HashMap, HashSet};

use anamnesis_core::{Id, Region, RegionTransition, Timestamp};
use serde::{Deserialize, Serialize};

use crate::anatomy::{AnatomyHint, AnatomyRequest, assign_anatomy};
use crate::hdbscan::{HdbscanParams, hdbscan};
use crate::labels::{RegionLabelInput, c_tf_idf, fallback_label, representatives};
use crate::linalg::{Rows, centroid, cosine, dot, normalize};
use crate::space::Space;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RegionParams {
    /// Below this many items, don't cluster ("unsorted").
    pub min_items: usize,
    /// HDBSCAN min cluster size; `None` = `max(10, n / 150)`.
    pub min_cluster_size: Option<usize>,
    pub min_samples: usize,
    pub survive_jaccard: f32,
    pub split_containment: f32,
    pub child_purity: f32,
    pub merge_containment: f32,
    /// A vector joins a region if its cosine to the centroid is at least the
    /// region's `assign_quantile` member cosine minus `assign_slack`.
    pub assign_quantile: f32,
    pub assign_slack: f32,
    /// Keywords per region.
    pub keywords: usize,
}

impl Default for RegionParams {
    fn default() -> Self {
        RegionParams {
            min_items: 30,
            min_cluster_size: None,
            min_samples: 5,
            survive_jaccard: 0.5,
            split_containment: 0.25,
            child_purity: 0.5,
            merge_containment: 0.5,
            assign_quantile: 0.05,
            assign_slack: 0.05,
            keywords: 10,
        }
    }
}

/// One region as remembered between runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionSnapshot {
    pub region: Region,
    pub members: Vec<Id>,
    /// Normalised centroid in the input embedding space.
    pub centroid: Vec<f32>,
    /// Mean cosine of members to the centroid.
    pub cohesion: f32,
    /// Minimum centroid cosine for [`RegionState::assign`].
    pub assign_threshold: f32,
    pub keywords: Vec<String>,
}

/// What the next run needs to know about this one. Persist it as json.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionState {
    pub run_at: Option<Timestamp>,
    /// Living regions.
    pub regions: Vec<RegionSnapshot>,
}

impl RegionState {
    /// Nearest region for each vector, or `None` ("unsorted") when no region
    /// centroid is close enough.
    pub fn assign(&self, vectors: &[Vec<f32>]) -> Vec<Option<Id>> {
        vectors
            .iter()
            .map(|v| {
                let mut best: Option<(f32, usize)> = None;
                for (k, r) in self.regions.iter().enumerate() {
                    let c = cosine(v, &r.centroid);
                    if c >= r.assign_threshold && best.is_none_or(|(b, _)| c > b) {
                        best = Some((c, k));
                    }
                }
                best.map(|(_, k)| self.regions[k].region.id.clone())
            })
            .collect()
    }

    pub fn get(&self, id: &Id) -> Option<&RegionSnapshot> {
        self.regions.iter().find(|r| &r.region.id == id)
    }
}

/// Internal change of a region between runs (MONIC "internal transitions").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionDrift {
    pub region: Id,
    /// `1 − cos(previous centroid, new centroid)`.
    pub drift: f32,
    pub cohesion_before: f32,
    pub cohesion_after: f32,
    pub size_before: u32,
    pub size_after: u32,
    /// Jaccard of the member sets (over items present in both runs).
    pub jaccard: f32,
    pub keywords_gained: Vec<String>,
    pub keywords_lost: Vec<String>,
}

/// Output of a full run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionRun {
    /// Living regions after this run.
    pub regions: Vec<Region>,
    /// Regions that ended in this run (death, split parents, merge parents),
    /// with `died_at` set. Log them together with `regions`.
    pub retired: Vec<Region>,
    pub transitions: Vec<RegionTransition>,
    /// Region of each input item (`None` = unsorted), aligned with the input.
    pub assignment: Vec<Option<Id>>,
    pub drift: Vec<RegionDrift>,
    /// Inputs for model labelling, one per living region.
    pub label_inputs: Vec<RegionLabelInput>,
    /// Regions whose membership changed enough to deserve a new model label
    /// (new regions and survivors with Jaccard < 0.8).
    pub needs_label: Vec<Id>,
    /// True when the corpus is below `min_items` and nothing was clustered.
    pub unsorted: bool,
    /// Pass to the next run.
    pub state: RegionState,
}

/// Optional per-item extras for a run.
#[derive(Default, Clone, Copy)]
pub struct RegionExtras<'a> {
    /// Title + body per item (aligned with ids) for keywords.
    pub texts: Option<&'a [String]>,
    /// Positions in the "anatomy" layout view, for naming new regions.
    pub anatomy_xyz: Option<&'a [[f32; 3]]>,
}

fn quantile(mut v: Vec<f32>, q: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() - 1) as f32 * q).round() as usize;
    v[i.min(v.len() - 1)]
}

/// Raw clustering: lists of member positions (sorted, deterministic order:
/// larger clusters first) plus unsorted positions.
pub fn cluster(space: &Space, params: &RegionParams) -> (Vec<Vec<usize>>, Vec<usize>) {
    let n = space.len();
    if n < params.min_items.max(2) {
        return (vec![], (0..n).collect());
    }
    let mcs = params.min_cluster_size.unwrap_or_else(|| (n / 150).max(10));
    let labels = hdbscan(
        &space.reduced,
        &space.knn,
        HdbscanParams { min_cluster_size: mcs, min_samples: params.min_samples, allow_single_cluster: true },
    );
    let nl = labels.iter().copied().max().unwrap_or(-1);
    let mut clusters: Vec<Vec<usize>> = vec![vec![]; (nl + 1).max(0) as usize];
    let mut noise = vec![];
    for (i, &l) in labels.iter().enumerate() {
        if l >= 0 {
            clusters[l as usize].push(i);
        } else {
            noise.push(i);
        }
    }
    clusters.retain(|c| !c.is_empty());
    // absorb noise points that sit within a cluster's typical spread
    let x = &space.normed;
    let cents: Vec<Vec<f32>> = clusters.iter().map(|m| centroid(x, m)).collect();
    let thresholds: Vec<f32> = clusters
        .iter()
        .zip(&cents)
        .map(|(m, c)| quantile(m.iter().map(|&i| dot(x.row(i), c)).collect(), params.assign_quantile) - params.assign_slack)
        .collect();
    let mut unsorted = vec![];
    for i in noise {
        let mut best: Option<(f32, usize)> = None;
        for (k, c) in cents.iter().enumerate() {
            let s = dot(x.row(i), c);
            if s >= thresholds[k] && best.is_none_or(|(b, _)| s > b) {
                best = Some((s, k));
            }
        }
        match best {
            Some((_, k)) => clusters[k].push(i),
            None => unsorted.push(i),
        }
    }
    for c in clusters.iter_mut() {
        c.sort_unstable();
    }
    clusters.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    (clusters, unsorted)
}

/// Run clustering and identity matching against `previous`.
pub fn recompute_regions(
    ids: &[Id],
    space: &Space,
    previous: &RegionState,
    now: Timestamp,
    params: &RegionParams,
    extras: RegionExtras<'_>,
) -> RegionRun {
    assert_eq!(ids.len(), space.len(), "ids and vectors must align");
    let x = &space.normed;
    let (clusters, unsorted) = cluster(space, params);
    let is_unsorted = clusters.is_empty();
    let nc = clusters.len();
    let no = previous.regions.len();

    // --- overlap between old regions and new clusters -----------------------
    let mut old_of: HashMap<&Id, usize> = HashMap::new();
    for (k, r) in previous.regions.iter().enumerate() {
        for m in &r.members {
            old_of.insert(m, k);
        }
    }
    let present: HashSet<&Id> = ids.iter().collect();
    // |A_i ∩ present|
    let old_size: Vec<usize> = previous.regions.iter().map(|r| r.members.iter().filter(|m| present.contains(m)).count()).collect();
    // |B_j ∩ old universe| and intersections
    let mut new_size = vec![0usize; nc];
    let mut inter: HashMap<(usize, usize), usize> = HashMap::new();
    for (j, c) in clusters.iter().enumerate() {
        for &i in c {
            if let Some(&o) = old_of.get(&ids[i]) {
                new_size[j] += 1;
                *inter.entry((o, j)).or_default() += 1;
            }
        }
    }
    let it = |o: usize, j: usize| *inter.get(&(o, j)).unwrap_or(&0) as f32;
    let jaccard = |o: usize, j: usize| {
        let i = it(o, j);
        let u = old_size[o] as f32 + new_size[j] as f32 - i;
        if u > 0.0 { i / u } else { 0.0 }
    };
    let c_old = |o: usize, j: usize| if old_size[o] > 0 { it(o, j) / old_size[o] as f32 } else { 0.0 };
    let c_new = |o: usize, j: usize| if new_size[j] > 0 { it(o, j) / new_size[j] as f32 } else { 0.0 };

    #[derive(Clone)]
    enum Fate {
        Survive(usize),
        Merge(Vec<usize>),
        Split(usize, usize), // (old, rank among children)
        Birth,
    }
    let mut new_fate: Vec<Option<Fate>> = vec![None; nc];
    let mut old_used = vec![false; no];
    let mut transitions_pre: Vec<(&'static str, Vec<usize>, Vec<usize>)> = vec![];

    // merges
    for j in 0..nc {
        let parents: Vec<usize> = (0..no).filter(|&o| !old_used[o] && c_old(o, j) >= params.merge_containment).collect();
        if parents.len() >= 2 {
            let mut parents = parents;
            parents.sort_by(|&a, &b| old_size[b].cmp(&old_size[a]).then(a.cmp(&b)));
            for &o in &parents {
                old_used[o] = true;
            }
            new_fate[j] = Some(Fate::Merge(parents.clone()));
            transitions_pre.push(("merge", parents, vec![j]));
        }
    }
    // splits
    for o in 0..no {
        if old_used[o] {
            continue;
        }
        let mut kids: Vec<usize> = (0..nc)
            .filter(|&j| new_fate[j].is_none() && c_old(o, j) >= params.split_containment && c_new(o, j) >= params.child_purity)
            .collect();
        if kids.len() >= 2 {
            kids.sort_by(|&a, &b| it(o, b).total_cmp(&it(o, a)).then(a.cmp(&b)));
            old_used[o] = true;
            for (rank, &j) in kids.iter().enumerate() {
                new_fate[j] = Some(Fate::Split(o, rank));
            }
            transitions_pre.push(("split", vec![o], kids));
        }
    }
    // survivors: Hungarian on Jaccard among the rest
    let rest_old: Vec<usize> = (0..no).filter(|&o| !old_used[o]).collect();
    let rest_new: Vec<usize> = (0..nc).filter(|&j| new_fate[j].is_none()).collect();
    if !rest_old.is_empty() && !rest_new.is_empty() {
        let transpose = rest_old.len() > rest_new.len();
        let (rows, cols) = if transpose { (&rest_new, &rest_old) } else { (&rest_old, &rest_new) };
        let weights: Vec<Vec<i64>> = rows
            .iter()
            .map(|&r| {
                cols.iter()
                    .map(|&c| {
                        let jac = if transpose { jaccard(c, r) } else { jaccard(r, c) };
                        (jac * 1_000_000.0) as i64
                    })
                    .collect()
            })
            .collect();
        let m = pathfinding::matrix::Matrix::from_rows(weights).expect("rectangular");
        let (_, assignment) = pathfinding::kuhn_munkres::kuhn_munkres(&m);
        for (ri, &ci) in assignment.iter().enumerate() {
            let (o, j) = if transpose { (cols[ci], rows[ri]) } else { (rows[ri], cols[ci]) };
            if jaccard(o, j) >= params.survive_jaccard {
                old_used[o] = true;
                new_fate[j] = Some(Fate::Survive(o));
            }
        }
    }
    for f in new_fate.iter_mut() {
        if f.is_none() {
            *f = Some(Fate::Birth);
        }
    }
    let new_fate: Vec<Fate> = new_fate.into_iter().map(|f| f.unwrap()).collect();

    // --- build regions --------------------------------------------------------
    let cents: Vec<Vec<f32>> = clusters.iter().map(|m| centroid(x, m)).collect();
    let new_ids: Vec<Id> = new_fate
        .iter()
        .map(|f| match f {
            Fate::Survive(o) => previous.regions[*o].region.id.clone(),
            _ => Id::new(),
        })
        .collect();

    // keywords
    let keywords: Vec<Vec<String>> = match extras.texts {
        Some(texts) => {
            let mut classes: Vec<Vec<&str>> = clusters.iter().map(|m| m.iter().map(|&i| texts[i].as_str()).collect()).collect();
            classes.push(unsorted.iter().map(|&i| texts[i].as_str()).collect());
            let kw = c_tf_idf(&classes, params.keywords);
            kw.into_iter().take(nc).map(|v| v.into_iter().map(|(w, _)| w).collect()).collect()
        }
        None => vec![vec![]; nc],
    };

    // anatomy
    let anatomy_pos: Vec<Option<[f32; 3]>> = clusters
        .iter()
        .map(|m| {
            extras.anatomy_xyz.map(|p| {
                let mut c = [0f32; 3];
                for &i in m {
                    for k in 0..3 {
                        c[k] += p[i][k];
                    }
                }
                c.map(|v| v / m.len() as f32)
            })
        })
        .collect();
    let prev_anat = |o: usize| previous.regions[o].region.anatomy.clone();
    let requests: Vec<AnatomyRequest> = (0..nc)
        .map(|j| {
            let hint = match &new_fate[j] {
                Fate::Survive(o) => match prev_anat(*o) {
                    Some(a) => AnatomyHint::Keep(a),
                    None => AnatomyHint::Birth,
                },
                Fate::Merge(ps) => AnatomyHint::Merge(ps.iter().filter_map(|&o| prev_anat(o)).collect()),
                Fate::Split(o, rank) => match prev_anat(*o) {
                    Some(a) => AnatomyHint::Split { parent: a, rank: *rank },
                    None => AnatomyHint::Birth,
                },
                Fate::Birth => AnatomyHint::Birth,
            };
            AnatomyRequest { region: new_ids[j].clone(), hint, position: anatomy_pos[j] }
        })
        .collect();
    let anatomy = assign_anatomy(&requests);

    let mut snapshots = Vec::with_capacity(nc);
    let mut regions = Vec::with_capacity(nc);
    let mut label_inputs = Vec::with_capacity(nc);
    let mut needs_label = vec![];
    let mut drift = vec![];
    for j in 0..nc {
        let members = &clusters[j];
        let sims: Vec<f32> = members.iter().map(|&i| dot(x.row(i), &cents[j])).collect();
        let cohesion = sims.iter().sum::<f32>() / sims.len().max(1) as f32;
        let assign_threshold = quantile(sims, params.assign_quantile) - params.assign_slack;
        let fb = fallback_label(&keywords[j]);
        let (label, born_at, parent_ids) = match &new_fate[j] {
            Fate::Survive(o) => {
                let r = &previous.regions[*o].region;
                (r.label.clone(), r.born_at, r.parent_ids.clone())
            }
            Fate::Merge(ps) => (fb.clone(), now, ps.iter().map(|&o| previous.regions[o].region.id.clone()).collect()),
            Fate::Split(o, _) => (fb.clone(), now, vec![previous.regions[*o].region.id.clone()]),
            Fate::Birth => (fb.clone(), now, vec![]),
        };
        let region = Region {
            id: new_ids[j].clone(),
            label,
            anatomy: Some(anatomy[j].clone()),
            size: members.len() as u32,
            born_at,
            died_at: None,
            parent_ids,
        };
        match &new_fate[j] {
            Fate::Survive(o) => {
                let prev = &previous.regions[*o];
                let jac = jaccard(*o, j);
                if jac < 0.8 {
                    needs_label.push(region.id.clone());
                }
                let before: HashSet<&String> = prev.keywords.iter().collect();
                let after: HashSet<&String> = keywords[j].iter().collect();
                drift.push(RegionDrift {
                    region: region.id.clone(),
                    drift: 1.0 - cosine(&prev.centroid, &cents[j]),
                    cohesion_before: prev.cohesion,
                    cohesion_after: cohesion,
                    size_before: prev.members.len() as u32,
                    size_after: members.len() as u32,
                    jaccard: jac,
                    keywords_gained: keywords[j].iter().filter(|k| !before.contains(k)).cloned().collect(),
                    keywords_lost: prev.keywords.iter().filter(|k| !after.contains(k)).cloned().collect(),
                });
            }
            _ => needs_label.push(region.id.clone()),
        }
        let reps = representatives(x, members, &cents[j], 6, 3);
        label_inputs.push(RegionLabelInput {
            region_id: region.id.clone(),
            keywords: keywords[j].clone(),
            representatives: reps.iter().map(|&i| ids[i].clone()).collect(),
            size: region.size,
            previous_label: match &new_fate[j] {
                Fate::Survive(_) => Some(region.label.clone()),
                _ => None,
            },
            fallback_label: fb,
        });
        snapshots.push(RegionSnapshot {
            region: region.clone(),
            members: members.iter().map(|&i| ids[i].clone()).collect(),
            centroid: cents[j].clone(),
            cohesion,
            assign_threshold,
            keywords: keywords[j].clone(),
        });
        regions.push(region);
    }

    // --- transitions + retired ------------------------------------------------
    let mut transitions = vec![];
    let mut retired = vec![];
    let retire = |o: usize| {
        let mut r = previous.regions[o].region.clone();
        r.died_at = Some(now);
        r
    };
    for (kind, olds, news) in &transitions_pre {
        match *kind {
            "merge" => {
                transitions.push(RegionTransition::Merge {
                    from: olds.iter().map(|&o| previous.regions[o].region.id.clone()).collect(),
                    into: new_ids[news[0]].clone(),
                });
                retired.extend(olds.iter().map(|&o| retire(o)));
            }
            _ => {
                transitions.push(RegionTransition::Split {
                    from: previous.regions[olds[0]].region.id.clone(),
                    into: news.iter().map(|&j| new_ids[j].clone()).collect(),
                });
                retired.push(retire(olds[0]));
            }
        }
    }
    for (j, f) in new_fate.iter().enumerate() {
        match f {
            Fate::Survive(_) => transitions.push(RegionTransition::Survive { region: new_ids[j].clone() }),
            Fate::Birth => transitions.push(RegionTransition::Birth { region: new_ids[j].clone() }),
            _ => {}
        }
    }
    for (o, used) in old_used.iter().enumerate() {
        if !used {
            transitions.push(RegionTransition::Death { region: previous.regions[o].region.id.clone() });
            retired.push(retire(o));
        }
    }

    let mut assignment: Vec<Option<Id>> = vec![None; ids.len()];
    for (j, m) in clusters.iter().enumerate() {
        for &i in m {
            assignment[i] = Some(new_ids[j].clone());
        }
    }

    RegionRun {
        regions,
        retired,
        transitions,
        assignment,
        drift,
        label_inputs,
        needs_label,
        unsorted: is_unsorted,
        state: RegionState { run_at: Some(now), regions: snapshots },
    }
}

/// Apply model labels (from [`crate::labels::parse_labels`]) to a run's
/// regions and state.
pub fn apply_labels(run: &mut RegionRun, labels: &HashMap<Id, String>) {
    for r in run.regions.iter_mut() {
        if let Some(l) = labels.get(&r.id) {
            r.label = l.clone();
        }
    }
    for s in run.state.regions.iter_mut() {
        if let Some(l) = labels.get(&s.region.id) {
            s.region.label = l.clone();
        }
    }
}

/// Centroid drift and cohesion of every region that exists in both states
/// (by id), computed from the stored snapshots alone.
pub fn drift_between(before: &RegionState, after: &RegionState) -> Vec<RegionDrift> {
    let mut out = vec![];
    for a in &after.regions {
        if let Some(b) = before.get(&a.region.id) {
            let sb: HashSet<&Id> = b.members.iter().collect();
            let sa: HashSet<&Id> = a.members.iter().collect();
            let inter = sa.intersection(&sb).count() as f32;
            let union = sa.union(&sb).count() as f32;
            let kb: HashSet<&String> = b.keywords.iter().collect();
            let ka: HashSet<&String> = a.keywords.iter().collect();
            out.push(RegionDrift {
                region: a.region.id.clone(),
                drift: 1.0 - cosine(&b.centroid, &a.centroid),
                cohesion_before: b.cohesion,
                cohesion_after: a.cohesion,
                size_before: b.members.len() as u32,
                size_after: a.members.len() as u32,
                jaccard: if union > 0.0 { inter / union } else { 0.0 },
                keywords_gained: a.keywords.iter().filter(|k| !kb.contains(k)).cloned().collect(),
                keywords_lost: b.keywords.iter().filter(|k| !ka.contains(k)).cloned().collect(),
            });
        }
    }
    out
}

/// Mean cosine to the centroid for a set of vectors (cohesion).
pub fn cohesion(vectors: &[Vec<f32>]) -> f32 {
    if vectors.is_empty() {
        return 0.0;
    }
    let x = Rows::from_vecs(vectors);
    let mut xn = x.clone();
    xn.normalize_rows();
    let mut c = vec![0f32; x.d];
    for i in 0..xn.n {
        for (a, b) in c.iter_mut().zip(xn.row(i)) {
            *a += b;
        }
    }
    normalize(&mut c);
    (0..xn.n).map(|i| dot(xn.row(i), &c)).sum::<f32>() / xn.n as f32
}
