//! "installed cli (claude)": the user's own installed, unmodified `claude` binary.
//!
//! Rules this adapter keeps (see docs: https://code.claude.com/docs/en/legal-and-compliance,
//! https://code.claude.com/docs/en/cli-reference):
//! - it only runs a `claude` the user installed (found on PATH or at a configured path);
//!   it never downloads, bundles or patches one;
//! - sign-in happens in the user's own terminal; the app never starts a login;
//! - sign-in state comes only from the cli's own `claude auth status` command and from
//!   the cli's output; the app never opens the cli's stored login, config directory or
//!   any os keychain entry;
//! - it never sets or overrides the cli's entrypoint / attribution variables and never
//!   injects a token; one account only, no rotation, no failover to another account;
//! - interactive, user-initiated chat only: the router never sends background work here;
//! - plain chat runs with every built-in tool disabled (`--tools ""`), only the brain's
//!   own mcp server is offered (`--strict-mcp-config --mcp-config …`), and no
//!   permission-skipping flag is ever passed.
//!
//! Chat runs `claude -p --output-format stream-json --verbose --include-partial-messages`
//! with the prompt on stdin.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::backend::Backend;
use crate::error::{HarnessError, Result};
use crate::types::*;

pub const NOTICE: &str = "uses your own claude login through anthropic's official cli. anthropic has not explicitly approved third-party apps doing this and could restrict it at any time; the safest routes are the mcp server and an api key. anamnesis never reads your credentials.";
pub const SIGN_IN_HINT: &str = "unknown — run `claude` in your terminal to sign in";

/// What the user is told before turning on [`ClaudeCliConfig::relabel_traffic`].
pub const RELABEL_NOTICE: &str = "label requests as \"anamnesis\" instead of the cli's own label. claude code meters requests differently depending on how they say they were made; scripted (agent sdk) requests have been reported to count about twice as heavily against your plan limits. with this on, anamnesis's requests identify themselves as coming from anamnesis rather than as agent sdk calls, so they are likely to be metered like interactive use. anthropic has not documented this weighting or said whether third-party apps may do this. it is a grey area: it could be treated as getting around your plan's limits, and anthropic can restrict or suspend accounts without notice. it affects only your own account. anamnesis turns this off for everyone if anthropic says it is not allowed.";

/// The label sent when [`ClaudeCliConfig::relabel_traffic`] is on.
pub const RELABEL_ENTRYPOINT: &str = "anamnesis";

/// Project-wide kill switch for relabelling. Set to `true` (and release) the moment
/// anthropic says third-party apps may not do this; it overrides every user setting.
pub const RELABEL_WITHDRAWN: bool = false;

/// Environment variables the adapter removes from the child so that a value inherited
/// from the parent (e.g. anamnesis started from inside another claude session) does not
/// relabel the traffic. Only the opt-in [`ClaudeCliConfig::relabel_traffic`] sets one.
const SCRUBBED_ENV: &[&str] = &["CLAUDE_CODE_ENTRYPOINT", "CLAUDECODE"];

/// Variables that would silently move the cli off the user's subscription login: onto an
/// api key (billed per token), another endpoint, a cloud provider, or `--bare` mode (which
/// ignores the login). Removed from the child so "use my claude plan" means exactly that.
/// `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_CONFIG_DIR` are left alone: both are the user's
/// own subscription login, set by the user.
const AUTH_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_FOUNDRY_AUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_SIMPLE",
];

/// Settings passed with `--settings` when isolated: the user's hooks and auto memory stay
/// out of anamnesis chats.
const ISOLATION_SETTINGS: &str = r#"{"disableAllHooks":true,"autoMemoryEnabled":false}"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaudeCliConfig {
    #[serde(default = "default_id")]
    pub id: String,
    /// Opt-in: the user enabled this after reading the notice.
    #[serde(default)]
    pub enabled: bool,
    /// Global kill switch. When true the adapter refuses every call, whatever else is set.
    #[serde(default)]
    pub disabled_by_policy: bool,
    /// Opt-in, off by default: label requests as [`RELABEL_ENTRYPOINT`] instead of the
    /// cli's own label. Grey area; the user enables it after reading [`RELABEL_NOTICE`].
    #[serde(default)]
    pub relabel_traffic: bool,
    /// Kill switch for relabelling alone (the route keeps working without it).
    #[serde(default)]
    pub relabel_disabled_by_policy: bool,
    /// Keep the user's own claude code setup (CLAUDE.md, auto memory, hooks, skills,
    /// slash commands) out of anamnesis chats. On by default.
    #[serde(default = "yes")]
    pub isolate: bool,
    /// Explicit path to the user's `claude`. Default: search PATH.
    #[serde(default)]
    pub binary: Option<PathBuf>,
    /// Model alias or id passed as `--model`; default: the cli's own default.
    #[serde(default)]
    pub model: Option<String>,
    /// Whether `available()` may run `claude auth status`.
    #[serde(default = "yes")]
    pub check_auth_status: bool,
    /// Working directory for the child (an empty directory, so no project files load).
    #[serde(default)]
    pub work_dir: Option<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// PATH to search instead of the process's PATH (tests, embedding). Not persisted.
    #[serde(skip)]
    pub search_path: Option<OsString>,
}

