# the demo brain

a second brain is only interesting once it holds years of writing, and nobody should have to show their private notes to see what anamnesis does. the demo brain is a real, dated corpus instead: the journals of henry david thoreau and ralph waldo emerson, 1820–1876, with their essays as the outside voice. two people who knew each other, wrote every day, and named each other in their journals.

a curated set of 1,100 notes ships with anamnesis: in this repository as `demo/transcendentalist/`, and inside release binaries. it also ships **already built**: `demo/transcendentalist/snapshot/` holds the demo brain as `anamnesis demo` leaves it (embedded with the local model, clustered into regions, laid out, with link suggestions), so the demo opens at once, with no model download and no waiting. the full corpus (about 9,200 notes) is built on your machine by the optional python lab. either way the demo brain is kept apart from your own (in `<data dir>/anamnesis/demo/`).

## show it

on a brand-new install (your own brain is empty) the app opens on the demo brain, with a banner: "this is a demo brain · bring your own notes", and two ways out: **import a folder…** (opens + add with a folder chosen) and **start empty**. either choice is remembered (a `first_launch_done` file in the data root), so the demo is not pushed on you again; **more → show a demo brain** brings it back any time.

```sh
anamnesis serve --demo    # opens the demo brain directly
```

the first time the demo is opened, `<data dir>/anamnesis/demo/` is seeded from the snapshot: each file is decompressed and checked against its sha256, the whole event chain is verified and must end at the head the snapshot's manifest names, and only then are the files moved into place (well under a second on a laptop). if anything does not check out, the demo is built from the notes instead, as below.

the local embedding model is then needed only for new text: your queries and anything you add. until it has downloaded (~465 MB, once) or loaded, search matches words only (full-text, bm25), and the ui says so under the search field: "search: words only until the model is ready". once the model is ready, search is hybrid again; the demo's vectors were made with the same model (`multilingual-e5-small`), so nothing is recomputed. with `--hash-embedder`, or if the model cannot load, the demo keeps its prebuilt layout rather than redraw itself from hashes.

### building it from the notes instead

```sh
anamnesis demo            # builds the demo brain from the notes, with progress (a few minutes on a laptop cpu)
```

an explicit `--demo-bundle <folder>` (or `ANAMNESIS_DEMO_BUNDLE`) always builds from those notes instead of seeding the snapshot; so does a binary that ships no snapshot. the first switch then imports the notes in a few seconds; the banner says how far the embeddings, regions and layout are, and the cloud redraws when they land. `anamnesis demo --rebuild` removes the demo brain and builds it afresh (after changing the embedder, for instance). `--hash-embedder` gives a quick, meaningless layout in seconds.

where the demo notes come from, first match wins:

1. `--demo-bundle <folder>` or `ANAMNESIS_DEMO_BUNDLE`;
2. a `demo/` folder next to the binary (a bundle folder, its `notes/`, or a folder holding `transcendentalist/notes`);
3. a full lab build in the repository (`lab/out/transcendentalist/notes`, then any other theme);
4. the curated `demo/transcendentalist/notes` in the repository;
5. the copy embedded in the binary (cargo feature `embed-demo`, on by default), extracted to `<data dir>/anamnesis/demo/transcendentalist/` on first use.

## the prebuilt demo brain

`demo/transcendentalist/snapshot/` is **generated data**: the curated notes below (public-domain texts, see `SOURCES.md`) run through a real `anamnesis demo` with the default local embedder. it holds `brain.sqlite.gz` (the event log and its projections, embeddings included), `mind_state.json.gz`, `views.json.gz` and `jobs.json.gz` (the layout and job caches the server keeps next to a brain), and `manifest.json` (the sha256 and size of each file, the event count and head hash, item and chunk counts, the embedding model). release builds embed it (cargo feature `embed-demo`); a debug build reads it from the repository.

to regenerate it (after changing the curated notes, the store schema or the pipeline):

```sh
cargo build --release -p anamnesis-server
python3 lab/scripts/demo_snapshot.py   # from the lab checkout          # python standard library only
```

