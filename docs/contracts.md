# contracts

the seams between the parts of anamnesis. each work package owns its files; these contracts are how they meet. change a contract only by coordinating through the integrator.

## layout of the repository

| path | owner (work package) | what |
|---|---|---|
| `crates/anamnesis-core` | core | data model (`model.rs`), event log, sqlite storage, search, export/import |
| `crates/anamnesis-connectors` | connectors | sources → `Document`s; connector manifest + plugin runtime |
| `crates/anamnesis-embed` | embed | embedding backends (local onnx, ollama, openai-compatible), chunking |
| `crates/anamnesis-mind` | mind | regions (clustering with stable identity + lineage), 3d layout, link suggestion, threads extraction, resurfacing |
| `crates/anamnesis-eval` | eval | metrics computed from the event log |
| `crates/anamnesis-harness` | harness | model backends: local, api keys, sign in with chatgpt, installed claude cli; routing, limits, usage, egress gate |
| `crates/anamnesis-mcp` | mcp | mcp server (stdio + streamable http) over the brain |
| `crates/anamnesis-server` | server | the `anamnesis` binary: http api below, background jobs, serves `ui/` |
| `ui/app` | ui | vanilla js + webgl2 app: the brain, panels, editor, chat |
| `ui/theme` | theming | design tokens, theme loader, fonts, marks, settings/first-launch |
| `lab/` | lab | optional python lab: benchmarks, umap comparison, demo-brain builder |
| `docs/` | docs | user + developer docs |

`model.rs` is shared. only the core package edits it; others define local types or ask.

## rust interfaces

- connectors produce `anamnesis_core::Document`; the server turns them into `Item`s via core storage.
- embed exposes `trait Embedder { fn id(&self) -> &str; fn dim(&self) -> usize; fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> }` (sync; callers use `spawn_blocking`). vectors are l2-normalised.
- mind operates on in-memory slices `(&[Id], &[Vec<f32>])` plus links/timestamps, returns model types (`Region`, `RegionTransition`, `Placement`, `Thread`) — it does not touch sqlite.
- eval reads `&[Event]` and returns serialisable metric structs.
- harness exposes a `Backend` trait for chat (streaming) and an `Router` that picks a backend per task (`chat`, `label`, `extract`). every non-local call goes through the egress gate, which returns an `EventKind::Egress` for the caller to append.
- mcp is generic over a `BrainApi` trait (defined in `anamnesis-mcp`) that the server implements, so mcp has no storage dependency.

## http api (served by `anamnesis`, default `http://127.0.0.1:7368`)

all json. times are rfc3339 utc. ids are ulid strings. the ui must work against `ui/app/mock/` fixtures with the same shapes.

| method + path | returns / body |
|---|---|
| `GET /api/cloud` | the brain, columnar: `{ ids: [id], xyz: [f32 × 3n], region: [i32 index into regions or -1], kind: [u8 index into kinds], speaker: [u8 0=you 1=assistants 2=other], t: [unix seconds], titles: [string], kinds: [string], regions: [{ id, label, anatomy, size, centroid: [x,y,z], born_at }], links: [[i, j, kind]] }` |
| `GET /api/items/:id` | `{ item, links_out: [link], links_in: [link], threads: [thread], region }` |
| `POST /api/items` | body `{ title, body, kind?, tags? }` (editor) → `item` |
| `PUT /api/items/:id` | body `{ title?, body?, tags? }` → `item` |
| `GET /api/search?q=&k=` | `{ hits: [hit] }` (hybrid fts + vector) |
| `POST /api/chat` | body `{ chat_id?, message, backend? }` → `text/event-stream` with events `token {text}`, `cited {ids}`, `usage {backend, input, output, estimated}`, `done {chat_id, item_id}`, `error {message}` |
| `GET /api/timeline` | `{ days: [{date, count}], months: [ym], by_region: [{region_id, counts: [n per month]}], by_speaker: {you: [n], assistants: [n], other: [n]}, threads_by_month: {decision: [n], open_question: [n], plan: [n], rejection: [n]} }` |
| `GET /api/threads?kind=&status=` | `{ threads: [thread] }` |
| `POST /api/threads/:id` | body `{ status, resolved_by? }` → `thread` |
| `GET /api/review` | `{ items: [{ resurface_id, item, reason }] }` |
| `POST /api/review/:resurface_id` | body `{ useful: bool }` |
| `GET /api/suggestions?item=` | `{ suggestions: [{ suggestion_id, to: item_summary, score }] }` |
| `POST /api/suggestions/:id` | body `{ accept: bool }` |
| `POST /api/links` | body `{ from, to }` (manual link) |
| `GET /api/lineage` | `{ runs: [{ run_id, at, regions: [region], transitions: [transition] }] }` |
| `GET /api/eval` | `{ links: {...}, retrieval: {...}, resurfacing: {...}, growth: {...} }` — each with `n`, point estimate, interval, and `status: "measuring" | "enough data"` |
| `POST /api/eval/label` | body `{ query, item_id, grade }` |
| `GET /api/skills` | `{ skills: [{ id, title, body }] }` (items of kind `skill`) |
| `GET /api/sources` / `POST /api/sources` / `DELETE /api/sources/:id` | sources |
| `POST /api/sources/:id/sync` | trigger a sync |
| `GET /api/backends` | `{ backends: [{ id, kind, label, available, capabilities, status, terms: "allowed"|"grey", notice? }], routing: {chat, label, extract} }` |
| `PUT /api/settings` / `GET /api/settings` | user settings json (backend routing, theme id, motion, privacy defaults) |
| `GET /api/egress?limit=` | `{ entries: [egress event] }` |
| `GET /api/events?after=&limit=` | `{ events: [event] }` (for replay) |
| `GET /api/export` | the whole brain as a portable bundle (json lines of events + items) |

## theme tokens (css custom properties on `:root`)

the ui reads only these; themes set them. light/dark via `data-theme` on `<html>`.

- colour roles: `--bg`, `--ink`, `--ink-soft`, `--line`, `--fog`, `--accent`, `--accent-text`, `--seal`
- brain: `--dot`, `--dot-dim`, `--dot-hot` (lit), `--thread`, `--region-label`
- type: `--font-display`, `--font-text`, `--font-data`, `--type-scale` (multiplier)
- motion: `--ease`, `--dur-short`, `--dur-mid`, `--dur-long`, `--motion` (`1` full, `0` reduced)
- spacing: `--space-1` … `--space-9` (fibonacci on 4px)
- marks: `--mark` (url of the logo svg), `--wordmark` (url)

the webgl renderer re-reads colour tokens on `anamnesis:themechange` (a `CustomEvent` on `window`).

## layout config

`ui/app` builds its panels from a layout object (`ui/theme/layouts/default.json`): which panels exist, where (left / right / bottom / overlay), their order and whether they start open. users can rearrange and save it via settings.
