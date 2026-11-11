//! The prepared vector space every full run shares: normalised vectors, a
//! PCA reduction and an exact kNN graph on it. Computing it once lets the
//! clustering and the layout share the expensive kNN step.

use crate::knn::{Knn, knn_graph};
use crate::linalg::{Pca, Rows};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct SpaceParams {
    /// Dimensions kept by PCA before the kNN graph (vectors with fewer
    /// dimensions are only centred).
    pub reduce_to: usize,
    /// Neighbours per point in the kNN graph.
    pub k: usize,
}

impl Default for SpaceParams {
    fn default() -> Self {
        SpaceParams {
            reduce_to: 48,
            k: 15,
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
        let reduced = pca.transform(&normed);
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
