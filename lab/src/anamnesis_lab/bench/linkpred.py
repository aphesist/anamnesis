"""Link prediction on a temporal split of a demo bundle.

Queries are the notes written after the ``split`` quantile of creation time that link
to other notes. For each query the "suggestions" are the notes that existed when it
was written (created on or before its date, itself excluded), ranked by embedding
similarity: the snapshot the app would have had at that moment. A hit is a suggestion
the note really linked to.

``targets="content"`` (default) scores only links to journal entries, letters and essays;
``"all"`` also counts links to the generated person (hub) and work-index notes.

Baselines: random order, and popularity (in-degree within the snapshot).

In the demo brain these links are explicit references found by a heuristic, not links
a person made by hand, so the numbers demonstrate the pipeline; they do not measure
"latent recollection".

    anamnesis-lab bench links --bundle out/transcendentalist --models hash,e5-small
"""

from __future__ import annotations

import datetime as dt
import json
import logging
from collections import Counter
from pathlib import Path

import numpy as np

from .bundle_io import BNote, read_bundle
from .embedders import cached_embed, get_embedder
from .metrics import bootstrap_ci, hits_at_k, reciprocal_rank, top_k

log = logging.getLogger("anamnesis_lab.bench")

KS = (1, 5, 10, 20, 50)


def temporal_split(
    notes: list[BNote], q: float, targets: str = "content"
) -> tuple[dt.datetime, list[tuple[int, list[str]]]]:
    """Return (cutoff, [(index of a later note, its link targets that existed by then)])."""
    by_name = {n.basename: n for n in notes}
    content = [n for n in notes if n.kind not in ("hub", "index")]
    times = sorted(n.created for n in content)
    cutoff = times[min(len(times) - 1, int(q * len(times)))]
    queries = []
    for i, n in enumerate(notes):
        if n.kind in ("hub", "index") or n.created < cutoff:
            continue
        ts = []
        for t in dict.fromkeys(n.links):
            tn = by_name.get(t)
            if tn is None or tn is n or tn.created > n.created:
                continue
            if targets == "content" and tn.kind in ("hub", "index"):
                continue
            ts.append(t)
        if ts:
            queries.append((i, ts))
    return cutoff, queries


def score_rankings(rankings: list[list[str]], truths: list[list[str]]) -> dict:
    res: dict = {"queries": len(rankings), "links": sum(len(t) for t in truths)}
    for k in KS:
        vals = [hits_at_k(r, t, k) for r, t in zip(rankings, truths)]
        res[f"hits@{k}"] = float(np.mean(vals)) if vals else float("nan")
    rr = [reciprocal_rank(r, t) for r, t in zip(rankings, truths)]
    res["mrr"] = float(np.mean(rr)) if rr else float("nan")
    res["mrr_ci95"] = bootstrap_ci(rr)
    rec = [len(set(r[:10]) & set(t)) / len(t) for r, t in zip(rankings, truths)]
    res["recall@10"] = float(np.mean(rec)) if rec else float("nan")
    return res


def run_linkpred(
    bundle: Path, models: list[str], cache: Path, results: Path, split: float = 0.5,
    targets: str = "content", seed: int = 0, depth: int = 100,
) -> dict:
    notes = sorted(read_bundle(bundle), key=lambda n: (n.created, n.basename))
    cutoff, queries = temporal_split(notes, split, targets)
    if not queries:
        raise SystemExit("no later notes link to earlier ones at this split; try a lower --split")
    names = [n.basename for n in notes]
    created = np.array([n.created.timestamp() for n in notes])
    truths = [t for _, t in queries]

    def snapshot(i: int) -> np.ndarray:
        mask = created <= created[i]
        mask[i] = False
        return mask

    rows = []
    rng = np.random.default_rng(seed)
    rand = []
    for i, _ in queries:
        cand = np.flatnonzero(snapshot(i))
        rand.append([names[j] for j in rng.permutation(cand)[:depth]])
    rows.append({"model": "random", **score_rankings(rand, truths)})

    index = {n: j for j, n in enumerate(names)}
    pop = []
    for i, _ in queries:
        snap = snapshot(i)
        indeg = Counter(t for j in np.flatnonzero(snap) for t in notes[j].links)
        pop.append([t for t, _ in indeg.most_common() if t in index and snap[index[t]]][:depth])
    rows.append({"model": "popularity", **score_rankings(pop, truths)})

    texts = [f"{n.title}. {n.plain_text()}" for n in notes]
    q_texts = [notes[i].plain_text() for i, _ in queries]
    for m in models:
        emb = get_embedder(m, cache)
        C = cached_embed(emb, texts, "passage", cache / "emb" / f"links-{bundle.name}-c")
        Q = cached_embed(emb, q_texts, "query", cache / "emb" / f"links-{bundle.name}-q")
        ranked = []
        for qi, (i, _) in enumerate(queries):
            scores = C @ Q[qi]
            scores[~snapshot(i)] = -np.inf
            ranked.append([names[j] for j in top_k(scores, depth)])
        rows.append({"model": m, **score_rankings(ranked, truths)})
        log.info("links %s hits@10=%.3f mrr=%.3f", m, rows[-1]["hits@10"], rows[-1]["mrr"])

    by_name = {n.basename: n for n in notes}
    target_kinds = Counter(by_name[t].kind for ts in truths for t in ts)
    earlier = [n for n in notes if n.created < cutoff]
    doc = {
        "benchmark": "links",
        "bundle": bundle.name,
        "date": dt.date.today().isoformat(),
        "split_quantile": split,
        "targets": targets,
        "cutoff": cutoff.isoformat(),
        "earlier_notes": len(earlier),
        "queries": len(queries),
        "target_kinds": dict(target_kinds),
        "rows": rows,
        "status": "demonstration",
        "note": "links are explicit cross-author references found by a heuristic, not hand-made links; "
                "suggestions come from the snapshot at each query note's date",
    }
    results.mkdir(parents=True, exist_ok=True)
    (results / f"links-{bundle.name}.json").write_text(json.dumps(doc, indent=2) + "\n")
    return doc