fn default_id() -> String {
    "claude-cli".into()
}
fn yes() -> bool {
    true
}
fn default_timeout() -> u64 {
    600
}

impl Default for ClaudeCliConfig {
    fn default() -> Self {
        Self {
            id: default_id(),
            enabled: false,
            disabled_by_policy: false,
            relabel_traffic: false,
            relabel_disabled_by_policy: false,
            isolate: true,
            binary: None,
            model: None,
            check_auth_status: true,
            work_dir: None,
            timeout_secs: default_timeout(),
            search_path: None,
        }
    }
}

/// The brain's own mcp server, offered to the cli during chat.
#[derive(Debug, Clone, PartialEq)]
pub struct McpOffer {
    /// Json string or path accepted by `--mcp-config`.
    pub config: String,
    /// Server name inside that config; its tools are allowed as `mcp__<name>`.
    pub server_name: String,
}

/// Sign-in state as the cli itself reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignInStatus {
    /// `Some(true/false)` from `claude auth status`; `None` when it could not tell.
    pub logged_in: Option<bool>,
    /// The cli's `authMethod` field, if it reported one (e.g. "claude.ai").
    pub auth_method: Option<String>,
    pub message: String,
}

/// Find an executable named `name` in a PATH-style list. On windows every extension in
/// `PATHEXT` that can be started (`.exe`, `.com`, `.cmd`, `.bat`) is tried, in that
/// variable's order, so a native `claude.exe` wins over an npm `claude.cmd` shim in the
/// same directory.
pub fn which_in(name: &str, path: &OsStr) -> Option<PathBuf> {
    let exts = exe_extensions();
    for dir in std::env::split_paths(path) {
        for cand in candidates(&dir, name, &exts) {
            if is_executable(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// Extensions tried by [`which_in`]: empty on unix (the bare name), from `PATHEXT` on
/// windows.
fn exe_extensions() -> Vec<String> {
    if cfg!(windows) {
        let raw = std::env::var("PATHEXT").unwrap_or_default();
        pathext_list(&raw)
    } else {
        Vec::new()
    }
}

/// The startable extensions in a `PATHEXT` value, lowercased and without the dot, in
/// order; the windows default when the value names none of them.
pub(crate) fn pathext_list(raw: &str) -> Vec<String> {
    const STARTABLE: &[&str] = &["exe", "com", "cmd", "bat"];
    let mut out: Vec<String> = Vec::new();
    for e in raw.split(';') {
        let e = e.trim().trim_start_matches('.').to_ascii_lowercase();
        if STARTABLE.contains(&e.as_str()) && !out.contains(&e) {
            out.push(e);
        }
    }
    if out.is_empty() {
        out = ["com", "exe", "bat", "cmd"].map(String::from).to_vec();
    }
    out
}

fn candidates(dir: &Path, name: &str, exts: &[String]) -> Vec<PathBuf> {
    if exts.is_empty() {
        vec![dir.join(name)]
    } else {
        exts.iter()
            .map(|e| dir.join(format!("{name}.{e}")))
            .collect()
    }
}

/// Where installers put `claude` when it is not on the server's PATH (a server started
/// before the install, or from a shortcut, can have an older PATH than a new terminal).
/// Only binary locations; nothing in them is read.
fn fallback_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        // the native installer, then npm's global prefix
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            dirs.push(PathBuf::from(profile).join(".local").join("bin"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
    } else {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

fn is_executable(p: &Path) -> bool {
    let Ok(md) = std::fs::metadata(p) else {
        return false;
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// How a located `claude` is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Launch {
    /// Run the file itself: a native `claude.exe`, or any unix executable.
    Direct(PathBuf),
    /// An npm shim (`claude.cmd`) resolved to `node <script>`, so no cmd.exe is involved
    /// and arguments reach the cli exactly as given.
    Node { node: PathBuf, script: PathBuf },
    /// A batch file that could not be resolved. Started through std's batch-file support
    /// (cmd.exe), which quotes every argument and refuses, rather than passes on, one it
    /// cannot quote safely. Multi-line arguments therefore go through files.
    Batch(PathBuf),
}

impl Launch {
    /// Decide how to start `bin`. `path` is the PATH used to find `node` when the shim's
    /// directory has none.
    pub(crate) fn for_binary(bin: &Path, path: Option<&OsStr>) -> Launch {
        let ext = bin
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "cmd" && ext != "bat" {
            return Launch::Direct(bin.to_path_buf());
        }
        match resolve_npm_shim(bin, path) {
            Some((node, script)) => Launch::Node { node, script },
            None => Launch::Batch(bin.to_path_buf()),
        }
    }

    fn command(&self) -> Command {
        match self {
            Launch::Direct(p) | Launch::Batch(p) => Command::new(p),
            Launch::Node { node, script } => {
                let mut c = Command::new(node);
                c.arg(script);
                c
            }
        }
    }

    fn is_batch(&self) -> bool {
        matches!(self, Launch::Batch(_))
    }
}

/// Resolve an npm / pnpm / yarn `.cmd` shim to the javascript file it runs and the node
/// that should run it. The shim's own text names the script relative to its directory
/// (`"%dp0%\node_modules\@anthropic-ai\claude-code\cli.js"` for npm, `%~dp0\…` for
/// others); failing that, the package's usual place next to the shim is tried.
pub(crate) fn resolve_npm_shim(shim: &Path, path: Option<&OsStr>) -> Option<(PathBuf, PathBuf)> {
    let dir = shim.parent()?;
    let text = std::fs::read_to_string(shim).unwrap_or_default();
    let from_text = text.split('"').find_map(|part| {
        let rel = part
            .strip_prefix("%dp0%\\")
            .or_else(|| part.strip_prefix("%~dp0\\"))
            .or_else(|| part.strip_prefix("%~dp0"))?;
        let lower = rel.to_ascii_lowercase();
        if !(lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".cjs")) {
            return None;
        }
        let rel: PathBuf = rel.split(['\\', '/']).filter(|c| !c.is_empty()).collect();
        Some(dir.join(rel))
    });
    let script = from_text
        .into_iter()
        .chain(std::iter::once(
            dir.join("node_modules")
                .join("@anthropic-ai")
                .join("claude-code")
                .join("cli.js"),
        ))
        .find(|p| p.is_file())?;
    // npm shims prefer a node.exe sitting next to them, then node on PATH
    let local = dir.join(if cfg!(windows) { "node.exe" } else { "node" });
    let node = if local.is_file() {
        local
    } else {
        which_in("node", path?)?
    };
    Some((node, script))
}

/// Deletes a file when dropped (the system prompt handed to a batch-launched cli).
struct RemoveOnDrop(Option<PathBuf>);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub struct ClaudeCli {
    cfg: ClaudeCliConfig,
    mcp: Option<McpOffer>,
}

impl ClaudeCli {
    pub fn new(cfg: ClaudeCliConfig) -> Self {
        Self { cfg, mcp: None }
    }

    /// Offer the brain's mcp server to the cli during chat.
    pub fn with_mcp(mut self, offer: Option<McpOffer>) -> Self {
        self.mcp = offer;
        self
    }

    pub fn config(&self) -> &ClaudeCliConfig {
        &self.cfg
    }

    /// Whether requests are relabelled: the user opted in and neither kill switch is set.
    pub fn relabels(&self) -> bool {
        self.cfg.relabel_traffic && !self.cfg.relabel_disabled_by_policy && !RELABEL_WITHDRAWN
    }

    /// The only variables the adapter ever sets on the child. The entrypoint label is
    /// present only when the user opted in to relabelling ([`Self::relabels`]).
    pub fn child_env(&self) -> Vec<(&'static str, &'static str)> {
        let mut env = Vec::new();
        if self.relabels() {
            env.push(("CLAUDE_CODE_ENTRYPOINT", RELABEL_ENTRYPOINT));
        }
        if self.cfg.isolate {
            env.push(("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"));
            env.push(("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1"));
        }
        env
    }

    fn policy_check(&self) -> Result<()> {
        if self.cfg.disabled_by_policy {
            return Err(HarnessError::DisabledByPolicy(
                "the installed claude cli route is turned off".into(),
            ));
        }
        if !self.cfg.enabled {
            return Err(HarnessError::NotConfigured(
                "installed claude cli is not enabled".into(),
            ));
        }
        Ok(())
    }

    /// Locate the user's installed `claude`.
    pub fn locate(&self) -> Option<PathBuf> {
        if let Some(b) = &self.cfg.binary {
            return is_executable(b).then(|| b.clone());
        }
        if let Some(p) = &self.cfg.search_path {
            // an explicit search path is the whole search (tests, locked-down setups)
            return which_in("claude", p);
        }
        if let Some(found) = std::env::var_os("PATH").and_then(|p| which_in("claude", &p)) {
            return Some(found);
        }
        let extra = std::env::join_paths(fallback_dirs()).ok()?;
        which_in("claude", &extra)
    }

    /// How to start `bin` (resolves windows npm shims to `node <script>`).
    pub(crate) fn launch(&self, bin: &Path) -> Launch {
        let path = self
            .cfg
            .search_path
            .clone()
            .or_else(|| std::env::var_os("PATH"));
        Launch::for_binary(bin, path.as_deref())
    }

    fn command(&self, bin: &Path) -> Command {
        let mut c = self.launch(bin).command();
        for k in SCRUBBED_ENV.iter().chain(AUTH_ENV) {
            c.env_remove(k);
        }
        for (k, v) in self.child_env() {
            c.env(k, v);
        }
        if let Some(p) = &self.cfg.search_path {
            c.env("PATH", p);
        }
        c.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        c
    }

    #[cfg(all(test, unix))]
    pub(crate) fn command_for_test(&self, bin: &Path) -> Command {
        self.command(bin)
    }

    /// `claude --version`.
    pub async fn version(&self) -> Option<String> {
        let bin = self.locate()?;
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            self.command(&bin).arg("--version").output(),
        )
        .await
        .ok()?
        .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Sign-in state from the cli's own `claude auth status` (json; exit 0 when logged in,
    /// 1 when not). Older clis without the command report "unknown".
    pub async fn sign_in_status(&self) -> SignInStatus {
        let unknown = || SignInStatus {
            logged_in: None,
            auth_method: None,
            message: SIGN_IN_HINT.into(),
        };
        let Some(bin) = self.locate() else {
            return unknown();
        };
        let fut = self.command(&bin).args(["auth", "status"]).output();
        let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(15), fut).await else {
            return unknown();
        };
        parse_auth_status(out.status.code(), &String::from_utf8_lossy(&out.stdout))
            .unwrap_or_else(unknown)
    }

    pub(crate) fn args(&self, req: &ChatRequest) -> Vec<String> {
        let mut a: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--no-session-persistence",
            "--tools",
            "",
            "--strict-mcp-config",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if self.cfg.isolate {
            a.extend([
                "--disable-slash-commands".into(),
                "--settings".into(),
                ISOLATION_SETTINGS.into(),
            ]);
        }
        if let Some(m) = req.model.as_ref().or(self.cfg.model.as_ref()) {
            a.extend(["--model".into(), m.clone()]);
        }
        if let Some(s) = &req.system {
            a.extend(["--system-prompt".into(), s.clone()]);
        }
        if let Some(mcp) = &self.mcp {
            a.extend(["--mcp-config".into(), mcp.config.clone()]);
            a.extend(["--allowedTools".into(), format!("mcp__{}", mcp.server_name)]);
        }
        a
    }
}

/// Render the conversation for `-p` (a single prompt on stdin).
pub(crate) fn render_prompt(req: &ChatRequest) -> String {
    let n = req.messages.len();
    if n == 1 {
        return req.messages[0].content.clone();
    }
    let mut s = String::new();
    if n > 1 {
        s.push_str("<conversation_so_far>\n");
        for m in &req.messages[..n - 1] {
            let who = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            s.push_str(&format!("<{who}>\n{}\n</{who}>\n", m.content));
        }
        s.push_str("</conversation_so_far>\n\n");
    }
    if let Some(last) = req.messages.last() {
        s.push_str(&last.content);
    }
    s
}

pub(crate) fn parse_auth_status(code: Option<i32>, stdout: &str) -> Option<SignInStatus> {
    let v: Value = serde_json::from_str(stdout.trim()).ok()?;
    let logged_in = v.get("loggedIn").and_then(|b| b.as_bool()).or(match code {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    })?;
    let auth_method = v
        .get("authMethod")
        .and_then(|m| m.as_str())
        .map(str::to_string);
    let message = if logged_in {
        match auth_method.as_deref() {
            Some(m) => format!("signed in ({m})"),
            None => "signed in".into(),
        }
    } else {
        "not signed in — run `claude` in your terminal to sign in".into()
    };
    Some(SignInStatus {
        logged_in: Some(logged_in),
        auth_method,
        message,
    })
}

/// Recognise the cli's usage-limit / auth messages in result or error text.
pub fn classify_cli_message(text: &str) -> Option<FailureKind> {
    let t = text.to_lowercase();
    // older clis: "Claude AI usage limit reached|1731340800"
    if let Some(rest) = text.split_once("usage limit reached|").map(|(_, r)| r) {
        let ts = rest
            .trim()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|d| d.parse::<i64>().ok());
        return Some(FailureKind::UsageLimit {
            resets_at: ts.and_then(|s| Utc.timestamp_opt(s, 0).single()),
        });
    }
    if t.contains("usage limit")
        || t.contains("hit your limit")
        || t.contains("limit reached")
        || t.contains("out of extra usage")
        || t.contains("weekly limit")
    {
        return Some(FailureKind::UsageLimit { resets_at: None });
    }
    if t.contains("rate limit") || t.contains("rate_limit") || t.contains("429") {
        return Some(FailureKind::RateLimited {
            retry_after_secs: None,
        });
    }
    if t.contains("not logged in")
        || t.contains("please run /login")
        || t.contains("invalid api key")
        || t.contains("authentication_error")
        || t.contains("oauth token has expired")
    {
        return Some(FailureKind::Auth);
    }
    if t.contains("overloaded") {
        return Some(FailureKind::Unavailable);
    }
    None
}

/// Decodes the cli's stream-json lines.
#[derive(Default)]
pub(crate) struct StreamJsonDecoder {
    saw_partial: bool,
    out_chars: u64,
    est_input: u64,
    done: bool,
    pending_limit: Option<FailureKind>,
}

impl StreamJsonDecoder {
    pub(crate) fn new(est_input: u64) -> Self {
        Self {
            est_input,
            ..Default::default()
        }
    }

    pub(crate) fn line(&mut self, line: &str) -> Vec<ChatEvent> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "stream_event" => {
                let ev = v.get("event").unwrap_or(&Value::Null);
                if ev.get("type").and_then(|t| t.as_str()) == Some("content_block_delta")
                    && ev.pointer("/delta/type").and_then(|t| t.as_str()) == Some("text_delta")
                    && let Some(t) = ev.pointer("/delta/text").and_then(|t| t.as_str())
                {
                    self.saw_partial = true;
                    self.out_chars += t.chars().count() as u64;
                    return vec![ChatEvent::Token {
                        text: t.to_string(),
                    }];
                }
                Vec::new()
            }
            "assistant" => {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    if self.pending_limit.is_none() {
                        self.pending_limit = classify_cli_message(err);
                    }
                    // an error message is not a reply: don't show its text as one
                    return Vec::new();
                }
                if self.saw_partial {
                    return Vec::new();
                }
                let mut out = Vec::new();
                if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                    for b in content {
                        if b.get("type").and_then(|t| t.as_str()) == Some("text")
                            && let Some(t) = b.get("text").and_then(|t| t.as_str())
                        {
                            self.out_chars += t.chars().count() as u64;
                            out.push(ChatEvent::Token {
                                text: t.to_string(),
                            });
                        }
                    }
                }
                out
            }
            "rate_limit_event" => {
                let info = v.get("rate_limit_info").unwrap_or(&v);
                if info.get("status").and_then(|s| s.as_str()) == Some("rejected") {
                    let resets_at = info
                        .get("resetsAt")
                        .and_then(|r| r.as_i64())
                        .and_then(|s| Utc.timestamp_opt(s, 0).single());
                    self.pending_limit = Some(FailureKind::UsageLimit { resets_at });
                }
                Vec::new()
            }
            "result" => {
                self.done = true;
                let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false)
                    || v.get("subtype")
                        .and_then(|s| s.as_str())
                        .is_some_and(|s| s != "success");
                let text = v.get("result").and_then(|r| r.as_str()).unwrap_or("");
                let usage = self.usage_from(&v);
                if is_error {
                    let from_text = classify_cli_message(text);
                    let failure = match (self.pending_limit.take(), from_text) {
                        // a stated reset time from the cli's rate-limit event wins
                        (Some(p @ FailureKind::UsageLimit { resets_at: Some(_) }), _) => p,
                        (_, Some(t)) => t,
                        (Some(p), None) => p,
                        (None, None) => FailureKind::Other,
                    };
                    let message = if text.is_empty() {
                        "the claude cli reported an error".to_string()
                    } else {
                        text.to_string()
                    };
                    return vec![
                        ChatEvent::Usage(usage),
                        ChatEvent::Error { message, failure },
                    ];
                }
                // if nothing streamed (no partials, no assistant text), use the result text
                let mut out = Vec::new();
                if self.out_chars == 0 && !text.is_empty() {
                    out.push(ChatEvent::Token {
                        text: text.to_string(),
                    });
                }
                out.push(ChatEvent::Usage(usage));
                out.push(ChatEvent::Done {
                    stop_reason: v
                        .get("stop_reason")
                        .and_then(|s| s.as_str())
                        .map(str::to_string),
                });
                out
            }
            _ => Vec::new(),
        }
    }

    fn usage_from(&self, v: &Value) -> Usage {
        let u = v.get("usage");
        let get = |k: &str| u.and_then(|u| u.get(k)).and_then(|x| x.as_u64());
        let input = get("input_tokens").map(|i| {
            i + get("cache_creation_input_tokens").unwrap_or(0)
                + get("cache_read_input_tokens").unwrap_or(0)
        });
        Usage {
            input: input.unwrap_or(self.est_input),
            output: get("output_tokens").unwrap_or(self.out_chars.div_ceil(4)),
            // plan usage: the token counts are the cli's, the cost is an api-equivalent estimate
            estimated: true,
            reported_cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()),
        }
    }

    /// End of stdout without a `result` line.
    pub(crate) fn eof(&mut self, exit_ok: bool, stderr: &str) -> Vec<ChatEvent> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let failure = classify_cli_message(stderr)
            .or(self.pending_limit.take())
            .unwrap_or(if exit_ok {
                FailureKind::Other
            } else {
                FailureKind::Unavailable
            });
        let tail: String = stderr
            .trim()
            .chars()
            .rev()
            .take(300)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let message = if tail.is_empty() {
            "the claude cli exited without a result".to_string()
        } else {
            tail
        };
        vec![ChatEvent::Error { message, failure }]
    }
}

