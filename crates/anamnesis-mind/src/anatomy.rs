//! Poetic anatomy: each region gets the name of a brain area.
//!
//! **This is ornament, not neuroscience.** The names carry no claim about
//! what the notes are "about" or where such thoughts live in a real brain;
//! they are memorable handles that stay put. The assignment is deterministic
//! and stable across runs:
//!
//! - survivors keep their name;
//! - a merge keeps the name of its largest parent;
//! - a split gives the parent's name to its largest child and the nearest
//!   free areas (to the parent's area) to the other children;
//! - a birth takes the free area nearest to where the region sits in the
//!   "anatomy" view of the layout, or the first free area in list order when
//!   no position is known;
//! - when every area is taken, names repeat with a roman-numeral suffix.
//!
//! Coordinates are in the anatomy view's frame: x left(−)/right(+),
//! y down(−)/up(+), z back(−)/front(+), roughly within the brain volume of
//! [`crate::layout`]'s anatomy warp.

use std::collections::HashSet;

use anamnesis_core::Id;

/// A named area and its rough centre in the anatomy frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub name: String,
    pub pos: [f32; 3],
}

/// Midline areas (one each).
const MIDLINE: &[(&str, [f32; 3])] = &[
    ("frontopolar cortex", [0.0, 0.10, 0.68]),
    ("ventromedial prefrontal cortex", [0.0, -0.12, 0.55]),
    ("anterior cingulate", [0.0, 0.12, 0.32]),
    ("posterior cingulate", [0.0, 0.18, -0.20]),
    ("precuneus", [0.0, 0.32, -0.38]),
    ("cuneus", [0.0, 0.05, -0.62]),
    ("supplementary motor area", [0.0, 0.45, 0.12]),
    ("corpus callosum", [0.0, 0.10, 0.0]),
    ("cerebellum", [0.0, -0.36, -0.50]),
    ("brainstem", [0.0, -0.40, -0.18]),
    ("thalamus", [0.0, -0.02, -0.05]),
    ("hypothalamus", [0.0, -0.20, 0.10]),
];

/// Bilateral areas: instantiated as "left …" (x < 0) and "right …" (x > 0).
const BILATERAL: &[(&str, [f32; 3])] = &[
    ("orbitofrontal cortex", [0.20, -0.22, 0.50]),
    ("dorsolateral prefrontal cortex", [0.36, 0.28, 0.40]),
    ("broca's area", [0.46, 0.0, 0.26]),
    ("wernicke's area", [0.46, 0.05, -0.20]),
    ("angular gyrus", [0.42, 0.20, -0.32]),
    ("supramarginal gyrus", [0.44, 0.24, -0.12]),
    ("insula", [0.32, 0.0, 0.06]),
    ("hippocampus", [0.24, -0.20, -0.12]),
    ("amygdala", [0.22, -0.22, 0.10]),
    ("temporal pole", [0.40, -0.25, 0.32]),
    ("fusiform gyrus", [0.30, -0.30, -0.30]),
    ("motor cortex", [0.30, 0.44, 0.04]),
    ("somatosensory cortex", [0.30, 0.44, -0.10]),
    ("superior parietal lobule", [0.24, 0.42, -0.30]),
    ("entorhinal cortex", [0.26, -0.28, 0.0]),
    ("caudate nucleus", [0.12, 0.10, 0.12]),
    ("putamen", [0.20, 0.0, 0.08]),
    ("visual cortex", [0.15, 0.0, -0.66]),
];

/// The fixed list of areas, in a stable order (left hemisphere first for
/// bilateral areas, so the very first regions tend to land in the "language"
/// hemisphere — again, purely poetic).
pub fn areas() -> Vec<Area> {
    let mut v = Vec::with_capacity(MIDLINE.len() + 2 * BILATERAL.len());
    // interleave so the list order (used when there is no position) spreads out
    let mut bi = BILATERAL.iter();
    let mut mi = MIDLINE.iter();
    loop {
        let mut any = false;
        if let Some((name, p)) = mi.next() {
            v.push(Area { name: name.to_string(), pos: *p });
            any = true;
        }
        if let Some((name, p)) = bi.next() {
            v.push(Area { name: format!("left {name}"), pos: [-p[0], p[1], p[2]] });
            v.push(Area { name: format!("right {name}"), pos: *p });
            any = true;
        }
        if !any {
            break;
        }
    }
    v
}

/// How a region relates to the previous run, for naming.
#[derive(Debug, Clone, PartialEq)]
pub enum AnatomyHint {
    /// Survivor: keep this name.
    Keep(String),
    /// Merge: inherit the first free name of the parents (largest first).
    Merge(Vec<String>),
    /// Split child: `rank` 0 is the largest child (inherits), others take
    /// free names nearest to the parent's area.
    Split { parent: String, rank: usize },
    /// Birth.
    Birth,
}

#[derive(Debug, Clone)]
pub struct AnatomyRequest {
    pub region: Id,
    pub hint: AnatomyHint,
    /// Region centre in the anatomy view, if known.
    pub position: Option<[f32; 3]>,
}

