"""Render ``results/*.json`` as Markdown tables for the README."""

from __future__ import annotations

import json
from pathlib import Path


def _f(x, nd: int = 3) -> str:
    if isinstance(x, (int,)) and not isinstance(x, bool):
        return str(x)
    if isinstance(x, float):
        return f"{x:.{nd}f}"
    return "–" if x is None else str(x)


def _ci(v) -> str:
    return f"[{v[0]:.3f}, {v[1]:.3f}]" if isinstance(v, (list, tuple)) and len(v) == 2 else ""


def table(headers: list[str], rows: list[list]) -> str:
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"]
    out += ["| " + " | ".join(_f(c) for c in r) + " |" for r in rows]
    return "\n".join(out)


def render_retrieval(doc: dict) -> str:
    rows = [[r["model"], r["docs"], r["queries"], r["ndcg@10"], _ci(r.get("ndcg@10_ci95")), r["recall@100"],
             _ci(r.get("recall@100_ci95")), f"{r.get('seconds', 0):.0f}"] for r in doc["rows"]]
    title = f"### retrieval — {doc['dataset']}{' (subset)' if doc.get('subset') else ''} ({doc['date']})"
    return title + "\n\n" + table(["model", "docs", "queries", "nDCG@10", "95% CI", "Recall@100", "95% CI", "s"], rows)


def render_links(doc: dict) -> str:
    rows = [[r["model"], r["queries"], r["hits@1"], r["hits@10"], r["hits@50"], r["recall@10"], r["mrr"]] for r in doc["rows"]]
    title = (f"### link prediction — {doc['bundle']} ({doc['date']}, {doc['status']})\n\n"
             f"split at {doc['cutoff'][:10]} (quantile {doc['split_quantile']}): {doc['earlier_notes']} earlier notes, "
             f"{doc['queries']} later notes with links to earlier ones; target kinds {doc.get('target_kinds')}")
    return title + "\n\n" + table(["model", "queries", "hits@1", "hits@10", "hits@50", "recall@10", "MRR"], rows)


def render_layout(doc: dict) -> str:
    q = [r for r in doc["rows"] if "trustworthiness" in r]
    d = [r for r in doc["rows"] if "drift_from_previous" in r]
    t1 = table(["method", "snapshot", "trustworthiness", "continuity"],
               [[r["method"], r["snapshot"], r["trustworthiness"], r["continuity"]] for r in q])
    t2 = table(["method", "snapshot", "shared", "disparity", "rms (relative)"],
               [[r["method"], r["snapshot"], r["drift_from_previous"].get("shared"),
                 r["drift_from_previous"].get("disparity"), r["drift_from_previous"].get("rms_relative")] for r in d])
    title = f"### layout stability — {doc['bundle']} ({doc['date']}, {doc['items']} notes, embeddings: {doc['model']})"
    return f"{title}\n\n{t1}\n\ndrift between consecutive snapshots:\n\n{t2}"


RENDER = {"retrieval": render_retrieval, "links": render_links, "layout": render_layout}


def render_markdown(results: Path) -> str:
    parts = []
    for p in sorted(results.glob("*.json")):
        doc = json.loads(p.read_text())
        fn = RENDER.get(doc.get("benchmark"))
        if fn:
            parts.append(fn(doc))
    return "\n\n".join(parts) if parts else "_no results yet: run `anamnesis-lab bench …` first_"
