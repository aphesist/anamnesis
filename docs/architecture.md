# architecture

one rust binary, one sqlite file, one append-only log. everything else is derived.

```
                          ┌──────────────────────────── your machine ────────────────────────────┐
 folders, vaults ───┐     │                                                                       │
 pdfs, feeds, web ──┼────▶│ anamnesis-connectors ──▶ ingest: normalise, dedupe, chunk, refs       │
 json apis, plugins ┘     │                                    │                                  │
 editor, chat ───────────▶│                                    ▼ events                           │
 mcp capture ────────────▶│   ┌───────────────────────────────────────────────────────────┐     │
                          │   │ anamnesis-core: EVENT LOG (append-only, hash-chained)      │     │
                          │   │   projections in sqlite: items, chunks, fts5, links,       │     │
                          │   │   threads, regions + lineage, suggestions, resurfacings,   │     │
                          │   │   settings, chat turns  ·  caches: embeddings, placements  │     │
                          │   └──────┬──────────────────┬──────────────────┬───────────────┘     │
                          │          ▼                  ▼                  ▼                     │
                          │   anamnesis-embed     background jobs     anamnesis-eval            │
                          │   (onnx, ollama,      (server): threads,  (metrics from the log)    │
                          │    openai-compat,     anamnesis-mind:                               │
                          │    hash)              regions, layout,                              │
                          │                       links, review                                 │
                          │                              │ labels (optional)                    │
                          │   hybrid search ◀────────────┤                                      │
                          │   (bm25 + vectors, rrf)      ▼                                      │
                          │          │          anamnesis-harness ── egress gate ── ledger ──────┼──▶ model provider
                          │          ▼          (local, api keys, chatgpt, claude cli; router)   │    (only if you chose one)
                          │   anamnesis-server: http api · chat (sse) · /mcp · ui               │
                          │          │                                     │                     │
                          │          ▼                                     ▼                     │
                          │   ui/app + ui/theme (browser, webgl2)   anamnesis-mcp (stdio, http) ◀┼── claude code, codex,
                          └───────────────────────────────────────────────────────────────────────┘    gemini cli, desktop…
```

## crates

| crate | role | depends on |
|---|---|---|
| `anamnesis-core` | the data model (`model.rs`), the event log, sqlite storage and projections, chunking, reference extraction, hybrid search, bundles | — |
| `anamnesis-connectors` | sources → `Document`s; manifests, incremental sync, the `exec` plugin runtime | core |
| `anamnesis-embed` | the `Embedder` trait and its backends | — |
| `anamnesis-mind` | regions with stable identity and lineage, the 3d layout and its views, link suggestion and hold-out, thread extraction, resurfacing. pure computation on in-memory slices: no storage, no network | core |
| `anamnesis-eval` | every metric from `&[Event]` | core |
| `anamnesis-harness` | chat backends, routing, limits, usage, the egress gate | core |
| `anamnesis-mcp` | the mcp server, generic over a `BrainApi` trait; transports; client snippets | core |
| `anamnesis-server` | the `anamnesis` binary: api, background jobs, chat, `BrainApi` over storage, the ui | all of the above |

the http api is specified in [contracts.md](contracts.md).

## the event log

every change to the brain is an `Event { seq, at, actor, kind, prev_hash, hash }`. `kind` is one of 22 `EventKind`s (`crates/anamnesis-core/src/model.rs`): sources added and updated, items created, updated and deleted, links added, suggestions made (with their arm), accepted or dismissed, threads added and updated, regions recomputed (with transitions), layout updated, resurfacings shown and rated, retrieval labels, egress, chat turns, references, settings.

- **append-only and hash-chained.** each event's hash is sha-256 over the previous hash and the event's canonical json. `anamnesis verify` recomputes the chain.
- **attributed.** `actor` says who caused it: you, a connector (`connector:obsidian`), a background job (`mind:cluster`), an mcp client (`mcp:claude-code`) or a model (`model:anthropic/…`).
- **portable.** an export is a manifest line plus every event as one json line. importing replays it into an empty brain.

the log is the source of truth because history that was not logged cannot be reconstructed later: the evaluation needs to know what was suggested *before* you made a link, and what was held out.

## projections and replay

every state-changing store method appends events and applies them to the projection tables (items, chunks, the fts5 index, links, threads, regions, suggestions, resurfacings, settings, chat turns) in one sqlite transaction. `apply` is a pure function of the current projection state and the event, so replaying the log reproduces the live state; `Store::rebuild_projections` does exactly that, and the tests check it.

two tables are caches, deliberately not in the log: **embeddings** (recomputable from chunk text with the same model) and **placements** (the 3d layout, which the log records only as a `LayoutUpdated { placements_hash, count }` digest). a few json files next to the database hold job bookkeeping (`mind_state.json`, `views.json`, `jobs.json`).

## chunking

bodies are normalised (line endings, trailing whitespace, blank-line runs) and hashed for deduplication. chunks are paragraph- and heading-aware slices of about 400–1,200 bytes (aiming at 1,000) with exact byte spans into the body, so a search hit or a chat citation can point at the passage. chunk ids are derived from `(item, ord, text)`, and embeddings are keyed by `(chunk text hash, model)`: replay yields the same chunks, and unchanged chunks keep their vectors across edits.

## search

