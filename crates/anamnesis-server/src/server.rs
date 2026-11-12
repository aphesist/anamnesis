//! Run the server from another crate: open the brain, bind 127.0.0.1, start the
//! background jobs and serve the http api, the ui and mcp at `/mcp`, then stop it all
//! cleanly. The cli's `serve` and the desktop shell (`anamnesis-desktop`) both use it.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! use anamnesis_server::{ServerConfig, default_root, server};
//! let cfg = ServerConfig::new(default_root(), 7368);
//! let handle = server::run(cfg, server::RunOptions { port_fallback: true, ..Default::default() }).await?;
//! println!("serving on {}", handle.url());
//! handle.shutdown(std::time::Duration::from_secs(5)).await;
//! # Ok(()) }
//! ```

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::app::{App, ServerConfig};

/// How [`run`] starts.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Show the demo brain instead of the user's.
    pub demo: bool,
    /// When the configured port is taken, serve on a free port the os picks instead of
    /// failing.
    pub port_fallback: bool,
}

/// A running server. Dropping it does not stop the server: call [`ServerHandle::shutdown`].
pub struct ServerHandle {
    pub app: Arc<App>,
    pub addr: SocketAddr,
    task: JoinHandle<Result<()>>,
}

impl ServerHandle {
    /// `http://127.0.0.1:<port>/`
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.addr.port())
    }

    /// Wait until the server stops (after [`App::cancel`] is cancelled, or on an error).
    pub async fn wait(&mut self) -> Result<()> {
        match (&mut self.task).await {
            Ok(r) => r,
            Err(e) => Err(anyhow::anyhow!("server task: {e}")),
        }
    }

    /// Stop the server and the background jobs, wait for a jobs pass in flight (at most
    /// `timeout`) and checkpoint the sqlite write-ahead log so the brain is one file again.
    pub async fn shutdown(mut self, timeout: Duration) {
        tracing::info!("shutting down");
        self.app.cancel.cancel();
        let deadline = tokio::time::Instant::now() + timeout;
        if tokio::time::timeout_at(deadline, self.wait())
            .await
            .is_err()
        {
            tracing::warn!("the http server did not stop in time");
            self.task.abort();
        }
        close_brains(&self.app, deadline).await;
    }
}

/// Wait for any jobs pass to finish and checkpoint the wal of the open brains.
async fn close_brains(app: &Arc<App>, deadline: tokio::time::Instant) {
    let mut brains = vec![app.brain()];
    if let Some(user) = app.user_brain()
        && !Arc::ptr_eq(&user, &brains[0])
    {
        brains.push(user);
    }
    for brain in brains {
        // the job loop holds this lock for a whole pass
        let guard = tokio::time::timeout_at(deadline, brain.jobs_lock.lock()).await;
        if guard.is_err() {
            tracing::warn!("a jobs pass is still running at shutdown; leaving it");
        }
        let r = brain
            .db(|s| {
                s.connection()
                    .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
                Ok(())
            })
            .await;
        match r {
            Ok(()) => tracing::info!("brain flushed: {}", brain.dir.display()),
            Err(e) => tracing::warn!("could not checkpoint {}: {e:#}", brain.dir.display()),
        }
    }
}

/// Bind `127.0.0.1:port`; with `fallback`, a free port when that one is taken.
pub async fn bind(port: u16, fallback: bool) -> std::io::Result<TcpListener> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
        Ok(l) => Ok(l),
        Err(e) if fallback && e.kind() == std::io::ErrorKind::AddrInUse => {
            tracing::info!("port {port} is taken; serving on a free port");
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await
        }
        Err(e) => Err(e),
    }
}

/// Open the brain from `cfg`, bind (see [`RunOptions`]) and start serving. `cfg.port` is
/// set to the port actually bound, so the mcp snippets name the right address.
pub async fn run(mut cfg: ServerConfig, opts: RunOptions) -> Result<ServerHandle> {
    let listener = bind(cfg.port, opts.port_fallback)
        .await
        .with_context(|| format!("binding 127.0.0.1:{}", cfg.port))?;
    cfg.port = listener.local_addr()?.port();
    let app = App::open(cfg, false)?;
    if opts.demo {
        crate::importer::set_demo(&app, true).await?;
    }
    start(app, listener)
}

/// Start the background jobs and serve on `listener` until `app.cancel` is cancelled.
/// Must be called inside a tokio runtime.
pub fn start(app: Arc<App>, listener: TcpListener) -> Result<ServerHandle> {
    let addr = listener.local_addr()?;
    match &app.cfg.ui_dir {
        Some(d) => tracing::info!("ui from {}", d.display()),
        None => tracing::info!("ui: embedded copy (or placeholder)"),
    }
    tracing::info!(
        "anamnesis listening on http://{addr}/ (brain: {})",
        app.brain().dir.display()
    );
    crate::start_background(&app);
    let router = crate::api::router(app.clone());
    let cancel = app.cancel.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { cancel.cancelled_owned().await })
            .await?;
        Ok(())
    });
    Ok(ServerHandle { app, addr, task })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_on_a_free_port_when_taken_and_shuts_down() {
        let dir = tempfile::tempdir().unwrap();
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let mut cfg = ServerConfig::for_tests(dir.path());
        cfg.port = port;
        assert!(
            run(cfg.clone(), RunOptions::default()).await.is_err(),
            "without fallback a taken port is an error"
        );
        let h = run(
            cfg,
            RunOptions {
                port_fallback: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_ne!(h.addr.port(), port);
        assert_eq!(h.app.cfg.port, h.addr.port());
        assert!(h.url().starts_with("http://127.0.0.1:"));

        // the api answers
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(h.addr).await.unwrap();
        s.write_all(b"GET /api/status HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let mut buf = String::new();
        s.read_to_string(&mut buf).await.unwrap();
        assert!(buf.starts_with("HTTP/1.0 200") || buf.starts_with("HTTP/1.1 200"));

        let addr = h.addr;
        h.shutdown(Duration::from_secs(5)).await;
        assert!(tokio::net::TcpStream::connect(addr).await.is_err());
        // the wal was checkpointed into the main file
        let wal = dir.path().join("brain.sqlite-wal");
        assert!(!wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0);
    }
}
