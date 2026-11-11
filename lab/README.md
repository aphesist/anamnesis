# anamnesis lab

an optional python workbench next to anamnesis. it is **not part of the user install**: the
`anamnesis` binary never runs python, and nothing here is needed to use the app. the lab does
two things:

1. **demo brains** (`anamnesis_lab.demo`): builds importable bundles of markdown notes from texts
   that are public domain in both the united states and brazil, so anamnesis can be shown with a
   real, dated brain instead of anyone's private notes.
2. **benchmarks** (`anamnesis_lab.bench`): retrieval on beir subsets, link prediction on a temporal
   split of a demo brain, and layout stability (umap vs pacmap vs the rust layout).

## setup

needs [uv](https://docs.astral.sh/uv/) and python ≥ 3.11. dependencies are pinned in
`pyproject.toml` and `uv.lock`.

```sh
cd lab
uv sync                  # builders + tests (numpy only)
uv sync --extra bench    # + onnxruntime, tokenizers, huggingface-hub, scipy, scikit-learn, umap-learn, pacmap
uv run pytest            # offline, about a second
```

downloads (texts, beir datasets, onnx models) go to `lab/cache/`; bundles to `lab/out/`. both are
git-ignored. **only explicit commands download anything; the tests never touch the network.**

## demo brains

```sh
uv run anamnesis-lab demo transcendentalist            # downloads ~30 MB once, writes lab/out/transcendentalist
uv run anamnesis-lab demo transcendentalist --small    # offline, ~290 notes from a bundled fixture (for ci/tests)
uv run anamnesis-lab demo pragmatist                   # partial, see below
```

### the transcendentalist brain

the demo owner is "the transcendentalists". journal entries are `origin: owner_written`, essays
and commonplace-book quotations are `origin: external`, so the "when you wrote" speaker chart has
both voices.

| part | source | edition |
|---|---|---|
| thoreau, journal 1837 – 15 sept 1851 | clean e-text (2 vols.) | *the writings of henry david thoreau*, vols. 7–8, journal i–ii (houghton mifflin, 1906), ed. bradford torrey |
| thoreau, journal 16 sept 1851 – 1861 | internet archive ocr | the same 1906 edition, vols. iii–xiv |
| emerson, journals 1820–1876 | internet archive ocr (10 vols.) | *journals of ralph waldo emerson, with annotations* (houghton mifflin, 1909–14), ed. edward waldo emerson and waldo emerson forbes |
| emerson, *nature*, *essays* i and ii | clean e-text | 1836/1849, 1841, 1844 |
| thoreau, *walden*, *a week*, *civil disobedience*, *walking* | clean e-text | 1854, 1849, 1849, 1862 |

what the builder does:

- strips every distributor wrapper: the licence header and footer, producer credits, transcriber's
  notes and every line naming the e-text project. each note is checked before it is written and the
  build fails if a mention survives. the source is credited only in `COLOPHON.md` (and listed in
  `SOURCES.md` / `manifest.json`); notes cite a scan of the same edition on archive.org as `source_url`.
- drops editors' matter: introductions, footnotes, bracketed editorial notes, page-foot annotations,
  running heads, notebook labels like "(From Journal B)".
- splits journals into dated entries. dates such as `_Oct. 24._`, `Sept. 16. Thursday.`,
  `Concord, March 23, 1835.` or the ocr misreading `Fanuary 3.` become `created`; the year comes from
  explicit years, chapter headings, year lines and printed running heads, and rolls over at december
  → january. emerson's long undated stretches become entries dated to the year (`date_precision: year`).
  long entries are split at paragraph boundaries.
- splits books into sections (essays, chapters, days of *a week*), with an index note per book.
- adds `[[wikilinks]]` only where one author explicitly names the other or a distinctive title of the
  other's ("Henry Thoreau", "Mr. Emerson", "R. W. E.", "Self-Reliance", "Week on the Concord"…).
  a name in a dated entry links to the other author's entry of the same day when there is one,
  otherwise to a short person note. thoreau's many citations of g. b. emerson's *report on the trees*
  are excluded. this gives "links made later" something to show; **it is a demonstration, not a
  measurement**, and the app should say so.

output:

```
out/transcendentalist/
  notes/<author>/<journal|essay|index>/<basename>.md   point the folder or obsidian connector here
  notes/anamnesis-lab/hub/<person>.md
  manifest.json      counts (by author, kind, origin, decade), date range, links, licence records
  SOURCES.md         per edition: contributors with birth/death years, us and brazil status
  COLOPHON.md        where the texts came from
```

each note:

```yaml
---
title: "Met the selectmen of Sudbury, and"
created: 1851-09-16T12:00:00Z
tags: ["thoreau", "journal", "1850s"]
author: "Henry David Thoreau"
source_url: "https://archive.org/details/thoreau-journal-1906lim-ed"
origin: owner_written
kind: journal            # journal | letter | essay | index | hub
date_precision: day      # day | year
source: "The Writings of Henry David Thoreau, … (1906)"
---
```

basenames are unique across the bundle, so `[[basename]]` resolves the way obsidian resolves it.
note that the dates are 1820–1876: `created` is before the unix epoch, so unix-second timestamps
are negative.

a real build (2026-11-11) gave 9,234 notes: 5,010 thoreau journal entries (1837-10-22 to
1861-05-14; after that the 1906 edition prints the last entries without dates in the text, so they
stay with the last dated entry), 3,543 emerson journal entries (3,249 with a day, 1820-03-11 to
1875-12-05; the rest dated to the year), 674 essay sections, 5 book index notes and 2 person notes;
about 3.1 million words and 366 cross-author links (51 of them to the other author's entry of the
same day). run it yourself to get exact counts; the corpus is never committed.

the ocr is imperfect (joined words like "ariver", occasional misread dates). the splitter is
heuristic; treat it as a demo corpus, not an edition.

### the pragmatist brain (partial)

same interface. builds *the letters of william james* (1920, 2 vols., dated letters, the editor's
connecting narrative skipped) and *pragmatism* (1907, lectures). **not built yet**: peirce's
1877–78 papers, dewey's pre-1931 books, *the will to believe*, cross-author links, and a dedicated
fixture (`--small` uses a tiny letters fixture). the manifest carries `"partial": …`.

## benchmarks

```sh
uv sync --extra bench
uv run anamnesis-lab bench retrieval --dataset scifact,nfcorpus --models hash,bm25,e5-small
uv run anamnesis-lab bench links  --bundle out/transcendentalist --models hash,e5-small
uv run anamnesis-lab bench layout --bundle out/transcendentalist --models hash --methods umap,umap-warm,pacmap \
                                  [--rust-placements placements.json]
uv run anamnesis-lab report > results/README-tables.md     # markdown tables from results/*.json
```

models: `hash` (feature-hashed words and bigrams, no download: a floor), `bm25` (retrieval only),
`e5-small` (`intfloat/multilingual-e5-small`, the onnx export at `Xenova/multilingual-e5-small`, run
with onnxruntime + tokenizers, mean pooling, l2-normalised, `query:`/`passage:` prefixes; the model the
rust embed crate is meant to default to), `e5-small-int8`, `minilm` (`all-MiniLM-L6-v2`). embeddings
are cached in `cache/emb/`.

- **retrieval**: beir test split, nDCG@10 (graded) and Recall@100, with 95% bootstrap intervals over
  queries. `--max-docs/--max-queries` make quick subsets (every judged document is kept).
- **links**: queries are notes written after the `--split` time quantile that link to existing notes;
  suggestions are the notes that existed on the query's date, ranked by similarity (the snapshot the
  app would have had). hits@k, recall@10, mrr against random and popularity baselines. in the demo
  brain the "true" links are heuristic references, so this is a demonstration of the pipeline.
