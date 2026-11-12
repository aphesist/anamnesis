//! The desktop app has no console on windows: the log goes to
//! `<data dir>/anamnesis/logs/anamnesis-desktop.log` (and to stderr where there is one).
//! `ANAMNESIS_LOG` sets the filter, as for the cli.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Start a new log file when the old one passes this size.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
struct Tee(Option<Arc<Mutex<File>>>);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Some(f) = &self.0 {
            let _ = f.lock().unwrap_or_else(|e| e.into_inner()).write_all(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = &self.0 {
            let _ = f.lock().unwrap_or_else(|e| e.into_inner()).flush();
        }
        Ok(())
    }
}

pub fn log_path(root: &Path) -> std::path::PathBuf {
    root.join("logs").join("anamnesis-desktop.log")
}

pub fn init(root: &Path) {
    use tracing_subscriber::EnvFilter;
    let path = log_path(root);
    let file = path
        .parent()
        .and_then(|d| std::fs::create_dir_all(d).ok())
        .and_then(|_| {
            let big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
            if big {
                let _ = std::fs::rename(&path, path.with_extension("log.old"));
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
        })
        .map(|f| Arc::new(Mutex::new(f)));
    let tee = Tee(file);
    let filter =
        EnvFilter::try_from_env("ANAMNESIS_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(move || tee.clone())
        .try_init();
}
