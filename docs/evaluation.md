# evaluation

anamnesis measures itself on your own use, on your machine, from the event log. every number in the evaluation panel (`GET /api/eval`, `anamnesis eval`) comes with its sample size, an interval and a status, and is classed as **measured**, **modelled** or **hypothesis**. the code is `crates/anamnesis-eval` ([its readme](../crates/anamnesis-eval/README.md) has the formulas in brief).

the guiding question is the one in the name:

> is learning recollection? what share of the links i later make by hand was already latent in what i had written?

## principles

- **from the log only.** every metric is a pure function of the hash-chained event log. random draws are seeded, so the same log always gives the same numbers, and you can recompute them from an export.
- **randomised where possible.** link suggestions keep a held-out control arm; resurfacing randomises algorithm against chance. without a control, a number cannot separate "the system was right" from "you did what it showed you".
- **intervals, always.** 95% intervals (bootstrap, wilson or bayesian credible intervals, as stated per metric).
- **"measuring" until there is enough data.** below the thresholds below, a claim is a hypothesis, whatever its method.
- **n = 1.** this is one person over time, not a population. observations are not independent (a busy week produces many links); intervals are approximate and describe *your* brain, not people in general.

## latent links

**what it answers:** of the links you made by hand, what share had anamnesis already predicted from what you had written, before you made them?

**how it is measured.**

- after each change, the brain suggests links for new and changed items (cosine neighbours rescored with adamic–adar over existing links, a same-region bonus, and penalties for near-duplicates and for notes from the same writing session).
- every suggestion is logged as `SuggestionMade` with `shown`. **about 20% of suggestions are randomly held out** (`shown: false`) and never displayed. the arm of a pair is a seeded function of the pair, so a pair keeps its arm if suggested again. the control arm cannot be seen or decided from the ui or the api.
- **eligible is not displayed.** `shown: true` only makes a suggestion eligible. it reaches you when the ui actually puts it on screen: opening an item fetches its suggestions (`GET /api/suggestions?item=`), and the server then records `SuggestionDisplayed` for the ones you have not been shown before. accepting or dismissing a suggestion also implies you saw it. everything about exposure (the shown arm, the exposure lift, acceptance) counts **displayed** suggestions only; eligible suggestions nobody looked at are in neither arm. suggestions handed to an external harness over mcp are not counted as displayed.
- later links are matched to suggestions as unordered pairs; the first link per pair counts. a pair is *predicted* if a suggestion with rank ≤ 5 was made strictly before the link.
- **hand links** are links you wrote in the text (`[[wikilinks]]`, markdown links) or made by hand (ui or mcp). accepting a suggestion is caused by being shown it, so accepted suggestions count only in the shown arm.

**the estimator.** let *H* be hand links on held-out predicted pairs, *U* hand links nobody predicted, and *π_h* the share of predicted pairs that were held out (read from the log). held-out pairs are a random sample of predictions that you never saw, so *H/π_h* estimates how many predicted pairs you would have linked anyway:

```
latent share = (H/π_h) / (H/π_h + U)
```

the same formula on the shown arm (*S* hand and accepted links on pairs you were actually shown, *π_s* the share of predicted pairs actually displayed) is reported beside it; it is contaminated by exposure, and by which items you chose to open, and the gap between the two is the **exposure lift** (how much showing a suggestion changes what you link). the interval is monte carlo over jeffreys posteriors of the counts.

**it is modelled, not measured:** the estimator assumes that showing suggestions does not change how many *unpredicted* links you make. that assumption is plausible but untestable from your data alone, so the honesty table keeps this claim in "modelled" even with plenty of data.

also reported: hit rate per arm (share of predicted pairs linked within 90 days, wilson interval; pairs whose 90 days are not over yet are excluded, not counted as misses), precision@k, recall@k and mrr of each item's first suggestion list (for all suggestions, and for held-out ones only), and days from prediction to link.

**enough data:** at least 50 hand links and 30 held-out pairs whose window has closed. expect months of use, not days.

**what to expect.** suggestions on personal notes should score far below link recommenders on encyclopaedic text; a low latent share is a real, reportable answer, not a failure of the method.

## why acceptance rate is not quality

the panel shows how often you accept suggestions you were actually shown (accepted ÷ displayed; the report also gives how many were eligible), labelled as *not* a quality measure. with nothing displayed yet it reads "measuring" and is a hypothesis; it needs 30 displayed suggestions. acceptance depends on what you were shown, when, how it was worded and how tired you were; it rewards suggestions that are easy to say yes to, such as near-duplicates. a system can raise acceptance while getting worse at finding what you would have linked anyway. the held-out arm is what measures that.

## retrieval

**what it answers:** does search find what you meant?

you label results (`POST /api/eval/label`, grade 0–3; the latest label for a query and item wins). anamnesis re-runs each labelled query through its current hybrid search and scores **ndcg@10** (gain 2^grade − 1), **p@5** (grade ≥ 1 counts as relevant) and **mrr**, with bootstrap 95% intervals over queries.

