"""Regenerate the --small fixtures under src/anamnesis_lab/fixtures/.

The fixtures are original text written for the lab (Apache-2.0), shaped like the real
sources so the same parsing code runs offline:

- ``transcendentalist/thoreau_journal.txt``: a wrapped e-text journal with a licence
  header and footer, chapter years, capitalised headings, ``_Oct. 24._`` dates and
  footnotes;
- ``transcendentalist/emerson_journal_ocr.txt``: OCR-shaped journal text with running
  heads, end-of-line hyphenation, page-foot notes, editor's bracketed notes, OCR
  misreadings ("Fanuary") and undated stretches;
- ``transcendentalist/emerson_essays.txt``: a wrapped e-text essay collection;
- ``pragmatist/james_letters.txt``: wrapped e-text letters with an editor's narrative.

Run from lab/:  uv run python scripts/make_fixtures.py
"""

from __future__ import annotations

import calendar
import random
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "src" / "anamnesis_lab" / "fixtures"

SUBJECTS = [
    "the river", "a heron", "the old pine", "the meadow", "a chickadee", "the pond", "the hill",
    "a farmer", "the wind", "the snow", "the frost", "a muskrat", "the oak", "the swamp",
    "the moon", "the village", "a stranger", "the woodchopper", "the mist", "a hawk",
]
VERBS = [
    "keeps", "remembers", "reflects", "teaches", "holds", "measures", "forgets", "repeats",
    "answers", "outlasts", "carries", "hides", "reveals", "gathers",
]
OBJECTS = [
    "the whole sky", "an older law", "the color of the season", "every footstep",
    "a thought I had not finished", "the work of many winters", "the silence of the woods",
    "the shape of the shore", "what the town has lost", "a private liberty",
]
PLACES = [
    "at the edge of the field", "under the cliff", "by the causeway", "near the bridge",
    "in the hollow", "beyond the railroad", "on the far bank", "along the wall",
]
REFLECTIONS = [
    "A man is rich in proportion to the things he can let alone.",
    "Every fact is a seed if we wait long enough.",
    "The best thought comes on foot and leaves on foot.",
    "Nature does not hurry, yet she is never late with her accounts.",
    "We are always nearer to the truth when we are alone with it.",
    "What we call ruin is often a new arrangement.",
    "The day is a book we read only once.",
    "To see one thing well is to see it many times.",
    "A journal is a kind of second memory that forgives nothing.",
    "Our habits are older than our opinions.",
]
HEADINGS = ["SOLITUDE", "THE RIVER IN WINTER", "FRIENDSHIP", "THE HAWK", "ECONOMY OF THE DAY", "SOUNDS"]
ABBR = ["Jan.", "Feb.", "March", "April", "May", "June", "July", "Aug.", "Sept.", "Oct.", "Nov.", "Dec."]

LICENCE_HEAD = """The Project Gutenberg eBook of {title}

This eBook is for the use of anyone anywhere in the United States and
most other parts of the world at no cost and with almost no restrictions
whatsoever. You may copy it, give it away or re-use it under the terms
of the Project Gutenberg License included with this eBook or online
at www.gutenberg.org.

Title: {title}

Release date: (fixture)

Credits: Produced by the lab fixture generator

*** START OF THE PROJECT GUTENBERG EBOOK {upper} ***



Produced by the lab fixture generator, imitating the volunteers at
the Online Distributed Proofreading Team at http://www.pgdp.net


Transcriber's Note:

  Spelling in this fixture is deliberately consistent.

"""

LICENCE_FOOT = """

*** END OF THE PROJECT GUTENBERG EBOOK {upper} ***

Updated editions will replace the previous one. Project Gutenberg is a
registered trademark, and may not be used if you charge for an eBook.

Section 1. General Terms of Use and Redistributing Project Gutenberg
electronic works. Visit www.gutenberg.org for more.
"""


def sentence(rng: random.Random) -> str:
    s = f"{rng.choice(SUBJECTS)} {rng.choice(VERBS)} {rng.choice(OBJECTS)} {rng.choice(PLACES)}."
    return s[0].upper() + s[1:]


def paragraph(rng: random.Random, n: int) -> str:
    parts = [sentence(rng) for _ in range(n)]
    if rng.random() < 0.5:
        parts.insert(rng.randrange(len(parts) + 1), rng.choice(REFLECTIONS))
    return " ".join(parts)


def wrap(text: str, width: int = 72) -> str:
    return "\n".join(textwrap.wrap(text, width))


