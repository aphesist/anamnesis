//! Static ui: `/` → `ui/app/index.html`, `/theme/…` → `ui/theme/…`, other paths →
//! `ui/app/…`. Served from disk when a ui directory is found (dev), otherwise from the
//! copy embedded at build time (feature `embed-ui`).

use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

#[cfg(feature = "embed-ui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../ui/"]
#[exclude = "*/node_modules/*"]
#[exclude = "theme/docs-screens/*"]
#[exclude = "*/test/*"]
#[exclude = "app/mock/*"]
struct Embedded;

/// Find the ui directory: `--ui-dir`, `ANAMNESIS_UI_DIR`, `ui/` next to the binary,
/// or the repository's `ui/` (dev builds).
pub fn locate_ui_dir(explicit: Option<PathBuf>) -> Option<PathBuf> {
    let ok = |p: &Path| p.join("app").is_dir() || p.join("theme").is_dir();
    if let Some(p) = explicit {
        return Some(p);
    }
    if let Ok(p) = std::env::var("ANAMNESIS_UI_DIR")
        && !p.is_empty()
    {
        return Some(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for cand in [dir.join("ui"), dir.join("../share/anamnesis/ui")] {
            if ok(&cand) {
                return Some(cand);
            }
        }
        // target/{debug,release}/anamnesis → <repo>/ui
        if let Some(repo) = dir.parent().and_then(|p| p.parent()) {
            let cand = repo.join("ui");
            if ok(&cand) && repo.join("Cargo.toml").is_file() {
                return Some(cand);
            }
        }
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui");
    if cfg!(debug_assertions) && ok(&dev) {
        return Some(dev);
    }
    None
}

/// Map a request path to candidate files relative to the ui root.
fn candidates(path: &str) -> Vec<String> {
    let p = path.trim_start_matches('/');
    if p.is_empty() || p == "index.html" {
        return vec!["app/index.html".into()];
    }
    if p.starts_with("theme/") || p.starts_with("app/") {
        return vec![p.to_string()];
    }
    vec![format!("app/{p}"), p.to_string()]
}

fn safe(rel: &str) -> bool {
    Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

fn file_response(rel: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(rel).first_or_octet_stream();
    let mut res = Response::new(Body::from(bytes));
    if let Ok(v) = HeaderValue::from_str(mime.as_ref()) {
        res.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res
}

const PLACEHOLDER: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>anamnesis</title>\
<style>body{background:#0d0c0b;color:#d8d2c4;font:16px/1.6 ui-monospace,monospace;display:grid;place-items:center;min-height:100vh;margin:0}main{max-width:36em;padding:1em}</style></head>\
<body><main><h1>anamnesis</h1><p>the brain is running, but the ui was not found.</p><p>start with <code>--ui-dir path/to/ui</code> or set <code>ANAMNESIS_UI_DIR</code>. the api answers at <code>/api/</code>.</p></main></body></html>";

/// Serve a ui file (fallback handler for every non-api path).
pub async fn serve(ui_dir: Option<PathBuf>, uri: Uri) -> Response {
    let path = uri.path();
    let wants_index = path == "/" || path == "/index.html";
    for rel in candidates(path) {
        if !safe(&rel) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        if let Some(dir) = &ui_dir {
            let full = dir.join(&rel);
            if full.is_file()
                && let Ok(b) = tokio::fs::read(&full).await
            {
                return file_response(&rel, b);
            }
            continue;
        }
        #[cfg(feature = "embed-ui")]
        if let Some(f) = Embedded::get(&rel) {
            return file_response(&rel, f.data.into_owned());
        }
    }
    if wants_index {
        return (
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
            PLACEHOLDER,
        )
            .into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_paths() {
        assert_eq!(candidates("/"), vec!["app/index.html"]);
        assert_eq!(candidates("/theme/theme.css"), vec!["theme/theme.css"]);
        assert_eq!(candidates("/main.js"), vec!["app/main.js", "main.js"]);
        assert!(!safe("../etc/passwd"));
        assert!(safe("app/x.js"));
    }
}
