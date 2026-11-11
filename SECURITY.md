# security

anamnesis holds what people write to themselves. treat a weakness in it as serious.

## reporting a vulnerability

please report privately, through github's **private vulnerability reporting** on this repository (the "security" tab → "report a vulnerability"). do not open a public issue for a security problem.

include what you found, how to reproduce it, the version or commit, and what an attacker gains. you should get an answer within a week. once a fix is released the report is credited, unless you ask not to be named.

## supported versions

anamnesis is pre-1.0. only the latest release and `main` receive fixes.

## what is in scope

anything that breaks the promises in [docs/privacy.md](docs/privacy.md), for example:

- text from the brain leaving the machine without an `egress` event, or an item marked `local_only` reaching a non-local model or an mcp client;
- the http api or `/mcp` being reachable from another host or another browser origin (the server binds to `127.0.0.1`, refuses non-loopback `Host` headers and requires a bearer token on `/mcp`);
- a way for text inside an item (a clipped page, a pdf, a feed entry) to make anamnesis delete data, fetch urls or send data somewhere: the mcp server has no delete, fetch or send tools, and that is deliberate;
- api keys or the app's own chatgpt token being written anywhere other than the os keychain (or the documented fallback file), logged, returned by the api or included in an exported bundle;
- anamnesis reading another program's credentials (the claude cli's login, codex's or gemini's auth files). it must never do this;
- breaking the hash chain of the event log without `anamnesis verify` noticing.

## out of scope

- connector plugins (`exec` runtime) run as ordinary processes with your permissions. their declared hosts and paths are shown, not enforced. a malicious plugin you installed is not a vulnerability in anamnesis.
- anything a model you chose does with text you chose to send it.
- attacks that need an attacker already running code as your user.
