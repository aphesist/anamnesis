<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme/hero-dark.svg">
  <img alt="anamnesis: the transcendentalist demo brain, 9,233 dated entries drawn as stipple, turning slowly while region names surface and fade" src="assets/readme/hero-light.svg" width="100%">
</picture>

a local-first second brain that measures itself.

</div>

anamnesis gathers what you write and read (notes, journals, obsidian vaults, pdfs, feeds, web clips, chats) into one brain on your own machine. every item is a dot in a turning 3d stipple cloud; similar items gather into regions that keep their identity as the brain grows, and you can replay how they were born, split and merged. everything that changes the brain is an event in an append-only, hash-chained log, and that log drives a built-in evaluation that asks one question of your own writing:

> is learning recollection? what share of the links i later make by hand was already latent in what i had written?

it is one rust binary with a browser ui. it works with no model at all, brings your own model when you want one (local, api key, or your chatgpt plan through openai's sign-in), and is also an mcp server, so claude code, codex, gemini cli and claude desktop can read and write your brain with the subscriptions you already have.

<table>
<tr>
<td width="50%"><img alt="the brain: a stipple point cloud in the shape of a brain, with region labels, the view panel on the right and the lineage timeline below" src="ui/app/docs-screens/brain.png"></td>
<td width="50%"><img alt="chat answering from the brain, with cited items" src="ui/app/docs-screens/chat.png"></td>
</tr>
<tr>
<td><img alt="the evaluation panel: latent links, retrieval, resurfacing, growth, each with an interval and a status" src="ui/app/docs-screens/eval.png"></td>
<td><img alt="when you wrote: writing over time by speaker" src="ui/app/docs-screens/when-you-wrote.png"></td>
</tr>
<tr>
<td><img alt="regions packed as spheres" src="ui/app/docs-screens/spheres.png"></td>
<td><img alt="threads: decisions, open questions, plans and rejections found in the notes" src="ui/app/docs-screens/threads.png"></td>
</tr>
<tr>
<td><img alt="a journal entry in the reader, with provenance and link suggestions" src="ui/app/docs-screens/notes.png"></td>
<td><img alt="settings: models, continue with chatgpt, the claude cli switch and routing" src="ui/app/docs-screens/settings.png"></td>
</tr>
</table>

<sub>the pictures show the transcendentalist demo brain (9,233 dated entries from thoreau's and emerson's public-domain journals, embedded locally), served by a real anamnesis. the evaluation panel reads "measuring" because nobody has used that brain yet: those numbers only exist once a person does.</sub>

## what it does

- **one brain from many sources.** folders and obsidian vaults (wikilinks, embeds, frontmatter), pdfs, rss / atom / json feeds, web clips, json apis, an editor, chat, and anything else through a json-lines plugin protocol. incremental sync; renames and deletes are followed.
- **regions with memory.** hdbscan clusters matched run to run (jaccard, hungarian matching, monic transitions), so a region keeps its id and name; births, splits, merges and deaths are logged and replayable.
- **a stable 3d layout.** umap-style, warm-started and procrustes-aligned, so the brain does not reshuffle as it grows; new items are placed without moving the rest. three forms to morph between: anatomy, plain layout, spheres.
- **provenance everywhere.** every item says who wrote it (you, you in a chat, a model, a model editing you, someone else), where it came from and when. model text never passes for yours.
- **hybrid search** (bm25 + on-device embeddings, fused) with citation spans; **chat** that answers from your brain and cites it, or, with no model, lists what it holds.
- **threads:** decisions, open questions, plans and rejections found in your writing, with the verbatim quote, tracked to resolution.
- **sync across your devices,** through storage you manage (a folder your own cloud drive syncs, or your own s3-compatible bucket), end-to-end encrypted on the device; the event log is synced, concurrent edits are kept and you choose; a plain `inbox/` folder takes in markdown from anything. no account, no service of ours ([docs/sync.md](docs/sync.md)).
- **distil:** after a chat, a few atomic notes (idea, decision, open question, plan) proposed from your own messages, each quoting your exact words; keep or drop each with one tap. on a local model, or with no model at all (the rules); never a subscription.
- **review:** a few items a day worth seeing again, run as a micro-randomised trial against random picks.
- **an evaluation you can trust:** held-out controls, intervals, sample sizes, "measuring" until there is enough data, and an honesty table.
- **privacy by construction:** local by default, per-source privacy flags, an egress ledger of every remote call, an mcp surface with no delete and no fetch.
- **yours to restyle:** themes, colours, fonts, logo, motion and panel layout, all from settings.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme/growth-dark.svg">
  <img alt="the demo brain growing: each dot appears in the order it was written, from 1820 to 1876, above a timeline of writing per year, with a red ring where a region is born" src="assets/readme/growth-light.svg" width="100%">
</picture>

the brain keeps its history. every entry carries the date it was written, so the brain can be replayed in that order, and every region carries its lineage: when it was born, what it split from, what it merged into.

## quick start

anamnesis is a desktop app: it opens in its own window, and the whole backend (storage, embeddings, search, the brain) runs inside it, on your machine. nothing goes to a server unless you connect a model and ask it something.

**install** (no build needed): download the installer for your system from the [releases page](https://github.com/aphesist/anamnesis/releases).

| system | file | notes |
|---|---|---|
| windows 10 / 11 | `anamnesis-<version>-windows-x86_64-setup.exe` (or the `.msi`, or a portable `.zip`) | installs for you only, no admin. unsigned for now: smartscreen says "windows protected your pc" → **more info → run anyway**. uses webview2, which windows 10 and 11 already have. |
| macos | `anamnesis_<version>_aarch64.dmg` | unsigned for now: right-click the app → **open** the first time. |
| linux | `.AppImage` or `.deb` | needs webkitgtk 4.1 (present on current gnome/kde desktops). |

open **anamnesis** from the start menu, launchpad or app list. the first launch opens straight onto a **demo brain**, already built and embedded: 1,100 dated entries from the journals of thoreau and emerson (1820–1876, public domain in the us and brazil). turn it, open regions, search ("walden pond", "friendship"), read entries, ask the chat. then **file → import folder…** (or "import a folder…" on the banner) to bring your own notes or obsidian vault, or "start empty". the demo stays apart from your brain; "show a demo brain" in the view panel brings it back.

in the background, the first launch downloads the local embedding model (~465 MB, once). until it is ready, search matches words only; everything else works at once.

**build from source:** [rust](https://rustup.rs) 1.90+ and a c compiler (on windows, the visual studio build tools with "desktop development with c++"; on linux, `libwebkit2gtk-4.1-dev`).

```sh
git clone https://github.com/aphesist/anamnesis
cd anamnesis
cargo run --release -p anamnesis-desktop     # the app
cargo build --release -p anamnesis-server    # the headless `anamnesis` cli: serve, import, mcp, verify, eval
```

the headless cli is also installed next to the app, for the mcp server (`anamnesis mcp`) and scripting. details per os, gpu builds, data locations and troubleshooting: [docs/install.md](docs/install.md); a tour: [docs/using.md](docs/using.md); the demo brain: [docs/demo.md](docs/demo.md).

## bring your own model

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme/harness-dark.svg">
  <img alt="the routes to a model (local models, api keys, continue with chatgpt, your installed claude cli, marked grey) and the brain as an mcp server, with threads of stipple flowing into the brain" src="assets/readme/harness-light.svg" width="100%">
</picture>

anamnesis never ships or resells a model. it routes each task to a backend you configure, local first; background work never runs on a subscription, and nothing falls back silently to a paid backend. the terms column is each provider's own terms as of 2026-11-11, with dated quotes and links in [docs/models.md](docs/models.md#provider-terms).

| provider | route | in anamnesis | terms |
|---|---|---|---|
| **anthropic** | your claude code / claude desktop / claude.ai connector, over mcp | yes | allowed |
| | api key | yes | allowed |
| | your own installed `claude` cli, signed in by you | opt-in, chat only | **grey** |
| | collecting claude.ai tokens, in-app claude login, account rotation | no | not offered |
| **openai** | codex over mcp | yes | allowed |
| | api key (or any openai-compatible host, openrouter) | yes | allowed |
| | **Continue with ChatGPT** (openai's sign-in for open-source local apps) | yes, chat only, experimental | allowed (preview) |
| | reusing codex's stored login | no | not offered |
| **google** | gemini cli / antigravity over mcp | yes | allowed |
| | gemini api key (ai studio) | yes | allowed |
| | consumer-plan logins (gemini cli / antigravity oauth) | no | not offered: google's terms prohibit third-party use |
| **local** | ollama, llama.cpp, lm studio, vllm (auto-detected) | yes, the default | allowed |

the installed-cli route is off until you turn it on, and shows this notice wherever it appears:

> uses your own claude login through anthropic's official cli. anthropic has not explicitly approved third-party apps doing this and could restrict it at any time; the safest routes are the mcp server and an api key. anamnesis never reads your credentials.

the cli route also cleans its environment (no stray api key, endpoint or cloud-provider switch can move it off your login) and, by default, keeps your own claude code setup (`CLAUDE.md`, auto memory, hooks, skills, slash commands) out of anamnesis chats.

### optional, grey: label requests as anamnesis

off by default. when you turn it on (settings → connections → claude cli · relabel), requests from the installed claude cli are labelled `anamnesis` instead of the cli's own label, which changes how they are likely to be metered against your plan. other tools do the same (openbot; nous research's hermes plugin). anthropic has not documented the weighting or said whether third-party apps may do this, so it could be treated as getting around your plan's limits; it affects only your own account, and the project removes the option the moment anthropic says it is not allowed. you see this before it turns on:

> label requests as "anamnesis" instead of the cli's own label. claude code meters requests differently depending on how they say they were made; scripted (agent sdk) requests have been reported to count about twice as heavily against your plan limits. with this on, anamnesis's requests identify themselves as coming from anamnesis rather than as agent sdk calls, so they are likely to be metered like interactive use. anthropic has not documented this weighting or said whether third-party apps may do this. it is a grey area: it could be treated as getting around your plan's limits, and anthropic can restrict or suspend accounts without notice. it affects only your own account. anamnesis turns this off for everyone if anthropic says it is not allowed.

### what anamnesis will not do

- read your credentials (the claude cli's login, codex's or gemini's auth files, another app's keychain entries)
- copy tokens: the only token it holds is its own, from openai's sign-in
- rotate between accounts or fail over from one plan to another
- relabel traffic without asking: the claude cli's own request label is left alone unless you turn on the optional "label requests as anamnesis" switch (above), and anamnesis never poses as another client
- run background jobs (labelling, extraction, anything scheduled) on a subscription

## privacy

- **local first.** one sqlite file in your data directory; no account, no telemetry. the server binds to `127.0.0.1` and refuses non-loopback hosts. embeddings run on your machine.
- **egress ledger.** every call to a remote model passes an egress gate and is logged as an event: provider, model, purpose, item ids, character count. never the text. mcp reads are logged the same way.
- **privacy flags,** set per source: `cloud_ok`, `ask` (chat asks before sending) or `local_only` (never sent to a remote model, withheld from mcp clients).
- **mcp is read + append only.** no delete tool, no fetch or send tool; every write is attributed to the client, and generated text is labelled as generated.
- **keys in the keychain,** never in settings, the log or an export.

[docs/privacy.md](docs/privacy.md) says exactly what is and is not covered.

## the evaluation

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/readme/latent-dark.svg">
  <img alt="a note is written; one suggested link is shown, another is predicted but held out and never shown; later a hand-drawn link lands on the held-out prediction, marked latent" src="assets/readme/latent-light.svg" width="100%">
</picture>

built in, measured on your own use, from the event log ([docs/evaluation.md](docs/evaluation.md)):

- **latent links.** about 20% of link suggestions are held out at random and never shown. later hand-made links that land on held-out suggestions estimate how much of what you link was already predictable from what you had written (an inverse-propensity estimate), next to the shown arm, so the effect of suggesting is visible.
- **retrieval** on your own relevance labels: ndcg@10, p@5, mrr with bootstrap intervals; "measuring" below 25 labelled queries.
- **resurfacing** as a micro-randomised trial: each daily slot is the algorithm's pick or a random one, rated useful or not; bayesian intervals and a power-based threshold.
- **growth and lineage:** items, links and regions over time; births, splits, merges, deaths.
- every metric has a novelty split (first 28 days vs after), because new features get rated kindly. acceptance rate is shown, but labelled as not a measure of quality.

### the honesty table

| claim | status | basis |
|---|---|---|
| on beir scifact, ndcg@10 is 0.327 (hash), 0.659 (bm25), 0.682 (multilingual-e5-small, the default embedder) | **measured** | lab benchmark, 300 queries, 95% bootstrap intervals in [lab/README.md](lab/README.md) |
| on beir nfcorpus, ndcg@10 is 0.125 / 0.305 / 0.306 | **measured** | lab benchmark, 323 queries |
| layout trustworthiness ≈ 0.67 (umap, warm-started umap and pacmap; 2,000 demo notes, hash embeddings); warm-started umap roughly halves drift between growing snapshots | **measured** | lab benchmark (python reference methods; the rust layout is not yet in that harness) |
| regions over 20,000 × 384-d vectors take 0.1–0.16 s; a fresh layout 5.5–7.9 s | **measured** | `anamnesis-mind` benchmark on a loaded 4-core machine |
| search finds what you consider relevant | measured per user, in the app | your relevance labels; "measuring" below 25 queries |
| resurfaced items are more useful than a random pick | measured per user, in the app | micro-randomised trial; a hypothesis until each arm has ~95 ratings |
| a share of the links you make by hand was already latent in what you had written | **modelled**, per user, in the app | inverse-propensity estimate on held-out suggestions; assumes exposure does not change your unpredicted links; needs ≥ 50 hand links |
| link suggestions on the demo brain | demonstration | its links were added by a script; never reported as a metric |
| "it thinks like you" | **hypothesis** | not measured |
| learning is recollection | **hypothesis** | the philosophical frame: the number informs it, it cannot prove it |

## architecture

```mermaid
flowchart LR
  subgraph sources
    F[folders · vaults · pdfs<br/>feeds · web · json apis · plugins]
    E[editor · chat]
    M[mcp clients<br/>claude code · codex · gemini cli · desktop]
  end
  F --> C[anamnesis-connectors]
  C --> L[(anamnesis-core<br/>append-only, hash-chained event log<br/>+ sqlite projections · fts5 · vectors)]
  E --> L
  M <--> P[anamnesis-mcp] <--> L
  L --> EM[anamnesis-embed<br/>onnx · ollama · openai-compatible]
  EM --> L
  L --> MI[anamnesis-mind<br/>regions + lineage · layout · links · threads · review]
  MI --> L
  L --> EV[anamnesis-eval<br/>metrics from the log]
  L --> S[anamnesis-server<br/>http api · jobs · chat]
  S --> H[anamnesis-harness<br/>router · egress gate]
  H -->|only if you chose one| X[model provider]
  H --> L
  S --> U[ui/app + ui/theme<br/>webgl2 brain · panels]
  D[anamnesis-desktop<br/>native window · menus] --> S
  D --> U
```

every change is an event; items, links, threads, regions and suggestions are projections that replay from the log; embeddings and the layout are caches. more in [docs/architecture.md](docs/architecture.md).

## repository layout

| path | what |
|---|---|
| `crates/anamnesis-core` | data model, event log, sqlite storage, chunking, hybrid search, bundles |
| `crates/anamnesis-connectors` | built-in connectors and the plugin runtime ([PLUGINS.md](crates/anamnesis-connectors/PLUGINS.md)) |
| `crates/anamnesis-embed` | embedding backends |
| `crates/anamnesis-mind` | regions, lineage, layout, link suggestion, threads, distil, resurfacing |
| `crates/anamnesis-sync` | sync across devices: the folder format, encryption, clocks, merging, the inbox |
| `crates/anamnesis-eval` | the built-in evaluation ([README](crates/anamnesis-eval/README.md)) |
| `crates/anamnesis-harness` | model backends, routing, limits, usage, egress gate |
| `crates/anamnesis-mcp` | the mcp server |
| `crates/anamnesis-server` | the backend as a library, and the headless `anamnesis` cli |
| `crates/anamnesis-desktop` | the desktop app (tauri 2): a native window with the backend in-process |
| `ui/app` | the interface (plain es modules, webgl2, no build step), shown in the app's window |
| `ui/theme` | tokens, themes, fonts, settings, layout config ([README](ui/theme/README.md)) |
| `demo/` | the demo brain: 1,100 curated public-domain entries and a prebuilt, embedded snapshot shipped inside the app |
| `packaging/` | installers (windows nsis / msi / zip cross-build) |
| `lab` branch | optional python lab: demo-brain builders, benchmarks and the generators of the readme pictures ([README](https://github.com/aphesist/anamnesis/tree/lab/lab/README.md)) |
| `site` branch | the landing page at [anamnesis.aphes.ist](https://anamnesis.aphes.ist) |
| `docs/` | [install](docs/install.md) · [using](docs/using.md) · [models](docs/models.md) · [mcp](docs/mcp.md) · [evaluation](docs/evaluation.md) · [privacy](docs/privacy.md) · [sync](docs/sync.md) · [connectors](docs/connectors.md) · [theming](docs/theming.md) · [architecture](docs/architecture.md) · [demo](docs/demo.md) · [faq](docs/faq.md) · [http api](docs/contracts.md) |
| `assets/readme/` | the animated pictures in this readme, drawn from the demo brain's real layout and dates (their generators are on the `lab` branch) |

contributing: [CONTRIBUTING.md](CONTRIBUTING.md). security: [SECURITY.md](SECURITY.md).

## licence

apache-2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). the bundled Special Elite font is apache-2.0, © 2010 Astigmatic.

the default theme ships the aphes.ist identity assets under their own terms: the Aphesis Script typeface under the [SIL Open Font License 1.1](ui/theme/themes/aphesis/assets/OFL.txt) (reserved font name "Aphesis Script"), and the φ mark and wordmark as trademarks of their owner, not licensed for use by forks. forks should set their own mark (the theme falls back to a placeholder φ without it). see [docs/theming.md](docs/theming.md#the-aphesis-theme-and-its-assets). the pictures in `assets/readme/` embed subsets of both fonts and the φ under the same terms.
