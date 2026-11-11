//! Dense linear algebra helpers: a row-major matrix, cosine maths, PCA and
//! orthogonal Procrustes alignment.

use nalgebra::{DMatrix, Matrix3, Vector3};

/// Row-major `n × d` f32 matrix (one row per item).
#[derive(Debug, Clone, PartialEq)]
pub struct Rows {
    pub n: usize,
    pub d: usize,
    pub data: Vec<f32>,
}

impl Rows {
    pub fn zeros(n: usize, d: usize) -> Self {
        Rows {
            n,
            d,
            data: vec![0.0; n * d],
        }
    }

    /// Copy a slice of vectors into a matrix. All vectors must share a dimension.
    pub fn from_vecs(vs: &[Vec<f32>]) -> Self {
        let n = vs.len();
        let d = vs.first().map(|v| v.len()).unwrap_or(0);
        let mut data = Vec::with_capacity(n * d);
        for v in vs {
            assert_eq!(v.len(), d, "all vectors must have the same dimension");
            data.extend_from_slice(v);
        }
        Rows { n, d, data }
    }

    #[inline]
    pub fn row(&self, i: usize) -> &[f32] {
        &self.data[i * self.d..(i + 1) * self.d]
    }

    #[inline]
    pub fn row_mut(&mut self, i: usize) -> &mut [f32] {
        &mut self.data[i * self.d..(i + 1) * self.d]
    }

    /// L2-normalise every row in place (zero rows stay zero).
    pub fn normalize_rows(&mut self) {
        for i in 0..self.n {
            normalize(self.row_mut(i));
        }
    }
}

#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    // chunks of 8 let the compiler vectorise
    let mut acc = [0f32; 8];
    let ca = a.chunks_exact(8);
    let cb = b.chunks_exact(8);
    let ra = ca.remainder();
    let rb = cb.remainder();
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

#[inline]
pub fn norm(a: &[f32]) -> f32 {
    dot(a, a).sqrt()
}

pub fn normalize(a: &mut [f32]) {
    let n = norm(a);
    if n > 1e-12 {
        for x in a.iter_mut() {
            *x /= n;
        }
    }
}

/// Cosine similarity (does not assume normalised inputs).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let na = norm(a);
    let nb = norm(b);
    if na < 1e-12 || nb < 1e-12 {
        return 0.0;
    }
    dot(a, b) / (na * nb)
}

#[inline]
pub fn sq_dist(a: &[f32], b: &[f32]) -> f32 {
    let mut s = 0.0;
    for (x, y) in a.iter().zip(b) {
        let d = x - y;
        s += d * d;
    }
    s
}

/// Normalised mean of a set of rows (a cosine centroid).
pub fn centroid(rows: &Rows, members: &[usize]) -> Vec<f32> {
    let mut c = vec![0f32; rows.d];
    for &i in members {
        for (a, b) in c.iter_mut().zip(rows.row(i)) {
            *a += b;
        }
    }
    normalize(&mut c);
    c
}

/// `C (m×n) = A (m×k) · B (k×n)` with explicit strides, via matrixmultiply.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sgemm(
    m: usize,
    k: usize,
    n: usize,
    a: &[f32],
    rsa: isize,
    csa: isize,
    b: &[f32],
    rsb: isize,
    csb: isize,
    c: &mut [f32],
    rsc: isize,
    csc: isize,
) {
    if m == 0 || n == 0 {
        return;
    }
    // SAFETY: the callers pass slices whose extents cover every index
    // reachable through (m, k, n) and the given strides.
    unsafe {
        matrixmultiply::sgemm(
            m,
            k,
            n,
            1.0,
            a.as_ptr(),
            rsa,
            csa,
            b.as_ptr(),
            rsb,
            csb,
            0.0,
            c.as_mut_ptr(),
            rsc,
            csc,
        );
    }
}

/// Result of a principal component analysis.
#[derive(Debug, Clone)]
pub struct Pca {
    pub mean: Vec<f32>,
    /// `k` components, each of length `d`, ordered by decreasing variance.
    pub components: Vec<Vec<f32>>,
    pub variances: Vec<f32>,
}