hybrid: sqlite fts5 (bm25) for words, brute-force cosine over the chunk vectors of the current model, fused with reciprocal rank fusion (k = 60). results are per item (best chunk) or per chunk. the vectors live as f32 blobs and are scanned from a cached, row-normalised matrix: at 100k chunks × 384 dimensions that is about 150 MB and tens of milliseconds per query, with no native extension to install. queries are embedded with the model's query prefix where it has one (e5's `query:` / `passage:`). until an embedder is loaded, search is full-text only.

## background jobs

`anamnesis serve` runs a job loop, woken by changes (debounced by 2 s) and every 30 s:

1. **embeddings** for chunks without a vector for the current model;
2. **threads**: decisions, open questions, plans and rejections found by cue phrases (english and portuguese), question marks and task checkboxes, each with a verbatim quote;
3. **the mind**: new items are placed into the existing layout immediately; a **full run** (space → layout → regions → views) happens on first run, when the embedder changes, when the brain has grown by half since the last full run, or when it changed and at least `mind_interval_minutes` (default 10) have passed;
4. **link suggestions** for new and changed items, with the hold-out ([evaluation.md](evaluation.md#latent-links));
5. **the daily review**, drawn once a day.

`POST /api/jobs/run`, `anamnesis sync` and `anamnesis import` run one pass on demand.

## regions and lineage

the shared space: vectors l2-normalised, reduced by pca, and an exact knn graph built once per full run, so clustering and layout share the expensive step.

- **clustering:** hdbscan on the knn graph (minimum cluster size `max(10, n/150)`); below 30 items nothing is clustered and the brain is one "unsorted" state. clusters whose cohesion is far below the median are dissolved. between full runs, a new item joins the nearest region if it is close enough to its centroid.
- **stable identity:** a library clustering gives new cluster numbers every run. anamnesis matches each run to the previous one by member overlap and classifies **monic-style transitions** (spiliopoulou et al., kdd 2006):
  - **merge**: two or more old regions each have ≥ 50% of their members in one new cluster → a new region with both as parents;
  - **split**: one old region has two or more new clusters each holding ≥ 25% of its members and made mostly (≥ 50%) of them → new regions with it as parent;
  - **survive**: remaining pairs matched by the hungarian algorithm on jaccard overlap; jaccard ≥ 0.5 keeps the old id, label, anatomy name and birth time;
  - **death** and **birth**: whatever is left unmatched.
  overlaps count only items that belonged to some old region, so a region that absorbs many new items still survives.
- **labels:** c-tf-idf keywords and representative items give a deterministic label; if a model is routed for `label`, it may propose a better one (through the egress gate).
- **anatomy names:** each region also gets the name of a brain area, stable across runs (survivors keep it, a merge keeps its largest parent's, a split gives it to its largest child). this is ornament, not neuroscience.

every full run is a `RegionsRecomputed { regions, transitions }` event, so `GET /api/lineage` and the time panel can replay how your regions were born, split, merged and died.

## the 3d layout and its stability

- **full layout:** a umap-like layout (fuzzy knn graph, attractive/repulsive sgd with negative sampling; mcinnes, healy & melville 2018). the first run starts from pca; later runs start from the previous coordinates (new items at the barycentre of their neighbours), take fewer, gentler epochs, and are aligned to the previous layout by orthogonal procrustes (rotation, translation, uniform scale) on the shared items. the lab's python comparison found warm starts roughly halve drift between growing snapshots at equal trustworthiness.
- **incremental placement:** between full runs a new item goes to the similarity-weighted barycentre of its nearest items, then a few local steps move only the new point. nothing else moves.
- **views:** `layout` (the coordinates themselves), `anatomy` (a rank-preserving radial warp into a two-hemisphere volume: nearness still means similarity) and `spheres` (each region packed as a ball). `/api/cloud` sends all three; the ui morphs between them.

coordinates are normalised (centred, 99th-percentile radius ≈ 1). runs are seeded; below a size threshold the sgd is sequential and reproducible for a seed, above it it runs in parallel.

## links

suggestions come from cosine neighbours, rescored: `cos + α·aa/(1+aa) + β·[same region] − near-duplicate penalty − same-session penalty`, where `aa` is adamic–adar over existing links. pairs already linked are never suggested; near-duplicates (cos > 0.95) are penalised. the hold-out arm of a pair is a seeded function of the pair. links you write in text (`[[…]]`) resolve when their target exists and stay pending until it appears.

## resurfacing

an fsrs-6 scheduler (fixed published defaults; the interval is used as a priority, not a memory model) for seeds and open questions, and a daily mix of *due*, *contextual* and *serendipity* picks, each slot randomly assigned to the algorithm or to a random pick ([evaluation.md](evaluation.md#resurfacing-a-micro-randomised-trial)).

## the harness

five chat adapters behind one `Backend` trait (openai-compatible, anthropic, gemini, chatgpt, claude cli) plus the built-in retrieval responder; a router with per-task routing, explicit fallbacks, cooldowns, usage accounting and the hard rule that background tasks never go to a subscription; and the egress gate in front of every non-local call. see [models.md](models.md).

## the ui

`ui/app` is plain es modules and webgl2, no build step: the point cloud with stipple shading and fog, panels built from a layout config, an editor, chat, review, threads, the time panel and the evaluation panel. `ui/theme` provides design tokens, themes and the settings panel ([theming.md](theming.md)). the release binary embeds both; `?mock=1` runs the ui against fixtures with no server.
