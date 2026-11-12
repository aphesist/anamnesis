//! The native menu: file (import a folder, open the data folder, quit), view (reload,
//! full screen, zoom) and help (about, the docs on github). Lowercase, like the rest of
//! anamnesis.

use std::path::{Path, PathBuf};

use anamnesis_server::api::{NewSourceArgs, add_source_checked};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

use crate::{MAIN, Shell};

pub const DOCS_URL: &str = "https://github.com/aphesist/anamnesis/tree/main/docs";
const REPO_URL: &str = "https://github.com/aphesist/anamnesis";

const ZOOM_STEP: f64 = 1.1;
const ZOOM_MIN: f64 = 0.5;
const ZOOM_MAX: f64 = 3.0;

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let item =
        |id: &str, text: &str, accel: Option<&str>| MenuItem::with_id(app, id, text, true, accel);
    let sep = || PredefinedMenuItem::separator(app);

    let file = Submenu::with_items(
        app,
        "file",
        true,
        &[
            &item("import-folder", "import folder…", Some("CmdOrCtrl+O"))?,
            &item("open-data", "open data folder", None)?,
            &sep()?,
            &item("quit", "quit", Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "view",
        true,
        &[
            &item("reload", "reload", Some("CmdOrCtrl+R"))?,
            &item(
                "fullscreen",
                "toggle full screen",
                Some(if cfg!(target_os = "macos") {
                    "Ctrl+Cmd+F"
                } else {
                    "F11"
                }),
            )?,
            &sep()?,
            &item("zoom-in", "zoom in", Some("CmdOrCtrl+="))?,
            &item("zoom-out", "zoom out", Some("CmdOrCtrl+-"))?,
            &item("zoom-reset", "actual size", Some("CmdOrCtrl+0"))?,
        ],
    )?;
    let help = Submenu::with_items(
        app,
        "help",
        true,
        &[
            &item("about", "about anamnesis", None)?,
            &item("docs", "open the docs on github", None)?,
        ],
    )?;

    if cfg!(target_os = "macos") {
        // macos: the first menu is the app menu, and copy / paste in the page only work
        // through an edit menu
        let app_menu = Submenu::with_items(
            app,
            "anamnesis",
            true,
            &[
                &item("about", "about anamnesis", None)?,
                &sep()?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &sep()?,
                &item("quit", "quit anamnesis", Some("Cmd+Q"))?,
            ],
        )?;
        let edit = Submenu::with_items(
            app,
            "edit",
            true,
            &[
                &PredefinedMenuItem::undo(app, None)?,
                &PredefinedMenuItem::redo(app, None)?,
                &sep()?,
                &PredefinedMenuItem::cut(app, None)?,
                &PredefinedMenuItem::copy(app, None)?,
                &PredefinedMenuItem::paste(app, None)?,
                &PredefinedMenuItem::select_all(app, None)?,
            ],
        )?;
        Menu::with_items(app, &[&app_menu, &file, &edit, &view, &help])
    } else {
        Menu::with_items(app, &[&file, &view, &help])
    }
}

pub fn on_event(app: &AppHandle, event: MenuEvent) {
    let window = app.get_webview_window(MAIN);
    match event.id().as_ref() {
        "import-folder" => import_folder(app),
        "open-data" => {
            let root = app.state::<Shell>().root.clone();
            if let Err(e) = app.opener().open_path(root.to_string_lossy(), None::<&str>) {
                tracing::warn!("could not open {}: {e}", root.display());
            }
        }
        "quit" => app.exit(0),
        "reload" => {
            if let Some(w) = window
                && let Err(e) = w.reload()
            {
                tracing::warn!("reload: {e}");
            }
        }
        "fullscreen" => {
            if let Some(w) = window {
                let on = w.is_fullscreen().unwrap_or(false);
                let _ = w.set_fullscreen(!on);
            }
        }
        id @ ("zoom-in" | "zoom-out" | "zoom-reset") => {
            let shell = app.state::<Shell>();
            let mut z = shell.zoom.lock().unwrap();
            *z = next_zoom(*z, id);
            if let Some(w) = window {
                let _ = w.set_zoom(*z);
            }
        }
        "about" => about(app),
        "docs" => {
            let _ = app.opener().open_url(DOCS_URL, None::<&str>);
        }
        _ => {}
    }
}

fn next_zoom(z: f64, id: &str) -> f64 {
    let n = match id {
        "zoom-in" => z * ZOOM_STEP,
        "zoom-out" => z / ZOOM_STEP,
        _ => 1.0,
    };
    // snap near 1 so in-then-out lands on exactly 100 %
    let n = if (n - 1.0).abs() < 0.01 { 1.0 } else { n };
    n.clamp(ZOOM_MIN, ZOOM_MAX)
}

pub fn about_text() -> String {
    format!(
        "anamnesis {version}\n\
         a local-first second brain.\n\n\
         apache license 2.0. {REPO_URL}\n\n\
         bundled: tauri and wry (mit / apache-2.0); onnx runtime (mit, microsoft); \
         the aphesis script font (sil open font licence 1.1); special elite (apache-2.0). \
         the anamnesis icon (six linked rings, from the aphes.ist design system), \
         the φ mark and the wordmark are trademarks of the owner of aphes.ist. \
         the full notices are in NOTICE and in the licenses folder next to the program.",
        version = env!("CARGO_PKG_VERSION")
    )
}

fn about(app: &AppHandle) {
    app.dialog()
        .message(about_text())
        .title("about anamnesis")
        .kind(MessageDialogKind::Info)
        .show(|_| {});
}

/// The connector for a picked folder: `obsidian` for a vault (it holds `.obsidian/`),
/// else `folder`.
fn connector_for(dir: &Path) -> &'static str {
    if dir.join(".obsidian").is_dir() {
        "obsidian"
    } else {
        "folder"
    }
}

