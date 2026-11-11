"""The transcendentalist brain: Thoreau's and Emerson's journals, plus essays.

Copyright reasoning (see ``sources.py`` for the rules):

- Henry David Thoreau †1862, Ralph Waldo Emerson †1882.
- Thoreau, *Journal* (1906, 14 vols., Houghton Mifflin), edited by Bradford Torrey
  (1843–1912) with Francis H. Allen (1866–1953). Brazil: PD since 2024. US: 1906.
- Emerson, *Journals* (1909–14, 10 vols.), edited by Edward Waldo Emerson (1844–1930)
  and Waldo Emerson Forbes (1879–1917). Brazil: PD since 1988 at the latest. US: 1909–14.
  Forbes's dates come from genealogy records; even if they were wrong by decades the
  builder also drops the editors' bracketed notes and page-foot annotations, leaving
  Emerson's own text, which is PD everywhere.
- Essays: Emerson's and Thoreau's own books, no editor.
- Not used: the Harvard *Journals and Miscellaneous Notebooks* (1960–82), in copyright.
"""

from __future__ import annotations

from ..links import LinkRule
from ..sources import Person, Source
from .base import Hub, Theme

THOREAU = "Henry David Thoreau"
EMERSON = "Ralph Waldo Emerson"

P_THOREAU = Person(THOREAU, "author", 1817, 1862, "Library of Congress authority record")
P_EMERSON = Person(EMERSON, "author", 1803, 1882, "Library of Congress authority record")
P_TORREY = Person("Bradford Torrey", "editor", 1843, 1912, "the distributor's catalogue authority data (1843-1912)")
P_ALLEN = Person(
    "Francis H. Allen", "editor", 1866, 1953,
    "Online Books Page / LC name authority 'Allen, Francis H. (Francis Henry), 1866-1953'",
)
P_EWE = Person("Edward Waldo Emerson", "editor", 1844, 1930, "LC name authority 'Emerson, Edward Waldo, 1844-1930'")
P_FORBES = Person(
    "Waldo Emerson Forbes", "editor", 1879, 1917,
    "genealogy records (b. 1879 Milton MA, d. 1917); no library authority found - verify",
)

PG = "Project Gutenberg"
IA = "Internet Archive"

_THOREAU_PG_PARSE = dict(
    start=r"^THE JOURNAL OF HENRY DAVID THOREAU",
    stop=[r"^\s+\[1\]\s", r"^\s*INDEX\s*$"],
    year_min=1837, year_max=1862, use_headings=True,
)

EMERSON_VOLUMES = [
    # (volume, roman of the first journal in it, start occurrence, years, published)
    (1, "I", 3, (1820, 1824), 1909),
    (2, "XV", 2, (1824, 1832), 1909),
    (3, "XXIV", 2, (1833, 1835), 1910),
    (4, "XXVII", 2, (1836, 1838), 1910),
    (5, "XXIX", 2, (1838, 1841), 1911),
    (6, "XXXII", 2, (1841, 1844), 1911),
    (7, "AXXVI", 1, (1845, 1848), 1912),  # OCR reads "XXXVI" as "AXXVI" at the start of the text
    (8, "XL", 2, (1849, 1855), 1912),
    (9, "XLVII", 2, (1856, 1863), 1913),
    (10, "LV", 2, (1864, 1876), 1914),
]


def _emerson_journal(vol: int, roman: str, nth: int, years: tuple[int, int], published: int) -> Source:
    ident = f"journalsofralphw{vol:02d}emer"
    return Source(
        id=f"emerson-journals-{vol:02d}",
        title=f"Journals of Ralph Waldo Emerson, vol. {vol} ({years[0]}–{years[1]})",
        author=EMERSON,
        persons=[P_EMERSON, P_EWE, P_FORBES],
        published=published,
        url=f"https://archive.org/download/{ident}/{ident}_djvu.txt",
        page_url=f"https://archive.org/details/{ident}",
        fmt="ocr",
        kind="journal",
        edition="Journals of Ralph Waldo Emerson, with annotations, ed. E. W. Emerson and W. E. Forbes (Houghton Mifflin, 1909–14); Boston Public Library scan",
        host=IA,
        parse=dict(
            start=rf"^JOURNAL {roman}_?\s*$", start_nth=nth,
            stop=[r"^END OF VOLUME", r"^\s*INDEX\s*$"],
            year_min=years[0], year_max=years[1] + 1,
            undated_year_breaks=True, max_chars=2000,
        ),
        notes="OCR text; editors' bracketed notes and page-foot annotations are dropped by the builder.",
    )


def _pg(
    num: int, sid: str, title: str, author_p: Person, kind: str, published: int, parse: dict,
    persons: list[Person] | None = None, notes: str = "", cite: str = "",
) -> Source:
    return Source(
        id=sid,
        title=title,
        author=author_p.name,
        persons=persons or [author_p],
        published=published,
        url=f"https://www.gutenberg.org/cache/epub/{num}/pg{num}.txt",
        page_url=f"https://www.gutenberg.org/ebooks/{num}",
        fmt="pg",
        kind=kind,
        edition=title,
        host=PG,
        parse=parse,
        notes=notes,
        cite_url=f"https://archive.org/details/{cite}" if cite else "",
    )


