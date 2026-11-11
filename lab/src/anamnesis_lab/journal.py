"""Split journals and letters into dated entries, and books into sections."""

from __future__ import annotations

import datetime as dt
import re
from dataclasses import dataclass, field

from .dates import chapter_year, parse_date_head, safe_date, year_only
from .textclean import Para, Token, WeakYear


@dataclass
class Entry:
    """One future note: a dated journal entry, a letter, or a section of a book."""

    author: str
    date: dt.date
    precision: str  # "day" | "month" | "year"
    kind: str  # "journal" | "letter" | "essay" | "book"
    source_id: str
    title: str | None = None
    paragraphs: list[str] = field(default_factory=list)
    origin: str = "owner_written"
    tags: list[str] = field(default_factory=list)
    part: int = 0  # >0 when a long entry was split

    @property
    def body(self) -> str:
        return "\n\n".join(self.paragraphs)


_ROMAN = re.compile(r"^[IVXLC]{1,7}\.?$")
_AGE = re.compile(r"^\((?:Æ|AE|2|A|Z)T\.?\s", re.I)


def is_heading(text: str) -> bool:
    """A short line in capitals, e.g. ``THE MOULD OUR DEEDS LEAVE``."""
    s = text.strip().rstrip(".")
    if "\n" in s or len(s) < 3 or len(s) > 60:
        return False
    words = s.split()
    if len(words) > 8:
        return False
    letters = [c for c in s if c.isalpha()]
    if len(letters) < 3:
        return False
    return all(c.isupper() for c in letters) and not _ROMAN.match(s)


def heading_title(text: str) -> str:
    small = {"a", "an", "the", "of", "and", "in", "on", "at", "to", "for", "by", "or", "with"}
    text = re.sub(r"^(?:CHAPTER\s+)?[IVXLC]+\.\s+", "", text.strip())
    words = text.strip().rstrip(".").lower().split()

    def cap(w: str) -> str:
        return "-".join(p[:1].upper() + p[1:] for p in w.split("-"))

    return " ".join(w if (i and w in small) else cap(w) for i, w in enumerate(words))


@dataclass
class JournalOptions:
    author: str
    source_id: str
    year_min: int
    year_max: int
    kind: str = "journal"
    use_headings: bool = False
    # Emerson's later journals run for pages without a date: start a new entry when the
    # running-head year moves past the current entry's year.
    undated_year_breaks: bool = False
    skip_heading_words: tuple[str, ...] = ("JOURNAL", "CONTENTS", "ILLUSTRATIONS", "INDEX")
    # Letters: a paragraph matching this starts a new entry titled by it ("_To his Father._").
    # Text before the first match, and after a "* * *" separator when
    # ``separator_ends_entry`` is set, is the editor's narrative and is skipped.
    title_regex: str | None = None
    separator_ends_entry: bool = False


def split_journal(tokens: list[Token], opt: JournalOptions) -> list[Entry]:
    """Walk paragraphs and year signals and cut them into dated entries.

    Year resolution: an explicit year in the date heading wins; then a chapter heading
    or a bare year line; otherwise the year carries over from the previous entry and
    rolls forward when the month jumps back by more than three (December → January).
    Running-head years only fill gaps of more than a year, or open a new undated
    entry when ``undated_year_breaks`` is set.
    """
    entries: list[Entry] = []
    cur: Entry | None = None
    strong_year: int | None = None
    weak_year: int | None = None
    last: dt.date | None = None
    pending_title: str | None = None
    title_rx = re.compile(opt.title_regex) if opt.title_regex else None
    skipping = title_rx is not None

    def in_range(y: int) -> bool:
        return opt.year_min <= y <= opt.year_max

    def close() -> None:
        nonlocal cur
        if cur is not None and cur.paragraphs:
            entries.append(cur)
        cur = None

    def resolve_year(month: int, explicit: int | None) -> int | None:
        nonlocal strong_year, weak_year
        if explicit and in_range(explicit):
            return explicit
        if strong_year is not None:
            y = strong_year
            if last is not None and last.year == y and month < last.month - 3:
                y += 1
            return y
        if last is None:
            return weak_year
        y = last.year
        if month < last.month - 3:
            y += 1
        if weak_year is not None and weak_year > y + 1:
            y = weak_year
        return y

    for tok in tokens:
        if isinstance(tok, WeakYear):
            if not in_range(tok.year):
                continue
            weak_year = tok.year
            if (
                opt.undated_year_breaks
                and cur is not None
                and cur.paragraphs
                and tok.year > cur.date.year
            ):
                close()
                d = dt.date(tok.year, 1, 1)
                cur = Entry(opt.author, d, "year", opt.kind, opt.source_id)
                last = d
            continue
        assert isinstance(tok, Para)
        text = tok.text.strip()
        if text == "* * *" and opt.separator_ends_entry:
            close()
            skipping = True
            continue
        if not text or text == "* * *":
            continue
        if title_rx is not None and title_rx.match(text):
            close()
            skipping = False
            title = text.strip("_. ")
            d = last or (dt.date(strong_year, 1, 1) if strong_year else None)
            if d is not None:
                cur = Entry(opt.author, d, "day" if last else "year", opt.kind, opt.source_id, title=title)
            else:
                pending_title = title
            continue
        if skipping:
            continue
        y = year_only(text)
        if y is None:
            y = chapter_year(text) if len(text) < 90 else None
        if y is not None:
            if in_range(y):
                strong_year = y
                weak_year = None
            continue
        if _ROMAN.match(text) or _AGE.match(text):
            continue
        if is_heading(text):
            if any(w in text.upper() for w in opt.skip_heading_words):
                continue
            if opt.use_headings:
                close()
                pending_title = heading_title(text)
                d = last or (dt.date(strong_year, 1, 1) if strong_year else None)
                if d is not None:
                    cur = Entry(opt.author, d, "day" if last else "year", opt.kind, opt.source_id, title=pending_title)
            continue
        head = parse_date_head(text) if len(text) > 3 else None
        if head is not None:
            year = resolve_year(head.month, head.year)
            if year is not None:
                d = safe_date(year, head.month, head.day)
                if cur is not None and not cur.paragraphs and cur.title:
                    cur.date, cur.precision = d, "day"
                else:
                    close()
                    cur = Entry(opt.author, d, "day", opt.kind, opt.source_id, title=pending_title)
                last = d
                strong_year = None
                weak_year = None
                pending_title = None
                if head.rest:
                    cur.paragraphs.append(head.rest)
                continue
        pending_title = None
        if cur is None:
            year = strong_year or weak_year or (last.year if last else None)
            if year is None:
                continue  # text before the first date we can place: front matter
            d = last or dt.date(year, 1, 1)
            cur = Entry(opt.author, d, "day" if last else "year", opt.kind, opt.source_id)
            last = d
        cur.paragraphs.append(text)
    close()
    return entries


