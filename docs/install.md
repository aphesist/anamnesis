# install

anamnesis is a desktop application: it opens in its own window on windows, macos and linux, with the brain running inside it. nothing goes through a web browser, and nothing leaves your machine unless a model you choose needs the network. the same brain also comes as a command line program, `anamnesis`, for mcp clients and headless use (a server, a terminal); the app installs it beside itself.

tagged releases attach installers for every platform (see [prebuilt packages](#prebuilt-packages)); otherwise build from source, which takes a few minutes.

<a id="download-the-installer-no-build-needed"></a>

## install the app

| os | file | then |
|---|---|---|
| windows 10 / 11 (x86_64) | `anamnesis-<version>-windows-x86_64-setup.exe` | run it; no administrator rights needed. start **anamnesis** from the start menu. |
| macos 11+ (apple silicon) | `anamnesis-<version>-macos-arm64.dmg` | open it and drag **anamnesis** into applications. intel macs: `-macos-x86_64.dmg`. |
| linux (debian, ubuntu 22.04+) | `anamnesis-<version>-linux-x86_64.deb` | `sudo apt install ./anamnesis-<version>-linux-x86_64.deb`, then start **anamnesis** from your applications. |
| linux (any distribution) | `anamnesis-<version>-linux-x86_64.AppImage` | `chmod +x` it and run it. |

the first start downloads the local embedding model (about 465 MB) in the background; until it has loaded, search is full-text only. on first launch the app asks whether to keep the full motion or calm it.

the window has a menu: **file** (import a folder or an obsidian vault with a folder picker, open the data folder, quit), **view** (reload, full screen, zoom) and **help** (about, the docs). links to other sites open in your browser, never in the app. starting the app again while it runs brings its window to the front. closing the window stops the background jobs and saves the brain.

### windows: webview2, smartscreen

- **webview2.** the window is drawn by microsoft edge webview2, which windows 11 and up-to-date windows 10 already have. if it is missing, the installer offers microsoft's evergreen installer (or download it from [microsoft](https://go.microsoft.com/fwlink/p/?LinkId=2124703)); the app says the same if started without it. the command line `anamnesis.exe` does not need it.
- **smartscreen.** the installers are not code-signed yet, so smartscreen says "windows protected your pc": choose **more info → run anyway**. a browser may also ask whether to keep the download.
- **the msi** (`anamnesis-<version>-windows-x86_64.msi`) installs for all users of the pc and needs administrator rights; the setup.exe installs for your account only.
- **where it goes.** the release setup.exe installs the app (`anamnesis-desktop.exe`, with `anamnesis.exe` beside it) into `%LOCALAPPDATA%\anamnesis\`, next to the `models\` folder of the embedder; the msi into `C:\Program Files\anamnesis\`. uninstalling (settings → apps) removes only the program; your brain stays in `%APPDATA%\anamnesis\`.

### macos: gatekeeper

the app is not signed or notarised yet. the first time, macos says it "cannot be opened because the developer cannot be verified": open **system settings → privacy & security** and choose **open anyway** (or run `xattr -dr com.apple.quarantine /Applications/anamnesis.app`). api keys go to the login keychain; the first time, macos may ask whether anamnesis may use it: choose "always allow".

### linux

the .deb pulls in webkitgtk (`libwebkit2gtk-4.1-0`) and gtk 3. the AppImage carries its own libraries. both install the command line `anamnesis` too (the .deb in `/usr/bin`).

## where your data lives

the app and the command line share one brain:

| what | windows | macos | linux |
|---|---|---|---|
| your brain | `%APPDATA%\anamnesis\` | `~/Library/Application Support/anamnesis/` | `~/.local/share/anamnesis/` |
| embedding models | `%LOCALAPPDATA%\anamnesis\models\` | `~/Library/Application Support/anamnesis/models/` | `~/.local/share/anamnesis/models/` |
| the app's log | `<brain>\logs\anamnesis-desktop.log` | `<brain>/logs/anamnesis-desktop.log` | `<brain>/logs/anamnesis-desktop.log` |
| window position, webview cache | `%APPDATA%\ist.aphes.anamnesis\`, `%LOCALAPPDATA%\ist.aphes.anamnesis\` | `~/Library/Application Support/ist.aphes.anamnesis/`, `~/Library/WebKit/` | `~/.local/share/ist.aphes.anamnesis/` |
| api keys | windows credential manager | login keychain | secret service, else a 0600 file in the brain |

**file → open data folder** opens the brain's folder. `ANAMNESIS_DATA_DIR` moves it, for the app as for the command line (the brain then lives in `<dir>/anamnesis/`).

## mcp and the command line next to the app

while the app is open, its server listens on `127.0.0.1:7368` (loopback only), so mcp clients can use the brain over http at `http://127.0.0.1:7368/mcp`; settings → connections in the app show the exact snippet with its token. if port 7368 is taken, the app picks a free port, and the snippets show that one.

the command line is installed beside the app: `anamnesis.exe` in the app's folder on windows, `/Applications/anamnesis.app/Contents/MacOS/anamnesis` on macos, `/usr/bin/anamnesis` from the .deb. give mcp clients that full path (the cross-built windows setup.exe can also add its folder to your `PATH`). `anamnesis mcp` serves mcp over stdio for harnesses that start it themselves; `anamnesis serve --open` (or `anamnesis` with no command) is the old way, serving the ui to your browser, for headless machines. see [the command line](#the-command-line).

## quick start from source

```sh
# needs rust (https://rustup.rs) and a c compiler: on windows the visual studio build tools "desktop development with c++"
# on linux also: sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
git clone https://github.com/aphesist/anamnesis
cd anamnesis
cargo run --release -p anamnesis-desktop      # the app
cargo build --release -p anamnesis-server     # the command line: target/release/anamnesis
```

installers are built with the tauri bundler: `cargo install tauri-cli --version "^2"`, then `cargo tauri build` in `crates/anamnesis-desktop` (see `.github/workflows/release.yml` for the exact steps, including the command line as a sidecar).

## windows

tested targets: windows 10 (1903 or newer) and windows 11, x86_64.

### the cross-built packages

`packaging/windows/build.sh` builds the windows packages on linux with mingw-w64, nsis and wixl, without any microsoft sdk: `anamnesis-<version>-windows-x86_64-setup.exe` (per user, no administrator rights, optional desktop shortcut and `PATH` entry), a per-user `.msi` and a `-portable.zip`. each holds the app (`anamnesis-desktop.exe`, started by the start-menu shortcut), the command line `anamnesis.exe`, microsoft's `onnxruntime.dll` and the licences. see [packaging/windows/](../packaging/windows/).

where things go:

| what | location |
|---|---|
| the program | `%LOCALAPPDATA%\Programs\anamnesis\` (`anamnesis-desktop.exe`, `anamnesis.exe`, licences) |
| start-menu entry | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\anamnesis.lnk` |
| your brain | `%APPDATA%\anamnesis\` |
| embedding models | `%LOCALAPPDATA%\anamnesis\models\` (about 465 mb, downloaded on first run) |

**on-device embeddings** in the cross-built packages use microsoft's `onnxruntime.dll`, installed next to the programs. it needs the [microsoft visual c++ redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe) (x64), which most pcs already have; the installer checks and offers the download page if it is missing. without it anamnesis still runs, with the offline hash embedder, and settings say why. gpu embeddings (directml, cuda) are not in the packages: build from source with the features below for those.

**uninstall** from settings → apps → installed apps → anamnesis (or control panel → programs and features). this removes the program, the shortcuts and the `PATH` entry. your brain (`%APPDATA%\anamnesis\`) and the downloaded models (`%LOCALAPPDATA%\anamnesis\`) stay, unless you tick "also delete my brain and downloaded models" in the setup.exe uninstaller (the msi never deletes them). api keys stay in the credential manager (see [uninstalling](#uninstalling)).

### build from source

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

## prebuilt packages

pushing a `v*` tag runs `.github/workflows/release.yml`, which builds everything natively on github's windows, macos and linux runners and attaches it to the github release:

| file | contents |
|---|---|
| `anamnesis-<version>-windows-x86_64-setup.exe` | the app, per-user installer (nsis; no administrator rights; checks for webview2) |
| `anamnesis-<version>-windows-x86_64.msi` | the app, for all users (needs administrator rights) |
| `anamnesis-<version>-macos-arm64.dmg` | the app for apple silicon (on-device embeddings) |
| `anamnesis-<version>-macos-x86_64.dmg` | the app for intel macs (no on-device embeddings: use ollama or an api embedder) |
| `anamnesis-<version>-linux-x86_64.deb`, `.AppImage` | the app (on-device embeddings; glibc 2.35 or newer) |
| `anamnesis-<version>-<os>.zip` / `.tar.gz` | the command line `anamnesis` alone (ui embedded), for servers and mcp |

every app package also installs the command line `anamnesis` beside the app. each file has a `.sha256` file next to it. nothing is code-signed yet (see [smartscreen](#windows-webview2-smartscreen) and [gatekeeper](#macos-gatekeeper)); for a downloaded command line binary on macos run `xattr -d com.apple.quarantine anamnesis`. if windows says `VCRUNTIME140.dll` is missing, install the [microsoft visual c++ redistributable](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist) (x64).

## first run of the command line

the app needs no command. for the command line on a headless machine:

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
| `local-embed-dynamic` | off | the same embeddings, with onnx runtime loaded at run time from `onnxruntime.dll` (`libonnxruntime.so`, `.dylib`) next to the binary, or from `ORT_DYLIB_PATH`. used by the windows packages; build with `--no-default-features --features embed-ui,embed-demo,local-embed-dynamic`. |
| `embed-demo` | on | embed the curated demo notes so a lone binary can show the demo brain. |
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
- **the app window stays on "waking the brain"** or shows an error: the reason is in the app's log (`<brain>/logs/anamnesis-desktop.log`, see [where your data lives](#where-your-data-lives)).
- **port 7368 is in use** (`address already in use` / `os error 10048`) for `anamnesis serve`: another `anamnesis serve`, or the app, is probably running (the app itself moves to a free port). stop it, or pick another port with `--port 7400` or `ANAMNESIS_PORT=7400`.
- **the build fails in `ort-sys` with a download error.** the onnx runtime library is downloaded from `cdn.pyke.io` during the first build. behind a proxy, set `HTTPS_PROXY` (powershell: `$env:HTTPS_PROXY = "http://proxy:8080"`) before `cargo build`. without network access to it, build without on-device embeddings (`--no-default-features --features embed-ui`) and use ollama for embeddings.
- **the model download stalls** (first run, behind a proxy or offline): it comes from hugging face. set `HTTPS_PROXY` before `anamnesis serve`, or run with `--hash-embedder` meanwhile.
- **`link.exe` not found or `cannot open input file 'kernel32.lib'`**: the visual studio build tools are missing the c++ workload or the windows sdk. rerun the installer and tick **desktop development with c++**.
- **`claude` is not detected** although it is installed: it is looked up on the server's `PATH` (then `%USERPROFILE%\.local\bin` and `%APPDATA%\npm` on windows). start `anamnesis serve` from a new terminal after installing it.
- **linux: keys disappear after a reboot** with an older build: that was the kernel keyring. current builds use the secret service or the file; enter the key again once.

## updating

pull and rebuild. the database schema migrates itself on open; your data is the event log, and every table besides it can be rebuilt from it. export a bundle first if you want a copy: `anamnesis export ~/anamnesis-backup.jsonl`.

## uninstalling

delete the binary and the data directory (and, on windows, `%LOCALAPPDATA%\anamnesis\` with the models). api keys stay in the os credential store under the service name `anamnesis` until you remove them there (or with `DELETE /api/backends/<id>/key` before uninstalling). if you used "Continue with ChatGPT", also remove anamnesis in chatgpt's settings: signing out locally does not revoke it.
