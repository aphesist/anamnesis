//! Exact k-nearest-neighbour graphs by blocked matrix products.
//!
//! At the scale anamnesis targets (≤ ~50k items) an exact brute-force kNN on
//! PCA-reduced vectors is fast enough (one 20k × 20k × 48 product is ~2·10¹⁰
//! flops) and avoids the approximation noise of ANN indexes, which would show
//! up as jitter in the regions and in the layout.

use rayon::prelude::*;

use crate::linalg::{Rows, sgemm};

/// A kNN graph: for each point, `k` neighbours sorted by increasing distance.
#[derive(Debug, Clone)]
pub struct Knn {
    pub n: usize,
    pub k: usize,
    /// `n × k` neighbour indices (row-major).
    pub idx: Vec<u32>,
    /// `n × k` Euclidean distances (not squared).
    pub dist: Vec<f32>,
}

impl Knn {
    #[inline]
    pub fn neighbors(&self, i: usize) -> &[u32] {
        &self.idx[i * self.k..(i + 1) * self.k]
    }
    #[inline]
    pub fn distances(&self, i: usize) -> &[f32] {
        &self.dist[i * self.k..(i + 1) * self.k]
    }
}

/// Insert `(d, j)` into a sorted top-k buffer if it beats the worst entry.
#[inline]
fn push_topk(buf: &mut Vec<(f32, u32)>, k: usize, d: f32, j: u32) {
    if buf.len() == k {
        let worst = buf[k - 1];
        if d > worst.0 || (d == worst.0 && j > worst.1) {
            return;
        }
        buf.pop();
    }
    let pos = buf.partition_point(|&(bd, bj)| bd < d || (bd == d && bj < j));
    buf.insert(pos, (d, j));
}

/// Exact kNN of every row of `x` (Euclidean), excluding the point itself.
pub fn knn_graph(x: &Rows, k: usize) -> Knn {
    let n = x.n;
    let p = x.d;
    let k = k.min(n.saturating_sub(1));
    if k == 0 {
        return Knn { n, k: 0, idx: vec![], dist: vec![] };
    }
    let sq: Vec<f32> = (0..n).map(|i| crate::linalg::dot(x.row(i), x.row(i))).collect();
    const BLOCK: usize = 128;
    let blocks: Vec<usize> = (0..n).step_by(BLOCK).collect();
    let results: Vec<Vec<(Vec<u32>, Vec<f32>)>> = blocks
        .par_iter()
        .map(|&s| {
            let e = (s + BLOCK).min(n);
            let b = e - s;
            let mut g = vec![0f32; b * n];
            sgemm(b, p, n, &x.data[s * p..], p as isize, 1, &x.data, 1, p as isize, &mut g, n as isize, 1);
            let mut out = Vec::with_capacity(b);
            let mut buf: Vec<(f32, u32)> = Vec::with_capacity(k + 1);
            for r in 0..b {
                let i = s + r;
                buf.clear();
                let row = &g[r * n..(r + 1) * n];
                let si = sq[i];
                for (j, &gij) in row.iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    let d2 = (si + sq[j] - 2.0 * gij).max(0.0);
                    if buf.len() < k || d2 <= buf[k - 1].0 {
                        push_topk(&mut buf, k, d2, j as u32);
                    }
                }
                out.push((buf.iter().map(|&(_, j)| j).collect(), buf.iter().map(|&(d, _)| d.sqrt()).collect()));
            }
            out
        })
        .collect();
    let mut idx = Vec::with_capacity(n * k);
    let mut dist = Vec::with_capacity(n * k);
    for block in results {
        for (i, d) in block {
            idx.extend(i);
            dist.extend(d);
        }
    }
    Knn { n, k, idx, dist }
}

/// Top-`k` rows of `x` by dot product with `q` (cosine when both are
/// normalised), skipping indices for which `skip` returns true. Sorted by
/// decreasing similarity.
pub fn top_k_dot(x: &Rows, q: &[f32], k: usize, skip: impl Fn(usize) -> bool) -> Vec<(usize, f32)> {
    let mut buf: Vec<(f32, u32)> = Vec::with_capacity(k + 1);
    if k == 0 {
        return vec![];
    }
    for j in 0..x.n {
        if skip(j) {
            continue;
        }
        let s = -crate::linalg::dot(x.row(j), q);
        if buf.len() < k || s <= buf[k - 1].0 {
            push_topk(&mut buf, k, s, j as u32);
        }
    }
    buf.into_iter().map(|(s, j)| (j as usize, -s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn matches_naive() {
        let mut r = Rng::new(9);
        let vs: Vec<Vec<f32>> = (0..300).map(|_| (0..7).map(|_| r.normal() as f32).collect()).collect();
        let x = Rows::from_vecs(&vs);
        let g = knn_graph(&x, 5);
        for i in [0usize, 17, 150, 299] {
            let mut all: Vec<(f32, usize)> = (0..300)
                .filter(|&j| j != i)
                .map(|j| (crate::linalg::sq_dist(x.row(i), x.row(j)), j))
                .collect();
            all.sort_by(|a, b| a.0.total_cmp(&b.0));
            let naive: Vec<u32> = all.iter().take(5).map(|&(_, j)| j as u32).collect();
            assert_eq!(g.neighbors(i), &naive[..]);
            assert!((g.distances(i)[0] - all[0].0.sqrt()).abs() < 1e-3);
        }
        let top = top_k_dot(&x, x.row(3), 3, |j| j == 3);
        assert_eq!(top.len(), 3);
        assert!(top[0].1 >= top[1].1);
    }
}
