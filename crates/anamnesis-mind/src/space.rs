//! The prepared vector space every full run shares: normalised vectors, a
//! PCA reduction and an exact kNN graph on it. Computing it once lets the
//! clustering and the layout share the expensive kNN step.

use crate::knn::{Knn, knn_graph};
use crate::linalg::{Pca, Rows};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpaceParams {
    /// Dimensions kept by PCA before the kNN graph (vectors with fewer
    /// dimensions are only centred).
    pub reduce_to: usize,
    /// Neighbours per point in the kNN graph.
    pub k: usize,
    /// Whiten the PCA reduction (unit variance per component) and
    /// renormalise each row before the kNN graph. Sentence embeddings are
    /// anisotropic (a few directions dominate and every pair looks similar);
    /// whitening spreads topics apart.
    pub whiten: bool,
}

impl Default for SpaceParams {
    fn default() -> Self {
        SpaceParams {
            reduce_to: 48,
            k: 15,
            whiten: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Space {
    /// The input vectors, L2-normalised (cosine geometry).
    pub normed: Rows,
    /// PCA-reduced normalised vectors (Euclidean here ≈ cosine distance).
    pub reduced: Rows,
    pub pca: Pca,
    pub knn: Knn,
}

impl Space {
    pub fn new(vectors: &[Vec<f32>], params: SpaceParams) -> Space {
        let mut normed = Rows::from_vecs(vectors);
        normed.normalize_rows();
        Self::from_normed(normed, params)
    }

    pub fn from_normed(normed: Rows, params: SpaceParams) -> Space {
        let k = params.reduce_to.min(normed.d).max(1);
        let pca = Pca::fit(&normed, k);
        let mut reduced = pca.transform(&normed);
        if params.whiten && reduced.n > 1 && !pca.variances.is_empty() {
            // regularised whitening: shrink each variance towards the mean
            // so weak (noise) components are not blown up to equal weight
            let mean_var = pca.variances.iter().sum::<f32>() / pca.variances.len() as f32;
            let scale: Vec<f32> = pca
                .variances
                .iter()
                .map(|v| 1.0 / (v + mean_var).max(1e-12).sqrt())
                .collect();
            for i in 0..reduced.n {
                for (v, s) in reduced.row_mut(i).iter_mut().zip(&scale) {
                    *v *= s;
                }
            }
            reduced.normalize_rows();
        }
        let knn = knn_graph(&reduced, params.k);
        Space {
            normed,
            reduced,
            pca,
            knn,
        }
    }

    pub fn len(&self) -> usize {
        self.normed.n
    }

    pub fn is_empty(&self) -> bool {
        self.normed.n == 0
    }
}