the script runs `anamnesis demo --rebuild --demo-bundle demo/transcendentalist` in a temporary data dir (the local model is downloaded once if it is not cached), runs `anamnesis verify` on it, then packs it: `VACUUM INTO` a fresh rollback-journal database (no wal, no free pages), gzip -9 with a fixed header, and the manifest. the mcp token and anything else in the brain directory stay out. `--from-brain <dir>` packs a demo brain you already built. the procedure is reproducible; the bytes are not, because every event records when it was written and each brain draws its own hold-out seed.

## the curated set

> the lab lives on the repository's `lab` branch, not on `main`. to use the commands below, check it out next to your clone: `git worktree add ../anamnesis-lab lab`, then run them from `../anamnesis-lab` (paths like `lab/out/…` are relative to that checkout).

`demo/transcendentalist/` holds:

| | |
|---|---|
| `notes/` | 1,100 notes, 2.6 MB of markdown, about 373,000 words |
| `COLOPHON.md`, `SOURCES.md` | where the texts came from; each edition, its contributors and their death years |
| `manifest.json` | counts, the date range and how the set was chosen |

the notes are a deterministic subset of a full build, chosen by `anamnesis-lab demo curate`:

- the two person notes (henry david thoreau, ralph waldo emerson);
- every journal entry in which one author names the other or one of the other's works, and the other's entry of the same day when the link points at it (327 linked entries; 358 links in all, 278 to an earlier note);
- dated journal entries spread over every year of both journals (thoreau 1837–1861, 20 years; emerson 1820–1876, 57 years), evenly through each year, preferring clean, mid-length entries over the ocr's worst pages: 1,061 journal entries in all (466 thoreau, 595 emerson);
- 32 essay sections (a few of each of *nature*, *essays* i and ii, *walden*, *a week*, *walking* and *civil disobedience*, `origin: external`) and the five work index notes, so "when you wrote" has two voices.

a link whose target is not in the set is removed (its text stays). the note bodies carry no distributor's name; the source of each text is credited in `COLOPHON.md` and `SOURCES.md` only.

to rebuild the curated set from a full build (byte for byte the same for the same full build):

```sh
cd lab
uv run anamnesis-lab demo transcendentalist
uv run anamnesis-lab demo curate --from out/transcendentalist --out ../demo/transcendentalist --target-notes 1100
```

`--max-bytes` (default 3.9 MB) caps the size of the markdown.

## what is in the full corpus

| part | source edition |
|---|---|
| thoreau, *journal*, 1837–1861 | *the writings of henry david thoreau*, vols. 7–20 (houghton mifflin, 1906), ed. bradford torrey (and francis h. allen) |
| emerson, *journals*, 1820–1876 | *journals of ralph waldo emerson, with annotations* (houghton mifflin, 1909–14), ed. edward waldo emerson and waldo emerson forbes |
| emerson, *nature*, *essays* i and ii | 1836/1849, 1841, 1844 |
| thoreau, *walden*, *a week on the concord and merrimack rivers*, *civil disobedience*, *walking* | 1854, 1849, 1849, 1862 |

journal entries are stored as written by the demo's owner (`owner_written`); the essays are `external`, so the "when you wrote" chart has both voices. a full build gave about 9,200 notes (5,010 thoreau journal entries, 3,543 emerson journal entries, 674 essay sections), about 3.1 million words, and 366 cross-author links. the exact counts depend on the run; the lab writes them to `manifest.json`.

the builder removes every distributor wrapper and the editors' own matter (introductions, footnotes, bracketed notes, running heads), splits the journals into dated entries, splits the books into sections, and adds `[[wikilinks]]` only where one author explicitly names the other or one of the other's distinctive titles. dates are parsed from the text; emerson's undated stretches are dated to the year. the ocr of the later volumes is imperfect. treat it as a demonstration corpus, not an edition.

## public-domain status

the bar for a demo text is **public domain in both the united states and brazil**:

- **united states:** works published before 1931 are in the public domain as of 2026.
- **brazil:** lei 9.610/98, art. 41 protects a work for seventy years counted from 1 january of the year after the author's death, so a person who died in year *d* is in the public domain from 1 january of *d* + 71. editors and translators are authors of their contributions, so every person on an edition is checked.

