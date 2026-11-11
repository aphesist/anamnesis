"""Retrieval on BEIR datasets (SciFact, NFCorpus): nDCG@10 and Recall@100 per model.

    anamnesis-lab bench retrieval --dataset scifact,nfcorpus --models hash,bm25,e5-small

The datasets are downloaded from the BEIR mirror into ``lab/cache/beir`` on first use.
"""

from __future__ import annotations

import csv
import datetime as dt
import io
import json
import logging
import re
import time
import zipfile
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

from .embedders import cached_embed, get_embedder
from .metrics import bootstrap_ci, ndcg_at_k, recall_at_k, top_k

log = logging.getLogger("anamnesis_lab.bench")

BEIR_URL = "https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/{name}.zip"


def load_beir(name: str, cache: Path) -> tuple[dict[str, str], dict[str, str], dict[str, dict[str, int]]]:
    """Return (corpus id→text, queries id→text, qrels query→{doc: grade}) for the test split."""
    import urllib.request

    root = cache / "beir"
    root.mkdir(parents=True, exist_ok=True)
    zpath = root / f"{name}.zip"
    if not zpath.exists():
        log.info("downloading %s", BEIR_URL.format(name=name))
        with urllib.request.urlopen(BEIR_URL.format(name=name), timeout=600) as r:
            zpath.write_bytes(r.read())
    with zipfile.ZipFile(zpath) as z:
        def read(member: str) -> str:
            return z.read(f"{name}/{member}").decode("utf-8")

        corpus = {}
        for line in read("corpus.jsonl").splitlines():
            d = json.loads(line)
            corpus[str(d["_id"])] = (d.get("title", "") + ". " + d.get("text", "")).strip(". ")
        queries = {}
        for line in read("queries.jsonl").splitlines():
            d = json.loads(line)
            queries[str(d["_id"])] = d["text"]
        qrels: dict[str, dict[str, int]] = defaultdict(dict)
        reader = csv.reader(io.StringIO(read("qrels/test.tsv")), delimiter="\t")
        next(reader)
        for qid, did, score in reader:
            if int(score) > 0:
                qrels[qid][did] = int(score)
    queries = {q: t for q, t in queries.items() if q in qrels}
    return corpus, queries, dict(qrels)


class BM25:
    """Okapi BM25 (k1=0.9, b=0.4, the Anserini defaults) over lower-cased word tokens."""

    def __init__(self, docs: list[str], k1: float = 0.9, b: float = 0.4):
        from scipy.sparse import csr_matrix

        self.k1, self.b = k1, b
        self.vocab: dict[str, int] = {}
        rows, cols, vals = [], [], []
        lengths = []
        for i, d in enumerate(docs):
            c = Counter(self.tokens(d))
            lengths.append(sum(c.values()))
            for t, n in c.items():
                j = self.vocab.setdefault(t, len(self.vocab))
                rows.append(i)
                cols.append(j)
                vals.append(n)
        tf = csr_matrix((vals, (rows, cols)), shape=(len(docs), len(self.vocab)), dtype=np.float64)
        dl = np.asarray(lengths, dtype=np.float64)
        avg = dl.mean() if len(dl) else 1.0
        df = np.bincount(tf.indices, minlength=len(self.vocab))
        self.idf = np.log(1 + (len(docs) - df + 0.5) / (df + 0.5))
        tf = tf.tocoo()
        denom = tf.data + k1 * (1 - b + b * dl[tf.row] / avg)
        w = tf.data * (k1 + 1) / denom * self.idf[tf.col]
        self.W = csr_matrix((w, (tf.row, tf.col)), shape=tf.shape).tocsc()

    @staticmethod
    def tokens(text: str) -> list[str]:
        return re.findall(r"[a-z0-9]+", text.lower())

    def scores(self, query: str) -> np.ndarray:
        cols = sorted({self.vocab[t] for t in self.tokens(query) if t in self.vocab})
        if not cols:
            return np.zeros(self.W.shape[0])
        return np.asarray(self.W[:, cols].sum(axis=1)).ravel()


def evaluate(rankings: dict[str, list[str]], qrels: dict[str, dict[str, int]]) -> dict:
    nd = [ndcg_at_k(rankings[q], qrels[q], 10) for q in rankings]
    rc = [recall_at_k(rankings[q], qrels[q].keys(), 100) for q in rankings]
    return {
        "queries": len(rankings),
        "ndcg@10": float(np.mean(nd)),
        "ndcg@10_ci95": bootstrap_ci(nd),
        "recall@100": float(np.mean(rc)),
        "recall@100_ci95": bootstrap_ci(rc),
    }


def run_retrieval(
    datasets: list[str], models: list[str], cache: Path, results: Path,
    max_docs: int | None = None, max_queries: int | None = None,
) -> list[dict]:
    results.mkdir(parents=True, exist_ok=True)
    out_rows = []
    for name in datasets:
        corpus, queries, qrels = load_beir(name, cache)
        if max_queries:
            queries = dict(list(queries.items())[:max_queries])
        doc_ids = list(corpus)
        if max_docs and len(doc_ids) > max_docs:
            # keep every judged document so the subset is still meaningful
            judged = {d for q in queries for d in qrels[q]}
            rest = [d for d in doc_ids if d not in judged]
            doc_ids = sorted(judged) + rest[: max(0, max_docs - len(judged))]
        docs = [corpus[d] for d in doc_ids]
        qids = list(queries)
        rows = []
        for model in models:
            t0 = time.time()
            rankings: dict[str, list[str]] = {}
            if model == "bm25":
                bm = BM25(docs)
                for q in qids:
                    rankings[q] = [doc_ids[i] for i in top_k(bm.scores(queries[q]), 100)]
            else:
                emb = get_embedder(model, cache)
                D = cached_embed(emb, docs, "passage", cache / "emb" / f"{name}-{len(docs)}")
                Q = cached_embed(emb, [queries[q] for q in qids], "query", cache / "emb" / f"{name}-q{len(qids)}")
                S = Q @ D.T
                for i, q in enumerate(qids):
                    rankings[q] = [doc_ids[j] for j in top_k(S[i], 100)]
            row = {"dataset": name, "model": model, "docs": len(docs), **evaluate(rankings, qrels),
                   "seconds": round(time.time() - t0, 1)}
            log.info("%s %s nDCG@10=%.3f R@100=%.3f", name, model, row["ndcg@10"], row["recall@100"])
            rows.append(row)
        doc = {
            "benchmark": "retrieval",
            "dataset": name,
            "date": dt.date.today().isoformat(),
            "subset": bool(max_docs or max_queries),
            "rows": rows,
            "note": "BEIR test split; nDCG@10 with graded qrels; 95% bootstrap intervals over queries",
        }
        (results / f"retrieval-{name}.json").write_text(json.dumps(doc, indent=2) + "\n")
        out_rows += rows
    return out_rows


__all__ = ["BM25", "evaluate", "load_beir", "run_retrieval"]