def split_long(entries: list[Entry], max_chars: int = 4000) -> list[Entry]:
    """Split entries longer than ``max_chars`` at paragraph boundaries into numbered parts."""
    out: list[Entry] = []
    for e in entries:
        if len(e.body) <= max_chars:
            out.append(e)
            continue
        chunks: list[list[str]] = [[]]
        size = 0
        for p in e.paragraphs:
            if size and size + len(p) > max_chars:
                chunks.append([])
                size = 0
            chunks[-1].append(p)
            size += len(p) + 2
        for i, ch in enumerate(chunks, start=1):
            out.append(
                Entry(
                    e.author, e.date, e.precision, e.kind, e.source_id,
                    title=e.title, paragraphs=ch, origin=e.origin, tags=list(e.tags), part=i,
                )
            )
    return out


_QUOTE_OPEN = ('"', "“", "> ", "‘")
_ATTRIBUTION = re.compile(r"—\s*_?(?:[A-Z][A-Za-z.']+|Ibid)[^\n]{0,60}$")


def looks_like_quotation(e: Entry) -> bool:
    """Commonplace-book extracts: the entry opens with a quote or verse block and names a source."""
    body = e.body.strip()
    if not body.startswith(_QUOTE_OPEN):
        return False
    paras = [p for p in e.paragraphs if p.strip()]
    quoted = sum(1 for p in paras if p.lstrip().startswith(_QUOTE_OPEN))
    return quoted / len(paras) >= 0.6 and bool(_ATTRIBUTION.search(body) or "_Ibid._" in body)


@dataclass
class SectionOptions:
    author: str
    source_id: str
    work: str
    date: dt.date
    kind: str = "essay"
    origin: str = "external"
    # Regex for a section heading line (an essay title, a chapter title, a lecture).
    heading: str = r"^(?!CHAPTER\b)(?:[IVXLC]+\.\s+)?[A-Z][A-Z' ,.\-—:;]+$"
    min_chars: int = 400
    # The paragraph after a heading is the real title ("Lecture I" / "What Pragmatism Means").
    title_next: bool = False


def split_sections(tokens: list[Token], opt: SectionOptions) -> list[Entry]:
    """Cut a book into sections at heading paragraphs; short fragments merge forward."""
    rx = re.compile(opt.heading)
    entries: list[Entry] = []
    cur: Entry | None = None
    for tok in tokens:
        if not isinstance(tok, Para):
            continue
        text = tok.text.strip()
        if not text or text == "* * *":
            continue
        if rx.match(text) and len(text) < 80 and not _ROMAN.match(text):
            if cur is not None and heading_title(text) == cur.title:
                continue  # the title repeated after an epigraph
            if cur is not None and len(cur.body) >= opt.min_chars:
                entries.append(cur)
                cur = None
            title = heading_title(text)
            if cur is None:
                cur = Entry(opt.author, opt.date, "year", opt.kind, opt.source_id, title=title, origin=opt.origin)
            elif not cur.paragraphs:
                cur.title = title
            continue
        if cur is None:
            continue
        if opt.title_next and not cur.paragraphs and len(text) < 80 and not text.endswith("."):
            cur.title = f"{cur.title}: {text}"
            continue
        cur.paragraphs.append(text)
    if cur is not None and cur.paragraphs:
        entries.append(cur)
    return entries
