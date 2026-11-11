//! `exec` plugin runtime: an external executable speaking json lines over stdin/stdout.
//! The protocol is documented in `PLUGINS.md` at the crate root.
//!
//! One process per call: anamnesis writes a single request line, closes stdin, and reads
//! response lines until the process exits or the timeout passes (then it is killed).
//!
//! `network_hosts` and `fs_paths` from the manifest are recorded and shown to the user; they
//! are **not enforced** — the plugin runs with the user's own permissions. Sandboxing is out
//! of scope for this runtime.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anamnesis_core::{Document, ItemKind, Origin, Timestamp};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::connector::{Batch, CheckReport, Connector, Rename};
use crate::error::{ConnectorError, Result};
use crate::manifest::{API_VERSION, Manifest, PLUGIN_MANIFEST_FILE, Runtime};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Longest accepted response line (a document with its body).
pub const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
struct Request<'a> {
    op: &'a str,
    api_version: u32,
    config: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<&'a str>,
}

/// A document as a plugin sends it: only `external_id`, `title` and `body` are required.
#[derive(Debug, Clone, Deserialize)]
struct PluginDocument {
    external_id: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    kind: Option<ItemKind>,
    title: String,
    body: String,
    #[serde(default)]
    origin: Option<Origin>,
    #[serde(default)]
    created_at: Option<Timestamp>,
    #[serde(default)]
    modified_at: Option<Timestamp>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    refs: Vec<String>,
    #[serde(default)]
    meta: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Response {
    Log {
        #[serde(default)]
        level: Option<String>,
        message: String,
    },
    Check {
        ok: bool,
        #[serde(default)]
        message: String,
    },
    Upsert {
        document: PluginDocument,
    },
    Delete {
        external_id: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Done {
        #[serde(default)]
        next_cursor: Option<String>,
        #[serde(default)]
        has_more: bool,
    },
    Error {
        message: String,
    },
}

pub struct ExecConnector {
    manifest: Manifest,
    program: PathBuf,
    dir: PathBuf,
    timeout: Duration,
}

impl ExecConnector {
    /// Load a plugin from its directory (containing `anamnesis-plugin.json`) or manifest path.
    pub fn load(path: &Path) -> Result<Self> {
        let manifest_path = if path.is_dir() {
            path.join(PLUGIN_MANIFEST_FILE)
        } else {
            path.to_path_buf()
        };
        let manifest = Manifest::from_path(&manifest_path)?;
        let dir = manifest_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::new(manifest, &dir)
    }

    /// `dir` is where the manifest lives; a relative `entry` is resolved against it.
    pub fn new(manifest: Manifest, dir: &Path) -> Result<Self> {
        manifest.validate_plugin()?;
        if manifest.runtime != Runtime::Exec {
            return Err(ConnectorError::Unsupported(format!(
                "runtime {:?} is not exec",
                manifest.runtime
            )));
        }
        let entry = manifest.entry.clone().unwrap_or_default();
        let p = Path::new(&entry);
        let program = if p.is_absolute() {
            p.to_path_buf()
        } else {
            dir.join(p)
        };
        if !program.is_file() {
            return Err(ConnectorError::InvalidManifest(format!(
                "entry {} does not exist",
                program.display()
            )));
        }
        Ok(ExecConnector {
            manifest,
            program,
            dir: dir.to_path_buf(),
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    async fn call(&self, op: &str, config: &Value, cursor: Option<&str>) -> Result<Vec<Response>> {
        let req = serde_json::to_string(&Request {
            op,
            api_version: API_VERSION,
            config,
            cursor,
        })
        .expect("request serialises");
        let mut child = Command::new(&self.program)
            .current_dir(&self.dir)
            .env("ANAMNESIS_CONNECTOR_API", API_VERSION.to_string())
            .env("ANAMNESIS_CONNECTOR_ID", &self.manifest.id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ConnectorError::io(&self.program, e))?;
        let mut stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let id = self.manifest.id.clone();

        let work = async {
            // A plugin that never reads stdin must not block us: ignore broken pipes.
            let _ = stdin.write_all(req.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            drop(stdin);
            let err_task = tokio::spawn(async move {
                // Keep the first 64 KiB, but drain everything so the plugin never blocks on it.
                let mut kept = Vec::new();
                let mut chunk = [0u8; 8192];
                while let Ok(n) = stderr.read(&mut chunk).await {
                    if n == 0 {
                        break;
                    }
                    let room = (64 * 1024usize).saturating_sub(kept.len());
                    kept.extend_from_slice(&chunk[..n.min(room)]);
                }
                String::from_utf8_lossy(&kept).into_owned()
            });
            let mut out = Vec::new();
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            let mut done = false;
            loop {
                line.clear();
                let n = (&mut reader)
                    .take(MAX_LINE_BYTES as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .await
                    .map_err(|e| ConnectorError::Plugin(format!("{id}: reading stdout: {e}")))?;
                if n == 0 {
                    break;
                }
                if line.len() > MAX_LINE_BYTES {
                    return Err(ConnectorError::Plugin(format!(
                        "{id}: response line longer than {MAX_LINE_BYTES} bytes"
                    )));
                }
                let text = String::from_utf8_lossy(&line);
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let resp: Response = serde_json::from_str(text).map_err(|e| {
                    ConnectorError::Plugin(format!(
                        "{id}: bad response line ({e}): {}",
                        truncate(text, 200)
                    ))
                })?;
                match &resp {
                    Response::Log { level, message } => {
                        tracing::info!(plugin = %id, level = level.as_deref().unwrap_or("info"), "{message}");
                        continue;
                    }
                    Response::Error { message } => {
                        return Err(ConnectorError::Plugin(format!("{id}: {message}")));
                    }
                    Response::Done { .. } | Response::Check { .. } => done = true,
                    _ => {}
                }
                out.push(resp);
            }
            let status = child
                .wait()
                .await
                .map_err(|e| ConnectorError::Plugin(format!("{id}: wait: {e}")))?;
            let err = err_task.await.unwrap_or_default();
            if !err.trim().is_empty() {
                tracing::debug!(plugin = %id, stderr = %truncate(&err, 2000), "plugin stderr");
            }
            if !status.success() {
                return Err(ConnectorError::Plugin(format!(
                    "{id}: exited with {status}: {}",
                    truncate(err.trim(), 500)
                )));
            }
            if !done {
                return Err(ConnectorError::Plugin(format!(
                    "{id}: exited without a final \"done\" or \"check\" line"
                )));
            }
            Ok(out)
        };
        match tokio::time::timeout(self.timeout, work).await {
            Ok(r) => r,
            Err(_) => Err(ConnectorError::Timeout(self.timeout)),
        }
        // On timeout the future (and with it `child`) is dropped; kill_on_drop kills the process.
    }

    fn to_document(&self, d: PluginDocument) -> Document {
        let mut meta = if d.meta.is_object() {
            d.meta
        } else {
            json!({ "plugin_meta": d.meta })
        };
        meta["connector"] = json!(self.manifest.id);
        meta["plugin_version"] = json!(self.manifest.version);
        Document {
            external_id: d.external_id,
            uri: d.uri,
            kind: d
                .kind
                .or(self.manifest.default_kind)
                .unwrap_or(ItemKind::Note),
            title: d.title,
            body: d.body,
            origin: d.origin.unwrap_or(self.manifest.default_origin),
            created_at: d.created_at,
            modified_at: d.modified_at,
            tags: d.tags,
            refs: d.refs,
            meta,
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[async_trait]
impl Connector for ExecConnector {
    fn manifest(&self) -> Manifest {
        self.manifest.clone()
    }

    async fn check(&self, config: &Value) -> Result<CheckReport> {
        let out = match self.call("check", config, None).await {
            Ok(o) => o,
            Err(e @ (ConnectorError::Plugin(_) | ConnectorError::Timeout(_))) => {
                return Ok(CheckReport::failed(e.to_string()));
            }
            Err(e) => return Err(e),
        };
        for r in out {
            if let Response::Check { ok, message } = r {
                return Ok(CheckReport { ok, message });
            }
        }
        Ok(CheckReport::failed("plugin did not answer the check"))
    }

    async fn list(&self, config: &Value, cursor: Option<&str>) -> Result<Batch> {
        let out = self.call("list", config, cursor).await?;
        let mut b = Batch::default();
        for r in out {
            match r {
                Response::Upsert { document } => b.upserts.push(self.to_document(document)),
                Response::Delete { external_id } => b.deletes.push(external_id),
                Response::Rename { from, to } => b.renames.push(Rename { from, to }),
                Response::Done {
                    next_cursor,
                    has_more,
                } => {
                    b.next_cursor = next_cursor;
                    b.has_more = has_more;
                }
                _ => {}
            }
        }
        Ok(b)
    }
}
