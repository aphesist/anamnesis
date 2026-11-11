"""Layout stability: UMAP vs PaCMAP vs the Rust layout, over incremental snapshots.

The notes of a bundle are sorted by creation time and laid out at ``snapshots`` growing
prefixes (50% … 100% of the notes), as the brain would grow. For each method:

- trustworthiness and continuity (k=10) of the 3-D layout against the embedding space;
- Procrustes drift between consecutive snapshots on the notes they share (after the best
  rotation, translation and uniform scale; ``disparity`` is scale-free).

Methods: ``umap`` (fresh fit per snapshot, fixed seed), ``umap-warm`` (initialised from the
previous snapshot, new notes placed at their nearest old neighbour), ``pacmap``, and
``rust`` when a placements file from the Rust layout is given.

Rust placements file (JSON), one of::

    {"snapshots": [{"label": "50%", "placements": [{"item_id": "...", "xyz": [x, y, z], "key": "..."}]}]}
    [{"label": ..., "placements": [...]}, ...]
    {"placements": [...]}                         # a single snapshot

``key`` (or ``basename``, ``title``, ``path``) ties a placement to a bundle note so its
embedding can be looked up; without it only the drift between snapshots is computed.
"""

from __future__ import annotations

import datetime as dt
import json
import logging
import time
from pathlib import Path

import numpy as np

from .bundle_io import read_bundle
from .embedders import cached_embed, get_embedder
from .metrics import continuity, procrustes_drift, trustworthiness

log = logging.getLogger("anamnesis_lab.bench")


def _umap(X: np.ndarray, init: np.ndarray | str = "spectral", seed: int = 42) -> np.ndarray:
    import umap

    reducer = umap.UMAP(n_components=3, n_neighbors=15, min_dist=0.1, metric="cosine", init=init,
                        random_state=seed, n_jobs=1)
    return reducer.fit_transform(X)


def _pacmap(X: np.ndarray, seed: int = 42) -> np.ndarray:
    import pacmap

    return pacmap.PaCMAP(n_components=3, n_neighbors=10, random_state=seed).fit_transform(X, init="pca")


def quality(X: np.ndarray, Y: np.ndarray, k: int = 10, max_n: int = 2000, seed: int = 0) -> dict[str, float]:
    n = X.shape[0]
    idx = np.arange(n)
    if n > max_n:
        idx = np.sort(np.random.default_rng(seed).choice(n, max_n, replace=False))
    return {"trustworthiness": trustworthiness(X[idx], Y[idx], k), "continuity": continuity(X[idx], Y[idx], k)}


def snapshot_sizes(n: int, snapshots: int, start: float = 0.5) -> list[int]:
    return [int(round(n * f)) for f in np.linspace(start, 1.0, snapshots)]


def load_rust_placements(path: Path) -> list[dict]:
    data = json.loads(path.read_text())
    if isinstance(data, dict) and "snapshots" in data:
        snaps = data["snapshots"]
    elif isinstance(data, dict) and "placements" in data:
        snaps = [data]
    elif isinstance(data, list):
        snaps = data
    else:
        raise ValueError("unrecognised placements file")
    out = []
    for i, s in enumerate(snaps):
        pl = s["placements"]
        out.append({
            "label": s.get("label", str(i)),
            "ids": [p["item_id"] for p in pl],
            "keys": [p.get("key") or p.get("basename") or p.get("title") or p.get("path") for p in pl],
            "xyz": np.asarray([p["xyz"] for p in pl], dtype=np.float64),
        })
    return out


def drift_series(snaps: list[tuple[list[str], np.ndarray]]) -> list[dict]:
    rows = []
    for (ids_a, A), (ids_b, B) in zip(snaps, snaps[1:]):
        pos_b = {k: i for i, k in enumerate(ids_b)}
        shared = [(i, pos_b[k]) for i, k in enumerate(ids_a) if k in pos_b]
        if len(shared) < 3:
            rows.append({"shared": len(shared)})
            continue
        ia, ib = map(list, zip(*shared))
        rows.append({"shared": len(shared), **procrustes_drift(A[ia], B[ib])})
    return rows