#[async_trait]
impl Backend for ClaudeCli {
    fn id(&self) -> &str {
        &self.cfg.id
    }
    fn kind(&self) -> BackendKind {
        BackendKind::ClaudeCli
    }
    fn label(&self) -> String {
        "installed cli (claude) — not affiliated with anthropic".into()
    }
    fn default_model(&self) -> &str {
        self.cfg.model.as_deref().unwrap_or("default")
    }
    fn provider(&self) -> &str {
        "anthropic (via installed claude cli)"
    }
    fn capabilities(&self) -> Capabilities {
        let mut c = Capabilities::base(CostModel::Subscription);
        c.background_allowed = false;
        c.tools = self.mcp.is_some();
        c.json_schema = false;
        c.forbidden_params = vec![
            "temperature".into(),
            "max_tokens".into(),
            "json_schema".into(),
        ];
        c.terms = Terms::Grey;
        c.notice = Some(NOTICE.into());
        c
    }
    fn is_local(&self) -> bool {
        false
    }
    async fn available(&self) -> Availability {
        if self.cfg.disabled_by_policy {
            return Availability::down("disabled by policy");
        }
        let Some(version) = self.version().await else {
            return Availability::down("claude cli not found on PATH");
        };
        if !self.cfg.enabled {
            return Availability::down(format!("found {version}; not enabled"));
        }
        if !self.cfg.check_auth_status {
            return Availability {
                available: true,
                status: format!("{version}; sign-in {SIGN_IN_HINT}"),
                models: vec![],
            };
        }
        let st = self.sign_in_status().await;
        Availability {
            // "unknown" still counts as available: the first call reports a sign-in error.
            available: st.logged_in != Some(false),
            status: format!("{version}; {}", st.message),
            models: vec![],
        }
    }
    async fn chat(&self, req: ChatRequest) -> Result<ChatStream> {
        self.policy_check()?;
        let bin = self
            .locate()
            .ok_or_else(|| HarnessError::Unavailable("claude cli not found on PATH".into()))?;
        let mut cmd = self.command(&bin);
        let dir = match &self.cfg.work_dir {
            Some(d) => d.clone(),
            None => std::env::temp_dir().join("anamnesis-claude-cli"),
        };
        std::fs::create_dir_all(&dir)?;
        let mut args = self.args(&req);
        let mut prompt_file = RemoveOnDrop(None);
        if self.launch(&bin).is_batch() {
            // cmd.exe cannot carry a line break inside an argument: hand the system
            // prompt over as a file instead (the prompt itself always goes on stdin)
            if let Some(i) = args.iter().position(|a| a == "--system-prompt") {
                let file = dir.join(format!("system-prompt-{}.txt", uuid::Uuid::new_v4()));
                std::fs::write(&file, args[i + 1].as_bytes())?;
                args[i] = "--system-prompt-file".into();
                args[i + 1] = file.display().to_string();
                prompt_file.0 = Some(file);
            }
        }
        cmd.args(&args).stdin(Stdio::piped());
        cmd.current_dir(&dir);
        let mut child = cmd.spawn()?;
        let prompt = render_prompt(&req);
        let mut stdin = child.stdin.take().expect("stdin piped");
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        let stdout = child.stdout.take().expect("stdout piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr piped");
        let stderr = Some(tokio::spawn(async move {
            // the system-prompt file (batch launch) lives until the cli closes stderr,
            // i.e. until it exits
            let _prompt_file = prompt_file;
            let mut buf = Vec::new();
            let _ = (&mut stderr_pipe)
                .take(64 * 1024)
                .read_to_end(&mut buf)
                .await;
            String::from_utf8_lossy(&buf).into_owned()
        }));
        let lines = tokio_lines(BufReader::new(stdout));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(self.cfg.timeout_secs);
        let dec = StreamJsonDecoder::new(req.estimated_input_tokens());
        let state = (
            Box::pin(lines),
            dec,
            std::collections::VecDeque::new(),
            Some(child),
            stderr,
            false,
        );
        let s = futures_util::stream::unfold(
            state,
            move |(mut lines, mut dec, mut q, mut child, mut stderr, mut ended)| async move {
                loop {
                    if let Some(ev) = q.pop_front() {
                        if crate::decode::is_terminal(&ev) {
                            ended = true;
                            q.clear();
                        }
                        return Some((ev, (lines, dec, q, child, stderr, ended)));
                    }
                    if ended {
                        if let Some(mut c) = child.take() {
                            let _ = tokio::time::timeout(Duration::from_secs(5), c.wait()).await;
                        }
                        return None;
                    }
                    match tokio::time::timeout_at(deadline, lines.next()).await {
                        Err(_) => {
                            q.push_back(ChatEvent::Error {
                                message: "the claude cli timed out".into(),
                                failure: FailureKind::Unavailable,
                            });
                        }
                        Ok(Some(line)) => q.extend(dec.line(&line)),
                        Ok(None) => {
                            let ok = match child.as_mut() {
                                Some(c) => matches!(
                                    tokio::time::timeout(Duration::from_secs(5), c.wait()).await,
                                    Ok(Ok(st)) if st.success()
                                ),
                                None => false,
                            };
                            // the stderr reader finishes once the process has exited
                            let err = match stderr.take() {
                                Some(h) => tokio::time::timeout(Duration::from_secs(2), h)
                                    .await
                                    .ok()
                                    .and_then(|r| r.ok())
                                    .unwrap_or_default(),
                                None => String::new(),
                            };
                            q.extend(dec.eof(ok, &err));
                            ended = true;
                        }
                    }
                }
            },
        );
        Ok(Box::pin(s))
    }
}

