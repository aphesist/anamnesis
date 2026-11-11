# install

anamnesis is one rust binary, `anamnesis`, that serves the ui, the http api and an mcp server on your own machine. it runs on windows, macos and linux. tagged releases attach prebuilt archives (see [prebuilt binaries](#prebuilt-binaries)); otherwise build it from source, which takes a few minutes.

## quick start

```sh
# needs rust (https://rustup.rs) and a c compiler: on windows the visual studio build tools "desktop development with c++"
git clone https://github.com/aphesist/anamnesis
cd anamnesis
cargo build --release -p anamnesis-server
# windows:        target\release\anamnesis.exe serve --open
# macos / linux:  ./target/release/anamnesis serve --open
```

the ui opens at `http://127.0.0.1:7368/`. the first run downloads the local embedding model (about 465 MB) in the background.

## windows

tested targets: windows 10 (1903 or newer) and windows 11, x86_64.

1. **install the c++ build tools.** download the [visual studio build tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and, in the installer, tick the workload **desktop development with c++** (it brings the msvc compiler and the windows sdk). a full visual studio with that workload works too.
2. **install rust.** run [rustup-init.exe](https://rustup.rs) and accept the defaults (the `x86_64-pc-windows-msvc` toolchain). open a new terminal afterwards so `cargo` is on your `PATH`, then check with `cargo --version` (1.90 or newer).
3. **install git** if you do not have it: [git for windows](https://git-scm.com/download/win).
4. **clone and build** (powershell or cmd, any drive, e.g. `X:\code`):

   ```powershell
   git clone -c core.autocrlf=false https://github.com/aphesist/anamnesis
   cd anamnesis
   cargo build --release -p anamnesis-server
   ```

   `core.autocrlf=false` keeps the files byte-for-byte as committed (some tests compare docs verbatim); the build itself works either way. the first build downloads crates and the onnx runtime library and takes several minutes.
5. **run it:**

   ```powershell
   target\release\anamnesis.exe serve --open
   ```

   the browser opens `http://127.0.0.1:7368/`. stop the server with `ctrl+c`. the `.exe` embeds the ui, so you can copy it anywhere, e.g. to `%LOCALAPPDATA%\Programs\anamnesis\`, and add that folder to your user `PATH` (settings → system → about → advanced system settings → environment variables).

where things live on windows:

| what | location |
|---|---|
| your brain | `%APPDATA%\anamnesis\` (e.g. `C:\Users\you\AppData\Roaming\anamnesis\`) |
| embedding models | `%LOCALAPPDATA%\anamnesis\models\` (kept out of the roaming profile) |
| api keys | windows credential manager (control panel → credential manager → windows credentials, entries named `anamnesis`) |
| the chatgpt token | the credential manager when it fits (it caps a secret at 2560 bytes), otherwise `%APPDATA%\anamnesis\chatgpt-auth.json` |

files anamnesis keeps private (the fallback `secrets.json`, `mcp_token`, `chatgpt-auth.json`) get mode 0600 on macos and linux. windows has no such mode: there they rely on the access control list of your profile folder, which only your account, system and administrators can read. keep the data directory inside your profile (the default) unless you set equivalent permissions on the other folder.

long paths (deeper than 260 characters, common in obsidian vaults) work: rust's standard library uses extended-length paths internally, so no registry change is needed.

## macos

1. install the command line tools: `xcode-select --install`.
2. install rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.
3. build and run:

   ```sh
   git clone https://github.com/aphesist/anamnesis
   cd anamnesis
   cargo build --release -p anamnesis-server
   ./target/release/anamnesis serve --open
   ```

on apple silicon (arm64) the default build includes on-device embeddings. on intel macs onnx runtime publishes no prebuilt library for rust, so build without them and use ollama or an api embedder (see [features](#features)):

```sh
cargo build --release -p anamnesis-server --no-default-features --features embed-ui
```

api keys go to the login keychain (keychain access → `anamnesis`). the first time, macos may ask whether `anamnesis` may use the keychain; choose "always allow".

## linux

1. install a c toolchain: `sudo apt install build-essential pkg-config` (debian, ubuntu), `sudo dnf install gcc make` (fedora), or your distribution's equivalent.
2. install rust with rustup as on macos.
3. build and run as on macos. to put the binary on your `PATH`: `install -m 755 target/release/anamnesis ~/.local/bin/`.

api keys go to the secret service (gnome keyring, kwallet, keepassxc) over the session d-bus when one answers. on a machine without one (a server, a container, a minimal desktop) they go to `~/.local/share/anamnesis/secrets.json`, mode 0600. the linux kernel keyring is never used: it is cleared on reboot.

## prebuilt binaries

pushing a `v*` tag runs `.github/workflows/release.yml`, which attaches these to the github release:

| archive | contents |
|---|---|
| `anamnesis-<version>-windows-x86_64.zip` | `anamnesis.exe` (ui embedded, on-device embeddings) |
| `anamnesis-<version>-macos-arm64.tar.gz` | `anamnesis` (ui embedded, on-device embeddings) |
| `anamnesis-<version>-macos-x86_64.tar.gz` | `anamnesis` (ui embedded, no on-device embeddings: use ollama or an api embedder) |
| `anamnesis-<version>-linux-x86_64.tar.gz` | `anamnesis` (ui embedded, on-device embeddings; glibc 2.35 or newer) |

each archive has a `.sha256` file next to it. the binaries are not code-signed: windows smartscreen will say "windows protected your pc" (choose more info → run anyway), and macos gatekeeper blocks a downloaded binary until you run `xattr -d com.apple.quarantine anamnesis` or allow it in system settings → privacy & security. if windows says `VCRUNTIME140.dll` is missing, install the [microsoft visual c++ redistributable](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist) (x64).

## first run

```sh
anamnesis serve --open
```

this listens on `http://127.0.0.1:7368/` (only on loopback) and opens the browser. on first launch the ui asks whether to keep the full motion or calm it; you can change that later in settings.

the first time the local embedder runs it downloads its model (`intfloat/multilingual-e5-small`, about 465 MB) into the model cache. until it has loaded, search is full-text only. to skip the download entirely, run with `--hash-embedder` (a deterministic, offline embedder: fine for trying things out, much weaker for search). try the demo brain with `anamnesis serve --demo --open`.

### local chat with ollama (optional)

anamnesis needs no model to collect, link and search your notes. for chat, summaries and labels with nothing leaving the machine, install [ollama](https://ollama.com/download) (windows, macos, linux), pull a model and restart `anamnesis serve`; it detects ollama on `127.0.0.1:11434`:

```sh
ollama pull qwen3:8b
```

an rtx 4070 (12 GB) runs 7–14b models comfortably in ollama. see [models.md](models.md) for api keys, "continue with chatgpt" and your installed `claude` cli.

## gpu embeddings (optional)

the on-device embedder runs on the cpu by default, which is fine for a few thousand notes. two opt-in cargo features run it on the gpu instead; both fall back to the cpu if the gpu cannot be used, and `ANAMNESIS_GPU=0` turns them off at run time.

| feature | platforms | needs |
|---|---|---|
| `gpu-directml` | windows | any directx 12 gpu (nvidia, amd, intel) and an up-to-date driver. nothing else to install. the simplest choice on windows. |
| `gpu-cuda` | windows, linux (x86_64) | an nvidia gpu, the cuda 13 runtime and cudnn 9 on the `PATH` (windows) or library path (linux). the cuda provider libraries are copied next to the built binary in `target\release\`; keep them beside `anamnesis.exe` if you move it. |

```powershell
cargo build --release -p anamnesis-server --features gpu-directml
```

with `ANAMNESIS_LOG=info` the log says when it falls back to the cpu. the default builds never depend on cuda or directml features.

## connecting claude code (mcp) on windows

the brain is an mcp server; any harness can use it (details in [mcp.md](mcp.md)). on windows, give the full path to the `.exe`, with forward slashes or doubled backslashes in json:

```powershell
claude mcp add --transport stdio --scope user anamnesis -- "X:/code/anamnesis/target/release/anamnesis.exe" mcp
```

or in a project's `.mcp.json`:

```json
{
  "mcpServers": {
    "anamnesis": { "type": "stdio", "command": "X:/code/anamnesis/target/release/anamnesis.exe", "args": ["mcp"] }
  }
}
```

`anamnesis mcp` opens the brain itself, so stop `anamnesis serve` first, or use the http transport while the server runs (`http://127.0.0.1:7368/mcp`; the ui's settings show the exact snippet with its token).

the "installed cli (claude)" chat backend finds `claude.exe` (the native installer, `%USERPROFILE%\.local\bin`) or the npm shim `claude.cmd` (`%APPDATA%\npm`), which it runs as `node …\cli.js` directly, without `cmd.exe`.

## features

| feature (crate `anamnesis-server`) | default | what |
|---|---|---|
| `embed-ui` | on | embed `ui/` in the release binary. debug builds read `ui/` from the repository. |
| `local-embed` | on | on-device embeddings with onnx runtime (fastembed). without it the default embedder is the hashing embedder, and you can point anamnesis at ollama or any openai-compatible embeddings endpoint instead. |
| `gpu-directml` | off | local embeddings on a directx 12 gpu (windows). |
| `gpu-cuda` | off | local embeddings with cuda (nvidia, windows and linux). |

```sh
cargo build --release -p anamnesis-server --no-default-features --features embed-ui
```

optional tools: [node](https://nodejs.org) 18+ to run the ui tests or rebuild theme tokens; [uv](https://docs.astral.sh/uv/) to build the demo brain or run the benchmarks in `lab/`.

## where things live

| what | default location | change with |
|---|---|---|
| your brain | `<data dir>/anamnesis/` | `--data-dir <dir>` or `ANAMNESIS_DATA_DIR` (the brain then lives in `<dir>/anamnesis/`) |
| the demo brain | `<data dir>/anamnesis/demo/` | same |
| embedding models | `<local data dir>/anamnesis/models/` | the embedder setting |
| api keys, the chatgpt token | the os credential store, service `anamnesis`; else a private file in the brain directory | `ANAMNESIS_KEY_STORAGE` |

`<data dir>` is `~/.local/share` on linux, `~/Library/Application Support` on macos and `%APPDATA%` on windows. `<local data dir>` is the same on linux and macos and `%LOCALAPPDATA%` on windows.

a brain directory holds `brain.sqlite` (the event log and everything derived from it), a few json caches for the background jobs (`mind_state.json`, `views.json`, `jobs.json`) and `mcp_token`, the bearer token for the http mcp endpoint.

### where api keys go

| os | store |
|---|---|
| windows | windows credential manager |
| macos | login keychain |
| linux | secret service over d-bus, when one answers within a few seconds; otherwise `secrets.json` (mode 0600) in the brain directory |

`ANAMNESIS_KEY_STORAGE=file` always uses the file; `ANAMNESIS_KEY_STORAGE=keychain` always uses the os store and reports an error instead of falling back. `GET /api/backends` shows the active store as `key_store` (`storage`, `backend`, `file`) and, per backend, where its key was found (`key_storage`: `keychain`, `file` or `env`). the provider's usual environment variable (`ANTHROPIC_API_KEY`, …) is read last.

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

in powershell, quote json with single quotes as above; in `cmd.exe`, use double quotes and escape the inner ones: `--config "{\"url\":\"https://example.org/feed.xml\"}"`. folder paths may use either slash: `anamnesis import X:\notes\vault`.

only one process should open a brain at a time: stop `anamnesis serve` before running `import`, `sync` or `export` against the same data directory.

## troubleshooting

- **windows defender or another antivirus slows the build or quarantines the binary.** the first build writes thousands of small files under `target\`; excluding the repository folder from real-time scanning speeds it up a lot. a freshly built, unsigned `anamnesis.exe` is occasionally flagged; restore it and add an exclusion.
- **smartscreen: "windows protected your pc"** for a downloaded release: more info → run anyway. a binary you built yourself is not affected.
- **the firewall asks about network access.** anamnesis listens on `127.0.0.1` only; you can deny access for public and private networks and it keeps working.
- **port 7368 is in use** (`address already in use` / `os error 10048`): another `anamnesis serve` is probably running. stop it, or pick another port with `--port 7400` or `ANAMNESIS_PORT=7400`.
- **the build fails in `ort-sys` with a download error.** the onnx runtime library is downloaded from `cdn.pyke.io` during the first build. behind a proxy, set `HTTPS_PROXY` (powershell: `$env:HTTPS_PROXY = "http://proxy:8080"`) before `cargo build`. without network access to it, build without on-device embeddings (`--no-default-features --features embed-ui`) and use ollama for embeddings.
- **the model download stalls** (first run, behind a proxy or offline): it comes from hugging face. set `HTTPS_PROXY` before `anamnesis serve`, or run with `--hash-embedder` meanwhile.
- **`link.exe` not found or `cannot open input file 'kernel32.lib'`**: the visual studio build tools are missing the c++ workload or the windows sdk. rerun the installer and tick **desktop development with c++**.
- **`claude` is not detected** although it is installed: it is looked up on the server's `PATH` (then `%USERPROFILE%\.local\bin` and `%APPDATA%\npm` on windows). start `anamnesis serve` from a new terminal after installing it.
- **linux: keys disappear after a reboot** with an older build: that was the kernel keyring. current builds use the secret service or the file; enter the key again once.

## updating

pull and rebuild. the database schema migrates itself on open; your data is the event log, and every table besides it can be rebuilt from it. export a bundle first if you want a copy: `anamnesis export ~/anamnesis-backup.jsonl`.

## uninstalling

delete the binary and the data directory (and, on windows, `%LOCALAPPDATA%\anamnesis\` with the models). api keys stay in the os credential store under the service name `anamnesis` until you remove them there (or with `DELETE /api/backends/<id>/key` before uninstalling). if you used "Continue with ChatGPT", also remove anamnesis in chatgpt's settings: signing out locally does not revoke it.
