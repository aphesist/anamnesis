//! The command line.

use std::path::PathBuf;
use std::sync::Arc;

use anamnesis_embed::EmbedderSpec;
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::app::{App, ServerConfig, default_root};

#[derive(Debug, Parser)]
#[command(
    name = "anamnesis",
    version,
    about = "a local-first second brain",
    long_about = None,
    after_help = "without a command, anamnesis serves on 127.0.0.1:7368 and opens the browser \
                  (like `serve --open`), so a double-click or a start-menu shortcut works."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    /// True when no subcommand was given (a double-click, a start-menu shortcut).
    pub fn is_bare(&self) -> bool {
        self.command.is_none()
    }
}

/// What a bare `anamnesis` runs: `serve --open` with the defaults (`ANAMNESIS_PORT` and
/// `ANAMNESIS_DATA_DIR` still apply).
fn default_command() -> Command {
    match Cli::try_parse_from(["anamnesis", "serve", "--open"]) {
        Ok(Cli {
            command: Some(c), ..
        }) => c,
        _ => unreachable!("`serve --open` always parses"),
    }
}

#[derive(Debug, Args, Clone)]
pub struct DataArgs {
    /// Data directory (default: the os data dir). The brain lives in `<data dir>/anamnesis`.
    #[arg(long, env = "ANAMNESIS_DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    /// Use exactly this brain directory (holding `brain.sqlite`), e.g. the demo brain.
    #[arg(long, hide = true)]
    pub brain_dir: Option<PathBuf>,
    /// Use the deterministic hash embedder instead of the configured one (offline, fast).
    #[arg(long)]
    pub hash_embedder: bool,
    /// Another folder of connector plugins (each in its own subfolder with a
    /// `manifest.json`), besides `<data dir>/anamnesis/plugins`. Repeatable.
    #[arg(long = "plugins-dir")]
    pub plugins_dirs: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Serve the http api, the ui and mcp at /mcp on 127.0.0.1.
    Serve {
        #[command(flatten)]
        data: DataArgs,
        #[arg(long, default_value_t = 7368, env = "ANAMNESIS_PORT")]
        port: u16,
        /// Open the ui in the browser.
        #[arg(long)]
        open: bool,
        /// Directory holding the ui (`app/`, `theme/`).
        #[arg(long)]
        ui_dir: Option<PathBuf>,
        /// Start with the demo brain (kept apart from yours, in `<data dir>/anamnesis/demo`).
        #[arg(long)]
        demo: bool,
        /// Demo bundle: a folder of notes or a bundle file.
        #[arg(long)]
        demo_bundle: Option<PathBuf>,
        /// Do not detect model backends (no probing of local ports, no `claude --version`).
        #[arg(long)]
        no_detect: bool,
    },
    /// Serve mcp over stdio (for harnesses that spawn `anamnesis mcp`). Logs go to stderr.
    Mcp {
        #[command(flatten)]
        data: DataArgs,
    },
    /// Import a folder (markdown / obsidian vault) or an exported bundle.
    Import {
        #[command(flatten)]
        data: DataArgs,
        path: PathBuf,
    },
    /// Export the whole brain as a bundle (json lines).
    Export {
        #[command(flatten)]
        data: DataArgs,
        file: PathBuf,
    },
    /// Sync every source once and run the background jobs once.
    Sync {
        #[command(flatten)]
        data: DataArgs,
    },
    /// Add a source, e.g. `add-source folder --name notes --config '{"path":"~/notes"}'`.
    AddSource {
        #[command(flatten)]
        data: DataArgs,
        connector: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "{}")]
        config: String,
        /// Privacy flag for the source's items: local_only, cloud_ok or ask (default: the
        /// `privacy.default` setting, else the connector's default).
        #[arg(long, value_parser = parse_privacy)]
        privacy: Option<anamnesis_core::Privacy>,
        /// Origin of the source's items when a document does not say (e.g. external,
        /// owner_written, ai_generated).
        #[arg(long, value_parser = parse_origin)]
        origin: Option<anamnesis_core::Origin>,
    },
    /// Build the demo brain (import, embeddings, regions, suggestions) in
    /// `<data dir>/anamnesis/demo`, apart from yours. `serve --demo` then shows it.
    Demo {
        #[command(flatten)]
        data: DataArgs,
        /// Remove the demo brain first and build it afresh.
        #[arg(long)]
        rebuild: bool,
        /// Demo notes to use instead of the shipped ones: a folder of notes or a bundle.
        #[arg(long)]
        demo_bundle: Option<PathBuf>,
    },
    /// Verify the hash chain of the event log.
    Verify {
        #[command(flatten)]
        data: DataArgs,
    },
    /// Print the honesty table of the built-in evaluation.
    Eval {
        #[command(flatten)]
        data: DataArgs,
        /// Print the whole report as json.
        #[arg(long)]
        json: bool,
    },
}

