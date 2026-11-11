"""Benchmarks: retrieval (BEIR), link prediction (temporal split of a bundle), layout stability.

Only ``metrics`` is imported eagerly; the runners import their heavy dependencies
(onnxruntime, umap-learn, pacmap, scipy) lazily, from the ``bench`` extra.
"""