fn tokio_lines<R: tokio::io::AsyncBufRead + Unpin + Send + 'static>(
    r: R,
) -> impl futures_util::Stream<Item = String> + Send {
    futures_util::stream::unfold(r.lines(), |mut l| async move {
        match l.next_line().await {
            Ok(Some(s)) => Some((s, l)),
            _ => None,
        }
    })
}

#[cfg(test)]
mod launch_tests {
    use super::*;

    /// npm's generated `claude.cmd` (abridged).
    const NPM_SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\cli.js\" %*\r\n";

    #[test]
    fn pathext_order_and_default() {
        assert_eq!(
            pathext_list(".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WSH;.MSC"),
            vec!["com", "exe", "bat", "cmd"]
        );
        assert_eq!(pathext_list(".exe;.CMD;.exe"), vec!["exe", "cmd"]);
        assert_eq!(pathext_list(""), vec!["com", "exe", "bat", "cmd"]);
    }

    #[test]
    fn exe_wins_over_cmd_in_the_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["claude.cmd", "claude.exe"] {
            std::fs::write(dir.path().join(f), "x").unwrap();
        }
        let exts = pathext_list(".COM;.EXE;.BAT;.CMD");
        let found = candidates(dir.path(), "claude", &exts)
            .into_iter()
            .find(|p| p.is_file())
            .unwrap();
        assert_eq!(found.file_name().unwrap(), "claude.exe");
    }

    #[test]
    fn exe_and_unix_binaries_run_directly() {
        let l = Launch::for_binary(Path::new("/x/claude.exe"), None);
        assert_eq!(l, Launch::Direct(PathBuf::from("/x/claude.exe")));
        let l = Launch::for_binary(Path::new("/usr/local/bin/claude"), None);
        assert!(matches!(l, Launch::Direct(_)));
    }

    #[test]
    fn npm_shim_resolves_to_node_and_the_script() {
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("claude.cmd");
        std::fs::write(&shim, NPM_SHIM).unwrap();
        let pkg = dir
            .path()
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("cli.js"), "// cli").unwrap();
        let node = dir
            .path()
            .join(if cfg!(windows) { "node.exe" } else { "node" });
        std::fs::write(&node, "x").unwrap();
        match Launch::for_binary(&shim, None) {
            Launch::Node { node: n, script } => {
                assert_eq!(n, node);
                assert_eq!(script, pkg.join("cli.js"));
            }
            other => panic!("expected node launch, got {other:?}"),
        }
    }

    #[test]
    fn unknown_batch_file_stays_a_batch_launch() {
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("claude.bat");
        std::fs::write(&shim, "@echo off\r\necho hi\r\n").unwrap();
        assert_eq!(
            Launch::for_binary(&shim, Some(OsStr::new(""))),
            Launch::Batch(shim.clone())
        );
        assert!(Launch::Batch(shim).is_batch());
    }
}

