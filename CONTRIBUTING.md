# contributing

thank you for wanting to help. anamnesis is a local-first second brain: a rust server, a plain-javascript ui and an optional python lab. this page says how to build it, test it and send a change.

## build and test

needs rust (stable, 1.90 or newer), node 18+ for the ui tests, and [uv](https://docs.astral.sh/uv/) for the lab.

```sh
cargo build -p anamnesis-server            # the `anamnesis` binary (debug)
cargo test --workspace                     # every crate; offline
cargo test -p anamnesis-mind               # one crate
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
node --test ui/app/tests/*.test.mjs
node --test ui/theme/test/*.test.mjs
node ui/theme/scripts/build-tokens.mjs --check
(cd lab && uv run pytest)
```

the default build includes on-device embeddings (`local` feature of `anamnesis-embed`): building it downloads the onnx runtime library once. to build without it:

```sh
cargo build -p anamnesis-server --no-default-features --features embed-ui
```

ci runs exactly the commands above (`.github/workflows/ci.yml`).

## ground rules for code

- **tests are offline and fast.** no network and no model downloads in tests. anything that needs a network goes behind `#[ignore]` or a feature flag. `--hash-embedder` and `EmbedderSpec::hash()` exist for this.
- **everything that changes the brain is an event.** add an `EventKind` rather than writing a projection table directly; projections must stay a pure function of the log (`Store::rebuild_projections` replays it in tests).
- **provenance is not optional.** every new item needs an honest `Origin`. text a model wrote is `ai_generated` or `ai_edited`, never `owner_*`.
- **nothing leaves the machine silently.** a new outbound model call goes through the harness router and the egress gate, and records an `Egress` event (counts and ids, never text).
- **no subscription shortcuts.** do not add code that reads another program's credentials, copies tokens, rotates accounts, sets attribution variables, or runs background work on a consumer plan. see [docs/models.md](docs/models.md).
- **no python at runtime.** the lab is for benchmarks and demo data only.
- the shared data model is `crates/anamnesis-core/src/model.rs`. changing it is a cross-crate change; say so in the pull request.
- the http api is documented in [docs/contracts.md](docs/contracts.md). change both together, and keep `ui/app/mock/` fixtures in the same shapes.
- dependencies: well maintained, pure rust or bundled builds where possible (e.g. `rusqlite` with `bundled`).

## ui and voice

ui copy is lowercase and calm. the one exception is openai's required button label, "Continue with ChatGPT". the ui reads only the theme tokens listed in [docs/contracts.md](docs/contracts.md); new colours go through tokens, not literals. respect `--motion: 0`.

## commits and pull requests

- small, logical commits; the message says what changed and why, in english.
- one topic per pull request. describe how you tested it.
- by contributing you agree that your contribution is licensed under the apache license 2.0, like the rest of the project.
- do not commit secrets, personal notes, model weights, demo corpora or other large binaries. the aphesis theme assets (see `ui/theme/themes/aphesis/assets/NOTICE.md`) must never be committed.

## security

report vulnerabilities privately; see [SECURITY.md](SECURITY.md).