fn roman(n: usize) -> String {
    const TABLE: &[(usize, &str)] = &[(10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut n = n;
    let mut s = String::new();
    for &(v, r) in TABLE {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// Strip a roman-numeral suffix (" ii") to find the base area of a name.
fn base_name(name: &str) -> &str {
    if let Some((head, tail)) = name.rsplit_once(' ')
        && !tail.is_empty()
        && tail.chars().all(|c| matches!(c, 'i' | 'v' | 'x'))
    {
        return head;
    }
    name
}

/// Assign anatomy names. Returns one name per request, in order.
pub fn assign_anatomy(requests: &[AnatomyRequest]) -> Vec<String> {
    let areas = areas();
    let pos_of = |name: &str| areas.iter().find(|a| a.name == base_name(name)).map(|a| a.pos);
    let mut taken: HashSet<String> = HashSet::new();
    let mut out: Vec<Option<String>> = vec![None; requests.len()];

    let free_nearest = |taken: &HashSet<String>, target: Option<[f32; 3]>| -> String {
        let mut best: Option<(f32, usize)> = None;
        for (i, a) in areas.iter().enumerate() {
            if taken.contains(&a.name) {
                continue;
            }
            let d = match target {
                Some(t) => (0..3).map(|k| (a.pos[k] - t[k]).powi(2)).sum::<f32>(),
                None => i as f32, // list order
            };
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        if let Some((_, i)) = best {
            return areas[i].name.clone();
        }
        // everything taken: nearest area with the first free numeral
        let mut ranked: Vec<(f32, usize)> = areas
            .iter()
            .enumerate()
            .map(|(i, a)| match target {
                Some(t) => ((0..3).map(|k| (a.pos[k] - t[k]).powi(2)).sum::<f32>(), i),
                None => (i as f32, i),
            })
            .collect();
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        for n in 2.. {
            for &(_, i) in &ranked {
                let cand = format!("{} {}", areas[i].name, roman(n));
                if !taken.contains(&cand) {
                    return cand;
                }
            }
        }
        unreachable!()
    };

    // 1. survivors
    for (k, r) in requests.iter().enumerate() {
        if let AnatomyHint::Keep(name) = &r.hint
            && !taken.contains(name)
        {
            taken.insert(name.clone());
            out[k] = Some(name.clone());
        }
    }
    // 2. merges and first split children inherit
    for (k, r) in requests.iter().enumerate() {
        if out[k].is_some() {
            continue;
        }
        match &r.hint {
            AnatomyHint::Merge(parents) => {
                if let Some(p) = parents.iter().find(|p| !taken.contains(*p)) {
                    taken.insert(p.clone());
                    out[k] = Some(p.clone());
                }
            }
            AnatomyHint::Split { parent, rank: 0 } if !taken.contains(parent) => {
                taken.insert(parent.clone());
                out[k] = Some(parent.clone());
            }
            _ => {}
        }
    }
    // 3. other split children: near the parent's area, in rank order
    let mut split_rest: Vec<usize> = (0..requests.len())
        .filter(|&k| out[k].is_none() && matches!(requests[k].hint, AnatomyHint::Split { .. }))
        .collect();
    split_rest.sort_by_key(|&k| match &requests[k].hint {
        AnatomyHint::Split { rank, .. } => *rank,
        _ => 0,
    });
    for k in split_rest {
        let AnatomyHint::Split { parent, .. } = &requests[k].hint else { unreachable!() };
        let target = pos_of(parent).or(requests[k].position);
        let name = free_nearest(&taken, target);
        taken.insert(name.clone());
        out[k] = Some(name);
    }
    // 4. the rest (births, merges whose parents' names are taken, duplicate keeps)
    for k in 0..requests.len() {
        if out[k].is_some() {
            continue;
        }
        let target = match &requests[k].hint {
            AnatomyHint::Merge(parents) => parents.first().and_then(|p| pos_of(p)).or(requests[k].position),
            AnatomyHint::Keep(n) => pos_of(n).or(requests[k].position),
            _ => requests[k].position,
        };
        let name = free_nearest(&taken, target);
        taken.insert(name.clone());
        out[k] = Some(name);
    }
    out.into_iter().map(|o| o.unwrap()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(hint: AnatomyHint, position: Option<[f32; 3]>) -> AnatomyRequest {
        AnatomyRequest { region: Id::new(), hint, position }
    }

    #[test]
    fn stable_and_inheriting() {
        let names = assign_anatomy(&[
            req(AnatomyHint::Keep("left insula".into()), None),
            req(AnatomyHint::Split { parent: "precuneus".into(), rank: 0 }, None),
            req(AnatomyHint::Split { parent: "precuneus".into(), rank: 1 }, None),
            req(AnatomyHint::Birth, Some([0.0, -0.36, -0.5])),
            req(AnatomyHint::Merge(vec!["left insula".into(), "cuneus".into()]), None),
        ]);
        assert_eq!(names[0], "left insula");
        assert_eq!(names[1], "precuneus");
        // the other child lands near the precuneus
        let areas = areas();
        let p = areas.iter().find(|a| a.name == names[2]).unwrap().pos;
        assert!((p[2] + 0.38).abs() < 0.3, "{} at {:?}", names[2], p);
        assert_eq!(names[3], "cerebellum");
        assert_eq!(names[4], "cuneus");
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn overflow_uses_numerals() {
        let n = areas().len() + 3;
        let reqs: Vec<_> = (0..n).map(|_| req(AnatomyHint::Birth, None)).collect();
        let names = assign_anatomy(&reqs);
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), n);
        assert!(names.iter().any(|s| s.ends_with(" ii")));
        assert_eq!(base_name("left insula ii"), "left insula");
    }
}
