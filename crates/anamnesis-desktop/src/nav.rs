//! Where a navigation may go: the splash page and the local server stay in the window,
//! web and mail links go to the system browser, everything else is refused.

use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// Load it in the app window.
    Inside,
    /// Hand it to the system browser (or mail client).
    Browser,
    /// Drop it (`file:`, `javascript:`, `data:`, another local port…).
    Refuse,
}

/// Classify `url` for a window whose server listens on `127.0.0.1:port` (0: not yet).
pub fn classify(url: &Url, port: u16) -> Target {
    match url.scheme() {
        // the embedded splash page: tauri://localhost (macos, linux),
        // http(s)://tauri.localhost (windows)
        "tauri" => Target::Inside,
        "about" if url.as_str() == "about:blank" => Target::Inside,
        "http" | "https" => {
            let host = url.host_str().unwrap_or("");
            if host == "tauri.localhost" {
                return Target::Inside;
            }
            let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]");
            if loopback {
                if port != 0 && url.scheme() == "http" && url.port() == Some(port) {
                    Target::Inside
                } else {
                    // another local service: not ours to show, nor to send to a browser
                    // without the owner seeing where it leads
                    Target::Browser
                }
            } else {
                Target::Browser
            }
        }
        "mailto" => Target::Browser,
        _ => Target::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str, port: u16) -> Target {
        classify(&Url::parse(s).unwrap(), port)
    }

    #[test]
    fn local_server_and_splash_stay_inside() {
        assert_eq!(c("http://127.0.0.1:7368/", 7368), Target::Inside);
        assert_eq!(c("http://localhost:7368/#notes", 7368), Target::Inside);
        assert_eq!(c("tauri://localhost/index.html", 0), Target::Inside);
        assert_eq!(c("http://tauri.localhost/index.html", 0), Target::Inside);
        assert_eq!(c("about:blank", 7368), Target::Inside);
    }

    #[test]
    fn the_web_goes_to_the_browser() {
        assert_eq!(
            c("https://github.com/aphesist/anamnesis", 7368),
            Target::Browser
        );
        assert_eq!(c("http://example.org/", 7368), Target::Browser);
        assert_eq!(c("mailto:someone@example.org", 7368), Target::Browser);
        // another local port, or ours before the server runs, is not loaded inside
        assert_eq!(c("http://127.0.0.1:11434/", 7368), Target::Browser);
        assert_eq!(c("http://127.0.0.1:7368/", 0), Target::Browser);
        assert_eq!(c("https://127.0.0.1:7368/", 7368), Target::Browser);
    }

    #[test]
    fn other_schemes_are_refused() {
        assert_eq!(c("file:///etc/passwd", 7368), Target::Refuse);
        assert_eq!(c("javascript:alert(1)", 7368), Target::Refuse);
        assert_eq!(c("data:text/html,hi", 7368), Target::Refuse);
    }
}
