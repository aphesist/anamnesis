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

- connectors produce `anamnesis_core::Document`; the server turns them into `Item`s via core storage. two `Document.meta` keys carry per-document provenance the server applies: `privacy` (`local_only` \| `cloud_ok` \| `ask`, e.g. from markdown frontmatter) and `origin_explicit: true` (the document stated its own `origin`, from frontmatter or connector config, so the source's `default_origin` does not replace it). plugins cannot set either.
- embed exposes `trait Embedder { fn id(&self) -> &str; fn dim(&self) -> usize; fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> }` (sync; callers use `spawn_blocking`). vectors are l2-normalised.
- mind operates on in-memory slices `(&[Id], &[Vec<f32>])` plus links/timestamps, returns model types (`Region`, `RegionTransition`, `Placement`, `Thread`) — it does not touch sqlite.
- eval reads `&[Event]` and returns serialisable metric structs.
- harness exposes a `Backend` trait for chat (streaming) and an `Router` that picks a backend per task (`chat`, `label`, `extract`). every non-local call goes through the egress gate, which returns an `EventKind::Egress` for the caller to append.
- mcp is generic over a `BrainApi` trait (defined in `anamnesis-mcp`) that the server implements, so mcp has no storage dependency.

## http api (served by `anamnesis`, default `http://127.0.0.1:7368`)

all json. times are rfc3339 utc. ids are ulid strings. the ui must work against `ui/app/mock/` fixtures with the same shapes. errors are `{ error: "message" }` with status 400 (invalid), 404 (not found) or 500. the api answers only to loopback host names (`localhost`, `127.0.0.1`, `::1`); any other `Host` header gets 403 (a dns-rebinding guard).

### the brain

| method + path | returns / body |
|---|---|
| `GET /api/cloud` | the brain, columnar: `{ ids: [id], xyz: [f32 × 3n], xyz_anatomy: [f32 × 3n], xyz_spheres: [f32 × 3n], region: [i32 index into regions or -1], kind: [u8 index into kinds], speaker: [u8 0=you 1=assistants 2=other], t: [unix seconds], titles: [string], kinds: [string], regions: [{ id, label, anatomy, size, centroid: [x,y,z], born_at }], links: [[i, j, kind]] }`. `xyz` is the `layout` view; `xyz_anatomy` / `xyz_spheres` are the other two views of the same layout (equal to `xyz` until the first full layout run). |
| `GET /api/items/:id` | `{ item, links_out: [link], links_in: [link], threads: [thread], region }` |
| `POST /api/items` | body `{ title, body, kind?, tags? }` (editor) → `item` |
| `PUT /api/items/:id` | body `{ title?, body?, tags?, privacy? }` → `item`. `privacy` (`local_only` \| `cloud_ok` \| `ask`) sets the item's flag by hand: it is recorded as `item.meta.privacy_explicit = true` and nothing re-flags it afterwards (syncs, source changes). `"privacy": null` clears the hand-set flag and returns the item to its document's flag (frontmatter) or its source's default. an `ItemUpdated` event either way; a privacy-only change leaves `modified_at` alone. |
| `GET /api/search?q=&k=` | `{ hits: [hit] }` (hybrid fts + vector; `k` defaults to 20, max 200) |
| `GET /api/timeline` | `{ days: [{date, count}], months: [ym], by_region: [{region_id, counts: [n per month]}], by_speaker: {you: [n], assistants: [n], other: [n]}, threads_by_month: {decision: [n], open_question: [n], plan: [n], rejection: [n]} }` |
| `GET /api/threads?kind=&status=` | `{ threads: [thread] }`. rule-extracted threads carry `extractor: "rule:v2"`; when the extractor version changes, items are re-read once and threads the rules no longer find are withdrawn with `ThreadRemoved { thread_id }` (unless the user changed them) |
| `POST /api/threads/:id` | body `{ status, resolved_by? }` → `thread` |
| `GET /api/review` | `{ items: [{ resurface_id, item, reason, arm }] }` — today's review, drawn on first request of the day if the job loop has not yet. `arm` is `"algorithm"` or `"random"`. |
| `POST /api/review/:resurface_id` | body `{ useful: bool }` → `{ ok: true }` |
| `GET /api/suggestions?item=` | `{ suggestions: [{ suggestion_id, to: item_summary, score }] }` — the shown arm only; held-out suggestions are never returned. returning a suggestion for the first time appends `SuggestionDisplayed { suggestion_ids }`: only displayed suggestions count towards acceptance and the shown-arm estimates (suggestions handed out over mcp are not counted) |
| `POST /api/suggestions/:id` | body `{ accept: bool }` → suggestion. held-out suggestions answer 404. |
| `POST /api/links` | body `{ from, to }` (manual link) → `link` |
| `GET /api/lineage` | `{ runs: [{ run_id, at, regions: [region], transitions: [transition] }] }` |
| `GET /api/skills` | `{ skills: [{ id, title, body }] }` (items of kind `skill`) |

### chat

`POST /api/chat`, body `{ chat_id?, message, backend?, approved?: [id], denied?: [id] }` → `text/event-stream` with events:

| event | data |
|---|---|
| `token` | `{ text }` |
| `cited` | `{ ids }` — context items the reply cited as `[n]` |
| `usage` | `{ backend, input, output, estimated }` |
| `done` | `{ chat_id, item_id }` — `item_id` is the stored reply (null if the reply was empty) |
| `error` | `{ message }` |
| `consent_required` | `{ ids, chat_id }` — the chosen backend is not local and the context holds items marked `ask` that are not in `approved`. the stream ends here; nothing was sent. resend the same body with the ids the user agreed to in `approved` and the ones they declined in `denied` (partial consent): denied items are left out of the prompt for that request and do not trigger `consent_required` again. an id in both lists counts as denied. |

`backend` picks a backend for this request (an id from `/api/backends`); `"retrieval"` is the built-in responder that lists matching items without any model. with no backend configured, chat falls back to `retrieval`. items marked `local_only` are left out of the context whenever the backend is not local.

### evaluation

| method + path | returns / body |
|---|---|
| `GET /api/eval` | `{ events, links: {...}, retrieval: {...}, resurfacing: {...}, growth: {...}, honesty: { claims: [...] }, config }` — each section with `n`, point estimate, interval, `status: "measuring" \| "enough data"` and `novelty` (see `crates/anamnesis-eval/README.md`) |
| `POST /api/eval/label` | body `{ query, item_id, grade }` (grade 0–3) → `{ ok: true }` |

### sources

| method + path | returns / body |
|---|---|
| `GET /api/sources` | `{ sources: [source], connectors: [manifest + { plugin: bool, manifest_path? }], plugins: { dirs: [path], errors: [{ path, error }] } }` — the built-in connectors' manifests and every loaded connector plugin (`plugin: true`), so a ui can render their config forms and show each manifest's declared `network_hosts` / `fs_paths` (declared, not enforced). plugins are read from `<data dir>/anamnesis/plugins/*/manifest.json` (or `anamnesis-plugin.json`) and each `--plugins-dir`, rescanned on every call; a plugin whose manifest is invalid or whose id is taken is listed in `plugins.errors` instead. |
| `POST /api/sources` | body `{ connector, name?, config, default_origin?, default_privacy? }` → `source`. `connector` is a built-in id or a loaded plugin's id. the config is validated against the connector's schema and checked (`check`) before the source is stored. without `default_privacy` the source takes the `privacy.default` setting, else the connector's default. |
| `PUT /api/sources/:id` | body `{ name?, config?, default_origin?, default_privacy? }` → `source + { items_reflagged: n }`. a changed config resyncs from scratch. a changed `default_privacy` re-flags the source's items whose flag came from the old default (not those set by hand or by their own frontmatter); `items_reflagged` counts them. a changed `default_origin` applies to items synced from then on whose document did not state an origin. |
| `DELETE /api/sources/:id` | → `{ ok: true, items_deleted: n }` |
| `POST /api/sources/:id/sync` | sync now → `{ created, updated, unchanged, duplicates, deleted, renamed, pages }` |

### model backends

| method + path | returns / body |
|---|---|
| `GET /api/backends` | `{ backends: [backend], routing: { chat, label, extract, summarise }, embedder: { id, backend, status, fallback, local }, usage: [backend_usage] \| null, detected: bool, notices: { claude_cli, chatgpt } }`. a backend row is `{ id, kind, label, available, status, capabilities, terms: "allowed" \| "grey", is_local: bool, leaves_machine: bool, key_storage: "keychain" \| "file" \| "env" \| null, notice?, cooldown?, models? }`. `kind` is exactly one of `openai_compatible`, `anthropic`, `gemini`, `chatgpt`, `claude_cli` (the harness `BackendKind` enum, snake_case), or `retrieval` for the built-in responder (always present). `is_local`: calls skip the egress gate (a loopback server, or one marked `local`); `leaves_machine`: the prompt travels to another machine (every provider, the `claude_cli` route, and a lan server marked `local`; false only for loopback servers and `retrieval`). `key_storage`: where the backend's api key was found at the last detection (null when it has none or needs none). `capabilities` holds `streaming, tools, json_schema, max_context, vision, cost_model ("free_local" \| "metered" \| "subscription"), background_allowed, forbidden_params, terms, maturity ("stable" \| "experimental"), notice`. `routing.<task>` is the backend that task would use now, or null (e.g. `label` / `extract` with no local model and no opt-in). `notices` holds the exact notices to show before enabling the claude cli route or the chatgpt sign-in, whether or not a row exists yet. `detected` is false until detection has run (or when the server runs with `--no-detect`). |
| `POST /api/backends/refresh` | re-read settings and keys, re-run detection → same as `GET /api/backends` |
| `PUT /api/backends/:id/key` | body `{ key }` → `{ ok: true, key_storage: "keychain" \| "file" }`. stores an api key in the os keychain under service `anamnesis`, account `:id` (e.g. `anthropic`, `gemini`, or an openai-compatible backend's id); when there is no usable keychain (or `ANAMNESIS_KEY_STORAGE=file`), in `<data dir>/anamnesis/secrets.json` with mode 0600, which is never exported and never logged. then re-detects in the background. the key is never returned by any endpoint. lookup order: keychain, file, then the provider's environment variable (`key_storage: "env"`). |
| `DELETE /api/backends/:id/key` | removes it from the keychain and the file → `{ ok: true }` |
| `GET /api/backends/chatgpt` | `{ button_label: "Continue with ChatGPT", notice, usage_settings_url, status: { signed_in, plan_usage, email, message }, sign_in: { in_progress, last_error } }` |
| `POST /api/backends/chatgpt/sign-in` | → `{ authorize_url, button_label }`. the ui opens `authorize_url` in the browser; a loopback listener waits for the callback, then the `chatgpt` backend is enabled and detection re-runs. poll `GET /api/backends/chatgpt` for the outcome. |
| `POST /api/backends/chatgpt/sign-out` | forgets the app's own token → `{ ok: true }`. this does not revoke the app's access on openai's side. |

### settings

`GET /api/settings` → the settings object. `PUT /api/settings` takes a json object and deep-merges it into the stored settings: objects merge key by key, recursively; a key set to `null` is removed (at any depth); strings, numbers, booleans and arrays replace the stored value. keys not sent are kept. it returns all settings. every touched top-level key is written as one `SettingSet` event carrying its merged value. the typed keys below are validated after merging, before anything is stored (400 on error):

| key | type |
|---|---|
| `embedder` | an `EmbedderSpec` (`crates/anamnesis-embed/src/spec.rs`), e.g. `{ "backend": "local", "model": "multilingual-e5-small" }`, `{ "backend": "ollama", "model": "…" }`, `{ "backend": "openai_compatible", "base_url": "…", "model": "…" }`, `{ "backend": "hash" }`. changing it reloads the embedder and re-embeds (ignored while the server runs with `--hash-embedder`). |
| `routing` | `{ chat?, label?, extract?, summarise?, fallbacks?: { task: [id] }, background_to_api_by_default?: bool }` — backend ids per task. `background_to_api_by_default` defaults to **false**: with no local model, background tasks (`label`, `extract`, `summarise`) run nowhere (deterministic labels, rule-based threads) unless this is true or the task names a backend. background work never runs on `chatgpt` or `claude_cli`. |
| `harness` | the harness configuration: `{ openai_compatible: [{ id, provider, base_url, model, label?, local?, … }], anthropic?: { chat_model?, label_model?, … }, gemini?: { model?, label_model? }, chatgpt?: { enabled, model? }, claude_cli: { enabled, binary?, model? }, prices?, probe_local? }`. api keys never go here. changing it re-runs detection. |
| `privacy` | `{ default: "local_only" \| "cloud_ok" \| "ask" }` — the flag new sources get when `POST /api/sources` (or `anamnesis add-source`, `anamnesis import <folder>`) names none |
| `theme`, `motion` | written by `ui/theme` (`{ theme: {…}, motion: "full" \| "reduced" }`) |

other keys are stored as given.

### log, export, status

| method + path | returns / body |
|---|---|
| `GET /api/egress?limit=` | `{ entries: [egress event] }` — newest first, default 100 |
| `GET /api/events?after=&limit=` | `{ events: [event] }` (for replay; `after` is a sequence number, default limit 1000, max 10000) |
| `GET /api/export` | the whole brain as a portable bundle (`application/x-ndjson`: a manifest line, then every event) |
| `GET /api/status` | `{ version, demo, items, events, head, embedder, jobs: { running, phase, done, total, error } }`. `jobs` is the progress of a one-shot jobs pass (`POST /api/jobs/run`, cli commands, the pass after a demo switch): `phase` is `embeddings`, `threads`, `mind`, `suggestions` or `review` (null when idle), `done` of `total` (5) phases finished, `error` the last pass's failure or null. |
| `POST /api/jobs/run` | run the background jobs once now → `{ embedded, mind: { full, items, placed_new, regions, labelled_by } \| null, suggestions, threads, resurfaced }`. `threads` counts rule-based threads plus those from the `extract` route: when the router has a background backend for `extract`, up to 8 owner-written items that are new or changed since a model last read them (newest first; only `cloud_ok` ones unless the backend is local) go through mind's extraction prompt with its json schema; answers are validated (verbatim quotes) and stored with `extractor` `"local:<model>"` or `"<provider>:<model>"`. |
| `GET /api/demo` | `{ active, available, bundle, items }` |
| `POST /api/demo` | body `{ enabled: bool }` (default true) — switch to the demo brain (imported on first use) or back to yours → same as `GET /api/demo`. 400 if no demo bundle is found. when the demo brain has no layout yet, the jobs run once in the background right after (minutes for the full demo); poll `GET /api/status` → `jobs`. the bundle is `--demo-bundle`, `ANAMNESIS_DEMO_BUNDLE`, `demo/` next to the binary, or the lab's `lab/out/<theme>/notes` (transcendentalist first); a lab bundle folder resolves to its `notes/`. |
| `GET /api/mcp/snippets` | `{ snippets: { claude_code_command, claude_code_http_command, claude_code_mcp_json, claude_code_mcp_json_http, claude_desktop_json, codex_toml, codex_toml_http, gemini_settings_json, gemini_settings_json_http }, http_url, token_env_var: "ANAMNESIS_MCP_TOKEN", token, tools: [name], stdio_args: [string] }` — ready-to-paste client config; the http variants reference the token through the environment variable, never inline. the stdio variants run `anamnesis <stdio_args>`: `["mcp"]`, plus `--data-dir <dir>` when the server runs on a non-default data dir, `--brain-dir <data dir>/anamnesis/demo` while the demo brain is active, and `--hash-embedder` when the server uses it. |

### mcp

`/mcp` (outside `/api`) is the streamable http mcp transport, guarded by `Authorization: Bearer <token>` (the token is in `<brain dir>/mcp_token` and in `GET /api/mcp/snippets`). browser requests must come from a loopback origin. `anamnesis mcp` serves the same tools over stdio. see `docs/mcp.md`.

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