/// file → import folder…: pick a folder and add it as a source; the sync supervisor reads
/// it in the background and keeps following it, as when it is added in the ui.
fn import_folder(app: &AppHandle) {
    let app2 = app.clone();
    app.dialog()
        .file()
        .set_title("import a folder or an obsidian vault")
        .pick_folder(move |picked| {
            let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
                return;
            };
            tauri::async_runtime::spawn(add_folder(app2, path));
        });
}

async fn add_folder(app: AppHandle, path: PathBuf) {
    let handle = {
        let shell = app.state::<Shell>();
        let guard = shell.server.lock().unwrap();
        guard.as_ref().map(|h| h.app.clone())
    };
    let Some(brain_app) = handle else {
        message(
            &app,
            "the brain is still waking. try again in a moment.",
            true,
        );
        return;
    };
    let connector = connector_for(&path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty());
    let args = NewSourceArgs {
        connector: connector.into(),
        name: name.clone(),
        config: serde_json::json!({ "path": path.to_string_lossy() }),
        default_origin: None,
        default_privacy: None,
    };
    match add_source_checked(&brain_app, args).await {
        Ok(src) => {
            tracing::info!("added {} as a {connector} source", path.display());
            brain_app.jobs_notify.notify_one();
            message(
                &app,
                &format!(
                    "{} is now a source. its notes join the brain as they are read, and later changes follow.",
                    src.name
                ),
                false,
            );
        }
        Err(e) => message(
            &app,
            &format!("{} could not be added: {}", path.display(), e.message()),
            true,
        ),
    }
}

fn message(app: &AppHandle, text: &str, warn: bool) {
    app.dialog()
        .message(text)
        .title("anamnesis")
        .kind(if warn {
            MessageDialogKind::Warning
        } else {
            MessageDialogKind::Info
        })
        .show(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_steps_and_clamps() {
        let mut z = 1.0;
        z = next_zoom(z, "zoom-in");
        assert!((z - 1.1).abs() < 1e-9);
        z = next_zoom(z, "zoom-out");
        assert_eq!(z, 1.0);
        for _ in 0..40 {
            z = next_zoom(z, "zoom-in");
        }
        assert_eq!(z, ZOOM_MAX);
        for _ in 0..80 {
            z = next_zoom(z, "zoom-out");
        }
        assert_eq!(z, ZOOM_MIN);
        assert_eq!(next_zoom(2.0, "zoom-reset"), 1.0);
    }

    #[test]
    fn vaults_use_the_obsidian_connector() {
        let d = std::env::temp_dir().join(format!("anamnesis-desktop-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(connector_for(&d), "folder");
        std::fs::create_dir_all(d.join(".obsidian")).unwrap();
        assert_eq!(connector_for(&d), "obsidian");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn about_names_the_version_and_licences() {
        let t = about_text();
        assert!(t.contains(env!("CARGO_PKG_VERSION")));
        assert!(t.contains("apache license 2.0"));
        assert!(t.contains("onnx runtime"));
        assert!(t.contains("anamnesis icon"));
    }
}