- **layout**: notes sorted by time, laid out at growing snapshots (50% … 100%). trustworthiness and
  continuity (k=10) against the embedding space, and procrustes drift between consecutive snapshots
  on the shared notes (`disparity` is scale-free; `rms_relative` is displacement over spread).
  `umap-warm` starts each snapshot from the previous layout. the rust layout is compared by passing a
  placements file:

  ```json
  {"snapshots": [{"label": "50%", "placements": [{"item_id": "01J…", "xyz": [0.1, 0.2, 0.3], "key": "<note basename>"}]}]}
  ```

  `key` (or `basename`/`title`/`path`) ties a placement to a bundle note so its embedding can be
  looked up for trustworthiness/continuity; drift needs only stable `item_id`s across snapshots.

results land in `results/<benchmark>-<name>.json` and are small enough to commit.

### results (2026-11-11, cpu, 2 threads)

retrieval matches published numbers closely (bm25 on scifact ≈ 0.66), and multilingual-e5-small edges bm25 on both sets. link prediction on the demo brain is a demonstration: only 30 later notes have content links (same-day cross-author references), and the hash baseline finds none of them in the top 10 — e5-small was not run on it. layout: 2,000 sampled notes, hash embeddings; warm-starting umap roughly halves drift in later snapshots at equal trustworthiness. no rust placements file exists yet, so the rust column is empty.

