# privacy

anamnesis holds what you write to yourself. it is built so that nothing leaves your machine unless you chose a model that needs it, and so that you can see every time something did.

## local first

- the brain is one sqlite file in your data directory ([install.md](install.md#where-things-live)). there is no account, no server of ours, no telemetry, no update check.
- the server listens on `127.0.0.1` only, and refuses requests whose `Host` header is not a loopback name (a guard against dns rebinding). the mcp endpoint at `/mcp` also needs a bearer token and refuses browser requests from non-loopback origins.
- embeddings run on your machine by default. the ui's fonts are bundled; rendering text makes no network request.
- without a model configured, chat answers by retrieval alone and nothing is sent anywhere.

what does reach the network, and only when you set it up: the sources you add (a feed or web page is fetched from its host), the first download of the local embedding model, the model backends you configure, and the aphesis theme assets fetched by `npm install` in `ui/theme` (optional, a developer step).

## privacy flags

every item has a privacy flag. it comes from, most specific first: the flag you set on the item by hand; the note's own frontmatter (`privacy: local_only`); the source's default; and for a new source with no default given, the `privacy.default` setting.

| flag | meaning |
|---|---|
| `cloud_ok` (default) | may be sent to the model you chose |
| `ask` | ask each time: chat stops with a `consent_required` event naming these items, and sends them only if you approve |
| `local_only` | never sent to a non-local model, never returned with its text over mcp |

set a source's default when you add it (`anamnesis add-source folder --privacy local_only …`), or change it later; its items that took the old default follow the new one, while flags set by hand or by frontmatter stay:

```sh
curl -X PUT http://127.0.0.1:7368/api/sources/<source id> -H 'content-type: application/json' -d '{"default_privacy":"local_only"}'
```

set one item by hand (`null` hands it back to its frontmatter or source):

```sh
curl -X PUT http://127.0.0.1:7368/api/items/<item id> -H 'content-type: application/json' -d '{"privacy":"ask"}'
```

every change is an `ItemUpdated` event in the log. one detail, stated plainly: an imported note whose frontmatter says `local_only` is created with the source's flag and re-flagged in the same write, so the log holds both events.

## the egress gate and ledger

every call to a non-local model goes through one gate in the harness:

1. items flagged `local_only` are refused;
2. items flagged `ask` need your approval for this call;
3. the call is recorded as an `Egress` event in the append-only log: time, provider, model, purpose (`chat`, `label`), the ids of the items in the prompt and the number of characters sent. **never the text.**

mcp reads are recorded the same way, with provider `mcp:<client>`, so you can see what an external harness pulled out of the brain. read the ledger:

```sh
curl 'http://127.0.0.1:7368/api/egress?limit=50'
```

local backends (loopback servers, or ones you marked `local`) are not gated or logged: nothing leaves the machine.

## mcp

external harnesses that connect over mcp get a deliberately small surface ([mcp.md](mcp.md)):

- **no delete tool**, no tool that removes links or edits existing text. writes are append-only (`capture`, `link`, `update_thread`) and attributed to the client in the log.
- **no fetch or send tool**: the server cannot be asked to open a url or send data anywhere. text inside items is untrusted data; this removes the path by which a malicious page could make an agent exfiltrate your notes through anamnesis.
- items not marked `cloud_ok` come back without their text (`[withheld: …]`), and threads quoting them are not listed.

## credentials

- api keys live in the os keychain (service `anamnesis`), or, where there is no usable keychain, in `secrets.json` in the data directory with mode 0600. they are not settings, so they never enter the event log or an exported bundle, nothing logs them, and no endpoint returns them (`GET /api/backends` says only where each key is: `keychain`, `file` or `env`).
- the "Continue with ChatGPT" token is anamnesis's own, from openai's sign-in, stored in the keychain (or a 0600 file if there is no keychain).
- the mcp bearer token is a 0600 file in the brain directory, deliberately not a setting, so it does not travel in exports.
- anamnesis never reads another program's credentials. see [models.md](models.md#what-anamnesis-will-not-do).

## what an export contains

`anamnesis export` and `GET /api/export` write every event: all your items, links, threads, settings and the egress ledger. no api keys, no tokens, no embeddings and no layout (those are recomputed after import). treat an export like the brain itself.

## provenance

every item records its `origin`: written by you, spoken, your annotation, said by you in a chat, generated by a model, edited by a model, or someone else's text. the "when you wrote" chart and any claim about how you think use only your own words. text a model wrote is never relabelled as yours, including text written through mcp (clients must say `origin`; the default is `assistant`).

## deleting

deleting a source removes its items from the brain. the event log is append-only and hash-chained, so the events that carried the text remain in the log (and in any export made before). removing text from the log itself is not possible yet: a redaction event that keeps the chain verifiable is future work.
