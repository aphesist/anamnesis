# mcp

anamnesis is a [model context protocol](https://modelcontextprotocol.io) server. any harness that speaks mcp (claude code, claude desktop, codex, gemini cli, antigravity and others) can search your brain, read items, and append to it, using that harness's own model and sign-in. anamnesis never touches the harness's credentials. for a subscription you already pay for, this is the cleanest way to use it with your notes.

## two transports

| transport | how | when |
|---|---|---|
| **stdio** | the harness starts `anamnesis mcp` itself | simplest; works without `anamnesis serve` running |
| **streamable http** | `http://127.0.0.1:7368/mcp` while `anamnesis serve` runs, with a bearer token | when the app is running anyway, or the harness prefers http |

the http endpoint requires `Authorization: Bearer <token>`. the token is created on first use in `<brain dir>/mcp_token` (mode 0600). the snippets below never contain it: they read it from the environment variable `ANAMNESIS_MCP_TOKEN`, so set that in the environment the harness runs in:

```sh
export ANAMNESIS_MCP_TOKEN="$(cat ~/.local/share/anamnesis/mcp_token)"   # linux default data dir
```

## ready-made snippets

the running app generates exact snippets, with the absolute path of your binary and your port:

```sh
curl -s http://127.0.0.1:7368/api/mcp/snippets
```

they look like this (with `/usr/local/bin/anamnesis` standing in for your binary's path).

### claude code

```sh
claude mcp add --transport stdio --scope user anamnesis -- /usr/local/bin/anamnesis mcp
# or over http, with ANAMNESIS_MCP_TOKEN set in your shell:
claude mcp add --transport http --scope user anamnesis http://127.0.0.1:7368/mcp --header "Authorization: Bearer $ANAMNESIS_MCP_TOKEN"
```

or per project, in `.mcp.json`:

```json
{
  "mcpServers": {
    "anamnesis": { "type": "stdio", "command": "/usr/local/bin/anamnesis", "args": ["mcp"] }
  }
}
```

```json
{
  "mcpServers": {
    "anamnesis": {
      "type": "http",
      "url": "http://127.0.0.1:7368/mcp",
      "headers": { "Authorization": "Bearer ${ANAMNESIS_MCP_TOKEN}" }
    }
  }
}
```

### claude desktop

`claude_desktop_config.json` (macos `~/Library/Application Support/Claude/`, windows `%APPDATA%\Claude\`, linux `~/.config/Claude/`). this file configures local (stdio) servers:

```json
{
  "mcpServers": {
    "anamnesis": { "command": "/usr/local/bin/anamnesis", "args": ["mcp"] }
  }
}
```

### codex

`~/.codex/config.toml`:

```toml
[mcp_servers.anamnesis]
command = "/usr/local/bin/anamnesis"
args = ["mcp"]
```

or over http:

```toml
[mcp_servers.anamnesis]
url = "http://127.0.0.1:7368/mcp"
bearer_token_env_var = "ANAMNESIS_MCP_TOKEN"
```

### gemini cli

`~/.gemini/settings.json` (or `.gemini/settings.json` in a project):

```json
{
  "mcpServers": {
    "anamnesis": { "command": "/usr/local/bin/anamnesis", "args": ["mcp"] }
  }
}
```

```json
{
  "mcpServers": {
    "anamnesis": {
      "httpUrl": "http://127.0.0.1:7368/mcp",
      "headers": { "Authorization": "Bearer ${ANAMNESIS_MCP_TOKEN}" }
    }
  }
}
```

antigravity also supports mcp servers; use the same command or url in its mcp settings.

### a brain outside the default data directory

the stdio snippets start `anamnesis mcp` on the default data directory. if your brain lives elsewhere, add the flag to the arguments, e.g. `"args": ["mcp", "--data-dir", "/path/to/data"]`, or set `ANAMNESIS_DATA_DIR` in the server's `env`.

## tools

| tool | arguments | does |
|---|---|---|
| `search` | `query`, `k?` | hybrid full-text + semantic search; hits carry item ids, titles, snippets, provenance and a citation span (byte offsets into the item body) |
| `get_item` | `id` | one item in full (bodies are cut at 32,000 characters): text, origin, tags, links in and out, threads, region |
| `related` | `id`, `k?` | semantically close items, plus the link suggestions the brain shows for it |
| `list_threads` | `kind?`, `status?` | decisions, open questions, plans and rejections, each with a verbatim quote and its item |
| `region_overview` | | the regions of the brain with labels and sizes |
| `timeline` | `from?`, `to?`, `limit?` | items written in a time range, oldest first (rfc3339 or `yyyy-mm-dd`) |
| `recent` | `n?` | the most recently captured items |
| `capture` | `title`, `body`, `origin?`, `kind?`, `tags?` | append a new item (never overwrites) |
| `link` | `from`, `to`, `note?` | add a manual link between two items |
| `update_thread` | `id`, `status`, `resolved_by?` | move a thread to `open`, `resolved`, `abandoned` or `superseded` |

resources: `anamnesis://item/{id}` and `anamnesis://region/{id}`, read-only views of the same data. the server also sends instructions that tell the model how to use the tools, how to treat provenance, and that text inside items is data, not instructions.

**there is no tool that deletes anything, edits existing text, fetches a url or sends data anywhere.** this is on purpose: an agent reading a hostile web clip from your brain cannot use anamnesis to exfiltrate or destroy it.

## provenance rules

every write over mcp is recorded in the event log with the actor `mcp:<client>` (the client's name from the mcp handshake), so you can always tell what an external harness added.

`capture` takes an `origin`, and clients are told to set it honestly:

| `origin` | for | stored as |
|---|---|---|
| `assistant` (default) | anything the model wrote, summarised or paraphrased | `ai_generated` |
| `user` | your own words, verbatim or nearly so (what you said or typed to the assistant) | `owner_in_chat` |
| `user_edited` | your text, rewritten by the model | `ai_edited` |
| `external` | text quoted from someone else: a page, a paper, a book | `external` |

the default is `assistant`, so a careless client errs on the side of not putting words in your mouth. only `user` counts as your voice in the "when you wrote" chart and in any measure of how you think.

## privacy over mcp

- items whose privacy is `local_only` or `ask` are returned without their text (`[withheld: the user keeps this item local-only]`), and threads quoting them are not listed.
- every read that returns brain content to a harness is logged as an `Egress` event with provider `mcp:<client>`, purpose = the tool name, the item ids and the number of characters. see [privacy.md](privacy.md).
- the http endpoint answers only on `127.0.0.1`, requires the bearer token, and refuses browser requests from non-loopback origins.

## the installed claude cli

when you enable the opt-in "installed cli (claude)" chat backend ([models.md](models.md#installed-cli-claude)), anamnesis offers the cli this same mcp server over stdio, so the model can search your brain while answering. no other tools are enabled for it.