**enough data:** 25 labelled queries. information-retrieval practice asks for at least 25 topics, 50 being the classic trec size (voorhees & buckley, *the effect of topic set size on retrieval experiment error*, sigir 2002); at 25–50 queries only large differences (about 0.1 ndcg) are reliably detectable. because the queries are re-run on every evaluation, you can compare embedders: switch the embedder, let it re-embed, and look again.

## resurfacing (a micro-randomised trial)

**what it answers:** are the notes the algorithm brings back more useful than random ones?

each day's review draws a few items from three pools: *due* (scheduled seeds and open questions, by an fsrs-style interval), *contextual* (near what you wrote in the last two days) and *serendipity* (old, far from recent attention, rarely shown). **each slot of each day is independently assigned to the algorithm arm or to a random arm** that picks a uniformly random eligible item. the ui does not show which arm a pick came from. you rate a pick useful or not.

**the analysis:** useful-rate per arm with jeffreys beta(k + ½, n − k + ½) credible intervals; the difference and p(algorithm > random) by seeded monte carlo. readiness from a two-proportion power calculation (α = 0.05 two-sided, power 0.8):

| effect | per arm | total |
|---|---|---|
| 70% vs 40% (large) | 40 | ~80 |
| 60% vs 40% (moderate) | 95 | ~190 |

**enough data:** once the smaller arm has 95 ratings. the report also says which effect the current sample could detect.

## distilled notes worth keeping

**what it answers:** of the notes distil proposed from your own words, how many did you keep?

every proposal is a `DistilProposed` event and every answer a `DistilDecided` (keep or drop). the headline is **kept / proposed over the proposals you answered**, with a 95% wilson interval; proposals still waiting are counted (`pending`) but are evidence of nothing, and are left out. the report also splits it by distiller (the rules, a local model, an api model), so you can see whether a model earns its keep over the rules.

**enough data:** 40 answered proposals (the interval is then about ±15 points around one half). before that it reads "measuring" and the honesty table calls it a hypothesis.

**what it does not say:** there is no control arm. a high rate says the proposals were worth keeping to you, not that distil beats anything else, and you may keep more of them while the feature is new (see the novelty split below).

## growth and lineage

items over time (by month, by speaker, by origin, by kind), links by kind, live regions per clustering run, and births, splits, merges and deaths of regions ([architecture.md](architecture.md#regions-and-lineage)). growth is descriptive: it is shown, not claimed.

## the novelty split

new features get rated kindly. every section carries `novelty`: the same metric for the feature's first 28 days and for after. if the "after" number is clearly lower, trust it more.

## the honesty table

`anamnesis eval` prints it; the panel shows it. each headline claim is classed:

| class | means |
|---|---|
| **measured** | a randomised comparison or a labelled test set, with an interval and enough data |
| **modelled** | an estimate that rests on a stated modelling assumption (e.g. the latent share) |
| **hypothesis** | not yet supported: too little data, or not measured at all |

a claim whose metric is still "measuring" is always a hypothesis. **n always counts observations of you**: hand links, suggestions actually displayed, labels, ratings. what the app does on its own (suggestions it computed, pairs it held out, clustering runs) never makes an n by itself, so a brain nobody has used yet reads "measuring" everywhere. in particular:

- the held-out hit rate needs 30 closed held-out pairs *and* 50 hand links: a hit rate of zero from a brain where you never link by hand says nothing.
- the exposure lift and acceptance need suggestions that were actually displayed (30 of them).
- growth needs clustering runs on at least two different days: one import followed by two quick recomputes says nothing about identity over time.
 "it thinks like you" stays a hypothesis: nothing here measures it, and any voice-like metric would use only items whose origin is your own words.

## the demo brain is a demonstration

the demo brain ([demo.md](demo.md)) has no hand-made links: its cross-author links were added by the builder where one author names the other. any "latent links" number on it shows the pipeline working, nothing more. the ui says so.

## project benchmarks (not your brain)

the lab (on the `lab` branch: [lab/README.md](https://github.com/aphesist/anamnesis/tree/lab/lab/README.md)) checks the components on public data. these numbers describe the parts, not your use:

| benchmark | result (2026-11-11) |
|---|---|
| beir scifact, ndcg@10 | hash 0.327 · bm25 0.659 · multilingual-e5-small 0.682 |
| beir nfcorpus, ndcg@10 | hash 0.125 · bm25 0.305 · multilingual-e5-small 0.306 |
| layout trustworthiness (k = 10), 2,000 demo notes, hash embeddings | ≈ 0.67 for umap, warm-started umap and pacmap |
| layout drift between growing snapshots | warm-started umap roughly halves drift in later snapshots at equal trustworthiness |
| `anamnesis-mind`, 20,000 × 384-d synthetic vectors, loaded 4-core machine | regions 0.1–0.16 s; fresh layout 5.5–7.9 s |

the layout rows compare python reference methods; the rust layout has not yet been run through the same harness. link prediction on the demo brain is reported in the lab as a demonstration only.

## try it on synthetic data

```sh
cargo run -p anamnesis-eval --example synthetic -- honesty
cargo run -p anamnesis-eval --example synthetic -- report > eval.json
```

this simulates about 6,000 events of one user over 180 days, with known effects, and prints what the evaluation would conclude.
