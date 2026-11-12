//! Errors before (or instead of) the window: on windows there is no console, so they are
//! shown in a message box; elsewhere they go to stderr. Both also go to the log.

/// Microsoft's evergreen webview2 bootstrapper.
#[cfg_attr(not(windows), allow(dead_code))]
pub const WEBVIEW2_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// Show `text` to the person. With `ask`, returns whether they chose yes.
#[cfg(windows)]
pub fn show(text: &str, ask: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (t, c) = (wide(text), wide("anamnesis"));
    let style = if ask {
        MB_YESNO | MB_ICONQUESTION
    } else {
        MB_OK | MB_ICONERROR
    };
    // SAFETY: both strings are nul-terminated utf-16 buffers that outlive the call
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), style) == IDYES }
}

#[cfg(not(windows))]
pub fn show(text: &str, _ask: bool) -> bool {
    eprintln!("anamnesis: {text}");
    false
}

#[cfg(not(windows))]
pub fn check_webview() -> bool {
    true
}

/// On windows the window needs the webview2 runtime: when it is missing, say so and offer
/// microsoft's installer, then stop. Returns false when the app cannot start.
#[cfg(windows)]
pub fn check_webview() -> bool {
    match wry::webview_version() {
        Ok(v) => {
            tracing::info!("webview2 {v}");
            true
        }
        Err(e) => {
            tracing::error!("webview2 runtime not found: {e}");
            let yes = show(
                "the anamnesis window needs the microsoft edge webview2 runtime, which this pc \
                 does not have (windows 11 and up-to-date windows 10 include it).\n\n\
                 download microsoft's installer for it now? run it, then start anamnesis again.\n\n\
                 (the command line anamnesis.exe works without it.)",
                true,
            );
            if yes {
                let _ = tauri_plugin_opener::open_url(WEBVIEW2_URL, None::<&str>);
            }
            false
        }
    }
}