SOURCES: list[Source] = [
    _pg(
        57393, "thoreau-journal-01", "The Writings of Henry David Thoreau, vol. 7: Journal I, 1837–1846 (1906)",
        P_THOREAU, "journal", 1906, _THOREAU_PG_PARSE, persons=[P_THOREAU, P_TORREY, P_ALLEN],
        notes="editor's introduction, footnotes and bracketed notes are dropped",
        cite="thoreau-journal-1906lim-ed",
    ),
    _pg(
        59031, "thoreau-journal-02", "The Writings of Henry David Thoreau, vol. 8: Journal II, 1850–September 15, 1851 (1906)",
        P_THOREAU, "journal", 1906, _THOREAU_PG_PARSE, persons=[P_THOREAU, P_TORREY, P_ALLEN],
        notes="footnotes and bracketed notes are dropped",
        cite="thoreau-journal-1906lim-ed",
    ),
    Source(
        id="thoreau-journal-1906-ocr",
        title="The Journal of Henry D. Thoreau, vols. III–XIV (Sept. 16, 1851–1861) (1906)",
        author=THOREAU,
        persons=[P_THOREAU, P_TORREY, P_ALLEN],
        published=1906,
        url="https://archive.org/download/thoreau-journal-1906lim-ed/ThoreauJournal1906lim-ed_djvu.txt",
        page_url="https://archive.org/details/thoreau-journal-1906lim-ed",
        fmt="ocr",
        kind="journal",
        edition="The Writings of Henry David Thoreau, Manuscript Edition, Journal (Houghton Mifflin, 1906), ed. Bradford Torrey [and Francis H. Allen]",
        host=IA,
        parse=dict(
            start=r"^SEPTEMBER AND OCTOBER, 1851",
            stop=[r"^\s*INDEX\s*$", r"^GENERAL INDEX"],
            drop=[(r"^END OF VOLUME", r"^THE JOURNAL OF\s*$")],
            year_min=1851, year_max=1862,
            date_after="1851-09-15",  # the two earlier volumes come from the cleaner e-text
            max_chars=3500,
        ),
        notes="OCR text of the full 1906 set; only entries after 1851-09-15 are used.",
    ),
] + [_emerson_journal(*v) for v in EMERSON_VOLUMES] + [
    _pg(29433, "emerson-nature", "Nature (1836; 1849 edition)", P_EMERSON, "book", 1849,
        dict(start=r"^INTRODUCTION\.$", work="Nature", year=1836, sections="chapters"), cite="naturemunroe00emerrich"),
    _pg(2944, "emerson-essays-1", "Essays, First Series (1841)", P_EMERSON, "essays", 1841,
        dict(start=r"^I\.\s*$", work="Essays, First Series", year=1841), cite="emersonralessays00emerrich"),
    _pg(2945, "emerson-essays-2", "Essays, Second Series (1844)", P_EMERSON, "essays", 1844,
        dict(start=r"^I\. THE POET", work="Essays, Second Series", year=1844), cite="essayssecondser01emergoog"),
    _pg(205, "thoreau-walden", "Walden (1854)", P_THOREAU, "book", 1854,
        dict(
            start=r"^Economy\s*$", stop=[r"^THE END"], work="Walden", year=1854, sections="chapters",
            heading=r"^(Economy|Where I Lived, and What I Lived For|Reading|Sounds|Solitude|Visitors|The Bean-Field|"
            r"The Village|The Ponds|Baker Farm|Higher Laws|Brute Neighbors|House-Warming|"
            r"Former Inhabitants and Winter Visitors|Winter Animals|The Pond in Winter|Spring|Conclusion)$",
        ), cite="waldenorlifei00thor"),
    _pg(205, "thoreau-civil-disobedience", "On the Duty of Civil Disobedience (1849)", P_THOREAU, "essays", 1849,
        dict(start=r"^ON THE DUTY OF CIVIL DISOBEDIENCE\s*$", start_nth=2, work="On the Duty of Civil Disobedience",
             year=1849, heading=r"^ON THE DUTY OF CIVIL DISOBEDIENCE$"), cite="aestheticpapers00peabrich"),
    _pg(4232, "thoreau-week", "A Week on the Concord and Merrimack Rivers (1849)", P_THOREAU, "book", 1849,
        dict(start=r"^CONCORD RIVER\s*$", stop=[r"^THE END\.?\s*$"], work="A Week on the Concord and Merrimack Rivers",
             year=1849, sections="chapters",
             heading=r"^(CONCORD RIVER|SATURDAY|SUNDAY|MONDAY|TUESDAY|WEDNESDAY|THURSDAY|FRIDAY)$"), cite="weekonconcordmer1849thor"),
    _pg(1022, "thoreau-walking", "Walking (1862)", P_THOREAU, "essays", 1862,
        dict(start=r"^WALKING\s*$", work="Walking", year=1862, heading=r"^WALKING$"), cite="excursions1863thor"),
]