#### layout stability — transcendentalist (2026-11-11, 2000 notes, embeddings: hash)

| method | snapshot | trustworthiness | continuity |
|---|---|---|---|
| umap | 1000 | 0.655 | 0.740 |
| umap | 1333 | 0.664 | 0.773 |
| umap | 1667 | 0.667 | 0.783 |
| umap | 2000 | 0.666 | 0.789 |
| umap-warm | 1000 | 0.655 | 0.740 |
| umap-warm | 1333 | 0.664 | 0.774 |
| umap-warm | 1667 | 0.667 | 0.786 |
| umap-warm | 2000 | 0.672 | 0.789 |
| pacmap | 1000 | 0.664 | 0.689 |
| pacmap | 1333 | 0.672 | 0.709 |
| pacmap | 1667 | 0.681 | 0.729 |
| pacmap | 2000 | 0.677 | 0.730 |

drift between consecutive snapshots:

| method | snapshot | shared | disparity | rms (relative) |
|---|---|---|---|---|
| umap | 1333 | 1000 | 0.309 | 0.556 |
| umap | 1667 | 1333 | 0.203 | 0.450 |
| umap | 2000 | 1667 | 0.292 | 0.540 |
| umap-warm | 1333 | 1000 | 0.316 | 0.563 |
| umap-warm | 1667 | 1333 | 0.180 | 0.425 |
| umap-warm | 2000 | 1667 | 0.154 | 0.393 |
| pacmap | 1333 | 1000 | 0.383 | 0.619 |
| pacmap | 1667 | 1333 | 0.243 | 0.493 |
| pacmap | 2000 | 1667 | 0.176 | 0.420 |

#### link prediction — transcendentalist (2026-11-11, demonstration)

split at 1852-11-01 (quantile 0.5): 4619 earlier notes, 30 later notes with links to earlier ones; target kinds {'journal': 30}

| model | queries | hits@1 | hits@10 | hits@50 | recall@10 | MRR |
|---|---|---|---|---|---|---|
| random | 30 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| popularity | 30 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| hash | 30 | 0.000 | 0.000 | 0.033 | 0.000 | 0.001 |

#### retrieval — nfcorpus (2026-11-11)

| model | docs | queries | nDCG@10 | 95% CI | Recall@100 | 95% CI | s |
|---|---|---|---|---|---|---|---|
| hash | 3633 | 323 | 0.125 | [0.103, 0.152] | 0.148 | [0.126, 0.172] | 3 |
| bm25 | 3633 | 323 | 0.305 | [0.269, 0.344] | 0.239 | [0.211, 0.269] | 0 |
| e5-small | 3633 | 323 | 0.306 | [0.270, 0.342] | 0.276 | [0.245, 0.303] | 526 |