fn parse_snake<T: serde::de::DeserializeOwned>(
    s: &str,
    what: &str,
    allowed: &str,
) -> Result<T, String> {
    let norm = s.trim().to_lowercase().replace('-', "_");
    serde_json::from_value(serde_json::Value::String(norm))
        .map_err(|_| format!("unknown {what} {s:?} (one of {allowed})"))
}

fn parse_privacy(s: &str) -> Result<anamnesis_core::Privacy, String> {
    parse_snake(s, "privacy", "local_only, cloud_ok, ask")
}

fn parse_origin(s: &str) -> Result<anamnesis_core::Origin, String> {
    parse_snake(
        s,
        "origin",
        "owner_written, owner_spoken, owner_annotation, owner_in_chat, ai_generated, ai_edited, external",
    )
}

fn config(data: &DataArgs, port: u16) -> ServerConfig {
    let root = data
        .data_dir
        .clone()
        .map(|d| d.join("anamnesis"))
        .unwrap_or_else(default_root);
    ServerConfig {
        model_cache: anamnesis_embed::default_cache_dir(),
        root,
        port,
        ui_dir: None,
        demo_bundle: None,
        embedder_override: data.hash_embedder.then(EmbedderSpec::hash),
        detect_backends: true,
        plugin_dirs: data.plugins_dirs.clone(),
        key_store: crate::secrets::KeyStoreMode::from_env(),
        // brain_dir handled by `open`
    }
}

