//! Ready-to-paste configuration for the common mcp harnesses.
//!
//! Formats checked against the official docs (2026-11):
//! - Claude Code: <https://code.claude.com/docs/en/mcp>
//!   `claude mcp add [options] <name> -- <command> [args...]`; options (`--transport`,
//!   `--scope`, `--env`, `--header`) go before the name, the server command after `--`.
//!   Project config is `.mcp.json` with `{"mcpServers": {name: {type, command, args, env}}}`
//!   or `{type: "http", url, headers}`; `${VAR}` is expanded in `.mcp.json`.
//! - Claude Desktop: <https://modelcontextprotocol.io/docs/develop/connect-local-servers>
//!   `claude_desktop_config.json` (macOS `~/Library/Application Support/Claude/`,
//!   Windows `%APPDATA%\Claude\`, Linux `~/.config/Claude/`) with
//!   `{"mcpServers": {name: {command, args, env}}}`. The file configures local (stdio)
//!   servers; remote servers are added in the app's connector settings instead.
//! - Codex CLI: <https://developers.openai.com/codex/mcp> (now served from
//!   <https://learn.chatgpt.com/docs/extend/mcp?surface=cli>)
//!   `~/.codex/config.toml`, table `[mcp_servers.<name>]` with `command`, `args`, `env`
//!   (stdio) or `url`, `bearer_token_env_var`, `http_headers` (streamable http).
//! - Gemini CLI: <https://geminicli.com/docs/tools/mcp-server/> and
//!   <https://geminicli.com/docs/reference/configuration/>
//!   `~/.gemini/settings.json` (user) or `.gemini/settings.json` (project), key
//!   `mcpServers` with `command`/`args`/`env` (stdio) or `httpUrl` + `headers`
//!   (streamable http; `url` would mean sse). String values expand `$VAR` / `${VAR}`.

use serde::Serialize;
use serde_json::json;

/// What to put in the snippets.
#[derive(Debug, Clone)]
pub struct SnippetOptions {
    /// Server name as the harness will show it.
    pub name: String,
    /// Absolute path of the `anamnesis` binary.
    pub command: String,
    /// Arguments that start the stdio server (by default `["mcp"]`).
    pub args: Vec<String>,
    /// Streamable http endpoint, e.g. `http://127.0.0.1:7368/mcp`, if http is enabled.
    pub http_url: Option<String>,
    /// Environment variable holding the bearer token for http, if one is required.
    /// The token itself never appears in the snippets.
    pub token_env_var: Option<String>,
}

impl Default for SnippetOptions {
    fn default() -> Self {
        Self {
            name: "anamnesis".into(),
            command: "anamnesis".into(),
            args: vec!["mcp".into()],
            http_url: None,
            token_env_var: None,
        }
    }
}

/// One snippet per harness. The `*_http` variants exist only when `http_url` is set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigSnippets {
    /// Shell command registering the stdio server with Claude Code (user scope).
    pub claude_code_command: String,
    pub claude_code_http_command: Option<String>,
    /// Project-level `.mcp.json` for Claude Code.
    pub claude_code_mcp_json: String,
    pub claude_code_mcp_json_http: Option<String>,
    /// `claude_desktop_config.json` (stdio only).
    pub claude_desktop_json: String,
    /// `~/.codex/config.toml` section.
    pub codex_toml: String,
    pub codex_toml_http: Option<String>,
    /// `~/.gemini/settings.json`.
    pub gemini_settings_json: String,
    pub gemini_settings_json_http: Option<String>,
}

