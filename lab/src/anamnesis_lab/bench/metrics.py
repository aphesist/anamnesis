"""Metric functions (numpy only, so the tests run without the bench extras)."""

from __future__ import annotations

import math
from collections.abc import Iterable, Mapping, Sequence

import numpy as np


# --- ranking ---------------------------------------------------------------------------

def dcg(gains: Sequence[float]) -> float:
    return float(sum(g / math.log2(i + 2) for i, g in enumerate(gains)))


def ndcg_at_k(ranked: Sequence[str], qrels: Mapping[str, float], k: int = 10) -> float:
    """nDCG@k with graded relevance (linear gain, as in BEIR / trec_eval's ndcg_cut)."""
    gains = [float(qrels.get(d, 0.0)) for d in ranked[:k]]
    ideal = sorted((float(g) for g in qrels.values() if g > 0), reverse=True)[:k]
    idcg = dcg(ideal)
    return dcg(gains) / idcg if idcg > 0 else 0.0


def recall_at_k(ranked: Sequence[str], relevant: Iterable[str], k: int = 100) -> float:
    rel = set(relevant)
    if not rel:
        return 0.0
    return len(rel.intersection(ranked[:k])) / len(rel)


def hits_at_k(ranked: Sequence[str], relevant: Iterable[str], k: int) -> float:
    rel = set(relevant)
    return 1.0 if rel.intersection(ranked[:k]) else 0.0


def reciprocal_rank(ranked: Sequence[str], relevant: Iterable[str]) -> float:
    rel = set(relevant)
    for i, d in enumerate(ranked):
        if d in rel:
            return 1.0 / (i + 1)
    return 0.0


def bootstrap_ci(values: Sequence[float], n: int = 1000, alpha: float = 0.05, seed: int = 0) -> tuple[float, float]:
    """Percentile bootstrap interval for the mean."""
    v = np.asarray(values, dtype=float)
    if v.size == 0:
        return (float("nan"), float("nan"))
    rng = np.random.default_rng(seed)
    means = v[rng.integers(0, v.size, size=(n, v.size))].mean(axis=1)
    return float(np.quantile(means, alpha / 2)), float(np.quantile(means, 1 - alpha / 2))


def top_k(scores: np.ndarray, k: int) -> np.ndarray:
    """Indices of the k largest scores, best first."""
    k = min(k, scores.shape[-1])
    idx = np.argpartition(-scores, k - 1)[:k]
    return idx[np.argsort(-scores[idx], kind="stable")]


# --- layout ----------------------------------------------------------------------------

def _pairwise_sq(X: np.ndarray) -> np.ndarray:
    X = np.asarray(X, dtype=np.float64)
    sq = (X * X).sum(1)
    D = sq[:, None] + sq[None, :] - 2 * X @ X.T
    np.maximum(D, 0, out=D)
    np.fill_diagonal(D, np.inf)
    return D


def _ranks(D: np.ndarray) -> np.ndarray:
    """ranks[i, j] = rank of j among i's neighbours (1 = nearest); self gets 0."""
    order = np.argsort(D, axis=1, kind="stable")
    n = D.shape[0]
    ranks = np.empty_like(order)
    ranks[np.arange(n)[:, None], order] = np.arange(1, n + 1)[None, :]
    ranks[np.arange(n), np.arange(n)] = 0
    return ranks


def trustworthiness(X_high: np.ndarray, X_low: np.ndarray, k: int = 10) -> float:
    """Venna & Kaski trustworthiness: are the low-dim neighbours true neighbours? (1 = perfect)."""
    n = X_high.shape[0]
    if not 0 < k < n / 2:
        raise ValueError("k must be in (0, n/2)")
    r_high = _ranks(_pairwise_sq(X_high))
    nn_low = np.argsort(_pairwise_sq(X_low), axis=1, kind="stable")[:, :k]
    rows = np.arange(n)[:, None]
    penalty = np.maximum(r_high[rows, nn_low] - k, 0).sum()
    return float(1 - 2.0 / (n * k * (2 * n - 3 * k - 1)) * penalty)


def continuity(X_high: np.ndarray, X_low: np.ndarray, k: int = 10) -> float:
    """Continuity: are true neighbours kept as neighbours? Trustworthiness with roles swapped."""
    return trustworthiness(X_low, X_high, k)


def procrustes_drift(A: np.ndarray, B: np.ndarray, scale: bool = True) -> dict[str, float]:
    """Align B onto A (translation, rotation/reflection, optional uniform scale).

    Returns ``disparity`` (sum of squared residuals after both are centred and scaled
    to unit Frobenius norm, as in ``scipy.spatial.procrustes``; 0 = identical shape)
    and ``rms`` (root-mean-square displacement after alignment, in A's units).
    """
    A = np.asarray(A, dtype=np.float64)
    B = np.asarray(B, dtype=np.float64)
    if A.shape != B.shape or A.shape[0] < 2:
        raise ValueError("A and B must have the same shape with at least two points")
    A0 = A - A.mean(0)
    B0 = B - B.mean(0)
    na, nb = np.linalg.norm(A0), np.linalg.norm(B0)
    if na == 0 or nb == 0:
        raise ValueError("degenerate configuration")
    An, Bn = A0 / na, B0 / nb
    U, s, Vt = np.linalg.svd(Bn.T @ An)
    R = U @ Vt
    disparity = float(max(0.0, 1 - s.sum() ** 2))
    Bal = B0 @ R
    if scale:
        Bal *= s.sum() * na / nb
    rms = float(np.sqrt(((A0 - Bal) ** 2).sum(1).mean()))
    return {"disparity": disparity, "rms": rms, "rms_relative": rms / float(np.sqrt((A0**2).sum(1).mean()))}
