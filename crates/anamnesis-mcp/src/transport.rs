//! Transports: stdio (for harnesses that spawn `anamnesis mcp`) and
//! Streamable HTTP (an axum router mounted at `/mcp`, loopback only).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::{
    ServiceExt,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use tokio_util::sync::CancellationToken;

use crate::{McpServer, api::BrainApi};

/// Serves the brain over stdin/stdout until the client disconnects.
///
/// Nothing else may write to stdout while this runs (log to stderr).
pub async fn serve_stdio<B: BrainApi>(server: McpServer<B>) -> anyhow::Result<()> {
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}

/// Options for the Streamable HTTP transport.
#[derive(Debug, Clone)]
pub struct HttpOptions {
    /// If set, every request must carry `Authorization: Bearer <token>`.
    pub bearer_token: Option<String>,
    /// Allowed `Host` values (dns-rebinding protection). Loopback names by default.
    pub allowed_hosts: Vec<String>,
    /// Allowed browser `Origin`s. Requests without an `Origin` (cli harnesses) always pass;
    /// browser requests must come from one of these. Loopback http origins by default.
    pub allowed_origins: Vec<String>,
    /// Client name recorded when the harness does not identify itself.
    pub fallback_client: String,
    /// Cancelled to stop all sessions.
    pub cancellation_token: CancellationToken,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            bearer_token: None,
            allowed_hosts: vec!["localhost".into(), "127.0.0.1".into(), "::1".into()],
            allowed_origins: vec!["http://127.0.0.1:*".into(), "http://localhost:*".into()],
            fallback_client: "http".into(),
            cancellation_token: CancellationToken::new(),
        }
    }
}

/// Constant-time byte comparison, so the token cannot be guessed by timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn require_bearer(State(token): State<Arc<String>>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .is_some_and(|t| ct_eq(t.trim().as_bytes(), token.as_bytes()));
    if ok {
        next.run(req).await
    } else {
        let mut res = (StatusCode::UNAUTHORIZED, Body::from("unauthorized")).into_response();
        res.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"anamnesis\""),
        );
        res
    }
}

/// An axum router serving mcp at `/mcp`. Merge or nest it into the app's router,
/// or use [`serve_http`]. The router itself does not bind anything.
pub fn http_router<B: BrainApi>(brain: Arc<B>, opts: HttpOptions) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(opts.allowed_hosts.clone())
        .with_allowed_origins(opts.allowed_origins.clone())
        .with_sse_keep_alive(Some(Duration::from_secs(15)))
        .with_cancellation_token(opts.cancellation_token.clone());
    let fallback = opts.fallback_client.clone();
    let service = StreamableHttpService::new(
        move || Ok(McpServer::new(brain.clone()).with_fallback_client(&fallback)),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = Router::new().nest_service("/mcp", service);
    match opts.bearer_token.filter(|t| !t.is_empty()) {
        Some(token) => router.layer(middleware::from_fn_with_state(
            Arc::new(token),
            require_bearer,
        )),
        None => router,
    }
}

/// Binds `127.0.0.1:<port>` (loopback only) and serves [`http_router`] until
/// the options' cancellation token fires. Returns the bound address via `on_bound`.
pub async fn serve_http<B: BrainApi>(
    brain: Arc<B>,
    port: u16,
    opts: HttpOptions,
    on_bound: impl FnOnce(SocketAddr),
) -> anyhow::Result<()> {
    let ct = opts.cancellation_token.clone();
    let router = http_router(brain, opts);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    on_bound(listener.local_addr()?);
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { ct.cancelled_owned().await })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ct_eq;

    #[test]
    fn constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }
}