fn open(data: &DataArgs, mut cfg: ServerConfig, demo: bool) -> Result<Arc<App>> {
    if let Some(b) = &data.brain_dir {
        // an explicit brain directory: the demo brain is `<root>/demo`
        let b = std::fs::canonicalize(b)
            .map(|c| plain_path(&c))
            .unwrap_or_else(|_| b.clone());
        if b.file_name().is_some_and(|n| n == "demo")
            && let Some(parent) = b.parent()
        {
            cfg.root = parent.to_path_buf();
            return App::open(cfg, true);
        }
        cfg.root = b;
    }
    App::open(cfg, demo)
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter =
        EnvFilter::try_from_env("ANAMNESIS_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

pub async fn run(cli: Cli) -> Result<()> {
    init_tracing();
    match cli.command.unwrap_or_else(default_command) {
        Command::Serve {
            data,
            port,
            open: open_browser,
            ui_dir,
            demo,
            demo_bundle,
            no_detect,
        } => {
            let mut cfg = config(&data, port);
            cfg.ui_dir = crate::ui::locate_ui_dir(ui_dir);
            cfg.demo_bundle = demo_bundle;
            cfg.detect_backends = !no_detect;
            let app = open(&data, cfg, false)?;
            if demo {
                crate::importer::set_demo(&app, true).await?;
            }
            serve(app, open_browser).await
        }
        Command::Mcp { data } => {
            let mut cfg = config(&data, 7368);
            cfg.detect_backends = false;
            let app = open(&data, cfg, false)?;
            {
                let app = app.clone();
                tokio::spawn(async move { app.load_embedder().await });
            }
            let server =
                anamnesis_mcp::McpServer::new(crate::mcp_brain::StoreBrain::new(app.clone()))
                    .with_fallback_client("stdio");
            anamnesis_mcp::serve_stdio(server).await
        }
        Command::Import { data, path } => {
            let app = open(&data, config(&data, 7368), false)?;
            let r = crate::importer::import_path(&app.brain(), &path).await?;
            eprintln!("imported: {}", serde_json::to_string(&r)?);
            let j = crate::jobs::run_jobs_once(&app).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "import": r, "jobs": j }))?
            );
            Ok(())
        }
        Command::Export { data, file } => {
            let app = open(&data, config(&data, 7368), false)?;
            let m = crate::importer::export_to(&app.brain(), &file).await?;
            println!("exported {} events to {}", m.events, file.display());
            Ok(())
        }
        Command::Sync { data } => {
            let app = open(&data, config(&data, 7368), false)?;
            for (src, r) in crate::sync::sync_all(&app).await? {
                match r {
                    Ok(r) => println!("{}: {}", src.name, serde_json::to_string(&r)?),
                    Err(e) => println!("{}: error: {e:#}", src.name),
                }
            }
            let j = crate::jobs::run_jobs_once(&app).await?;
            println!("jobs: {}", serde_json::to_string(&j)?);
            Ok(())
        }
        Command::AddSource {
            data,
            connector,
            name,
            config: cfg_json,
            privacy,
            origin,
        } => {
            let app = open(&data, config(&data, 7368), false)?;
            let config: serde_json::Value =
                serde_json::from_str(&cfg_json).context("--config is not json")?;
            let src = crate::api::add_source_checked(
                &app,
                crate::api::NewSourceArgs {
                    connector,
                    name,
                    config,
                    default_origin: origin,
                    default_privacy: privacy,
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.message()))?;
            println!(
                "added source {} ({}), privacy {}",
                src.name,
                src.id,
                serde_json::to_value(src.default_privacy)?
                    .as_str()
                    .unwrap_or("")
            );
            let r = crate::sync::sync_source(&app.brain(), &app.plugins, &src.id).await?;
            println!("synced: {}", serde_json::to_string(&r)?);
            Ok(())
        }
        Command::Demo {
            data,
            rebuild,
            demo_bundle,
        } => {
            let mut cfg = config(&data, 7368);
            cfg.demo_bundle = demo_bundle;
            cfg.detect_backends = false;
            if rebuild && crate::importer::remove_demo_brain(&cfg)? {
                eprintln!("removed the old demo brain");
            }
            let app = App::open(cfg, true)?;
            crate::importer::build_demo(&app).await
        }
        Command::Verify { data } => {
            let app = open(&data, config(&data, 7368), false)?;
            let r = app.brain().db(|s| Ok(s.verify_chain()?)).await?;
            println!(
                "chain ok: {} events, head {}",
                r.events,
                r.head.as_deref().unwrap_or("-")
            );
            Ok(())
        }
        Command::Eval { data, json } => {
            let app = open(&data, config(&data, 7368), false)?;
            app.load_embedder().await;
            let r = crate::api::eval_report(&app).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                println!("{}", r.honesty.to_markdown());
            }
            Ok(())
        }
    }
}

async fn serve(app: Arc<App>, open_browser: bool) -> Result<()> {
    let port = app.cfg.port;
    let listener = match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await
    {
        Ok(l) => l,
        Err(e) => {
            // with --open (and a bare `anamnesis`), a second launch while one already
            // serves on this port just opens the browser on the running one
            if open_browser
                && e.kind() == std::io::ErrorKind::AddrInUse
                && is_anamnesis_running(port).await
            {
                let url = format!("http://127.0.0.1:{port}/");
                eprintln!("anamnesis is already running on {url}; opening it");
                open_url(&url);
                return Ok(());
            }
            return Err(e).with_context(|| format!("binding 127.0.0.1:{port}"));
        }
    };
    let url = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
    match &app.cfg.ui_dir {
        Some(d) => tracing::info!("ui from {}", d.display()),
        None => tracing::info!("ui: embedded copy (or placeholder)"),
    }
    tracing::info!(
        "anamnesis listening on {url} (brain: {})",
        app.brain().dir.display()
    );
    crate::start_background(&app);
    if open_browser {
        open_url(&url);
    }
    let router = crate::api::router(app.clone());
    let cancel = app.cancel.clone();
    tokio::spawn({
        let cancel = cancel.clone();
        async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
            cancel.cancel();
        }
    });
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { cancel.cancelled_owned().await })
        .await?;
    Ok(())
}