def run_layout(
    bundle: Path, model: str, cache: Path, results: Path, rust_placements: Path | None = None,
    snapshots: int = 5, max_items: int = 4000, methods: list[str] | None = None, seed: int = 0,
) -> dict:
    methods = methods or ["umap", "pacmap"]
    notes = sorted(read_bundle(bundle), key=lambda n: (n.created, n.basename))
    if len(notes) > max_items:
        keep = np.sort(np.random.default_rng(seed).choice(len(notes), max_items, replace=False))
        notes = [notes[i] for i in keep]
    names = [n.basename for n in notes]
    emb = get_embedder(model, cache)
    X = cached_embed(emb, [f"{n.title}. {n.plain_text()}" for n in notes], "passage",
                     cache / "emb" / f"layout-{bundle.name}-{len(notes)}")
    sizes = snapshot_sizes(len(notes), snapshots)
    out_rows = []

    for method in methods:
        t0 = time.time()
        layouts: list[tuple[list[str], np.ndarray]] = []
        prev: np.ndarray | None = None
        for m in sizes:
            Xs = X[:m]
            if method == "umap":
                Y = _umap(Xs)
            elif method == "umap-warm":
                if prev is None:
                    Y = _umap(Xs)
                else:
                    init = np.zeros((m, 3))
                    init[: len(prev)] = prev
                    sims = Xs[len(prev):] @ Xs[: len(prev)].T
                    nearest = sims.argmax(1)
                    jitter = np.random.default_rng(m).normal(0, 1e-3, size=(m - len(prev), 3))
                    init[len(prev):] = prev[nearest] + jitter
                    Y = _umap(Xs, init=init)
            elif method == "pacmap":
                Y = _pacmap(Xs)
            else:
                raise ValueError(f"unknown method {method!r}")
            prev = Y
            layouts.append((names[:m], Y))
            q = quality(Xs, Y)
            out_rows.append({"method": method, "snapshot": m, **q})
            log.info("layout %s n=%d T=%.3f C=%.3f", method, m, q["trustworthiness"], q["continuity"])
        for size, d in zip(sizes[1:], drift_series(layouts)):
            out_rows.append({"method": method, "snapshot": size, "drift_from_previous": d})
        out_rows.append({"method": method, "seconds": round(time.time() - t0, 1)})

    if rust_placements:
        index = {n: i for i, n in enumerate(names)}
        rsnaps = load_rust_placements(rust_placements)
        layouts = []
        for s in rsnaps:
            layouts.append((s["ids"], s["xyz"]))
            rows_idx = [(i, index[k]) for i, k in enumerate(s["keys"]) if k in index]
            row = {"method": "rust", "snapshot": s["label"], "placements": len(s["ids"]), "matched_notes": len(rows_idx)}
            if len(rows_idx) > 30:
                ia, ix = map(list, zip(*rows_idx))
                row.update(quality(X[ix], s["xyz"][ia]))
            out_rows.append(row)
        for s, d in zip(rsnaps[1:], drift_series(layouts)):
            out_rows.append({"method": "rust", "snapshot": s["label"], "drift_from_previous": d})

    doc = {
        "benchmark": "layout",
        "bundle": bundle.name,
        "date": dt.date.today().isoformat(),
        "model": model,
        "items": len(notes),
        "snapshots": sizes,
        "rows": out_rows,
        "note": "trustworthiness/continuity at k=10 on up to 2000 sampled notes; drift = Procrustes on shared notes",
    }
    results.mkdir(parents=True, exist_ok=True)
    (results / f"layout-{bundle.name}.json").write_text(json.dumps(doc, indent=2) + "\n")
    return doc
