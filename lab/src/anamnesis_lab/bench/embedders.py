"""Embedding backends for the benchmarks.

- ``hash``: a feature-hashing bag of words and bigrams (no model, no download). A floor
  that any learned model must beat.
- ``e5-small``: intfloat/multilingual-e5-small as ONNX (the Xenova export), run with
  onnxruntime + tokenizers, mean pooling, L2 normalisation, "query: "/"passage: "
  prefixes. This is the model the anamnesis embed crate is meant to default to; the
  lab runs the same ONNX weights so results transfer.
- ``minilm``: sentence-transformers/all-MiniLM-L6-v2 (English-only comparison).

Vectors are L2-normalised, like the Rust ``Embedder`` contract.
"""

from __future__ import annotations

import hashlib
import re
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

import numpy as np

_TOKEN = re.compile(r"[^\W\d_]+|\d+", re.UNICODE)


class Embedder:
    id: str = "base"
    dim: int = 0

    def embed(self, texts: Sequence[str], kind: str = "passage") -> np.ndarray:  # pragma: no cover - interface
        raise NotImplementedError


def _l2(x: np.ndarray) -> np.ndarray:
    n = np.linalg.norm(x, axis=1, keepdims=True)
    n[n == 0] = 1.0
    return (x / n).astype(np.float32)


class HashEmbedder(Embedder):
    """Signed feature hashing of lower-cased words and word bigrams, log-scaled counts."""

    def __init__(self, dim: int = 512, bigrams: bool = True):
        self.dim = dim
        self.bigrams = bigrams
        self.id = f"hash-{dim}"

    @staticmethod
    def tokens(text: str) -> list[str]:
        return [t.lower() for t in _TOKEN.findall(text)]

    def _feat(self, tok: str) -> tuple[int, float]:
        h = int.from_bytes(hashlib.blake2b(tok.encode(), digest_size=8).digest(), "little")
        return h % self.dim, 1.0 if (h >> 63) & 1 else -1.0

    def embed(self, texts: Sequence[str], kind: str = "passage") -> np.ndarray:
        out = np.zeros((len(texts), self.dim), dtype=np.float64)
        cache: dict[str, tuple[int, float]] = {}
        for i, t in enumerate(texts):
            toks = self.tokens(t)
            feats = toks + ([f"{a} {b}" for a, b in zip(toks, toks[1:])] if self.bigrams else [])
            counts: dict[int, float] = {}
            for f in feats:
                if f not in cache:
                    cache[f] = self._feat(f)
                j, sgn = cache[f]
                counts[j] = counts.get(j, 0.0) + sgn
            for j, c in counts.items():
                out[i, j] = np.sign(c) * np.log1p(abs(c))
        return _l2(out)


@dataclass(frozen=True)
class OnnxSpec:
    repo: str
    onnx_file: str
    query_prefix: str = ""
    passage_prefix: str = ""
    max_length: int = 512


ONNX_MODELS: dict[str, OnnxSpec] = {
    "e5-small": OnnxSpec("Xenova/multilingual-e5-small", "onnx/model.onnx", "query: ", "passage: "),
    "e5-small-int8": OnnxSpec("Xenova/multilingual-e5-small", "onnx/model_quantized.onnx", "query: ", "passage: "),
    "minilm": OnnxSpec("Xenova/all-MiniLM-L6-v2", "onnx/model.onnx", max_length=256),
}


class OnnxEmbedder(Embedder):
    """A sentence-embedding model exported to ONNX, with mean pooling."""

    def __init__(self, name: str, cache: Path, batch_size: int = 32, max_length: int | None = None, threads: int = 2):
        import onnxruntime as ort
        from huggingface_hub import hf_hub_download
        from tokenizers import Tokenizer

        spec = ONNX_MODELS[name]
        self.spec = spec
        self.id = name
        hf_cache = cache / "hf"
        model_path = hf_hub_download(spec.repo, spec.onnx_file, cache_dir=hf_cache)
        tok_path = hf_hub_download(spec.repo, "tokenizer.json", cache_dir=hf_cache)
        self.tokenizer = Tokenizer.from_file(tok_path)
        self.max_length = max_length or spec.max_length
        self.tokenizer.enable_truncation(self.max_length)
        self.tokenizer.enable_padding()
        opts = ort.SessionOptions()
        opts.intra_op_num_threads = threads
        opts.inter_op_num_threads = 1
        self.session = ort.InferenceSession(model_path, sess_options=opts, providers=["CPUExecutionProvider"])
        self.input_names = {i.name for i in self.session.get_inputs()}
        self.batch_size = batch_size
        self.dim = int(self.session.get_outputs()[0].shape[-1] or 0)

    def embed(self, texts: Sequence[str], kind: str = "passage") -> np.ndarray:
        prefix = self.spec.query_prefix if kind == "query" else self.spec.passage_prefix
        # Sort by length so padding stays small; restore the order afterwards.
        order = np.argsort([len(t) for t in texts])
        out = np.zeros((len(texts), self.dim or 384), dtype=np.float32)
        for start in range(0, len(texts), self.batch_size):
            idx = order[start : start + self.batch_size]
            enc = self.tokenizer.encode_batch([prefix + texts[i] for i in idx])
            ids = np.array([e.ids for e in enc], dtype=np.int64)
            mask = np.array([e.attention_mask for e in enc], dtype=np.int64)
            feeds = {"input_ids": ids, "attention_mask": mask}
            if "token_type_ids" in self.input_names:
                feeds["token_type_ids"] = np.zeros_like(ids)
            hidden = self.session.run(None, feeds)[0]
            m = mask[..., None].astype(np.float32)
            pooled = (hidden * m).sum(1) / np.maximum(m.sum(1), 1e-9)
            if out.shape[1] != pooled.shape[1]:
                out = np.zeros((len(texts), pooled.shape[1]), dtype=np.float32)
                self.dim = pooled.shape[1]
            out[idx] = pooled
        return _l2(out)


def get_embedder(name: str, cache: Path) -> Embedder:
    if name == "hash" or name.startswith("hash-"):
        dim = int(name.split("-", 1)[1]) if "-" in name else 512
        return HashEmbedder(dim)
    if name in ONNX_MODELS:
        return OnnxEmbedder(name, cache)
    raise ValueError(f"unknown model {name!r}; choose hash, {', '.join(ONNX_MODELS)} (bm25 is retrieval-only)")


def cached_embed(emb: Embedder, texts: Sequence[str], kind: str, path: Path) -> np.ndarray:
    """Embed with an on-disk cache keyed by model, kind and a hash of the texts."""
    h = hashlib.sha1()
    for t in texts:
        h.update(t.encode("utf-8", "replace"))
        h.update(b"\0")
    key = path.with_name(f"{path.stem}-{emb.id}-{kind}-{h.hexdigest()[:12]}.npy")
    if key.exists():
        return np.load(key)
    key.parent.mkdir(parents=True, exist_ok=True)
    vecs = emb.embed(texts, kind=kind)
    np.save(key, vecs)
    return vecs
