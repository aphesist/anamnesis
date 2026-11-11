//! The 3D brain: a UMAP-like layout with stability across growth, plus the
//! "anatomy" and "spheres" views the UI morphs between.
//!
//! - **Full layout** ([`layout_full`]): fuzzy kNN graph as in UMAP (McInnes,
//!   Healy & Melville 2018) on the shared [`Space`], optimised by the same
//!   attractive/repulsive SGD with negative sampling. The first run starts
//!   from PCA; later runs start from the previous coordinates (new items at
//!   the barycentre of their neighbours), take fewer, gentler epochs, and are
//!   then aligned to the previous layout with orthogonal Procrustes (rotation,
//!   translation, uniform scale) on the shared items.
//! - **Incremental placement** ([`place_new`]): a new item goes to the
//!   similarity-weighted barycentre of its k nearest existing items, followed
//!   by a few local attraction/repulsion steps that move only the new point.
//! - **Views** ([`views`]): `layout` (the normalised layout itself),
//!   `anatomy` (a rank-preserving radial warp into a two-hemisphere brain
//!   volume — ornament, nearness is still the meaning) and `spheres` (each
//!   region packed as a ball around its centroid).
//!
//! Coordinates are normalised to a unit-ish box: centred, 99th-percentile
//! radius ≈ 1. Frame: x left/right, y down/up, z back/front.

use anamnesis_core::{Id, Placement};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::knn::top_k_dot;
use crate::linalg::{Rows, procrustes};
use crate::rng::Rng;
use crate::space::Space;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LayoutParams {
    /// Epochs for a fresh layout; `None` = 300 below 10k points, else 200.
    pub n_epochs: Option<usize>,
    /// Epochs when refining from previous coordinates.
    pub refine_epochs: usize,
    pub learning_rate: f32,
    pub refine_learning_rate: f32,
    /// UMAP curve parameters (defaults ≈ min_dist 0.1, spread 1).
    pub a: f32,
    pub b: f32,
    pub negative_sample_rate: usize,
    pub seed: u64,
    /// Neighbours used for incremental placement.
    pub place_k: usize,
    /// Local relaxation steps for incremental placement.
    pub place_iters: usize,
    /// Run the SGD in parallel (Hogwild, not bit-reproducible) when there
    /// are at least this many points; `None` = always sequential and
    /// deterministic for a seed.
    pub parallel_above: Option<usize>,
}

impl Default for LayoutParams {
    fn default() -> Self {
        LayoutParams {
            n_epochs: None,
            refine_epochs: 60,
            learning_rate: 1.0,
            refine_learning_rate: 0.3,
            a: 1.577,
            b: 0.8951,
            negative_sample_rate: 5,
            seed: 0x5eed,
            place_k: 10,
            place_iters: 40,
            parallel_above: Some(5_000),
        }
    }
}

/// The normalised 3D layout ("layout" view), aligned with `ids`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub ids: Vec<Id>,
    pub xyz: Vec<[f32; 3]>,
}

impl Layout {
    pub fn index(&self) -> HashMap<&Id, usize> {
        self.ids.iter().enumerate().map(|(i, id)| (id, i)).collect()
    }