#### retrieval — scifact (2026-11-11)

| model | docs | queries | nDCG@10 | 95% CI | Recall@100 | 95% CI | s |
|---|---|---|---|---|---|---|---|
| hash | 5183 | 300 | 0.327 | [0.276, 0.376] | 0.614 | [0.558, 0.667] | 3 |
| bm25 | 5183 | 300 | 0.659 | [0.614, 0.704] | 0.889 | [0.854, 0.923] | 1 |
| e5-small | 5183 | 300 | 0.682 | [0.636, 0.723] | 0.941 | [0.916, 0.966] | 1030 |


## copyright notes

the bar for a demo brain is **public domain in the united states and in brazil**.

- united states: works published before 1931 are public domain as of 2026.
- brazil: lei 9.610/98, art. 41: "setenta anos contados de 1° de janeiro do ano subseqüente ao de seu
  falecimento". a person who died in year *d* is public domain from 1 january of *d* + 71. editors and
  translators are authors of their contributions, so every person on an edition is checked, and the
  builder refuses a source whose last contributor is not yet clear.

the editions used:

| person | role | dates | brazil-pd from |
|---|---|---|---|
| henry david thoreau | author | 1817–1862 | 1933 |
| ralph waldo emerson | author | 1803–1882 | 1953 |
| bradford torrey | editor, thoreau's journal (1906) | 1843–1912 | 1983 |
| francis h. allen | co-editor, thoreau's journal (1906) | 1866–1953 | 2024 |
| edward waldo emerson | editor, emerson's journals (1909–14) | 1844–1930 | 2001 |
| waldo emerson forbes | co-editor, emerson's journals | 1879–1917 (genealogy records; no library authority found) | 1988 |
| william james | author | 1842–1910 | 1981 |
| henry james (the son) | editor, letters of william james (1920) | 1879–1947 | 2018 |

forbes's dates should be confirmed against a library authority record; the builder drops the
editors' annotations anyway, which leaves only emerson's own text. the harvard *journals and
miscellaneous notebooks* (1960–82) are in copyright and are not used.

why other themes are not possible, and the traps:

- **nick land**: alive, so protected everywhere.
- **julius evola**: died 1974, so protected in brazil until 1 january 2045. a few italian originals
  from before 1931 are us-pd but not brazil-pd, and the english translations carry their own copyrights.
- **seneca in gummere's loeb translation** (1917–25): gummere died in 1969. us-pd, but not
  brazil-pd until 2040. use aubrey stewart's bohn translations or lodge (1614).
- **nietzsche in kaufmann's translations**: kaufmann died in 1980, protected in both countries.
  common's and zimmern's translations are fine; ludovici's *selected letters* is us-pd but not brazil-pd.
- **guénon in english**: the 1942 luzac translators died long after 1955 (not brazil-pd) and the
  translation was most likely restored in the us until 2037. the french originals up to 1930 clear both.
- **page's 1956 revision of mackenna's plotinus**: a separate, later work. only mackenna's
  1917–1930 volumes qualify.
- sites with their own reuse conditions (e.g. sacred-texts' non-commercial file notice) are avoided
  even for public-domain texts.

a distributor's trademark terms are avoided by removing its licence and name from the text and
crediting it only in the colophon. outside the us, local law applies, which is why the brazil test is
part of the bar.

## layout

```
lab/
  pyproject.toml, uv.lock
  src/anamnesis_lab/
    demo/            base pipeline, transcendentalist.py, pragmatist.py
    bench/           metrics.py (numpy only), embedders.py, retrieval.py, linkpred.py, layout.py, report.py
    fixtures/        original fixture texts for --small (regenerate: uv run python scripts/make_fixtures.py)
    gutenberg.py, dates.py, textclean.py, journal.py, links.py, bundle.py, sources.py, cli.py
  tests/
  results/
```

code is apache-2.0 like the rest of anamnesis. the fixtures are original text written for the lab.
