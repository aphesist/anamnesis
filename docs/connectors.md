# connectors

a connector turns a source (a folder, a vault, a feed, a page, an api) into documents. each configured source is synced incrementally: on a timer, and for folders also on file changes. every document becomes an item with its provenance (`origin`, source, timestamps, the uri it came from) and every change is an event in the log.

add sources from the ui (sidebar → **+ add**), the cli (`anamnesis import <folder>`, `anamnesis add-source …`) or the api (`POST /api/sources`). the config is validated against the connector's json schema and the connector's `check` runs before the source is stored. `GET /api/sources` lists the built-in connectors' manifests, schemas included.

## built-in connectors

| id | reads | default origin · kind | sync |
|---|---|---|---|
| `folder` | markdown and plain-text files in a folder, recursively | `owner_written` · `note` | every 60 s and on file changes; deletes and renames |
| `obsidian` | an obsidian vault: frontmatter, aliases, tags, wikilinks, embeds, daily notes (ignores `.obsidian/` and `.trash/`) | `owner_written` · `note` | as `folder` |
| `pdf` | the text layer of a pdf file or a folder of pdfs, with page offsets for citations (no ocr) | `external` · `pdf` | every 5 min; deletes and renames |
| `rss` | rss, atom and json feeds | `external` · `feed` | hourly |
| `web` | the main text of one web page, with author, site, date and canonical url | `external` · `clip` (or `article`) | once; again only with `"refetch": true` |
| `json-api` | a paged json endpoint with an `updated_at` cursor and a field mapping | `owner_written` · `article` | hourly; incremental, deletes, paging |

the items written by anamnesis itself have reserved source ids: `editor` (notes you write in the app), `chat` (your chats) and `mcp` (captures from external harnesses).

### config

```sh
anamnesis add-source folder   --name notes   --config '{"path":"~/notes"}'
anamnesis add-source obsidian --name vault   --config '{"path":"~/vault","ignore":["templates/**"]}'
anamnesis add-source pdf      --name papers  --config '{"path":"~/papers"}'
anamnesis add-source rss      --name a-blog  --config '{"url":"https://example.org/feed.xml"}'
anamnesis add-source web      --config '{"url":"https://example.org/an-essay"}'
```

`anamnesis import <folder>` is a shortcut for `folder` (or `obsidian` when the folder has `.obsidian/`).

common options: `kind` and `origin` override the defaults for every item of the source (e.g. a folder of quotes you collected: `"origin":"external","kind":"quote"`). `folder` and `obsidian` also take `extensions`, `ignore` (gitignore-style globs), `include_hidden`, `follow_symlinks` and `max_file_bytes`; in markdown files, a frontmatter `kind` overrides it per note. `rss`, `web` and `json-api` take `timeout_secs`; `web` takes a css `selector` for the main text.

`json-api` example (a personal site's content api):

```json
{
  "url": "https://me.example/api/posts",
  "items_path": "data",
  "fields": { "id": "id", "title": "title", "body": "content", "updated_at": "updated_at",
              "url": "links.html", "tags": "tags", "deleted": "deleted" },
  "since_param": "updated_after",
  "page_param": "page",
  "token_env": "MY_SITE_TOKEN"
}
```

configs never hold secrets: `token_env` names an environment variable whose value is sent as a bearer token (or in `auth_header`).

the source api also takes `default_origin` and `default_privacy` (`local_only`, `cloud_ok`, `ask`), which new items inherit; see [privacy.md](privacy.md#privacy-flags).

### how sync behaves

- **incremental.** each source keeps a cursor. the cursor carries a hash of the config, so changing a source's config resyncs it from scratch.
- **stable identity.** an item is keyed by its id inside the source (relative path, feed guid, canonical url). renames keep the item, its links and its history.
- **deduplication.** content is normalised and hashed; an identical live text is not stored twice.
- **links from text.** `[[wikilinks]]`, embeds and relative markdown links become `written` links when they resolve, and stay pending until a matching item appears. these count as your own links in the [evaluation](evaluation.md).
- **deleting a source** removes its items from the brain (`DELETE /api/sources/:id`).

## plugins

anything else (a notes service, a read-later app, an export from a chat app) can be a plugin: an executable, in any language, that speaks **json lines** over stdin and stdout.

```
my-plugin/
  anamnesis-plugin.json   # the manifest: id, config schema, capabilities, declared hosts and paths, defaults
  plugin                  # the executable named by "entry"
```

anamnesis starts the executable once per call and writes one request line, `{"op":"check" | "list", "api_version":1, "config":{…}, "cursor":"…"}`. the plugin answers with lines such as `{"type":"upsert","document":{…}}`, `{"type":"delete","external_id":"…"}`, `{"type":"rename","from":"…","to":"…"}` and finally `{"type":"done","next_cursor":"…","has_more":false}`. only `external_id`, `title` and `body` are required in a document; the manifest supplies default `origin`, `kind` and privacy.

the full protocol, the manifest fields and a shell example are in **[crates/anamnesis-connectors/PLUGINS.md](../crates/anamnesis-connectors/PLUGINS.md)**.

two things to know:

- **permissions are declared, not enforced.** `network_hosts` and `fs_paths` are shown to you, but a plugin runs as an ordinary process with your permissions. install only plugins you trust.
- **the server does not load plugins yet.** the runtime is implemented and tested in `anamnesis-connectors` (`load_plugin`), but `anamnesis serve` only offers the built-in connectors today. a `wasm` runtime is reserved and not implemented.