# Explicit references only. Thoreau often cites George B. Emerson's report on the trees
# of Massachusetts; those mentions are excluded.
LINK_RULES: list[LinkRule] = [
    LinkRule(
        r"(?<!G\. B\. )(?<!George B\. )(?<!B\. )(?<!G\. )\b(?:Mr\. )?(?:R\. ?W\. )?Emerson\b(?![’']s (?:Report|Trees|report))(?! on (?:the )?[Tt]rees)",
        "Ralph Waldo Emerson", frozenset({THOREAU}),
    ),
    LinkRule(r"\bR\. ?W\. ?E\.", "Ralph Waldo Emerson", frozenset({THOREAU})),
    LinkRule(r"\bSelf-[Rr]eliance\b", "Self-Reliance", frozenset({THOREAU})),
    LinkRule(r"\b(?:[Tt]he )?Over-[Ss]oul\b", "The Over-Soul", frozenset({THOREAU})),
    LinkRule(r"\bSpiritual Laws\b", "Spiritual Laws", frozenset({THOREAU})),
    LinkRule(r"\bNew England Reformers\b", "New England Reformers", frozenset({THOREAU})),
    LinkRule(r"\b(?:Henry (?:D\. )?Thoreau|Thoreau|H\. ?D\. ?T\.)", "Henry David Thoreau", frozenset({EMERSON})),
    LinkRule(r"\b(?:Henry[’']s|Thoreau[’']s|his) (?:book,? )?[\"“]?Walden\b", "Walden", frozenset({EMERSON})),
    LinkRule(r"\bWeek on the Concord\b|\b(?:Henry[’']s|Thoreau[’']s) [\"“]?Week\b", "A Week on the Concord and Merrimack Rivers", frozenset({EMERSON})),
    LinkRule(r"\bCivil Disobedience\b|\bResistance to Civil Government\b", "On the Duty of Civil Disobedience", frozenset({EMERSON})),
]

HUBS = [
    Hub(
        "Henry David Thoreau", THOREAU,
        "henry david thoreau (1817–1862), surveyor, naturalist and writer of concord, massachusetts. "
        "he kept a journal from october 1837 until november 1861.\n\n"
        "in this brain: his journal (1906 edition), [[Walden]], [[A Week on the Concord and Merrimack Rivers]], "
        "[[Walking]] and [[On the Duty of Civil Disobedience]].\n",
        tags=["person", "thoreau"],
    ),
    Hub(
        "Ralph Waldo Emerson", EMERSON,
        "ralph waldo emerson (1803–1882), essayist, lecturer and poet of concord, massachusetts. "
        "he kept journals from 1820 into the 1870s.\n\n"
        "in this brain: his journals (1909–14 edition), [[Nature]], [[Essays, First Series]] and [[Essays, Second Series]].\n",
        tags=["person", "emerson"],
    ),
]

AUTHOR_TAGS = {THOREAU: "thoreau", EMERSON: "emerson"}


def _fixture(name: str, sid: str, title: str, author_p: Person, kind: str, published: int, parse: dict) -> Source:
    return Source(
        id=sid, title=title, author=author_p.name, persons=[author_p], published=published,
        url=name, page_url="https://github.com/aphesimos/anamnesis/tree/main/lab", fmt="fixture",
        kind=kind, edition=f"{title} (lab fixture)", host="bundled fixture", parse=parse,
    )


# The --small fixtures are original text written for the lab in the shape of the real
# sources (a wrapped e-text journal, an OCR journal with running heads and footnotes,
# and an essay collection). They exercise the same code paths offline.
SMALL_SOURCES: list[Source] = [
    _fixture("thoreau_journal.txt", "fixture-thoreau-journal", "Fixture journal in the Thoreau shape", P_THOREAU,
             "journal", 1906, dict(_THOREAU_PG_PARSE, wrapper="pg")),
    _fixture("emerson_journal_ocr.txt", "fixture-emerson-journal", "Fixture journal in the Emerson OCR shape", P_EMERSON,
             "journal", 1909, dict(wrapper="ocr", start=r"^JOURNAL I\s*$", start_nth=2, stop=[r"^END OF VOLUME"],
                                   year_min=1836, year_max=1856, undated_year_breaks=True)),
    _fixture("emerson_essays.txt", "fixture-emerson-essays", "Fixture essays in the Emerson shape", P_EMERSON,
             "essays", 1841, dict(wrapper="pg", start=r"^I\.\s*$", work="Essays, First Series", year=1841)),
]

THEME = Theme(
    id="transcendentalist",
    title="the transcendentalist brain",
    owner_label="the transcendentalists",
    sources=SOURCES,
    small_sources=SMALL_SOURCES,
    link_rules=LINK_RULES,
    hubs=HUBS,
    author_tags=AUTHOR_TAGS,
)
