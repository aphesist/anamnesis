# anamnesis-eval

honest, user-facing metrics computed from the event log. pure functions over
`&[anamnesis_core::Event]` → serialisable structs. no storage, no network; random
draws are seeded, so the same log always gives the same numbers.

```rust
let report = anamnesis_eval::evaluate(&events, Some(&mut |q: &str| search(q)), None, &Default::default());
// report is the body of GET /api/eval; report.honesty is the eval panel's table
```

## links — "what share of the links i later make by hand was already latent?"

- suggestions are `SuggestionMade { shown }`; about 20% of top-k are randomly held out.
- links match suggestions as unordered pairs; first link per pair; `rank <= k` (k = 5) is a prediction.
- hand links are `manual` + `written`. accepted suggestions only ever count in the shown arm.
- **latent share** = (H/π_h) / (H/π_h + U): H = hand links on held-out predicted pairs,
  U = hand links nobody predicted, π_h = share of predicted pairs held out. the same
  estimator on the shown arm is reported next to it; the gap is the exposure lift.
  it is *modelled*: it assumes exposure does not change how many unpredicted links you make.
  interval: monte carlo over jeffreys posteriors of the counts.
- **hit rate** per arm: share of predicted pairs linked within 90 days, wilson interval.
  pairs whose window is still open are excluded (right-censoring), not counted as misses.
- **precision@k / recall@k / mrr** of each item's first suggestion list, for all
  suggestions (contaminated) and for held-out suggestions only.
- **acceptance rate**: reported, labelled as *not* a quality measure.
- **time to link**: days from first prediction to the link, per arm.
- status: "enough data" at ≥ 50 hand links and ≥ 30 held-out pairs with a closed window (an estimate).

## retrieval

ndcg@10 (gain 2^grade − 1), p@5 (grade ≥ 1), mrr over queries the user labelled
(`RetrievalLabeled`, latest label wins), bootstrap 95% intervals over queries.
status is "measuring" below 25 labelled queries: ir practice asks for at least 25 topics,
50 being the classic trec size (buckley & voorhees; voorhees & buckley, *the effect of
topic set size on retrieval experiment error*, sigir 2002). at 25–50 queries only large
differences (≈ 0.1 ndcg) are reliably detectable.

## resurfacing (micro-randomised trial)

useful-rate per arm (`algorithm` vs `random`) with jeffreys beta(k + ½, n − k + ½)
credible intervals; the difference and p(algorithm > random) by seeded monte carlo.
readiness from a two-proportion power calculation (α = 0.05 two-sided, power 0.8):

| effect | per arm | total |
|---|---|---|
| 70% vs 40% (large) | 40 | ~80 |
| 60% vs 40% (moderate) | 95 | ~190 |

status is "enough data" once the smaller arm reaches 95; `readiness.detectable` says which
effect the current sample could detect. n = 1 over time, so intervals are approximate.

## growth

items over time (by month, speaker, origin, kind), links by kind, live regions per
clustering run, births / splits / merges / deaths, mean region cohesion when supplied.

## novelty guard

every section carries `novelty: { feature_start, days: 28, first_days, after }`: the same
metric for the feature's first 28 days and after, because new features are rated kindly.

## honesty

`honesty(&report)` classifies each headline claim as **measured** (randomised or labelled,
with an interval and enough data), **modelled** (rests on an assumption) or **hypothesis**
(too little data, or not measured — "it thinks like you" stays here). `to_markdown()`
renders the table.

## synthetic

`synthetic::generate(&SyntheticConfig::default())` builds a deterministic log for a
simulated user (≈ 6k events over 180 days); `SyntheticSearch` is a matching fake search.

```sh
cargo run -p anamnesis-eval --example synthetic -- events  > events.jsonl
cargo run -p anamnesis-eval --example synthetic -- report  > eval.json
cargo run -p anamnesis-eval --example synthetic -- honesty
```
