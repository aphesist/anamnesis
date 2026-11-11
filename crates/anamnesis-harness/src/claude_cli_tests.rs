//! The installed-claude-cli adapter against a fake `claude` script on a temp PATH.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};

use crate::adapters::claude_cli::{ClaudeCli, ClaudeCliConfig, NOTICE, SIGN_IN_HINT};
use crate::backend::Backend;
use crate::error::HarnessError;
use crate::testutil::collect;
use crate::types::*;

const SUCCESS: &str = r#"{"type":"system","subtype":"init","session_id":"s1","tools":[],"mcp_servers":[],"model":"claude-sonnet-5-5"}
{"type":"stream_event","event":{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello "}}}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"there"}}}
{"type":"assistant","message":{"content":[{"type":"text","text":"hello there"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"hello there","total_cost_usd":0.0123,"usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":5}}
"#;

const NO_PARTIALS: &str = r#"{"type":"system","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"whole reply"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"whole reply","usage":{"input_tokens":3,"output_tokens":2}}
"#;

const LIMIT: &str = r#"{"type":"system","subtype":"init"}
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1900000000,"rateLimitType":"five_hour"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"You've hit your limit · resets 3pm"}]},"error":"rate_limit"}
{"type":"result","subtype":"success","is_error":true,"result":"You've hit your limit · resets 3pm"}
"#;

const NOT_LOGGED_IN: &str = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}
"#;

struct Fake {
    _dir: tempfile::TempDir,
    bin_dir: PathBuf,
    log: PathBuf,
    work: PathBuf,
}

/// Write a fake `claude` that logs its args, env and stdin, then prints `fixture`.
fn fake(fixture: &str, exit: i32, auth: &str) -> Fake {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    let log = dir.path().join("log");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(&log).unwrap();
    std::fs::write(dir.path().join("fixture"), fixture).unwrap();
    let script = format!(
        r#"#!/bin/sh
L="{log}"
case "$1" in
  --version) echo "2.1.300 (Claude Code)"; exit 0;;
  auth) {auth}
esac
printf '%s\n' "$@" > "$L/args"
echo "entry=${{CLAUDE_CODE_ENTRYPOINT:-unset}}" > "$L/env"
pwd > "$L/cwd"
cat > "$L/stdin"
cat "{fixture}"
echo "fake stderr line" >&2
exit {exit}
"#,
        log = log.display(),
        fixture = dir.path().join("fixture").display(),
    );
    let p = bin_dir.join("claude");
    std::fs::write(&p, script).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fake {
        _dir: dir,
        bin_dir,
        log,
        work,
    }
}

const AUTH_OK: &str = r#"echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0;;"#;
const AUTH_NO: &str = r#"echo '{"loggedIn":false,"authMethod":"none"}'; exit 1;;"#;
const AUTH_MISSING: &str = r#"echo "error: unknown command 'auth'" >&2; exit 2;;"#;