/// Quotes a word for a posix shell when needed.
fn sh(word: &str) -> String {
    let safe = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if safe {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// A toml basic string.
fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A toml table key, quoted if needed.
fn toml_key(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        s.to_string()
    } else {
        toml_str(s)
    }
}

fn pretty(v: serde_json::Value) -> String {
    serde_json::to_string_pretty(&v).expect("json value serialises")
}

/// Builds the snippets for every supported harness.
pub fn config_snippets(o: &SnippetOptions) -> ConfigSnippets {
    let name = &o.name;
    let stdio_cmd: Vec<String> = std::iter::once(o.command.as_str())
        .chain(o.args.iter().map(String::as_str))
        .map(sh)
        .collect();
    let claude_code_command = format!(
        "claude mcp add --transport stdio --scope user {} -- {}",
        sh(name),
        stdio_cmd.join(" ")
    );

    let stdio_entry = json!({ "command": o.command, "args": o.args });
    let claude_code_mcp_json = pretty(json!({
        "mcpServers": { name: { "type": "stdio", "command": o.command, "args": o.args } }
    }));
    let claude_desktop_json = pretty(json!({ "mcpServers": { name: stdio_entry.clone() } }));
    let gemini_settings_json = pretty(json!({ "mcpServers": { name: stdio_entry } }));

    let args_toml: Vec<String> = o.args.iter().map(|a| toml_str(a)).collect();
    let codex_toml = format!(
        "[mcp_servers.{}]\ncommand = {}\nargs = [{}]\n",
        toml_key(name),
        toml_str(&o.command),
        args_toml.join(", ")
    );

    let (mut cc_http_cmd, mut cc_http_json, mut codex_http, mut gemini_http) =
        (None, None, None, None);
    if let Some(url) = &o.http_url {
        let token = o.token_env_var.as_deref();
        let mut cmd = format!(
            "claude mcp add --transport http --scope user {} {}",
            sh(name),
            sh(url)
        );
        if let Some(var) = token {
            // double quotes so the shell substitutes the variable at add time
            cmd.push_str(&format!(" --header \"Authorization: Bearer ${var}\""));
        }
        cc_http_cmd = Some(cmd);

        let mut entry = json!({ "type": "http", "url": url });
        if let Some(var) = token {
            entry["headers"] = json!({ "Authorization": format!("Bearer ${{{var}}}") });
        }
        cc_http_json = Some(pretty(json!({ "mcpServers": { name: entry } })));

        let mut t = format!(
            "[mcp_servers.{}]\nurl = {}\n",
            toml_key(name),
            toml_str(url)
        );
        if let Some(var) = token {
            t.push_str(&format!("bearer_token_env_var = {}\n", toml_str(var)));
        }
        codex_http = Some(t);

        let mut g = json!({ "httpUrl": url });
        if let Some(var) = token {
            g["headers"] = json!({ "Authorization": format!("Bearer ${{{var}}}") });
        }
        gemini_http = Some(pretty(json!({ "mcpServers": { name: g } })));
    }

    ConfigSnippets {
        claude_code_command,
        claude_code_http_command: cc_http_cmd,
        claude_code_mcp_json,
        claude_code_mcp_json_http: cc_http_json,
        claude_desktop_json,
        codex_toml,
        codex_toml_http: codex_http,
        gemini_settings_json,
        gemini_settings_json_http: gemini_http,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn opts() -> SnippetOptions {
        SnippetOptions {
            command: "/opt/my apps/anamnesis".into(),
            http_url: Some("http://127.0.0.1:7368/mcp".into()),
            token_env_var: Some("ANAMNESIS_MCP_TOKEN".into()),
            ..Default::default()
        }
    }

    #[test]
    fn claude_code_command_puts_options_before_name_and_command_after_dashes() {
        let s = config_snippets(&opts());
        assert_eq!(
            s.claude_code_command,
            "claude mcp add --transport stdio --scope user anamnesis -- '/opt/my apps/anamnesis' mcp"
        );
        assert_eq!(
            s.claude_code_http_command.unwrap(),
            "claude mcp add --transport http --scope user anamnesis http://127.0.0.1:7368/mcp \
             --header \"Authorization: Bearer $ANAMNESIS_MCP_TOKEN\""
        );
    }

    #[test]
    fn json_snippets_have_the_documented_shape() {
        let s = config_snippets(&opts());
        let v: Value = serde_json::from_str(&s.claude_code_mcp_json).unwrap();
        assert_eq!(v["mcpServers"]["anamnesis"]["type"], "stdio");
        assert_eq!(v["mcpServers"]["anamnesis"]["args"][0], "mcp");
        let v: Value = serde_json::from_str(&s.claude_code_mcp_json_http.unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["anamnesis"]["type"], "http");
        assert_eq!(
            v["mcpServers"]["anamnesis"]["headers"]["Authorization"],
            "Bearer ${ANAMNESIS_MCP_TOKEN}"
        );
        let v: Value = serde_json::from_str(&s.claude_desktop_json).unwrap();
        assert_eq!(
            v["mcpServers"]["anamnesis"]["command"],
            "/opt/my apps/anamnesis"
        );
        let v: Value = serde_json::from_str(&s.gemini_settings_json_http.unwrap()).unwrap();
        assert_eq!(
            v["mcpServers"]["anamnesis"]["httpUrl"],
            "http://127.0.0.1:7368/mcp"
        );
        assert!(v["mcpServers"]["anamnesis"].get("url").is_none());
    }

    #[test]
    fn codex_toml() {
        let s = config_snippets(&opts());
        assert_eq!(
            s.codex_toml,
            "[mcp_servers.anamnesis]\ncommand = \"/opt/my apps/anamnesis\"\nargs = [\"mcp\"]\n"
        );
        assert_eq!(
            s.codex_toml_http.unwrap(),
            "[mcp_servers.anamnesis]\nurl = \"http://127.0.0.1:7368/mcp\"\n\
             bearer_token_env_var = \"ANAMNESIS_MCP_TOKEN\"\n"
        );
        let mut o = opts();
        o.command = r#"C:\a "b"\anamnesis.exe"#.into();
        assert!(
            config_snippets(&o)
                .codex_toml
                .contains(r#""C:\\a \"b\"\\anamnesis.exe""#)
        );
    }

    #[test]
    fn no_http_snippets_without_url() {
        let s = config_snippets(&SnippetOptions::default());
        assert!(s.claude_code_http_command.is_none());
        assert!(s.codex_toml_http.is_none());
        assert!(s.gemini_settings_json_http.is_none());
    }
}