def dates(rng: random.Random, years: list[int], per_year: int) -> list[tuple[int, int, int]]:
    out = []
    for y in years:
        days = sorted(rng.sample(range(1, 366), per_year))
        for doy in days:
            m, d = 1, doy
            while d > calendar.monthrange(y, m)[1]:
                d -= calendar.monthrange(y, m)[1]
                m += 1
            out.append((y, m, d))
    return out


def thoreau_journal() -> str:
    rng = random.Random(1837)
    title = "Journal 00, a fixture"
    out = [LICENCE_HEAD.format(title=title, upper=title.upper())]
    out.append("INTRODUCTION\n\n" + wrap("This introduction is the editor's own matter and is not part of the journal.") + "\n\n")
    out.append("THE JOURNAL OF HENRY DAVID THOREAU\n\n\n")
    years = [1840, 1841, 1842, 1845, 1850, 1851, 1852, 1853, 1854, 1855]
    all_dates = dates(rng, years, 18)
    chapter = 0
    cur_year = None
    footnote = 0
    for i, (y, m, d) in enumerate(all_dates):
        if y != cur_year:
            chapter += 1
            roman = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"][chapter - 1]
            out.append(f"\n\n{roman}\n\n{y}\n\n(ÆT. {y - 1817}-{y - 1816})\n\n\n")
            cur_year = y
        if i % 9 == 0:
            out.append(rng.choice(HEADINGS) + "\n\n")
        body = paragraph(rng, rng.randint(1, 3))
        if i % 17 == 5 and y >= 1845:
            body += " Talked with Emerson in the evening about the lecture."
        if i % 29 == 7 and y >= 1850:
            body += " Mr. Emerson lent me a book on Persian poets."
        if i % 41 == 11:
            body += " Reread Self-Reliance by the stove."
        if i % 23 == 3:
            body += " G. B. Emerson's Report gives the oak a different name."
        if i % 13 == 0:
            footnote += 1
            body += f"[{footnote}]"
        out.append(wrap(f"_{ABBR[m - 1]} {d}._ {body}") + "\n\n")
        if i % 7 == 2:
            out.append(wrap(paragraph(rng, 2)) + "\n\n")
        if i % 31 == 4:
            verse = ["The pond is a mirror the hills look into,", "And the hills grow quieter for it."]
            out.append("     " + "\n     ".join(verse) + "—_Ibid._\n\n")
    out.append("\n\n")
    for k in range(1, footnote + 1):
        out.append(f"     [{k}] [A note by the editor on the manuscript, number {k}.]\n\n")
    out.append(LICENCE_FOOT.format(upper=title.upper()))
    return "".join(out)


def ocr_mangle(line: str, rng: random.Random) -> str:
    # OCR misreads a capital J as F, and drops a few spaces.
    line = line.replace("January", "Fanuary" if rng.random() < 0.5 else "January")
    line = line.replace("June", "Fune" if rng.random() < 0.3 else "June")
    if rng.random() < 0.1:
        line = line.replace(" the ", " the", 1)
    return line