/// A fake `claude.cmd` run through cmd.exe (windows only).
#[cfg(all(test, windows))]
mod windows_batch_tests {
    use super::*;
    use crate::testutil::collect;

    #[tokio::test]
    async fn batch_cli_streams_and_gets_the_system_prompt_as_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = concat!(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello there"}}}"#,
            "\r\n",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"hello there","usage":{"input_tokens":3,"output_tokens":2}}"#,
            "\r\n"
        );
        std::fs::write(dir.path().join("fixture.jsonl"), fixture).unwrap();
        let args_log = dir.path().join("args.txt");
        let script = format!(
            "@echo off\r\nif \"%~1\"==\"--version\" (\r\n  echo 9.9.9 (Claude Code)\r\n  exit /b 0\r\n)\r\necho %* > \"{}\"\r\ntype \"%~dp0fixture.jsonl\"\r\n",
            args_log.display()
        );
        std::fs::write(dir.path().join("claude.cmd"), script).unwrap();
        let cfg = ClaudeCliConfig {
            enabled: true,
            search_path: Some(dir.path().as_os_str().to_owned()),
            work_dir: Some(dir.path().join("work")),
            check_auth_status: false,
            ..ClaudeCliConfig::default()
        };
        let cli = ClaudeCli::new(cfg);
        let bin = cli.locate().expect("finds claude.cmd");
        assert!(cli.launch(&bin).is_batch());
        let mut req = ChatRequest::new(vec![Message::user("hi & bye | 100%")]);
        req.system = Some("line one\nline \"two\" & more".into());
        let (text, _events) = collect(cli.chat(req).await.unwrap()).await;
        assert_eq!(text, "hello there");
        let logged = std::fs::read_to_string(&args_log).unwrap();
        assert!(logged.contains("--system-prompt-file"), "{logged}");
        assert!(!logged.contains("line one"), "{logged}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_limit_messages() {
        assert_eq!(
            classify_cli_message("Claude AI usage limit reached|1731340800"),
            Some(FailureKind::UsageLimit {
                resets_at: Utc.timestamp_opt(1731340800, 0).single()
            })
        );
        assert!(matches!(
            classify_cli_message("You've hit your limit · resets 3pm (Europe/Berlin)"),
            Some(FailureKind::UsageLimit { resets_at: None })
        ));
        assert_eq!(
            classify_cli_message("Not logged in · Please run /login"),
            Some(FailureKind::Auth)
        );
        assert_eq!(classify_cli_message("hello"), None);
    }

    #[test]
    fn auth_status_parsing() {
        let s =
            parse_auth_status(Some(0), r#"{"loggedIn":true,"authMethod":"claude.ai"}"#).unwrap();
        assert_eq!(s.logged_in, Some(true));
        assert_eq!(s.auth_method.as_deref(), Some("claude.ai"));
        let s = parse_auth_status(Some(1), r#"{"authMethod":"none"}"#).unwrap();
        assert_eq!(s.logged_in, Some(false));
        assert!(parse_auth_status(Some(2), "error: unknown command 'auth'").is_none());
    }

    #[test]
    fn prompt_rendering() {
        let r = ChatRequest::new(vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
        ]);
        let p = render_prompt(&r);
        assert!(p.starts_with("<conversation_so_far>"));
        assert!(p.ends_with("\nc"));
        assert_eq!(
            render_prompt(&ChatRequest::new(vec![Message::user("x")])),
            "x"
        );
    }

    #[test]
    fn args_disable_tools_and_never_skip_permissions() {
        let cli = ClaudeCli::new(ClaudeCliConfig {
            enabled: true,
            ..Default::default()
        });
        let a = cli.args(&ChatRequest::new(vec![Message::user("x")]));
        let i = a.iter().position(|s| s == "--tools").unwrap();
        assert_eq!(a[i + 1], "");
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert!(
            !a.iter()
                .any(|s| s.contains("dangerously") || s.contains("bypass") || s == "--bare")
        );
        let cli = cli.with_mcp(Some(McpOffer {
            config: "{}".into(),
            server_name: "anamnesis".into(),
        }));
        let a = cli.args(&ChatRequest::new(vec![Message::user("x")]));
        assert!(
            a.windows(2)
                .any(|w| w[0] == "--allowedTools" && w[1] == "mcp__anamnesis")
        );
    }
}