    pub fn placements(&self, assignment: &[Option<Id>]) -> Vec<Placement> {
        self.ids
            .iter()
            .zip(&self.xyz)
            .enumerate()
            .map(|(i, (id, p))| Placement {
                item_id: id.clone(),
                xyz: *p,
                region_id: assignment.get(i).cloned().flatten(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutRun {
    pub layout: Layout,
    /// Normalised Procrustes residual against the previous layout on shared
    /// items (0 = identical up to rotation/scale/translation); `None` on a
    /// first run.
    pub procrustes_residual: Option<f32>,
    pub shared: usize,
}

/// UMAP scale: optimisation runs in coordinates of roughly ±10.
const UMAP_SCALE: f32 = 10.0;

/// Symmetric fuzzy simplicial set of the kNN graph as a directed edge list.
fn fuzzy_graph(space: &Space) -> (Vec<(u32, u32)>, Vec<f32>) {
    let knn = &space.knn;
    let n = knn.n;
    let k = knn.k;
    if k == 0 {
        return (vec![], vec![]);
    }
    let target = (k as f32).log2();
    let mut raw: Vec<(u32, u32, f32)> = Vec::with_capacity(n * k);
    for i in 0..n {
        let ds = knn.distances(i);
        let rho = ds.iter().copied().find(|&d| d > 0.0).unwrap_or(0.0);
        let (mut lo, mut hi, mut mid) = (0.0f32, f32::INFINITY, 1.0f32);
        for _ in 0..64 {
            let s: f32 = ds.iter().map(|&d| (-(d - rho).max(0.0) / mid).exp()).sum();
            if (s - target).abs() < 1e-5 {
                break;
            }
            if s > target {
                hi = mid;
                mid = (lo + hi) / 2.0;
            } else {
                lo = mid;
                mid = if hi.is_infinite() {
                    mid * 2.0
                } else {
                    (lo + hi) / 2.0
                };
            }
        }
        let mean_d = ds.iter().sum::<f32>() / k as f32;
        let sigma = mid.max(1e-3 * mean_d).max(1e-12);
        for (&j, &d) in knn.neighbors(i).iter().zip(ds) {
            let w = (-(d - rho).max(0.0) / sigma).exp();
            raw.push((i as u32, j, w));
        }
    }
    // symmetrise: w = a + b − ab
    let mut keyed: Vec<(u32, u32, f32, bool)> = raw
        .into_iter()
        .map(|(i, j, w)| {
            if i < j {
                (i, j, w, true)
            } else {
                (j, i, w, false)
            }
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut edges = Vec::with_capacity(keyed.len() * 2);
    let mut weights = Vec::with_capacity(keyed.len() * 2);
    let mut t = 0;
    while t < keyed.len() {
        let (i, j, w1, _) = keyed[t];
        let mut w = w1;
        if t + 1 < keyed.len() && keyed[t + 1].0 == i && keyed[t + 1].1 == j {
            let w2 = keyed[t + 1].2;
            w = w1 + w2 - w1 * w2;
            t += 2;
        } else {
            t += 1;
        }
        edges.push((i, j));
        weights.push(w);
        edges.push((j, i));
        weights.push(w);
    }
    (edges, weights)
}

/// Fast `x^p` for x > 0 via bit-level log2/exp2 approximations (relative
/// error ~1e-3, plenty for layout gradients).
#[inline]
fn fast_pow(x: f32, p: f32) -> f32 {
    // log2 via exponent + cubic on the mantissa
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // [1, 2)
    let l = e as f32 + (-0.344_845 * m + 2.024_658) * m - 1.674_873;
    let y = (p * l).clamp(-126.0, 126.0);
    // exp2: integer part via exponent bits, fraction by a cubic
    let yi = y.floor();
    let f = y - yi;
    let pf = 1.0 + f * (0.695_556_9 + f * (0.226_173_6 + f * 0.078_024_5));
    f32::from_bits(((yi as i32 + 127) as u32) << 23) * pf
}

#[inline]
fn clip(v: f32) -> f32 {
    v.clamp(-4.0, 4.0)
}

/// UMAP's SGD. `fixed[i]` points never move.
/// UMAP's SGD with negative sampling. Edges are processed in fixed chunks,
/// each with its own seeded generator. With `parallel` the chunks run on the
/// rayon pool Hogwild-style (lock-free relaxed atomic positions, as UMAP's
/// numba version does), which is fast but not bit-reproducible; otherwise
/// they run in order and the result is deterministic for a seed.
#[allow(clippy::too_many_arguments)]
fn optimize(
    y: &mut [[f32; 3]],
    edges: &[(u32, u32)],
    weights: &[f32],
    n_epochs: usize,
    lr: f32,
    p: &LayoutParams,
    seed: u64,
    parallel: bool,
) {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
    let n = y.len();
    if edges.is_empty() || n < 2 || n_epochs == 0 {
        return;
    }
    let max_w = weights.iter().copied().fold(0.0f32, f32::max);
    let mut eps: Vec<f32> = Vec::with_capacity(edges.len());
    let mut keep: Vec<(u32, u32)> = Vec::with_capacity(edges.len());
    for (e, &w) in weights.iter().enumerate() {
        if n_epochs as f32 * w / max_w >= 1.0 {
            eps.push(max_w / w);
            keep.push(edges[e]);
        }
    }
    let eps_neg: Vec<f32> = eps
        .iter()
        .map(|e| e / p.negative_sample_rate as f32)
        .collect();
    let mut next_s: Vec<f32> = eps.clone();
    let mut next_neg: Vec<f32> = eps_neg.clone();
    let pos: Vec<AtomicU32> = y
        .iter()
        .flat_map(|q| q.iter().map(|v| AtomicU32::new(v.to_bits())))
        .collect();
    let (a, b) = (p.a, p.b);
    const CHUNK: usize = 8192;

    let run_chunk = |epoch: usize, ci: usize, ns: &mut [f32], nn: &mut [f32]| {
        let alpha = lr * (1.0 - epoch as f32 / n_epochs as f32);
        let ef = epoch as f32;
        let mut rng = Rng::new(crate::rng::mix64(seed ^ ((epoch as u64) << 32) ^ ci as u64));
        let load = |i: usize| {
            [
                f32::from_bits(pos[3 * i].load(Relaxed)),
                f32::from_bits(pos[3 * i + 1].load(Relaxed)),
                f32::from_bits(pos[3 * i + 2].load(Relaxed)),
            ]
        };
        let store = |i: usize, v: [f32; 3]| {
            for k in 0..3 {
                pos[3 * i + k].store(v[k].to_bits(), Relaxed);
            }
        };
        let base = ci * CHUNK;
        for s in 0..ns.len() {
            if ns[s] > ef {
                continue;
            }
            let slot = base + s;
            let (j, k) = (keep[slot].0 as usize, keep[slot].1 as usize);
            let mut cur = load(j);
            let mut other = load(k);
            let d2 = (cur[0] - other[0]).powi(2)
                + (cur[1] - other[1]).powi(2)
                + (cur[2] - other[2]).powi(2);
            if d2 > 0.0 {
                let pw = fast_pow(d2, b);
                let gc = -2.0 * a * b * pw / d2 / (a * pw + 1.0);
                for dd in 0..3 {
                    let g = clip(gc * (cur[dd] - other[dd])) * alpha;
                    cur[dd] += g;
                    other[dd] -= g;
                }
                store(k, other);
            }
            ns[s] += eps[slot];
            let n_neg = ((ef - nn[s]) / eps_neg[slot]).max(0.0) as usize;
            for _ in 0..n_neg {
                let kk = rng.below(n);
                if kk == j {
                    continue;
                }
                let other = load(kk);
                let d2 = (cur[0] - other[0]).powi(2)
                    + (cur[1] - other[1]).powi(2)
                    + (cur[2] - other[2]).powi(2);
                let gc = if d2 > 0.0 {
                    2.0 * b / ((0.001 + d2) * (a * fast_pow(d2, b) + 1.0))
                } else {
                    0.0
                };
                for dd in 0..3 {
                    let g = if gc > 0.0 {
                        clip(gc * (cur[dd] - other[dd]))
                    } else {
                        4.0
                    };
                    cur[dd] += g * alpha;
                }
            }
            store(j, cur);
            nn[s] += n_neg as f32 * eps_neg[slot];
        }
    };

    for epoch in 0..n_epochs {
        if parallel {
            next_s
                .par_chunks_mut(CHUNK)
                .zip(next_neg.par_chunks_mut(CHUNK))
                .enumerate()
                .for_each(|(ci, (ns, nn))| run_chunk(epoch, ci, ns, nn));
        } else {
            for (ci, (ns, nn)) in next_s
                .chunks_mut(CHUNK)
                .zip(next_neg.chunks_mut(CHUNK))
                .enumerate()
            {
                run_chunk(epoch, ci, ns, nn);
            }
        }
    }
    for (i, q) in y.iter_mut().enumerate() {
        for k in 0..3 {
            q[k] = f32::from_bits(pos[3 * i + k].load(Relaxed));
        }
    }
}

fn center_and_scale(y: &mut [[f32; 3]]) {
    if y.is_empty() {
        return;
    }
    let n = y.len() as f32;
    let mut m = [0f32; 3];
    for p in y.iter() {
        for k in 0..3 {
            m[k] += p[k] / n;
        }
    }
    let mut r: Vec<f32> = y
        .iter()
        .map(|p| ((p[0] - m[0]).powi(2) + (p[1] - m[1]).powi(2) + (p[2] - m[2]).powi(2)).sqrt())
        .collect();
    r.sort_by(|a, b| a.total_cmp(b));
    let r99 = r[((r.len() - 1) as f32 * 0.99) as usize].max(1e-9);
    for p in y.iter_mut() {
        for k in 0..3 {
            p[k] = (p[k] - m[k]) / r99;
        }
    }
}

/// Rotate so the principal axes of the cloud are z (longest), x, y (shortest)
/// — the anatomy frame's front-back, left-right and up-down extents.
fn principal_frame(y: &mut [[f32; 3]]) {
    if y.len() < 4 {
        return;
    }
    let v: Vec<Vec<f32>> = y.iter().map(|p| p.to_vec()).collect();
    let pca = crate::linalg::Pca::fit(&Rows::from_vecs(&v), 3);
    if pca.k() < 3 {
        return;
    }
    // new z = pc0, new x = pc1, new y = pc2 (right-handed: x × y = z)
    let (c0, c1) = (&pca.components[0], &pca.components[1]);
    let c2 = [
        c0[1] * c1[2] - c0[2] * c1[1],
        c0[2] * c1[0] - c0[0] * c1[2],
        c0[0] * c1[1] - c0[1] * c1[0],
    ];
    // x = c1, y = ?, z = c0 with x × y = z → y = z × x = c0 × c1 = c2
    for p in y.iter_mut() {
        let q = *p;
        let d = |c: &[f32]| {
            c[0] * (q[0] - pca.mean[0]) + c[1] * (q[1] - pca.mean[1]) + c[2] * (q[2] - pca.mean[2])
        };
        *p = [d(c1), d(&c2), d(c0)];
    }
}

/// Full layout of every item in `space`, optionally continuing `previous`.
pub fn layout_full(
    ids: &[Id],
    space: &Space,
    previous: Option<&Layout>,
    params: &LayoutParams,
) -> LayoutRun {
    let n = space.len();
    assert_eq!(ids.len(), n);
    let mut rng = Rng::new(params.seed);
    if n == 0 {
        return LayoutRun {
            layout: Layout::default(),
            procrustes_residual: None,
            shared: 0,
        };
    }
    let (edges, weights) = fuzzy_graph(space);
    let parallel = params.parallel_above.is_some_and(|m| n >= m);
    let prev_index = previous.map(|p| p.index());
    let mut has = vec![false; n];
    let mut y = vec![[0f32; 3]; n];
    let mut shared = 0usize;
    if let (Some(prev), Some(pi)) = (previous, prev_index.as_ref()) {
        for (i, id) in ids.iter().enumerate() {
            if let Some(&k) = pi.get(id) {
                let p = prev.xyz[k];
                y[i] = [p[0] * UMAP_SCALE, p[1] * UMAP_SCALE, p[2] * UMAP_SCALE];
                has[i] = true;
                shared += 1;
            }
        }
    }
    let refine = shared >= 3;
    if refine {
        // new items: barycentre of placed kNN neighbours (repeat to propagate)
        for _ in 0..8 {
            let mut changed = false;
            for i in 0..n {
                if has[i] {
                    continue;
                }
                let mut acc = [0f32; 3];
                let mut wsum = 0.0;
                for (&j, &d) in space.knn.neighbors(i).iter().zip(space.knn.distances(i)) {
                    let j = j as usize;
                    if has[j] {
                        let w = 1.0 / (d + 1e-3);
                        for k in 0..3 {
                            acc[k] += w * y[j][k];
                        }
                        wsum += w;
                    }
                }
                if wsum > 0.0 {
                    y[i] = [
                        acc[0] / wsum + 0.01 * rng.normal() as f32,
                        acc[1] / wsum + 0.01 * rng.normal() as f32,
                        acc[2] / wsum + 0.01 * rng.normal() as f32,
                    ];
                    has[i] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for (i, h) in has.iter().enumerate() {
            if !h {
                y[i] = [
                    rng.normal() as f32,
                    rng.normal() as f32,
                    rng.normal() as f32,
                ];
            }
        }
        optimize(
            &mut y,
            &edges,
            &weights,
            params.refine_epochs,
            params.refine_learning_rate,
            params,
            params.seed,
            parallel,
        );
    } else {
        // PCA init (the reduced space is already PCA-ordered)
        let r = &space.reduced;
        for (i, p) in y.iter_mut().enumerate() {
            let row = r.row(i);
            *p = [
                row.first().copied().unwrap_or(0.0),
                row.get(1).copied().unwrap_or(0.0),
                row.get(2).copied().unwrap_or(0.0),
            ];
        }
        let maxabs = y
            .iter()
            .flat_map(|p| p.iter())
            .fold(0f32, |m, v| m.max(v.abs()))
            .max(1e-9);
        for p in y.iter_mut() {
            for v in p.iter_mut() {
                *v = *v / maxabs * UMAP_SCALE + 1e-4 * rng.normal() as f32;
            }
        }
        let epochs = params
            .n_epochs
            .unwrap_or(if n > 10_000 { 200 } else { 300 });
        optimize(
            &mut y,
            &edges,
            &weights,
            epochs,
            params.learning_rate,
            params,
            params.seed,
            parallel,
        );
    }

    let mut residual = None;
    if let (true, Some(prev), Some(pi)) = (refine, previous, prev_index.as_ref()) {
        let mut a = Vec::with_capacity(shared);
        let mut b = Vec::with_capacity(shared);
        for (i, id) in ids.iter().enumerate() {
            if let Some(&k) = pi.get(id) {
                a.push(prev.xyz[k]);
                b.push(y[i]);
            }
        }
        let (sim, res) = procrustes(&a, &b, true);
        for p in y.iter_mut() {
            *p = sim.apply(*p);
        }
        residual = Some(res);
    } else {
        principal_frame(&mut y);
        center_and_scale(&mut y);
    }
    LayoutRun {
        layout: Layout {
            ids: ids.to_vec(),
            xyz: y,
        },
        procrustes_residual: residual,
        shared,
    }
}

/// Place new items into an existing layout without moving anything else.
/// `existing` holds the vectors of `layout.ids` (same order). Returns the
/// positions of the new items and appends them to `layout`.
pub fn place_new(
    layout: &mut Layout,
    existing: &[Vec<f32>],
    new_ids: &[Id],
    new_vectors: &[Vec<f32>],
    params: &LayoutParams,
) -> Vec<[f32; 3]> {
    assert_eq!(existing.len(), layout.ids.len());
    assert_eq!(new_ids.len(), new_vectors.len());
    let mut x = Rows::from_vecs(existing);
    x.normalize_rows();
    let d = x.d.max(new_vectors.first().map(|v| v.len()).unwrap_or(0));
    if x.n == 0 {
        x = Rows::zeros(0, d);
    }
    let mut rng = Rng::new(params.seed ^ (layout.ids.len() as u64).wrapping_mul(0x9E37));
    let (a, b) = (params.a, params.b);
    let mut out = Vec::with_capacity(new_ids.len());
    for (id, v) in new_ids.iter().zip(new_vectors) {
        let mut q = v.clone();
        crate::linalg::normalize(&mut q);
        let n = layout.xyz.len();
        let pos = if n == 0 {
            [0.0; 3]
        } else {
            let nn = top_k_dot(&x, &q, params.place_k.min(n), |_| false);
            let mut acc = [0f32; 3];
            let mut wsum = 0f32;
            let ws: Vec<f32> = nn.iter().map(|&(_, s)| s.max(0.0).powi(3) + 1e-6).collect();
            for (&(j, _), &w) in nn.iter().zip(&ws) {
                for k in 0..3 {
                    acc[k] += w * layout.xyz[j][k];
                }
                wsum += w;
            }
            // barycentre, in UMAP scale for relaxation
            let mut p = [
                acc[0] / wsum * UMAP_SCALE,
                acc[1] / wsum * UMAP_SCALE,
                acc[2] / wsum * UMAP_SCALE,
            ];
            for k in 0..3 {
                p[k] += 0.02 * rng.normal() as f32;
            }
            let wmax = ws.iter().copied().fold(0f32, f32::max);
            for it in 0..params.place_iters {
                let alpha = 0.1 * (1.0 - it as f32 / params.place_iters as f32);
                for (&(j, _), &w) in nn.iter().zip(&ws) {
                    let o = layout.xyz[j].map(|c| c * UMAP_SCALE);
                    let d2 = (0..3).map(|k| (p[k] - o[k]).powi(2)).sum::<f32>();
                    if d2 > 0.0 {
                        let pw = d2.powf(b);
                        let gc = -2.0 * a * b * pw / d2 / (a * pw + 1.0) * (w / wmax);
                        for k in 0..3 {
                            p[k] += clip(gc * (p[k] - o[k])) * alpha;
                        }
                    }
                }
                for _ in 0..params.negative_sample_rate {
                    let j = rng.below(n);
                    let o = layout.xyz[j].map(|c| c * UMAP_SCALE);
                    let d2 = (0..3).map(|k| (p[k] - o[k]).powi(2)).sum::<f32>();
                    let gc = 2.0 * b / ((0.001 + d2) * (a * d2.powf(b) + 1.0));
                    for k in 0..3 {
                        p[k] += clip(gc * (p[k] - o[k])) * alpha * 0.2;
                    }
                }
            }
            p.map(|c| c / UMAP_SCALE)
        };
        layout.ids.push(id.clone());
        layout.xyz.push(pos);
        // the new point can anchor later new points
        x.data.extend_from_slice(&q);
        x.n += 1;
        out.push(pos);
    }
    out
}

/// The three positions per item the UI morphs between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutViews {
    pub ids: Vec<Id>,
    /// The normalised layout.
    pub layout: Vec<[f32; 3]>,
    /// The brain-shaped warp of the layout.
    pub anatomy: Vec<[f32; 3]>,
    /// Regions packed as spheres; unsorted items as a loose halo.
    pub spheres: Vec<[f32; 3]>,
}

impl LayoutViews {
    /// Positions for a view by name: "layout", "anatomy" or "spheres".
    pub fn mode(&self, name: &str) -> Option<&[[f32; 3]]> {
        match name {
            "layout" => Some(&self.layout),
            "anatomy" => Some(&self.anatomy),
            "spheres" => Some(&self.spheres),
            _ => None,
        }
    }
}

/// Fitted brain warp: maps layout coordinates into a two-hemisphere volume
/// by a rank-preserving radial remap (directions kept, radial order kept).
#[derive(Debug, Clone)]
pub struct AnatomyWarp {
    mean: [f32; 3],
    std: [f32; 3],
    radii: Vec<f32>,
}

/// Ellipsoids (centre, semi-axes) making up the brain volume, in the
/// anatomy frame. The first two contain the origin.
const BRAIN: &[([f32; 3], [f32; 3])] = &[
    ([-0.17, 0.05, 0.0], [0.40, 0.46, 0.70]), // left hemisphere
    ([0.17, 0.05, 0.0], [0.40, 0.46, 0.70]),  // right hemisphere
    ([-0.36, -0.20, 0.12], [0.20, 0.18, 0.40]), // left temporal lobe
    ([0.36, -0.20, 0.12], [0.20, 0.18, 0.40]), // right temporal lobe
    ([0.0, -0.34, -0.50], [0.36, 0.16, 0.22]), // cerebellum
];

/// Distance from the origin to the brain surface along unit direction `u`.
fn brain_radius(u: [f32; 3]) -> f32 {
    let mut best = 0f32;
    for (c, s) in BRAIN {
        let qa: f32 = (0..3).map(|k| (u[k] / s[k]).powi(2)).sum();
        let qb: f32 = (0..3).map(|k| -2.0 * u[k] * c[k] / (s[k] * s[k])).sum();
        let qc: f32 = (0..3).map(|k| (c[k] / s[k]).powi(2)).sum::<f32>() - 1.0;
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            let t = (-qb + disc.sqrt()) / (2.0 * qa);
            if t > best {
                best = t;
            }
        }
    }
    best.max(0.2)
}

impl AnatomyWarp {
    pub fn fit(xyz: &[[f32; 3]]) -> AnatomyWarp {
        let n = xyz.len().max(1) as f32;
        let mut mean = [0f32; 3];
        for p in xyz {
            for k in 0..3 {
                mean[k] += p[k] / n;
            }
        }
        let mut std = [0f32; 3];
        for p in xyz {
            for k in 0..3 {
                std[k] += (p[k] - mean[k]).powi(2) / n;
            }
        }
        let std = std.map(|v| v.sqrt().max(1e-6));
        let mut radii: Vec<f32> = xyz.iter().map(|p| Self::whiten(p, &mean, &std).2).collect();
        radii.sort_by(|a, b| a.total_cmp(b));
        AnatomyWarp { mean, std, radii }
    }

    fn whiten(p: &[f32; 3], mean: &[f32; 3], std: &[f32; 3]) -> ([f32; 3], [f32; 3], f32) {
        let q = [
            (p[0] - mean[0]) / std[0],
            (p[1] - mean[1]) / std[1],
            (p[2] - mean[2]) / std[2],
        ];
        let r = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
        let u = if r > 1e-9 {
            q.map(|v| v / r)
        } else {
            [0.0, 0.0, 1.0]
        };
        (q, u, r)
    }

    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let (_, u, r) = Self::whiten(&p, &self.mean, &self.std);
        let n = self.radii.len().max(1);
        let rank = self.radii.partition_point(|&x| x < r) as f32;
        let f = ((rank + 0.5) / n as f32).clamp(0.0, 1.0);
        let rad = brain_radius(u) * f.cbrt() * 0.97;
        let mut out = u.map(|v| v * rad);
        // a sagittal fissure between the hemispheres
        if out[1] > -0.15 {
            out[0] += 0.035 * out[0].signum();
        }
        out
    }
}

/// Fitted sphere packing for the "spheres" view.
#[derive(Debug, Clone)]
pub struct SpherePack {
    /// Region centroid in the layout.
    pub source: Vec<[f32; 3]>,
    /// Packed centre per region.
    pub centers: Vec<[f32; 3]>,
    pub radius: Vec<f32>,
    /// Sorted member distances to the source centroid, per region.
    radii: Vec<Vec<f32>>,
    halo_radii: Vec<f32>,
    halo_inner: f32,
}

impl SpherePack {
    /// `region_of[i]` is the region index of item i (`None` = unsorted).
    pub fn fit(xyz: &[[f32; 3]], region_of: &[Option<usize>], n_regions: usize) -> SpherePack {
        let mut source = vec![[0f32; 3]; n_regions];
        let mut count = vec![0usize; n_regions];
        for (p, r) in xyz.iter().zip(region_of) {
            if let Some(r) = *r {
                for k in 0..3 {
                    source[r][k] += p[k];
                }
                count[r] += 1;
            }
        }
        for (s, &c) in source.iter_mut().zip(&count) {
            if c > 0 {
                *s = s.map(|v| v / c as f32);
            }
        }
        // radius ∝ cbrt(size), total volume fraction fixed
        let total: f32 = count.iter().map(|&c| c as f32).sum::<f32>().max(1.0);
        let fill = 0.30f32; // Σ r³
        let radius: Vec<f32> = count
            .iter()
            .map(|&c| (fill * c as f32 / total).cbrt())
            .collect();
        // separate overlapping spheres, weakly pulled to their source
        let mut centers = source.clone();
        let gap = 0.03;
        for _ in 0..200 {
            let mut moved = false;
            for i in 0..n_regions {
                for j in i + 1..n_regions {
                    if count[i] == 0 || count[j] == 0 {
                        continue;
                    }
                    let d: Vec<f32> = (0..3).map(|k| centers[j][k] - centers[i][k]).collect();
                    let mut dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    let need = radius[i] + radius[j] + gap;
                    if dist < need {
                        let dir = if dist > 1e-6 {
                            [d[0] / dist, d[1] / dist, d[2] / dist]
                        } else {
                            dist = 0.0;
                            // deterministic direction for coincident centres
                            let a = (i * 31 + j * 17) as f32;
                            [a.cos(), a.sin(), 0.0]
                        };
                        let push = (need - dist) / 2.0;
                        let (wi, wj) = (
                            radius[j] / (radius[i] + radius[j]),
                            radius[i] / (radius[i] + radius[j]),
                        );
                        for k in 0..3 {
                            centers[i][k] -= dir[k] * push * wi * 2.0;
                            centers[j][k] += dir[k] * push * wj * 2.0;
                        }
                        moved = true;
                    }
                }
            }
            for i in 0..n_regions {
                for k in 0..3 {
                    centers[i][k] += 0.02 * (source[i][k] - centers[i][k]);
                }
            }
            if !moved {
                break;
            }
        }
        let mut radii = vec![vec![]; n_regions];
        let mut halo_radii = vec![];
        for (p, r) in xyz.iter().zip(region_of) {
            match *r {
                Some(r) => radii[r].push(dist3(p, &source[r])),
                None => halo_radii.push(norm3(p)),
            }
        }
        for r in radii.iter_mut() {
            r.sort_by(|a, b| a.total_cmp(b));
        }
        halo_radii.sort_by(|a, b| a.total_cmp(b));
        let halo_inner = centers
            .iter()
            .zip(&radius)
            .zip(&count)
            .filter(|(_, c)| **c > 0)
            .map(|((c, r), _)| norm3(c) + r)
            .fold(0.0f32, f32::max)
            .max(0.8)
            + 0.08;
        SpherePack {
            source,
            centers,
            radius,
            radii,
            halo_radii,
            halo_inner,
        }
    }

    pub fn apply(&self, p: [f32; 3], region: Option<usize>) -> [f32; 3] {
        match region {
            Some(r) if r < self.centers.len() => {
                let o = [
                    p[0] - self.source[r][0],
                    p[1] - self.source[r][1],
                    p[2] - self.source[r][2],
                ];
                let d = norm3(&o);
                let u = if d > 1e-9 {
                    o.map(|v| v / d)
                } else {
                    [0.0, 0.0, 0.0]
                };
                let rs = &self.radii[r];
                let f = if rs.is_empty() {
                    0.5
                } else {
                    (rs.partition_point(|&x| x < d) as f32 + 0.5) / rs.len() as f32
                };
                let rad = self.radius[r] * f.clamp(0.0, 1.0).cbrt();
                [
                    self.centers[r][0] + u[0] * rad,
                    self.centers[r][1] + u[1] * rad,
                    self.centers[r][2] + u[2] * rad,
                ]
            }
            _ => {
                let d = norm3(&p);
                let u = if d > 1e-9 {
                    p.map(|v| v / d)
                } else {
                    [0.0, 1.0, 0.0]
                };
                let hs = &self.halo_radii;
                let f = if hs.is_empty() {
                    0.5
                } else {
                    (hs.partition_point(|&x| x < d) as f32 + 0.5) / hs.len() as f32
                };
                u.map(|v| v * (self.halo_inner + 0.25 * f))
            }
        }
    }
}

fn norm3(p: &[f32; 3]) -> f32 {
    (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
}

fn dist3(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Map a region assignment (ids) to dense indices for [`SpherePack`].
pub fn region_indices(assignment: &[Option<Id>]) -> (Vec<Option<usize>>, Vec<Id>) {
    let mut order: Vec<Id> = vec![];
    let mut index: HashMap<&Id, usize> = HashMap::new();
    let mut out = Vec::with_capacity(assignment.len());
    for a in assignment {
        out.push(a.as_ref().map(|id| {
            *index.entry(id).or_insert_with(|| {
                order.push(id.clone());
                order.len() - 1
            })
        }));
    }
    (out, order)
}

/// Compute all three views for a layout and a region assignment (aligned
/// with `layout.ids`).
pub fn views(layout: &Layout, assignment: &[Option<Id>]) -> LayoutViews {
    let warp = AnatomyWarp::fit(&layout.xyz);
    let anatomy = layout.xyz.iter().map(|p| warp.apply(*p)).collect();
    let (region_of, order) = region_indices(assignment);
    let pack = SpherePack::fit(&layout.xyz, &region_of, order.len());
    let spheres = layout
        .xyz
        .iter()
        .zip(&region_of)
        .map(|(p, r)| pack.apply(*p, *r))
        .collect();
    LayoutViews {
        ids: layout.ids.clone(),
        layout: layout.xyz.clone(),
        anatomy,
        spheres,
    }
}

/// Trustworthiness-style check: the fraction of each point's `k` nearest
/// neighbours in `space` that are also among its `k` nearest in the layout,
/// averaged over a sample of points (1.0 = neighbourhoods fully preserved).
pub fn neighbourhood_preservation(space: &Space, xyz: &[[f32; 3]], k: usize, sample: usize) -> f32 {
    let n = xyz.len();
    if n < 2 {
        return 1.0;
    }
    let k = k.min(space.knn.k).min(n - 1);
    let step = (n / sample.max(1)).max(1);
    let mut total = 0f32;
    let mut count = 0;
    for i in (0..n).step_by(step) {
        let mut d: Vec<(f32, usize)> = (0..n)
            .filter(|&j| j != i)
            .map(|j| (dist3(&xyz[i], &xyz[j]), j))
            .collect();
        d.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
        let low: std::collections::HashSet<usize> = d[..k].iter().map(|&(_, j)| j).collect();
        let hits = space.knn.neighbors(i)[..k]
            .iter()
            .filter(|&&j| low.contains(&(j as usize)))
            .count();
        total += hits as f32 / k as f32;
        count += 1;
    }
    total / count as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_pow_is_close() {
        for &x in &[1e-4f32, 0.01, 0.3, 1.0, 2.5, 17.0, 400.0] {
            for &p in &[0.8951f32, -0.1049, 1.0] {
                let (a, b) = (fast_pow(x, p), x.powf(p));
                assert!((a - b).abs() / b < 5e-3, "{x}^{p}: {a} vs {b}");
            }
        }
    }
}
