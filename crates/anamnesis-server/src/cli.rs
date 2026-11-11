//! The command line.

use std::path::PathBuf;
use std::sync::Arc;

use anamnesis_embed::EmbedderSpec;
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::app::{App, ServerConfig, default_root};

#[derive(Debug, Parser)]
#[command(name = "anamnesis", version, about = "a local-first second brain", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
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
        // brain_dir handled by `open`
    }
}

fn open(data: &DataArgs, mut cfg: ServerConfig, demo: bool) -> Result<Arc<App>> {
    if let Some(b) = &data.brain_dir {
        // an explicit brain directory: the demo brain is `<root>/demo`
        let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.clone());
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
    let filter = EnvFilter::try_from_env("ANAMNESIS_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).try_init();
}

pub async fn run(cli: Cli) -> Result<()> {
    init_tracing();
    match cli.command {
        Command::Serve { data, port, open: open_browser, ui_dir, demo, demo_bundle, no_detect } => {
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
            let server = anamnesis_mcp::McpServer::new(crate::mcp_brain::StoreBrain::new(app.clone()))
                .with_fallback_client("stdio");
            anamnesis_mcp::serve_stdio(server).await
        }
        Command::Import { data, path } => {
            let app = open(&data, config(&data, 7368), false)?;
            let r = crate::importer::import_path(&app.brain(), &path).await?;
            eprintln!("imported: {}", serde_json::to_string(&r)?);
            let j = crate::jobs::run_jobs_once(&app).await?;
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "import": r, "jobs": j }))?);
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
        Command::AddSource { data, connector, name, config: cfg_json } => {
            let app = open(&data, config(&data, 7368), false)?;
            let config: serde_json::Value = serde_json::from_str(&cfg_json).context("--config is not json")?;
            let src = crate::api::add_source_checked(
                &app,
                crate::api::NewSourceArgs { connector, name, config, default_origin: None, default_privacy: None },
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.message()))?;
            println!("added source {} ({})", src.name, src.id);
            let r = crate::sync::sync_source(&app.brain(), &src.id).await?;
            println!("synced: {}", serde_json::to_string(&r)?);
            Ok(())
        }
        Command::Verify { data } => {
            let app = open(&data, config(&data, 7368), false)?;
            let r = app.brain().db(|s| Ok(s.verify_chain()?)).await?;
            println!("chain ok: {} events, head {}", r.events, r.head.as_deref().unwrap_or("-"));
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
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("binding 127.0.0.1:{port}"))?;
    let url = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
    match &app.cfg.ui_dir {
        Some(d) => tracing::info!("ui from {}", d.display()),
        None => tracing::info!("ui: embedded copy (or placeholder)"),
    }
    tracing::info!("anamnesis listening on {url} (brain: {})", app.brain().dir.display());
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

fn open_url(url: &str) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(cmd).arg(url).spawn() {
        tracing::warn!("could not open the browser ({e}); visit {url}");
    }
}