| person | role | died | public domain in brazil from |
|---|---|---|---|
| henry david thoreau | author | 1862 | 1933 |
| ralph waldo emerson | author | 1882 | 1953 |
| bradford torrey | editor, thoreau's journal (1906) | 1912 | 1983 |
| francis h. allen | co-editor, thoreau's journal (1906) | 1953 | 2024 |
| edward waldo emerson | editor, emerson's journals (1909–14) | 1930 | 2001 |
| waldo emerson forbes | co-editor, emerson's journals | 1917 (genealogy records; to be confirmed against a library authority record) | 1988 |

every edition used was published before 1931, so the curated set in this repository and in the binary is public domain in both countries. the builder drops the editors' annotations in any case, leaving the authors' own text. the harvard *journals and miscellaneous notebooks* (1960–82) are still in copyright and are not used. the source of each text is credited in the build's `COLOPHON.md` and `SOURCES.md`, not inside the notes.

outside the united states and brazil, check your local law before redistributing a build.

## build the full corpus

> the lab lives on the repository's `lab` branch, not on `main`. to use the commands below, check it out next to your clone: `git worktree add ../anamnesis-lab lab`, then run them from `../anamnesis-lab` (paths like `lab/out/…` are relative to that checkout).

needs [uv](https://docs.astral.sh/uv/) and python 3.11 or newer.

```sh
cd lab
uv sync
uv run anamnesis-lab demo transcendentalist           # downloads ~30 MB of texts once, writes lab/out/transcendentalist
uv run anamnesis-lab demo transcendentalist --small   # offline, ~290 notes from a bundled fixture, seconds
```

a full build in `lab/out/transcendentalist` is found automatically and preferred over the curated set (`anamnesis demo --rebuild` to switch an existing demo brain over). to point at another build:

```sh
anamnesis demo --rebuild --demo-bundle lab/out/transcendentalist/notes
anamnesis serve --demo
```

or set `ANAMNESIS_DEMO_BUNDLE=lab/out/transcendentalist/notes` and switch with `POST /api/demo {"enabled": true}` (and back with `false`); `GET /api/demo` reports `jobs`, `chunks` and `chunks_embedded` while the brain is being arranged. with the default local embedder, embedding ~9,000 notes takes a long while on a laptop cpu. add `--hash-embedder` for a quick look.

you can also import the notes into a brain of your own like any folder: `anamnesis import lab/out/transcendentalist/notes`.

## a demonstration, not a measurement

the demo's links were added by a script, from explicit mentions. they are not links anyone made by hand while reading, so any "latent links" number on the demo brain shows the pipeline working and measures nothing. the lab's link-prediction benchmark on it is labelled the same way. the evaluation that means something is the one on your own brain ([evaluation.md](evaluation.md)).

## why not other authors

the double bar rules out more than one might expect:

- **nick land:** alive, so protected everywhere.
- **julius evola:** died 1974, protected in brazil until 1 january 2045. a few italian originals from before 1931 are public domain in the us but not in brazil, and the english translations carry their own copyrights.
- **seneca in gummere's loeb translation** (1917–25): gummere died in 1969, so public domain in the us but not in brazil until 2040. aubrey stewart's bohn translations or lodge (1614) would do.
- **nietzsche in kaufmann's translations:** kaufmann died in 1980; protected in both countries. common's and zimmern's translations are fine.
- **guénon in english:** the 1942 translators died long after 1955, and the translation was most likely restored in the us until 2037. the french originals up to 1930 clear both countries.
- **plotinus:** only mackenna's 1917–1930 volumes qualify; page's 1956 revision is a separate, later work.
- texts from sites with their own reuse conditions (for example a non-commercial notice on a public-domain file) are avoided.

a second, partial theme exists: the **pragmatist brain** (`uv run anamnesis-lab demo pragmatist`) with the dated *letters of william james* (1920) and *pragmatism* (1907). peirce, dewey and cross-author links are not built yet. details in [lab/README.md](../lab/README.md).
