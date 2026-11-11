import json
import math

import numpy as np
import pytest

from anamnesis_lab.bench.layout import drift_series, load_rust_placements, snapshot_sizes
from anamnesis_lab.bench.linkpred import score_rankings, temporal_split
from anamnesis_lab.bench.metrics import (
    bootstrap_ci,
    continuity,
    hits_at_k,
    ndcg_at_k,
    procrustes_drift,
    recall_at_k,
    reciprocal_rank,
    top_k,
    trustworthiness,
)
from anamnesis_lab.bench.embedders import HashEmbedder


def test_ndcg_recall_mrr_hits():
    qrels = {"a": 2, "b": 1}
    assert ndcg_at_k(["a", "b", "c"], qrels, 10) == pytest.approx(1.0)
    worst = ndcg_at_k(["c", "b", "a"], qrels, 10)
    expected = (1 / math.log2(3) + 2 / math.log2(4)) / (2 + 1 / math.log2(3))
    assert worst == pytest.approx(expected)
    assert ndcg_at_k(["c"], {}, 10) == 0.0
    assert recall_at_k(["a", "c"], ["a", "b"], 100) == 0.5
    assert recall_at_k(["a", "c"], ["a", "b"], 1) == 0.5
    assert reciprocal_rank(["c", "b", "a"], ["b"]) == 0.5
    assert hits_at_k(["c", "b"], ["b"], 1) == 0.0 and hits_at_k(["c", "b"], ["b"], 2) == 1.0


def test_top_k_and_bootstrap():
    s = np.array([0.1, 0.9, 0.5, 0.7])
    assert top_k(s, 2).tolist() == [1, 3]
    lo, hi = bootstrap_ci([1.0] * 10)
    assert lo == hi == 1.0
    lo, hi = bootstrap_ci([0, 1] * 50, n=500)
    assert 0.35 < lo < 0.5 < hi < 0.65


def test_trustworthiness_continuity_identity_and_noise():
    rng = np.random.default_rng(0)
    X = rng.normal(size=(120, 8))
    assert trustworthiness(X, X, 5) == pytest.approx(1.0)
    assert continuity(X, X, 5) == pytest.approx(1.0)
    Y = rng.normal(size=(120, 3))  # unrelated layout
    assert trustworthiness(X, Y, 5) < 0.75
    # a linear projection keeps more structure than noise
    P = X[:, :3]
    assert trustworthiness(X, P, 5) > trustworthiness(X, Y, 5)
    with pytest.raises(ValueError):
        trustworthiness(X, X, 100)


def test_procrustes_invariances():
    rng = np.random.default_rng(1)
    A = rng.normal(size=(50, 3))
    q, _ = np.linalg.qr(rng.normal(size=(3, 3)))
    B = 2.5 * A @ q + np.array([3.0, -1.0, 7.0])
    d = procrustes_drift(A, B)
    assert d["disparity"] == pytest.approx(0.0, abs=1e-10)
    assert d["rms"] == pytest.approx(0.0, abs=1e-8)
    noisy = procrustes_drift(A, A + rng.normal(scale=0.3, size=A.shape))
    assert 0 < noisy["disparity"] < 0.2
    with pytest.raises(ValueError):
        procrustes_drift(A, A[:10])


def test_drift_series_and_rust_placements(tmp_path):
    rng = np.random.default_rng(2)
    xyz = rng.normal(size=(20, 3))
    snaps = {"snapshots": [
        {"label": "a", "placements": [{"item_id": f"i{k}", "xyz": xyz[k].tolist(), "key": f"n{k}"} for k in range(15)]},
        {"label": "b", "placements": [{"item_id": f"i{k}", "xyz": (xyz[k] * 2).tolist()} for k in range(20)]},
    ]}
    p = tmp_path / "placements.json"
    p.write_text(json.dumps(snaps))
    loaded = load_rust_placements(p)
    assert [s["label"] for s in loaded] == ["a", "b"]
    assert loaded[0]["keys"][0] == "n0" and loaded[1]["keys"][0] is None
    rows = drift_series([(s["ids"], s["xyz"]) for s in loaded])
    assert rows[0]["shared"] == 15 and rows[0]["disparity"] == pytest.approx(0.0, abs=1e-10)
    assert snapshot_sizes(100, 3) == [50, 75, 100]


def test_hash_embedder_is_normalised_and_lexical():
    e = HashEmbedder(256)
    V = e.embed(["the river in winter", "winter on the river", "a theory of taxes"])
    assert V.shape == (3, 256)
    assert np.allclose(np.linalg.norm(V, axis=1), 1.0)
    assert V[0] @ V[1] > V[0] @ V[2]


def test_temporal_split_and_scoring():
    import datetime as dt

    from anamnesis_lab.bench.bundle_io import BNote

    def note(name, day, links=(), kind="journal"):
        return BNote(name, name, name, dt.datetime(1850, 1, day), "A", "owner_written", kind, "", [], list(links))

    notes = [note("a", 1), note("b", 2), note("hub", 1, kind="hub"), note("c", 3, ["a", "hub", "z"]), note("d", 4, ["c"])]
    cutoff, queries = temporal_split(notes, 0.5)
    assert cutoff == dt.datetime(1850, 1, 3)
    assert queries == [(3, ["a"]), (4, ["c"])]
    _, q_all = temporal_split(notes, 0.5, targets="all")
    assert q_all[0] == (3, ["a", "hub"])
    res = score_rankings([["a", "b"], ["b", "c"]], [["a"], ["c"]])
    assert res["hits@1"] == 0.5 and res["mrr"] == pytest.approx(0.75)