/// Whether an anamnesis server answers on `127.0.0.1:port`: a plain http/1.0
/// `GET /api/status` whose json carries the fields only anamnesis reports.
async fn is_anamnesis_running(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let probe = async {
        let mut s = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        s.write_all(
            format!(
                "GET /api/status HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;
        let mut buf = Vec::new();
        s.take(64 * 1024).read_to_end(&mut buf).await?;
        std::io::Result::Ok(String::from_utf8_lossy(&buf).into_owned())
    };
    match tokio::time::timeout(std::time::Duration::from_secs(3), probe).await {
        Ok(Ok(resp)) => looks_like_status(&resp),
        _ => false,
    }
}

fn looks_like_status(resp: &str) -> bool {
    let ok = resp
        .lines()
        .next()
        .is_some_and(|l| l.starts_with("HTTP/") && l.contains(" 200"));
    let body = resp.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    ok && serde_json::from_str::<serde_json::Value>(body).is_ok_and(|v| {
        v.get("version").is_some() && v.get("embedder").is_some() && v.get("events").is_some()
    })
}

/// Open the ui in the default browser (`open` on macos, the registered handler through
/// the shell api on windows, `xdg-open` and friends on linux). No shell is involved.
fn open_url(url: &str) {
    if let Err(e) = webbrowser::open(url) {
        tracing::warn!("could not open the browser ({e}); visit {url}");
    }
}

/// `std::fs::canonicalize` on windows returns verbatim paths (`\\?\C:\…`), which some
/// programs (and people) do not accept. Strip the prefix from plain drive and unc paths;
/// std re-adds it internally where a long path needs it.
pub fn plain_path(p: &std::path::Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(rest);
    }
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_prefixes_are_stripped() {
        use std::path::Path;
        assert_eq!(
            plain_path(Path::new(r"\\?\X:\notes\vault")),
            PathBuf::from(r"X:\notes\vault")
        );
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        // a verbatim path that is not a drive path is kept as it is
        assert_eq!(
            plain_path(Path::new(r"\\?\Volume{abc}\x")),
            PathBuf::from(r"\\?\Volume{abc}\x")
        );
        assert_eq!(
            plain_path(Path::new("/home/me/notes")),
            PathBuf::from("/home/me/notes")
        );
    }

    #[test]
    fn no_subcommand_means_serve_and_open() {
        let cli = Cli::try_parse_from(["anamnesis"]).unwrap();
        assert!(cli.is_bare());
        match default_command() {
            Command::Serve {
                open, demo, port, ..
            } => {
                assert!(open);
                assert!(!demo);
                // 7368 unless ANAMNESIS_PORT is set in the test environment
                if std::env::var_os("ANAMNESIS_PORT").is_none() {
                    assert_eq!(port, 7368);
                }
            }
            other => panic!("{other:?}"),
        }
        // the subcommands still parse
        assert!(
            !Cli::try_parse_from(["anamnesis", "verify"])
                .unwrap()
                .is_bare()
        );
    }

    #[test]
    fn status_probe_recognises_anamnesis() {
        let ok = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n\
                  {\"version\":\"0.1.0\",\"events\":3,\"embedder\":{}}";
        assert!(looks_like_status(ok));
        assert!(!looks_like_status(
            "HTTP/1.1 200 OK\r\n\r\n<html>something else</html>"
        ));
        assert!(!looks_like_status("HTTP/1.1 404 Not Found\r\n\r\n{}"));
    }

    #[tokio::test]
    async fn status_probe_on_a_closed_port_is_false() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(!is_anamnesis_running(port).await);
    }

    #[test]
    fn add_source_takes_privacy_and_origin() {
        let cli = Cli::try_parse_from([
            "anamnesis",
            "add-source",
            "folder",
            "--config",
            "{\"path\":\"/tmp\"}",
            "--privacy",
            "local-only",
            "--origin",
            "external",
            "--plugins-dir",
            "/opt/plugins",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Command::AddSource {
                privacy,
                origin,
                data,
                ..
            } => {
                assert_eq!(privacy, Some(anamnesis_core::Privacy::LocalOnly));
                assert_eq!(origin, Some(anamnesis_core::Origin::External));
                assert_eq!(data.plugins_dirs, vec![PathBuf::from("/opt/plugins")]);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            Cli::try_parse_from(["anamnesis", "add-source", "folder", "--privacy", "public"])
                .is_err()
        );
    }
}
