# models

anamnesis needs no model to work: it stores, links, clusters, searches and evaluates on its own, and chat falls back to a retrieval-only responder that lists what your brain holds. models make chat write answers and give regions better names. you bring your own; anamnesis never ships or resells one.

- [backends](#backends) · [configuring them](#configuring-backends) · [routing](#routing) · [usage and limits](#usage-and-limits) · [privacy and egress](#privacy-and-egress) · [embeddings](#embeddings) · [provider terms](#provider-terms) · [what anamnesis will not do](#what-anamnesis-will-not-do)

## backends

| backend | `kind` | runs | paid by | background work | terms |
|---|---|---|---|---|---|
| local server: ollama, llama.cpp `llama-server`, lm studio, vllm… | `openai_compatible` | on your machine | nothing | yes | allowed |
| api key: openai, openrouter, any openai-compatible host | `openai_compatible` | provider | your key, per token | yes | allowed |
| api key: anthropic | `anthropic` | provider | your key, per token | yes | allowed |
| api key: gemini (google ai studio) | `gemini` | provider | your key, per token | yes | allowed |
| "Continue with ChatGPT" | `chatgpt` | provider | your chatgpt plan | **never** | allowed (openai preview), experimental adapter |
| installed cli (claude) | `claude_cli` | your own `claude` binary | your claude plan | **never** | grey, opt-in |
| retrieval only | `retrieval` | in anamnesis | nothing | — | allowed |

and in the other direction, any mcp client (claude code, claude desktop, codex, gemini cli and others) can use the brain with whatever model and plan it already has. see [mcp.md](mcp.md). for most people with a subscription, that is the route to prefer.

## configuring backends

backends are configured through settings (`PUT /api/settings`, key `harness`) and keys through the keychain endpoints; see [contracts.md](contracts.md). the settings panel in the ui covers appearance today; model configuration is done through the api (examples below) until it has its own panel.

`PUT /api/settings` deep-merges what you send into what is stored: objects merge key by key, `null` removes a key, and anything else (strings, numbers, lists) replaces. so `{"harness":{"claude_cli":{"enabled":true}}}` leaves the rest of `harness` alone, but a list such as `harness.openai_compatible` is replaced whole: send every entry you want to keep. `GET /api/settings` shows the current one.

### local models

nothing to do for the usual servers: at startup anamnesis probes `127.0.0.1:11434` (ollama), `:8080` (llama.cpp) and `:1234` (lm studio) and adds any that answer. local backends are the default for every task. to configure one by hand:

```sh
curl -X PUT http://127.0.0.1:7368/api/settings -H 'content-type: application/json' -d '{
  "harness": { "openai_compatible": [
    { "id": "ollama", "provider": "ollama", "base_url": "http://127.0.0.1:11434/v1", "model": "qwen3:8b" }
  ] }
}'
```

a base url on a loopback host counts as local; set `"local": true` for your own server elsewhere on your network, or `"local": false` to treat a loopback proxy as remote.

### api keys

keys go to the os keychain (service `anamnesis`, account = backend id), or, when there is no usable keychain, to `secrets.json` in the data dir (mode 0600); never into settings, never into the event log or an export. `GET /api/backends` shows where each key was found as `key_storage`: `keychain`, `file` or `env`. set `ANAMNESIS_KEY_STORAGE=file` to skip the keychain:

```sh
curl -X PUT http://127.0.0.1:7368/api/backends/anthropic/key -H 'content-type: application/json' -d '{"key":"…"}'
curl -X PUT http://127.0.0.1:7368/api/settings -H 'content-type: application/json' -d '{"harness":{"anthropic":{}}}'
```

the same works for `gemini` (`"gemini": {}`) and for an openai-compatible entry with a remote `base_url` (the key's account is the entry's `id`). if neither the keychain nor the file has a key, anamnesis falls back to the usual environment variables, read when `anamnesis serve` starts: `ANTHROPIC_API_KEY`, `GEMINI_API_KEY` or `GOOGLE_API_KEY`, `OPENAI_API_KEY` for `"provider": "openai"`, `OPENROUTER_API_KEY` for `"provider": "openrouter"`. models are configurable per backend (`chat_model` and `label_model` for anthropic, `model` and `label_model` for gemini, `model` for openai-compatible).

### Continue with ChatGPT

openai lets open-source, locally run apps use a person's chatgpt plan through its own sign-in, with the person's consent and a per-app cap. anamnesis uses that sign-in and nothing else:

1. `POST /api/backends/chatgpt/sign-in` returns an `authorize_url`; the ui opens it in your browser, labelled **Continue with ChatGPT** (openai's required wording, the one exception to the lowercase voice).
2. you sign in and consent on openai's site. a listener on `127.0.0.1` receives the callback; the app's *own* token is stored in the keychain (or a 0600 file if no keychain is available).
3. the `chatgpt` backend appears, for chat only.

you can cap anamnesis's weekly usage in chatgpt settings → usage. signing out in anamnesis forgets the token but does not revoke access: remove the app in chatgpt's settings for that. requests follow openai's preview limits (responses api, `store: false`, streaming, no hosted tools, unsupported fields stripped). the adapter is marked **experimental**: it follows the published documentation but has not yet been exercised against the live service from this codebase.

### installed cli (claude)

if you have anthropic's `claude` cli installed and signed in, anamnesis can send your chat messages through it. it is **off by default**; turn it on only after reading the notice, which the provider list shows next to it:

> uses your own claude login through anthropic's official cli. anthropic has not explicitly approved third-party apps doing this and could restrict it at any time; the safest routes are the mcp server and an api key. anamnesis never reads your credentials.

```sh
curl -X PUT http://127.0.0.1:7368/api/settings -H 'content-type: application/json' -d '{"harness":{"claude_cli":{"enabled":true}}}'
```

what the adapter does and does not do:

- it runs only a `claude` you installed (found on `PATH`, or at `"binary"`); it never downloads, bundles or patches one.
- you sign in in your own terminal (`claude`); anamnesis never starts a login. it learns the sign-in state only from `claude auth status` and the cli's output, and never opens the cli's stored login, config directory or keychain entry.
- it never sets the cli's entrypoint or attribution variables (it removes inherited ones) and never injects a token. one account; no rotation, no failover to another account.
- interactive chat only: the router never sends background work to it.
- chat runs `claude -p` with every built-in tool disabled, in an empty working directory, with only the brain's own mcp server offered (over stdio, so no token appears on a command line). no permission-skipping flag is ever passed.

the safer ways to use a claude plan are the [mcp route](mcp.md) or an anthropic api key.

### google

anamnesis uses gemini only with an api key from google ai studio. consumer google logins (gemini cli or antigravity sign-in) are not offered: google's terms prohibit using them from third-party software. antigravity and gemini cli users can reach their brain through [mcp](mcp.md) instead.

## routing

each task goes to one backend:

| task | what | used today for |
|---|---|---|
| `chat` | interactive, you asked | the chat panel |
| `label` | background | naming new regions (a few representative snippets per region) |
| `extract` | background | finding decisions, open questions, plans and rejections in your own new or changed notes (a small batch per pass); without a model the rule-based extractor does it |
| `summarise` | background | reserved |

rules, enforced in the router and tested:

1. **background work never runs on a subscription** (`chatgpt`, `claude_cli`): not by default, not if you name one in `routing`, not as a fallback.
2. **defaults:** a local backend first. with none, chat goes to the cheapest metered api backend (by the editable price table); **background work (labels, extraction) stays local only** and falls back to deterministic labels and rule-based threads, so an api key is never spent on work you did not ask for. to let background work use the cheapest api key when no local model runs, opt in with `"background_to_api_by_default": true` in `routing`, or name a backend for the task (`"label": "anthropic"`). plan backends are used only when you pick them for chat.
3. **no silent fallback:** if the chosen backend is unavailable, cooling down or rate limited, only the fallbacks you listed in `routing.fallbacks` are tried, in order.
4. the chat box can pick a backend for one message (`backend` in `POST /api/chat`); `"retrieval"` uses no model.

```json
{ "routing": { "chat": "anthropic", "label": "ollama", "fallbacks": { "chat": ["ollama"] }, "background_to_api_by_default": true } }
```

## usage and limits

`GET /api/backends` reports usage per backend: requests and tokens; money for metered backends (tokens × the price table, editable in `harness.prices`); for plan backends, tokens and an api-equivalent estimate, always flagged as an estimate; local backends show tokens only. rate and usage limits (http 429, `retry-after`, plan limit messages) put a backend in a cooldown shown as `cooldown` in the same response. when a plan is exhausted anamnesis stops and says so; it never switches you to a paid backend on its own.

## privacy and egress

every call to a non-local backend passes the egress gate first:

- items whose privacy is `local_only` are never put in the prompt. chat leaves them out of the context; background labelling is refused for them.
- items marked `ask` need your consent for that request: chat answers with a `consent_required` event listing them, and resends only with the ids you approved.
- the call is appended to the event log as an `Egress` event: provider, model, purpose, the ids of the items in the prompt and the number of characters sent. never the text. read it with `GET /api/egress`.

local backends do not pass the gate and are not logged: nothing leaves the machine. more in [privacy.md](privacy.md).

## embeddings

embeddings are separate from chat models and local by default.

| `embedder` setting | notes |
|---|---|
| `{"backend":"local","model":"multilingual-e5-small"}` | the default: on-device onnx, 384 dimensions, english and portuguese and ~100 other languages, mit licence, ~465 MB download on first use. also: `multilingual-e5-base`, `multilingual-e5-large`, `bge-m3`, `embeddinggemma-300m` (gemma terms), `paraphrase-multilingual-minilm-l12-v2-q`. |
| `{"backend":"ollama","model":"qwen3-embedding:0.6b"}` | a local ollama server |
| `{"backend":"openai_compatible","base_url":"http://127.0.0.1:8080/v1","model":"…"}` | llama.cpp, lm studio, vllm, or a remote api. a remote endpoint's key is read from the keychain account `embed` or `ANAMNESIS_EMBED_API_KEY`. |
| `{"backend":"hash"}` | deterministic feature hashing: offline and instant, weak. used by `--hash-embedder` and tests. |

changing the embedder re-embeds everything and recomputes the layout. vectors from different models are never mixed. a remote embeddings endpoint sends the text of every chunk to that provider; it is your explicit choice, and local remains the default.

## provider terms

these are the provider rules anamnesis is designed around, summarised from the providers' own pages as read on **2026-11-11**. terms change: check the linked pages, not this table, before relying on a route. "allowed" means the provider's documents permit it; "grey" means they neither clearly permit nor clearly forbid it; "not offered" means anamnesis does not implement it.

### anthropic (claude free, pro, max)

| route | in anamnesis | status | what the provider says |
|---|---|---|---|
| the brain as an mcp server in claude desktop, claude code or a claude.ai connector | yes ([mcp.md](mcp.md)) | **allowed** | custom connectors are available on all plans ([claude help: custom connectors](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)) |
| your own api key | yes | **allowed**, the prescribed route | products that interact with claude "should use API key authentication through Claude Console or a supported cloud provider" ([claude code: legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)) |
| spawning your own installed, unmodified `claude`, signed in by you | yes, opt-in, chat only | **grey** | the legal page does not prevent an end user from signing in to the unmodified binary with their own subscription, and assumes "ordinary, individual usage" ([legal](https://code.claude.com/docs/en/legal-and-compliance)); the consumer terms (effective 2025-10-08) bar access "through automated or non-human means" unless explicitly permitted ([consumer terms](https://www.anthropic.com/legal/consumer-terms)); the help center (update of 2026-06-15) says `claude -p` and third-party app usage still draw from subscription limits, which describes billing, not permission ([help center](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)) |
| offering claude.ai login inside the app (agent sdk) | no | not offered | not allowed "unless previously approved" ([agent sdk overview](https://code.claude.com/docs/en/agent-sdk/overview)) |
| collecting `claude setup-token` output or any session token | no | not offered (prohibited) | developers "may not collect, store, or intermediate Claude.ai credentials or session tokens" ([legal](https://code.claude.com/docs/en/legal-and-compliance)) |
| sharing a plan, rotating or failing over between accounts, hosting it for others | no | not offered (prohibited) | you may not make your account available to anyone else ([consumer terms](https://www.anthropic.com/legal/consumer-terms)); third parties may not route requests through plan credentials on behalf of their users ([legal](https://code.claude.com/docs/en/legal-and-compliance)) |

### openai (chatgpt plus, pro)

| route | in anamnesis | status | what the provider says |
|---|---|---|---|
| the brain as an mcp server for codex | yes ([mcp.md](mcp.md)) | **allowed** | codex configures `[mcp_servers]` over stdio or streamable http ([codex mcp](https://developers.openai.com/codex/mcp)) |
| your own api key | yes | **allowed** | "API keys are still the recommended default for automation" ([codex auth](https://learn.chatgpt.com/docs/auth)) |
| Continue with ChatGPT (plan usage) in an open-source, locally run app | yes, chat only, experimental | **allowed (preview)** | the docs "explain ChatGPT plan usage for open-source and locally hosted apps"; paid or remotely hosted apps go through an interest form ([sign in with chatgpt: open-source apps](https://developers.openai.com/siwc/token-sharing-open-source); [quickstart](https://developers.openai.com/siwc/quickstart); [preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)) |
| reusing the codex cli's stored chatgpt login | no | not offered | the auth file is to be treated "like a password"; no page sanctions third-party reuse ([codex auth](https://learn.chatgpt.com/docs/auth)) |

### google (ai pro, ultra)

| route | in anamnesis | status | what the provider says |
|---|---|---|---|
| gemini api key (google ai studio) | yes | **allowed** | for third-party agents google recommends "a Gemini Enterprise or Google AI Studio API key" ([antigravity faq](https://antigravity.google/docs/faq.md)) |
| the brain as an mcp server for antigravity or gemini cli | yes ([mcp.md](mcp.md)) | **allowed** | antigravity supports mcp in its app, cli and ide ([antigravity mcp](https://antigravity.google/docs/mcp)) |
| consumer-plan logins (gemini cli or antigravity oauth), or wrapping `agy` | no | not offered (prohibited) | using the services behind gemini cli from third-party software is "a violation of applicable terms and policies" ([gemini cli terms](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/tos-privacy.md)); wrapping `agy` in third-party applications or automating account rotation "is not supported under consumer plans" ([antigravity faq](https://antigravity.google/docs/faq.md)); gemini cli stopped serving ai pro and ultra plans on 2026-06-18 ([google developers blog, 2026-05-19](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/)) |

free-tier api keys may come with different data-use terms than paid ones; read google's terms for unpaid services before sending your notes through one.

## optional, grey: relabelling claude cli requests

`settings.harness.claude_cli.relabel_traffic` (off by default; settings → connections → claude cli · relabel) sets `CLAUDE_CODE_ENTRYPOINT=anamnesis` on the cli child, so requests identify themselves as coming from anamnesis rather than with the cli's own label. claude code has been reported to weigh scripted (agent sdk) requests about twice as heavily against plan limits; anthropic has not documented this. the same technique ships in openbot and in nous research's hermes "claude subscription directsdk" plugin. it is a grey area: anthropic's legal page says plan limits "assume ordinary, individual usage" and that it may enforce "without prior notice". it affects only the user's own account.

two kill switches: `relabel_disabled_by_policy` in settings (per install), and `RELABEL_WITHDRAWN` in `crates/anamnesis-harness/src/adapters/claude_cli.rs` (project-wide; set it to `true` and release when anthropic says third-party apps may not do this — the ui then hides the option). the notice shown before it turns on:

> label requests as "anamnesis" instead of the cli's own label. claude code meters requests differently depending on how they say they were made; scripted (agent sdk) requests have been reported to count about twice as heavily against your plan limits. with this on, anamnesis's requests identify themselves as coming from anamnesis rather than as agent sdk calls, so they are likely to be metered like interactive use. anthropic has not documented this weighting or said whether third-party apps may do this. it is a grey area: it could be treated as getting around your plan's limits, and anthropic can restrict or suspend accounts without notice. it affects only your own account. anamnesis turns this off for everyone if anthropic says it is not allowed.

### hygiene on the cli route

- **clean environment.** the child never inherits `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, the foundry keys, `CLAUDE_CODE_USE_BEDROCK|VERTEX|FOUNDRY|MANTLE|ANTHROPIC_AWS` or `CLAUDE_CODE_SIMPLE`, any of which would silently move it off your subscription login (to a per-token key, another endpoint or a cloud provider). `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_CONFIG_DIR` are yours and are left alone.
- **isolation** (`settings.harness.claude_cli.isolate`, on by default): `--disable-slash-commands`, `--settings '{"disableAllHooks":true,"autoMemoryEnabled":false}'`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` and `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`, plus the existing `--tools ""`, `--strict-mcp-config`, `--no-session-persistence` and an empty working directory. your CLAUDE.md, memory, hooks, skills and slash commands stay out of anamnesis chats. values in the `env` block of your own claude code `settings.json` still apply; check it if you set an api key or endpoint there.

## what anamnesis will not do

- **read your credentials.** not the claude cli's login, not codex's or gemini's auth files, not another app's keychain entries.
- **copy tokens.** the only token it holds is its own, from openai's sign-in for open-source apps.
- **rotate accounts** or fail over from one plan to another.
- **relabel traffic without asking.** the claude cli's own request label is left alone unless you turn on the optional relabel switch (below); anamnesis never presents itself as another client.
- **run background jobs on a subscription.** labelling, extraction and anything scheduled run locally (or on an api key only if you opted in or named it), never on a chatgpt or claude plan.
- **fall back silently** to a paid backend.