impl Pca {
    /// Fit the top `k` components of `x`.
    pub fn fit(x: &Rows, k: usize) -> Pca {
        let (n, d) = (x.n, x.d);
        let k = k.min(d).max(1);
        let mut mean = vec![0f32; d];
        for i in 0..n {
            for (m, v) in mean.iter_mut().zip(x.row(i)) {
                *m += v;
            }
        }
        if n > 0 {
            for m in mean.iter_mut() {
                *m /= n as f32;
            }
        }
        let mut xc = x.clone();
        for i in 0..n {
            for (v, m) in xc.row_mut(i).iter_mut().zip(&mean) {
                *v -= m;
            }
        }
        // covariance = Xcᵀ Xc / n  (d × d)
        let mut cov = vec![0f32; d * d];
        sgemm(
            d, n, d, &xc.data, 1, d as isize, &xc.data, d as isize, 1, &mut cov, d as isize, 1,
        );
        let denom = (n.max(2) - 1) as f64;
        let m = DMatrix::<f64>::from_fn(d, d, |i, j| cov[i * d + j] as f64 / denom);
        let eig = nalgebra::SymmetricEigen::new(m);
        let mut order: Vec<usize> = (0..d).collect();
        order.sort_by(|&a, &b| eig.eigenvalues[b].total_cmp(&eig.eigenvalues[a]));
        let mut components = Vec::with_capacity(k);
        let mut variances = Vec::with_capacity(k);
        for &j in order.iter().take(k) {
            let col = eig.eigenvectors.column(j);
            let mut v: Vec<f32> = col.iter().map(|&x| x as f32).collect();
            // deterministic sign: largest-magnitude coordinate positive
            let (mut best, mut bi) = (0f32, 0usize);
            for (i, x) in v.iter().enumerate() {
                if x.abs() > best {
                    best = x.abs();
                    bi = i;
                }
            }
            if v[bi] < 0.0 {
                for x in v.iter_mut() {
                    *x = -*x;
                }
            }
            components.push(v);
            variances.push(eig.eigenvalues[j].max(0.0) as f32);
        }
        Pca {
            mean,
            components,
            variances,
        }
    }

    pub fn k(&self) -> usize {
        self.components.len()
    }

    /// Project rows onto the components.
    pub fn transform(&self, x: &Rows) -> Rows {
        let (n, d, k) = (x.n, x.d, self.k());
        let mut xc = x.clone();
        for i in 0..n {
            for (v, m) in xc.row_mut(i).iter_mut().zip(&self.mean) {
                *v -= m;
            }
        }
        let mut w = vec![0f32; d * k]; // d × k row-major
        for (j, c) in self.components.iter().enumerate() {
            for i in 0..d {
                w[i * k + j] = c[i];
            }
        }
        let mut out = Rows::zeros(n, k);
        sgemm(
            n,
            d,
            k,
            &xc.data,
            d as isize,
            1,
            &w,
            k as isize,
            1,
            &mut out.data,
            k as isize,
            1,
        );
        out
    }

    pub fn transform_one(&self, v: &[f32]) -> Vec<f32> {
        self.components
            .iter()
            .map(|c| {
                c.iter()
                    .zip(v)
                    .zip(&self.mean)
                    .map(|((c, x), m)| c * (x - m))
                    .sum()
            })
            .collect()
    }
}

/// A similarity transform `x ↦ s·R·x + t` found by Procrustes analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Similarity {
    pub rotation: [[f32; 3]; 3],
    pub scale: f32,
    pub translation: [f32; 3],
}

impl Similarity {
    pub fn identity() -> Self {
        Similarity {
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
            translation: [0.0; 3],
        }
    }

    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let r = &self.rotation;
        let mut out = [0f32; 3];
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.scale * (r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2])
                + self.translation[i];
        }
        out
    }
}