fn cli(f: &Fake) -> ClaudeCli {
    ClaudeCli::new(ClaudeCliConfig {
        enabled: true,
        search_path: Some(format!("{}:/usr/bin:/bin", f.bin_dir.display()).into()),
        work_dir: Some(f.work.clone()),
        timeout_secs: 20,
        ..Default::default()
    })
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

#[tokio::test]
async fn streams_partial_messages_and_reports_plan_usage() {
    let f = fake(SUCCESS, 0, AUTH_OK);
    let b = cli(&f);
    let mut r = ChatRequest::new(vec![
        Message::user("first"),
        Message::assistant("ok"),
        Message::user("what now?"),
    ])
    .with_system("you are the brain");
    r.temperature = Some(0.9);
    let (text, evs) = collect(b.chat(r).await.unwrap()).await;
    assert_eq!(
        text, "hello there",
        "partials are used, the full assistant message is not duplicated"
    );
    let usage = evs
        .iter()
        .find_map(|e| {
            if let ChatEvent::Usage(u) = e {
                Some(*u)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(usage.input, 100);
    assert_eq!(usage.output, 5);
    assert!(usage.estimated, "plan usage is always marked estimated");
    assert_eq!(usage.reported_cost_usd, Some(0.0123));
    assert!(matches!(evs.last(), Some(ChatEvent::Done { .. })));

    let args: Vec<String> = read(&f.log.join("args"))
        .lines()
        .map(str::to_string)
        .collect();
    for want in [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--strict-mcp-config",
        "--no-session-persistence",
    ] {
        assert!(args.iter().any(|a| a == want), "missing {want}: {args:?}");
    }
    let i = args.iter().position(|a| a == "--tools").unwrap();
    assert_eq!(args[i + 1], "", "all built-in tools disabled");
    assert!(
        !args
            .iter()
            .any(|a| a.contains("dangerously") || a.contains("bypassPermissions") || a == "--bare")
    );
    assert!(
        !args
            .iter()
            .any(|a| a.contains(".claude") || a.contains("credentials"))
    );
    assert!(
        !args.iter().any(|a| a.contains("0.9")),
        "temperature is not passed"
    );
    let i = args.iter().position(|a| a == "--system-prompt").unwrap();
    assert_eq!(args[i + 1], "you are the brain");
    // the prompt goes on stdin, not argv
    let stdin = read(&f.log.join("stdin"));
    assert!(stdin.ends_with("what now?"));
    assert!(stdin.contains("<conversation_so_far>"));
    assert!(!args.iter().any(|a| a.contains("what now?")));
    // runs in its own empty work dir
    assert_eq!(
        read(&f.log.join("cwd")).trim(),
        f.work.canonicalize().unwrap().display().to_string()
    );
}

#[tokio::test]
async fn never_sets_the_entrypoint_and_scrubs_inherited_values() {
    let f = fake(SUCCESS, 0, AUTH_OK);
    let b = cli(&f);
    collect(
        b.chat(ChatRequest::new(vec![Message::user("x")]))
            .await
            .unwrap(),
    )
    .await;
    // with nothing inherited the child sees no entrypoint
    assert_eq!(read(&f.log.join("env")).trim(), "entry=unset");
}

#[tokio::test]
async fn falls_back_to_assistant_text_without_partials() {
    let f = fake(NO_PARTIALS, 0, AUTH_OK);
    let (text, _) = collect(
        cli(&f)
            .chat(ChatRequest::new(vec![Message::user("x")]))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text, "whole reply");
}

#[tokio::test]
async fn usage_limit_with_stated_reset() {
    let f = fake(LIMIT, 1, AUTH_OK);
    let (text, evs) = collect(
        cli(&f)
            .chat(ChatRequest::new(vec![Message::user("x")]))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text, "", "the limit message is not shown as a reply");
    match evs.last() {
        Some(ChatEvent::Error {
            failure: FailureKind::UsageLimit { resets_at },
            ..
        }) => {
            assert_eq!(*resets_at, Utc.timestamp_opt(1_900_000_000, 0).single());
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn not_logged_in_is_an_auth_failure() {
    let f = fake(NOT_LOGGED_IN, 1, AUTH_NO);
    let (_, evs) = collect(
        cli(&f)
            .chat(ChatRequest::new(vec![Message::user("x")]))
            .await
            .unwrap(),
    )
    .await;
    assert!(matches!(
        evs.last(),
        Some(ChatEvent::Error {
            failure: FailureKind::Auth,
            ..
        })
    ));
}

#[tokio::test]
async fn crash_without_result_reports_stderr() {
    let f = fake("", 3, AUTH_OK);
    let (_, evs) = collect(
        cli(&f)
            .chat(ChatRequest::new(vec![Message::user("x")]))
            .await
            .unwrap(),
    )
    .await;
    match evs.last() {
        Some(ChatEvent::Error { message, failure }) => {
            assert!(message.contains("fake stderr line"));
            assert_eq!(*failure, FailureKind::Unavailable);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn availability_uses_version_and_auth_status() {
    let f = fake(SUCCESS, 0, AUTH_OK);
    let a = cli(&f).available().await;
    assert!(a.available);
    assert!(
        a.status.contains("2.1.300") && a.status.contains("signed in (claude.ai)"),
        "{}",
        a.status
    );

    let f = fake(SUCCESS, 0, AUTH_NO);
    let a = cli(&f).available().await;
    assert!(!a.available);
    assert!(a.status.contains("not signed in"));

    let f = fake(SUCCESS, 0, AUTH_MISSING);
    let a = cli(&f).available().await;
    assert!(a.available, "unknown sign-in still lets the user try");
    assert!(a.status.contains(SIGN_IN_HINT));
}

#[tokio::test]
async fn not_found_on_path() {
    let dir = tempfile::tempdir().unwrap();
    let b = ClaudeCli::new(ClaudeCliConfig {
        enabled: true,
        search_path: Some(dir.path().as_os_str().to_owned()),
        ..Default::default()
    });
    assert!(b.locate().is_none());
    assert!(!b.available().await.available);
    assert!(matches!(
        b.chat(ChatRequest::new(vec![Message::user("x")])).await,
        Err(HarnessError::Unavailable(_))
    ));
}

#[tokio::test]
async fn kill_switch_and_opt_in() {
    let f = fake(SUCCESS, 0, AUTH_OK);
    let mut cfg = cli(&f).config().clone();
    cfg.disabled_by_policy = true;
    let b = ClaudeCli::new(cfg.clone());
    assert!(matches!(
        b.chat(ChatRequest::new(vec![Message::user("x")])).await,
        Err(HarnessError::DisabledByPolicy(_))
    ));
    assert_eq!(b.available().await.status, "disabled by policy");
    assert!(
        !f.log.join("args").exists(),
        "the binary was never run for chat"
    );

    cfg.disabled_by_policy = false;
    cfg.enabled = false;
    let b = ClaudeCli::new(cfg);
    assert!(matches!(
        b.chat(ChatRequest::new(vec![Message::user("x")])).await,
        Err(HarnessError::NotConfigured(_))
    ));
}

#[test]
fn capabilities_are_grey_interactive_only() {
    let c = ClaudeCli::new(ClaudeCliConfig::default()).capabilities();
    assert_eq!(c.terms, Terms::Grey);
    assert_eq!(c.cost_model, CostModel::Subscription);
    assert!(!c.background_allowed);
    assert_eq!(c.notice.as_deref(), Some(NOTICE));
}

#[test]
fn removes_inherited_entrypoint_from_child_env() {
    let f = fake(SUCCESS, 0, AUTH_OK);
    let b = cli(&f);
    let bin = b.locate().unwrap();
    let cmd = b.command_for_test(&bin);
    let envs: Vec<_> = cmd
        .as_std()
        .get_envs()
        .map(|(k, v)| (k.to_owned(), v.map(|v| v.to_owned())))
        .collect();
    let entry = envs
        .iter()
        .find(|(k, _)| k == "CLAUDE_CODE_ENTRYPOINT")
        .expect("explicitly handled");
    assert!(entry.1.is_none(), "removed, never set to a value");
    assert!(
        !envs
            .iter()
            .any(|(k, v)| k.to_string_lossy().starts_with("CLAUDE") && v.is_some())
    );
}

/// Static guard: the adapter source never names the cli's credential stores or attribution
/// knobs in code (comments aside, nothing here may open them).
#[test]
fn adapter_source_never_touches_cli_credentials() {
    let src = include_str!("adapters/claude_cli.rs");
    let code: String = src
        .split("#[cfg(test)]")
        .next()
        .unwrap()
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        ".credentials",
        "\".claude",
        "/.claude",
        "keyring",
        "find-generic-password",
        "CLAUDE_CONFIG_DIR",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "setup-token",
        "home_dir",
        "\"HOME\"",
        ".env(\"CLAUDE",
        "auth login",
    ] {
        assert!(
            !code.contains(forbidden),
            "claude_cli.rs must not contain {forbidden:?}"
        );
    }
}
