# install

anamnesis is one rust binary, `anamnesis`, that serves the ui, the http api and an mcp server on your own machine. there are no prebuilt packages yet: build it from source.

## what you need

- **rust**, stable, 1.90 or newer ([rustup](https://rustup.rs)).
- a c toolchain (for the bundled sqlite) and, on linux, the usual build packages (`build-essential`, `pkg-config`, `libssl-dev` on debian and ubuntu).
- network access during the first build: with the default features the `ort` crate downloads the onnx runtime library for on-device embeddings.
- optional: [node](https://nodejs.org) 18+ to run the ui tests or rebuild theme tokens; [uv](https://docs.astral.sh/uv/) to build the demo brain or run the benchmarks in `lab/`.

## build

```sh
git clone https://github.com/aphesimos/anamnesis
cd anamnesis
cargo build --release -p anamnesis-server
./target/release/anamnesis --help
```

the release binary embeds the ui (`ui/app` and `ui/theme`), so it can be copied anywhere on its own. put it on your `PATH` if you like:

```sh
install -m 755 target/release/anamnesis ~/.local/bin/
```

### features

| feature (crate `anamnesis-server`) | default | what |
|---|---|---|
| `embed-ui` | on | embed `ui/` in the release binary. debug builds read `ui/` from the repository. |
| `local-embed` | on | on-device embeddings with onnx runtime (fastembed). without it the default embedder is the hashing embedder, and you can point anamnesis at ollama or any openai-compatible embeddings endpoint instead. |

```sh
cargo build --release -p anamnesis-server --no-default-features --features embed-ui
```

## first run

```sh
anamnesis serve --open
```

this listens on `http://127.0.0.1:7368/` (only on loopback) and opens the browser. on first launch the ui asks whether to keep the full motion or calm it; you can change that later in settings.

the first time the local embedder runs it downloads its model (`intfloat/multilingual-e5-small`, about 465 MB) into the model cache. until it has loaded, search is full-text only. to skip the download entirely, run with `--hash-embedder` (a deterministic, offline embedder: fine for trying things out, much weaker for search).

## where things live

| what | default location | change with |
|---|---|---|
| your brain | `<data dir>/anamnesis/` | `--data-dir <dir>` or `ANAMNESIS_DATA_DIR` (the brain then lives in `<dir>/anamnesis/`) |
| the demo brain | `<data dir>/anamnesis/demo/` | same |
| embedding models | `<data dir>/anamnesis/models/` | the embedder setting |
| api keys, the chatgpt token | the os keychain, service `anamnesis` | |

`<data dir>` is `~/.local/share` on linux, `~/Library/Application Support` on macos and `%APPDATA%` on windows.

a brain directory holds `brain.sqlite` (the event log and everything derived from it), a few json caches for the background jobs (`mind_state.json`, `views.json`, `jobs.json`) and `mcp_token` (mode 0600), the bearer token for the http mcp endpoint.

## the command line

| command | does |
|---|---|
| `anamnesis serve [--port 7368] [--open] [--demo] [--demo-bundle <path>] [--no-detect] [--ui-dir <dir>]` | the ui, the http api and mcp at `/mcp` |
| `anamnesis import <folder or bundle>` | add a folder (an obsidian vault if it has `.obsidian/`) as a source and sync it once, or import an exported bundle into an empty brain; then run the background jobs once |
| `anamnesis add-source <connector> [--name <n>] --config '<json>'` | add any built-in connector as a source and sync it, e.g. `add-source rss --config '{"url":"https://example.org/feed.xml"}'` |
| `anamnesis sync` | sync every source once and run the jobs once |
| `anamnesis export <file>` | write the whole brain as a bundle (json lines) |
| `anamnesis verify` | check the hash chain of the event log |
| `anamnesis eval [--json]` | print the honesty table of the built-in evaluation (or the full report) |
| `anamnesis mcp` | serve mcp over stdio, for harnesses that start it themselves |

every command takes `--data-dir <dir>` and `--hash-embedder`. `serve` also reads `ANAMNESIS_PORT`; logging follows `ANAMNESIS_LOG` (e.g. `ANAMNESIS_LOG=debug`). `--no-detect` stops `serve` from probing local model ports and running `claude --version` at startup.

only one process should open a brain at a time: stop `anamnesis serve` before running `import`, `sync` or `export` against the same data directory.

## updating

pull and rebuild. the database schema migrates itself on open; your data is the event log, and every table besides it can be rebuilt from it. export a bundle first if you want a copy: `anamnesis export ~/anamnesis-backup.jsonl`.

## uninstalling

delete the binary and the data directory. api keys stay in the os keychain under the service name `anamnesis` until you remove them there (or with `DELETE /api/backends/<id>/key` before uninstalling). if you used "Continue with ChatGPT", also remove anamnesis in chatgpt's settings: signing out locally does not revoke it.
