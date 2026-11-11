# faq

**does anything leave my machine?**
not unless you configure a remote model (or add a feed or web page, which is fetched from its site). the default embedder runs locally, and chat without a model uses retrieval only. every call to a remote model is logged as an `Egress` event with counts and ids, never the text. [privacy.md](privacy.md).

**do i need a model at all?**
no. storing, linking, clustering, the layout, search, threads, review and the evaluation all work without one. a model makes chat write answers and gives regions better names.

**can i use my claude / chatgpt / gemini subscription?**
the cleanest way is to connect your own claude code, claude desktop, codex, gemini cli or antigravity to anamnesis over [mcp](mcp.md): the plan stays inside the provider's own app. inside anamnesis, chatgpt plans work through openai's "Continue with ChatGPT" sign-in for open-source apps (experimental). for claude there is an opt-in, clearly labelled grey route through your own installed `claude` cli, for interactive chat only. google consumer logins are not offered, because google's terms prohibit using them from third-party software; use a gemini api key. the table with sources is in [models.md](models.md#provider-terms).

**why won't anamnesis just use my claude.ai login token?**
because anthropic's terms say developers "may not collect, store, or intermediate" those credentials, and google's say the same about gemini cli logins. tools that copy tokens put their users' accounts at risk. anamnesis never reads another program's credentials, never copies tokens, never rotates accounts and never runs background work on a plan.

**is the "brain" shape meaningful?**
nearness is: items close together are similar. the two-hemisphere shape and the anatomical region names are ornament, chosen to be memorable and stable. the *layout* end of the form slider shows the plain similarity layout.

**why do region names stay the same when the regions change?**
on purpose. each clustering run is matched to the last one, so a region keeps its id, label and name while its members change, and splits and merges are recorded as lineage. without that, every recompute would rename everything and history would be unreadable.

**what is the "latent links" number?**
an estimate of the share of the links you made by hand that anamnesis had already predicted, before you made them, from what you had written. it relies on suggestions that were held out and never shown. it needs months of use to mean anything, and it is classed as *modelled*, not measured. [evaluation.md](evaluation.md#latent-links).

**why are some suggestions never shown?**
about one in five is held back at random as a control group. without it, the app could not tell "i predicted your link" from "you linked what i showed you".

**the evaluation says "measuring". is it broken?**
no. it means there is not yet enough data to say anything. the thresholds are in [evaluation.md](evaluation.md); for links and resurfacing they take months, not days.

**can i trust numbers from the demo brain?**
no, and the app says so. its links were added by a script from explicit mentions. it demonstrates the pipeline; it measures nothing. [demo.md](demo.md).

**obsidian?**
point the `obsidian` connector (or `anamnesis import`) at your vault. frontmatter, aliases, tags, wikilinks and embeds are read; your files are never written to.

**where is my data, and how do i back it up?**
one sqlite file plus a few caches in `<data dir>/anamnesis/` ([install.md](install.md#where-things-live)). `anamnesis export <file>` writes the whole event log as json lines; `anamnesis import <file>` restores it into an empty brain. `anamnesis verify` checks the hash chain.

**can i delete something completely?**
removing a source removes its items from the brain, but the append-only log still holds the events that carried their text. true redaction is not implemented yet. [privacy.md](privacy.md#deleting).

**which languages work?**
the default embedder (multilingual-e5-small) covers about a hundred languages and is tuned for retrieval; english and portuguese were the design targets. thread detection has english and portuguese cue phrases.

**windows, macos, linux?**
it is plain rust with bundled sqlite and should build on all three. ci runs on linux.

**can i run it on a server and open it from another device?**
not as shipped: it binds to `127.0.0.1` and refuses non-loopback `Host` headers, and there is no login. an ssh tunnel (`ssh -L 7368:127.0.0.1:7368 host`) works if you need it.

**how do i make it calmer?**
more → reduce motion, or settings → motion. the first-launch dialog asks once.

**can i change the look completely?**
yes: themes, colours, fonts, logo and panel layout. [theming.md](theming.md).

**how do i add a source anamnesis does not support?**
write a connector plugin: an executable in any language that prints json lines. [connectors.md](connectors.md#plugins). note that `anamnesis serve` does not load plugins yet.

**what does "anamnesis" mean?**
recollection. in plato's *meno* and *phaedo*, learning is not taking in something new but recalling what the soul already knew. the evaluation turns that into a question you can put to your own notes: how much of what you later connect was already there?
