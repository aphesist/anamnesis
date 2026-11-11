"""The pragmatist brain (partial): William James's dated letters plus *Pragmatism*.

Status: partial. It builds the two volumes of *The Letters of William James* (1920)
as dated letters and *Pragmatism* (1907) as lectures. Not yet included: Peirce's
1877–78 papers, Dewey's pre-1931 books, *The Will to Believe*, cross-author links
(only James's mentions of his own books are linked), and a --small fixture of its own
(``--small`` reuses a tiny letters fixture).

Copyright reasoning: William James †1910. The letters were edited by his son
Henry James (1879–1947): Brazil-PD since 2018; published 1920, so US-PD. The
builder also skips the editor's connecting narrative between letters.
"""

from __future__ import annotations

from ..links import LinkRule
from ..sources import Person, Source
from .base import Hub, Theme

JAMES = "William James"
P_JAMES = Person(JAMES, "author", 1842, 1910, "Library of Congress authority record")
P_HJ3 = Person("Henry James", "editor", 1879, 1947, "LC name authority 'James, Henry, 1879-1947' (the son)")

_LETTERS_PARSE = dict(
    stop=[r"^FOOTNOTES:?\s*$", r"^\s*INDEX\s*$"],
    year_min=1842, year_max=1910,
    title_regex=r"^_To [^\n]{2,80}_\.?_?$|^_To [^\n]{2,80}\._$",
    separator_ends_entry=True,
)


def _letters(num: int, vol: int) -> Source:
    return Source(
        id=f"james-letters-{vol}",
        title=f"The Letters of William James, vol. {vol} (1920)",
        author=JAMES,
        persons=[P_JAMES, P_HJ3],
        published=1920,
        url=f"https://www.gutenberg.org/cache/epub/{num}/pg{num}.txt",
        page_url=f"https://www.gutenberg.org/ebooks/{num}",
        fmt="pg",
        kind="letters",
        edition=f"The Letters of William James, ed. by his son Henry James, vol. {vol} (Atlantic Monthly Press, 1920)",
        host="Project Gutenberg",
        parse=dict(_LETTERS_PARSE),
        notes="the editor's narrative between letters and the footnotes are dropped",
        cite_url="https://archive.org/details/" + ("lettersofjames01jameuoft" if vol == 1 else "thelettersofwill02jameuoft"),
    )


SOURCES = [
    _letters(40307, 1),
    _letters(38091, 2),
    Source(
        id="james-pragmatism",
        title="Pragmatism: A New Name for Some Old Ways of Thinking (1907)",
        author=JAMES, persons=[P_JAMES], published=1907,
        url="https://www.gutenberg.org/cache/epub/5116/pg5116.txt",
        page_url="https://www.gutenberg.org/ebooks/5116",
        fmt="pg", kind="book", edition="Pragmatism (Longmans, Green, 1907)", host="Project Gutenberg",
        parse=dict(start=r"^Lecture I\s*$", start_nth=2, work="Pragmatism", year=1907, sections="chapters",
                   heading=r"^Lecture [IVX]+$", title_next=True),
        cite_url="https://archive.org/details/pragmatismnewnam00jameuoft",
    ),
]

LINK_RULES = [
    LinkRule(r"\b(?:my|the) (?:little )?(?:book on )?[\"“]?Pragmatism\b", "Pragmatism", frozenset({JAMES})),
]

HUBS = [
    Hub(
        "William James", JAMES,
        "william james (1842–1910), psychologist and philosopher of cambridge, massachusetts.\n\n"
        "in this brain: his letters (1920 edition) and [[Pragmatism]].\n",
        tags=["person", "james"],
    ),
]

SMALL_SOURCES = [
    Source(
        id="fixture-james-letters", title="Fixture letters in the James shape", author=JAMES, persons=[P_JAMES],
        published=1920, url="james_letters.txt", page_url="https://github.com/aphesimos/anamnesis/tree/main/lab",
        fmt="fixture", kind="letters", edition="lab fixture", host="bundled fixture",
        parse=dict(_LETTERS_PARSE, wrapper="pg"),
    ),
]

THEME = Theme(
    id="pragmatist",
    title="the pragmatist brain",
    owner_label="the pragmatists",
    sources=SOURCES,
    small_sources=SMALL_SOURCES,
    link_rules=LINK_RULES,
    hubs=HUBS,
    author_tags={JAMES: "james"},
    partial=True,
    partial_note="letters of william james and pragmatism only; peirce, dewey and cross-author links are not built yet",
)
