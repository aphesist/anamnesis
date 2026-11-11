# connector plugins (`exec` runtime)

A plugin is an executable that turns some source into documents for anamnesis. It speaks
**json lines** over stdin/stdout. Any language works: a shell script, a python file with a
shebang, a compiled binary.

## layout

```
my-plugin/
  anamnesis-plugin.json   # the manifest
  plugin                  # the executable named by "entry"
```

Load it with `anamnesis_connectors::load_plugin(Path::new("my-plugin"))`.

## manifest

```json
{
  "id": "my-notes",
  "name": "my notes service",
  "description": "notes from my self-hosted notes app",
  "version": "0.1.0",
  "api_version": 1,
  "runtime": "exec",
  "entry": "plugin",
  "config_schema": {
    "type": "object",
    "required": ["base_url"],
    "properties": { "base_url": { "type": "string" } },
    "additionalProperties": false
  },
  "capabilities": ["check", "incremental", "deletes"],
  "network_hosts": ["notes.example.com"],
  "fs_paths": [],
  "default_origin": "owner_written",
  "default_privacy": "cloud_ok",
  "default_kind": "note",
  "poll_interval_secs": 900
}
```

| field | meaning |
|---|---|
| `id` | `[a-z0-9][a-z0-9_-]*`, max 64 chars. Must not be a built-in id (`folder`, `obsidian`, `pdf`, `rss`, `web`, `json-api`) or a reserved one (`editor`, `chat`, `mcp`). |
| `version` | the plugin's own version, `major.minor.patch`. |
| `api_version` | the protocol version targeted. This build speaks `1`. |
| `runtime` | `exec` (this document). `wasm` is reserved and not runnable yet. `builtin` is for connectors compiled into anamnesis. |
| `entry` | path to the executable, relative to the manifest's directory (or absolute). |
| `config_schema` | JSON schema for the source config. anamnesis validates the config before every call. Config must never hold secrets: ask for the *name* of an environment variable instead. |
| `capabilities` | `check`, `incremental`, `deletes`, `renames`, `watch`, `paged`. Unknown values are tolerated. |
| `network_hosts`, `fs_paths` | what the plugin says it will touch. `<config.x>` placeholders mean "whatever the config says". |
| `default_origin` | provenance for documents that do not say: `owner_written`, `owner_spoken`, `owner_annotation`, `owner_in_chat`, `ai_generated`, `ai_edited`, `external`. |
| `default_privacy` | `local_only`, `cloud_ok` (default), `ask`. |
| `default_kind` | `note`, `journal`, `quote`, `article`, `pdf`, `book`, `chat`, `clip`, `feed`, `skill`. |

**Permissions are declared, not enforced.** `network_hosts` and `fs_paths` are recorded and
shown to the user before the plugin is enabled. The plugin runs as an ordinary process with the
user's permissions; there is no sandbox in this runtime. Only install plugins you trust.

## protocol

anamnesis starts the executable once per call, with the manifest's directory as working
directory and these environment variables: `ANAMNESIS_CONNECTOR_API` (`1`) and
`ANAMNESIS_CONNECTOR_ID`. It writes **one request line** to stdin and closes stdin, then reads
stdout line by line until the process exits. The whole call has a timeout (120 s by default);
on timeout the process is killed and the call fails. stderr is captured for diagnostics (first
64 KiB) and never parsed. A non-zero exit status fails the call.

### requests

```json
{"op": "check", "api_version": 1, "config": { ... }}
{"op": "list",  "api_version": 1, "config": { ... }, "cursor": "opaque string or absent"}
```

`cursor` is absent on a full sync. Otherwise it is exactly the `next_cursor` the plugin
returned last time (anamnesis stores it after the batch is applied).

### responses

One json object per line, discriminated by `type`. Blank lines are ignored.

| line | when |
|---|---|
| `{"type": "check", "ok": true, "message": "found 12 notes"}` | answer to `check`; ends the call. `ok: false` with a lowercase reason when the source cannot be reached. |
| `{"type": "upsert", "document": {...}}` | a new or changed document. |
| `{"type": "delete", "external_id": "..."}` | a document that no longer exists. |
| `{"type": "rename", "from": "old id", "to": "new id"}` | a document moved; anamnesis re-keys the item (keeping its identity and links). Also send an `upsert` under the new id. |
| `{"type": "log", "level": "info", "message": "..."}` | progress, written to anamnesis' log. |
| `{"type": "error", "message": "..."}` | fails the call immediately. |
| `{"type": "done", "next_cursor": "...", "has_more": false}` | ends a `list`. `next_cursor` may be null. `has_more: true` asks anamnesis to call `list` again right away with `next_cursor` (paging). |

A `list` that exits without a `done` line, or a `check` without a `check` line, fails.

### document

```json
{
  "external_id": "stable id inside the source (path, guid, url)",
  "title": "required",
  "body": "required: plain text or markdown",
  "uri": "https://... or file://... (optional)",
  "kind": "note",
  "origin": "owner_written",
  "created_at": "2024-03-05T09:30:00Z",
  "modified_at": "2024-03-06T10:00:00Z",
  "tags": ["philosophy"],
  "refs": ["other external id", "https://..."],
  "meta": {"anything": "kept verbatim for provenance"}
}
```

Only `external_id`, `title` and `body` are required. Missing `kind` / `origin` fall back to the
manifest's defaults. `refs` are outgoing references: other documents' external ids or urls.
anamnesis adds `connector` and `plugin_version` to `meta`.

Lines may be up to 32 MiB.

## example (sh)

```sh
#!/bin/sh
read -r req
case "$req" in
  *'"op":"check"'*)
    echo '{"type":"check","ok":true,"message":"ready"}' ;;
  *'"op":"list"'*)
    echo '{"type":"upsert","document":{"external_id":"hello","title":"hello","body":"a first note"}}'
    echo '{"type":"done","next_cursor":"v1"}' ;;
esac
```

A real plugin should parse the request with a json parser rather than pattern matching.
