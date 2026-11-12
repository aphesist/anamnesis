//! anamnesis, the desktop application: a native window (tauri 2: webview2 on windows,
//! wkwebview on macos, webkitgtk on linux) around the anamnesis server, which runs in
//! this process.
//!
//! The server binds 127.0.0.1 (port 7368, or a free port when that one is taken), as the
//! `anamnesis` binary does, so mcp over http stays reachable at `/mcp` for harnesses while
//! the app is open. The window first shows a small splash page from the embedded assets,
//! then loads the ui from the server. Links to other sites open in the system browser.
//! See docs/architecture.md, "the desktop shell".

// no console window on windows (release and debug alike: the log goes to a file)
#![windows_subsystem = "windows"]

mod fatal;
mod logging;
mod menu;
mod nav;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use anamnesis_server::server::{self, RunOptions, ServerHandle};
use anamnesis_server::{ServerConfig, default_root};
use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindow};

/// The label of the one window.
pub const MAIN: &str = "main";
/// The port the server tries first (the same as `anamnesis serve`).
pub const DEFAULT_PORT: u16 = 7368;
/// The aphesis dark theme's ground (`--bg`, oklch(0.11 0.004 25)), painted before the
/// page has loaded.
const BACKGROUND: tauri::window::Color = tauri::window::Color(5, 4, 4, 255);

/// What the shell keeps between events.
pub struct Shell {
    /// `<data dir>/anamnesis`: the brain, the logs.
    pub root: PathBuf,
    /// The port the server bound (0 until it runs); navigation is allowed to it only.
    pub port: AtomicU16,
    pub server: Mutex<Option<ServerHandle>>,
    /// The page zoom (view → zoom).
    pub zoom: Mutex<f64>,
}

/// `ANAMNESIS_DATA_DIR` as in the cli (the brain lives in `<data dir>/anamnesis`), else
/// the os data dir.
fn data_root() -> PathBuf {
    match std::env::var_os("ANAMNESIS_DATA_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("anamnesis"),
        _ => default_root(),
    }
}

fn first_port() -> u16 {
    std::env::var("ANAMNESIS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn main() {
    let root = data_root();
    logging::init(&root);
    tracing::info!(
        "anamnesis {} (desktop) starting; brain in {}",
        env!("CARGO_PKG_VERSION"),
        root.display()
    );
    if !fatal::check_webview() {
        std::process::exit(1);
    }

    let app = tauri::Builder::default()
        // first, so a second launch stops before anything else starts
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tracing::info!("a second launch: focusing the window");
            focus_main(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Shell {
            root: root.clone(),
            port: AtomicU16::new(0),
            server: Mutex::new(None),
            zoom: Mutex::new(1.0),
        })
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .setup(move |app| {
            let window = create_window(app.handle())?;
            start_server(app.handle().clone(), window, root.clone());
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("could not start the window: {e}");
            fatal::show(
                &format!(
                    "the window could not start: {e}\n\nthe log is in {}",
                    logging::log_path(&data_root()).display()
                ),
                false,
            );
            std::process::exit(1);
        }
    };
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            shutdown(app);
        }
    });
}

/// The main window, on the splash page until the server runs.
fn create_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let nav_app = app.clone();
    let popup_app = app.clone();
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("anamnesis")
        .inner_size(1440.0, 900.0)
        .min_inner_size(900.0, 600.0)
        .center()
        .background_color(BACKGROUND)
        // the page asks for the system's light / dark preference itself
        .on_navigation(move |url| {
            let port = nav_app.state::<Shell>().port.load(Ordering::SeqCst);
            match nav::classify(url, port) {
                nav::Target::Inside => true,
                nav::Target::Browser => {
                    open_in_browser(&nav_app, url.as_str());
                    false
                }
                nav::Target::Refuse => {
                    tracing::warn!("refused a navigation to {url}");
                    false
                }
            }
        })
        // `target=_blank` links and `window.open`: never a second app window
        .on_new_window(move |url, _features| {
            match nav::classify(&url, 0) {
                nav::Target::Browser => open_in_browser(&popup_app, url.as_str()),
                _ => tracing::warn!("refused a new window for {url}"),
            }
            NewWindowResponse::Deny
        })
        .build()
}

fn open_in_browser(app: &AppHandle, url: &str) {
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        tracing::warn!("could not open {url} in the browser: {e}");
    }
}

/// Bring the window to the front (a second launch, the dock icon).
pub fn focus_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Start the server in this process, then point the window at it.
fn start_server(app: AppHandle, window: WebviewWindow, root: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let mut cfg = ServerConfig::new(root, first_port());
        // the offline hash embedder, as `--hash-embedder` on the cli (tests, slow machines)
        if std::env::var_os("ANAMNESIS_HASH_EMBEDDER").is_some_and(|v| v == "1") {
            cfg.embedder_override = Some(anamnesis_embed::EmbedderSpec::hash());
        }
        let opts = RunOptions {
            port_fallback: true,
            ..Default::default()
        };
        match server::run(cfg, opts).await {
            Ok(handle) => {
                let url = handle.url();
                let shell = app.state::<Shell>();
                shell.port.store(handle.addr.port(), Ordering::SeqCst);
                *shell.server.lock().unwrap() = Some(handle);
                tracing::info!("loading the ui from {url}");
                match url.parse() {
                    Ok(u) => {
                        if let Err(e) = window.navigate(u) {
                            tracing::error!("could not load the ui: {e}");
                        }
                    }
                    Err(e) => tracing::error!("bad server url {url}: {e}"),
                }
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::error!("the server could not start: {msg}");
                let js = format!(
                    "window.anamnesisSplashError && window.anamnesisSplashError({})",
                    js_string(&msg)
                );
                let _ = window.eval(js);
            }
        }
    });
}

/// A javascript string literal (json is a subset).
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\u003c"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Stop the server and the jobs and flush the brain (the window is already gone).
fn shutdown(app: &AppHandle) {
    let handle = app.state::<Shell>().server.lock().unwrap().take();
    if let Some(h) = handle {
        tauri::async_runtime::block_on(h.shutdown(Duration::from_secs(8)));
    }
    tracing::info!("anamnesis closed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_strings_are_escaped() {
        assert_eq!(js_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(js_string("</script>"), "\"\\u003c/script>\"");
        assert_eq!(js_string("\u{1}"), "\"\\u0001\"");
    }
}
