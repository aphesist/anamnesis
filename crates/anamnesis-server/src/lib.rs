//! The `anamnesis` server: the http api of `docs/contracts.md`, background jobs
//! (connector sync, embeddings, regions + layout, link suggestions, threads, daily
//! review), the mcp server (stdio and streamable http at `/mcp`) and the static ui.
//!
//! Entry points for embedding and tests: [`App::open`], [`api::router`],
//! [`jobs::run_jobs_once`], [`start_background`], [`mcp_brain::StoreBrain`], and
//! [`server::run`] to run the whole server from another crate (the desktop shell).

pub mod api;
pub mod app;
pub mod backends;
pub mod brain;
pub mod chat;
pub mod cli;
pub mod devices;
pub mod distil;
pub mod importer;
pub mod jobs;
pub mod mcp_brain;
pub mod plugins;
pub mod privacy;
pub mod secrets;
pub mod server;
pub mod sync;
pub mod ui;

pub use app::{App, ServerConfig, default_root};
pub use brain::Brain;
pub use jobs::run_jobs_once;

use std::sync::Arc;

/// Start every background task (embedder load, backend detection, sync supervisor, job
/// loop). They stop when `app.cancel` is cancelled.
pub fn start_background(app: &Arc<App>) {
    {
        let app = app.clone();
        tokio::spawn(async move { app.load_embedder().await });
    }
    backends::detect_in_background(app);
    tokio::spawn(sync::sync_supervisor(app.clone()));
    tokio::spawn(jobs::jobs_loop(app.clone()));
    devices::load(app);
    tokio::spawn(devices::devices_loop(app.clone()));
}