def emerson_ocr() -> str:
    rng = random.Random(1803)
    lines: list[str] = []
    lines += ["UNIVERSITY LIBRARY", "", "CONTENTS", "", "JOURNAL I", "", "Nature and books. The scholar.", "", ""]
    lines += ["ILLUSTRATIONS", "", "Tue Op Manse . . . . 360", "", "", "JOURNAL", "", "JOURNAL I", "", "1836", "", "(From Journal B)", ""]
    page_lines: list[str] = []
    page_no = 1
    year_at_page = 1836
    footnote_pending = False

    def flush_page() -> None:
        nonlocal page_lines, page_no, footnote_pending
        if footnote_pending:
            page_lines += ["", "1 The editor's note at the foot of the page, which the builder drops."]
            footnote_pending = False
        lines.extend(page_lines)
        lines.append("")
        page_no += 1
        if page_no % 2 == 0:
            lines.append(f"{page_no} JOURNAL [AcE {year_at_page - 1803}")
        else:
            lines.append(f"{year_at_page}] THE SCHOLAR AND NATURE {page_no}")
        lines.append("")
        page_lines = []

    def emit_paragraph(text: str) -> None:
        nonlocal page_lines
        wrapped = textwrap.wrap(text, 48)
        # end-of-line hyphenation on long words
        out = []
        for w in wrapped:
            out.append(w)
        for ln in out:
            page_lines.append(ocr_mangle(ln, rng) + " ")
            if len(page_lines) >= 34:
                flush_page()
        page_lines.append("")

    years_dated = [1836, 1837, 1838, 1840, 1841, 1842, 1845, 1847]
    for (y, m, d) in dates(rng, years_dated, 12):
        if y != year_at_page:
            emit_paragraph(str(y))  # the printed edition sets each new year as a heading
        year_at_page = y
        name = calendar.month_name[m]
        prefix = rng.choice(["", "", "Concord, ", "Saturday evening, ", ""])
        emit_paragraph(f"{prefix}{name} {d}.")
        body = paragraph(rng, rng.randint(2, 5))
        if y >= 1841 and rng.random() < 0.35:
            body += " Henry Thoreau walked with me to the cliff and talked of the Indians."
        if rng.random() < 0.15:
            body += " I talked with my brother of an extraor- dinary book."
        emit_paragraph(body)
        if rng.random() < 0.2:
            emit_paragraph("[Mr. Emerson here copied a passage into his lecture; the editors' note.]")
        if rng.random() < 0.25:
            footnote_pending = True
    # later years: undated stretches, only the running heads carry the year
    for y in [1850, 1851, 1853, 1855]:
        flush_page()
        year_at_page = y
        flush_page()
        for _ in range(9):
            body = paragraph(rng, rng.randint(3, 5))
            if rng.random() < 0.3:
                body += " Thoreau's Walden is printed at last."
            emit_paragraph(body)
    flush_page()
    lines += ["", "END OF VOLUME I", "", "INDEX", "", "Abbott, 12"]
    return "\n".join(lines) + "\n"


def emerson_essays() -> str:
    rng = random.Random(1841)
    title = "Essays, a fixture"
    out = [LICENCE_HEAD.format(title=title, upper=title.upper())]
    out.append("ESSAYS, FIXTURE SERIES\n\nContents\n\n  I. HISTORY\n  II. SELF-RELIANCE\n  III. CIRCLES\n\n\n")
    for roman, name in [("I.", "HISTORY"), ("II.", "SELF-RELIANCE"), ("III.", "CIRCLES"), ("IV.", "THE OVER-SOUL")]:
        out.append(f"{roman}\n{name}\n\n\n")
        out.append("     A short epigraph in verse stands here,\n     and the title repeats below it.\n\n\n")
        out.append(f"{name}\n\n\n")
        for _ in range(rng.randint(8, 14)):
            out.append(wrap(paragraph(rng, rng.randint(4, 7))) + "\n\n")
    out.append(LICENCE_FOOT.format(upper=title.upper()))
    return "".join(out)


def james_letters() -> str:
    rng = random.Random(1842)
    title = "Letters, a fixture"
    out = [LICENCE_HEAD.format(title=title, upper=title.upper())]
    out.append(wrap("The editor's introduction describes the family and is skipped by the builder.") + "\n\n")
    people = ["his Father", "his Brother", "Thomas W. Ward", "his Sister", "Henry James"]
    for i, (y, m, d) in enumerate(dates(rng, [1865, 1867, 1868, 1872, 1880], 6)):
        out.append(f"_To {people[i % len(people)]}._\n\n\nCAMBRIDGE,\n_{ABBR[m - 1]} {d}, {y}_.\n\n")
        out.append(wrap("DEAR FRIEND,--" + paragraph(rng, 4)) + "\n\n")
        out.append(wrap(paragraph(rng, 3)) + "\n\n")
        if i % 4 == 3:
            out.append("       *       *       *       *       *\n\n")
            out.append(wrap("The editor explains what happened next; this narrative is skipped.") + "\n\n")
    out.append(LICENCE_FOOT.format(upper=title.upper()))
    return "".join(out)


def main() -> None:
    (ROOT / "transcendentalist").mkdir(parents=True, exist_ok=True)
    (ROOT / "pragmatist").mkdir(parents=True, exist_ok=True)
    (ROOT / "transcendentalist" / "thoreau_journal.txt").write_text(thoreau_journal(), encoding="utf-8")
    (ROOT / "transcendentalist" / "emerson_journal_ocr.txt").write_text(emerson_ocr(), encoding="utf-8")
    (ROOT / "transcendentalist" / "emerson_essays.txt").write_text(emerson_essays(), encoding="utf-8")
    (ROOT / "pragmatist" / "james_letters.txt").write_text(james_letters(), encoding="utf-8")


if __name__ == "__main__":
    main()