/// Orthogonal Procrustes with uniform scale: find `s, R, t` (R a proper
/// rotation) minimising `Σ |s R b_i + t − a_i|²`. Returns the transform and the
/// normalised residual `Σ|…|² / Σ|a_i − ā|²` (0 = perfect fit).
pub fn procrustes(a: &[[f32; 3]], b: &[[f32; 3]], allow_scale: bool) -> (Similarity, f32) {
    assert_eq!(a.len(), b.len());
    let n = a.len();
    if n < 3 {
        return (Similarity::identity(), 0.0);
    }
    let mean = |p: &[[f32; 3]]| {
        let mut m = Vector3::<f64>::zeros();
        for q in p {
            m += Vector3::new(q[0] as f64, q[1] as f64, q[2] as f64);
        }
        m / n as f64
    };
    let ma = mean(a);
    let mb = mean(b);
    let mut h = Matrix3::<f64>::zeros();
    let mut var_b = 0.0;
    let mut var_a = 0.0;
    for (pa, pb) in a.iter().zip(b) {
        let va = Vector3::new(pa[0] as f64, pa[1] as f64, pa[2] as f64) - ma;
        let vb = Vector3::new(pb[0] as f64, pb[1] as f64, pb[2] as f64) - mb;
        h += vb * va.transpose();
        var_b += vb.norm_squared();
        var_a += va.norm_squared();
    }
    let svd = h.svd(true, true);
    let u = svd.u.unwrap();
    let vt = svd.v_t.unwrap();
    let mut dmat = Matrix3::<f64>::identity();
    if (vt.transpose() * u.transpose()).determinant() < 0.0 {
        dmat[(2, 2)] = -1.0;
    }
    let r = vt.transpose() * dmat * u.transpose();
    let trace: f64 = (0..3).map(|i| svd.singular_values[i] * dmat[(i, i)]).sum();
    let s = if allow_scale && var_b > 1e-12 {
        trace / var_b
    } else {
        1.0
    };
    let t = ma - s * r * mb;
    let mut rotation = [[0f32; 3]; 3];
    for (i, row) in rotation.iter_mut().enumerate() {
        for (j, x) in row.iter_mut().enumerate() {
            *x = r[(i, j)] as f32;
        }
    }
    let sim = Similarity {
        rotation,
        scale: s as f32,
        translation: [t[0] as f32, t[1] as f32, t[2] as f32],
    };
    let mut resid = 0.0f64;
    for (pa, pb) in a.iter().zip(b) {
        let q = sim.apply(*pb);
        resid += (0..3).map(|i| ((q[i] - pa[i]) as f64).powi(2)).sum::<f64>();
    }
    let rel = if var_a > 1e-12 { resid / var_a } else { 0.0 };
    (sim, rel as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn pca_finds_main_axis() {
        let mut r = Rng::new(3);
        let vs: Vec<Vec<f32>> = (0..500)
            .map(|_| {
                let t = r.normal() as f32 * 5.0;
                vec![
                    t,
                    t * 0.5 + r.normal() as f32 * 0.1,
                    r.normal() as f32 * 0.1,
                ]
            })
            .collect();
        let x = Rows::from_vecs(&vs);
        let p = Pca::fit(&x, 2);
        let c = &p.components[0];
        let expected = [2.0f32 / 5f32.sqrt(), 1.0 / 5f32.sqrt(), 0.0];
        assert!(cosine(c, &expected).abs() > 0.99);
        assert!(p.variances[0] > 10.0 * p.variances[1]);
        let t = p.transform(&x);
        let one = p.transform_one(x.row(4));
        assert!((t.row(4)[0] - one[0]).abs() < 1e-3);
    }

    #[test]
    fn procrustes_recovers_rotation() {
        let mut r = Rng::new(5);
        let a: Vec<[f32; 3]> = (0..100)
            .map(|_| [r.normal() as f32, r.normal() as f32, r.normal() as f32])
            .collect();
        let (c, s) = (0.6f32.cos(), 0.6f32.sin());
        let b: Vec<[f32; 3]> = a
            .iter()
            .map(|p| {
                [
                    2.0 * (c * p[0] - s * p[1]) + 1.0,
                    2.0 * (s * p[0] + c * p[1]) - 3.0,
                    2.0 * p[2],
                ]
            })
            .collect();
        let (sim, resid) = procrustes(&a, &b, true);
        assert!(resid < 1e-6, "resid {resid}");
        assert!((sim.scale - 0.5).abs() < 1e-3);
        let q = sim.apply(b[7]);
        for i in 0..3 {
            assert!((q[i] - a[7][i]).abs() < 1e-3);
        }
    }
}
