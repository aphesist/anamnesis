"""Turn raw source text (clean e-text or OCR) into a stream of paragraphs and year signals."""

from __future__ import annotations

import re
from dataclasses import dataclass

from .dates import parse_date_head, running_head, year_only


@dataclass(frozen=True)
class Para:
    text: str


@dataclass(frozen=True)
class WeakYear:
    """A year read from a printed running head (reliable-ish, but may lead the text)."""

    year: int


Token = Para | WeakYear

_FOOTNOTE_MARK = re.compile(r"\[(?:\d{1,3}|[a-z])\]")
_ILLUSTRATION = re.compile(r"\[Illustration[^\]]*\]", re.S | re.I)
_SEPARATOR = re.compile(r"^\s*(?:\*\s+){2,}\*\s*$")
_FOOTNOTE_START_OCR = re.compile(r"^\s*[1-9*†]\s+[A-Z(\[“\"'‘]")
_TERMINAL = tuple(".!?:;\"”’')]_—")


def slice_text(
    text: str,
    start: str | None = None,
    stop: list[str] | None = None,
    start_nth: int = 1,
    drop: list[tuple[str, str]] | None = None,
) -> str:
    """Keep text from the ``start_nth`` line matching ``start`` up to the first line matching any ``stop``.

    ``drop`` removes spans between a line matching the first regex and the next line
    matching the second (inclusive), e.g. the title pages between bound volumes.
    """
    lines = text.split("\n")
    i0 = 0
    if start:
        rx = re.compile(start)
        hits = [i for i, ln in enumerate(lines) if rx.search(ln)]
        if len(hits) < start_nth:
            raise ValueError(f"start marker {start!r} (occurrence {start_nth}) not found")
        i0 = hits[start_nth - 1]
    i1 = len(lines)
    if stop:
        rxs = [re.compile(s) for s in stop]
        for i in range(i0 + 1, len(lines)):
            if any(r.search(lines[i]) for r in rxs):
                i1 = i
                break
    kept = lines[i0:i1]
    for a, b in drop or []:
        ra, rb = re.compile(a), re.compile(b)
        out: list[str] = []
        dropping = False
        for ln in kept:
            if not dropping and ra.search(ln):
                dropping = True
                continue
            if dropping:
                if rb.search(ln):
                    dropping = False
                continue
            out.append(ln)
        kept = out
    return "\n".join(kept)


def _join_lines(lines: list[str]) -> str:
    """Join wrapped lines, undoing end-of-line hyphenation ("exclu-\\nsively")."""
    out = ""
    for ln in lines:
        s = ln.strip()
        if not s:
            continue
        if not out:
            out = s
        elif re.search(r"[a-z]-$", out) and s[:1].islower():
            out = out[:-1] + s
        else:
            out = out + " " + s
    return re.sub(r"[ \t]{2,}", " ", out)


def pg_paragraphs(body: str) -> list[Token]:
    """Paragraphs from a clean (already PG-stripped) e-text.

    Indented blocks (verse, quoted letters) are rendered as Markdown block quotes so
    they survive as separate lines; footnote markers and illustration captions go.
    """
    body = _ILLUSTRATION.sub("", body)
    tokens: list[Token] = []
    for raw in re.split(r"\n[ \t]*\n", body):
        lines = [ln for ln in raw.split("\n") if ln.strip()]
        if not lines:
            continue
        if all(_SEPARATOR.match(ln) for ln in lines):
            tokens.append(Para("* * *"))
            continue
        if lines[0].lstrip().startswith("[") and _FOOTNOTE_MARK.match(lines[0].lstrip()):
            continue  # a footnote body like "[4] [_Week_, p. 375.]"
        if all(ln.startswith("    ") for ln in lines) and len(lines) > 1:
            text = "\n".join("> " + _FOOTNOTE_MARK.sub("", ln.strip()) for ln in lines)
        else:
            text = _FOOTNOTE_MARK.sub("", _join_lines(lines))
        text = text.strip()
        if text:
            tokens.append(Para(text))
    return tokens


def _is_junk(line: str) -> bool:
    s = line.strip()
    if not s or year_only(s) is not None or parse_date_head(s) is not None:
        return False
    letters = sum(c.isalpha() for c in s)
    if letters < 3:
        return True
    return letters / len(s) < 0.5


def ocr_paragraphs(text: str, year_min: int, year_max: int) -> list[Token]:
    """Paragraphs and running-head years from archive.org ``_djvu.txt`` OCR.

    - running heads are removed; odd-page heads become :class:`WeakYear` signals
    - footnotes at the bottom of a page (``1 Probably great bidens.``) are removed
    - OCR debris lines (mostly non-letters) are dropped
    - paragraphs broken by a page turn are re-joined
    """
    lines = text.replace("\r\n", "\n").replace("\f", "\n").split("\n")
    # 1. cut into pages at running heads, remembering each head's year
    pages: list[tuple[int | None, list[str]]] = []
    cur: list[str] = []
    cur_year: int | None = None
    for ln in lines:
        is_head, year = running_head(ln)
        if is_head:
            pages.append((cur_year, cur))
            cur = []
            cur_year = year if (year and year_min <= year <= year_max) else None
            continue
        cur.append(ln)
    pages.append((cur_year, cur))

    tokens: list[Token] = []
    for year, page in pages:
        if year:
            tokens.append(WeakYear(year))
        # 2. footnotes: the first footnote-looking line in the lower half of the page
        nonblank = [i for i, ln in enumerate(page) if ln.strip()]
        if len(nonblank) >= 4:
            half = nonblank[len(nonblank) // 2]
            for i in nonblank:
                if i >= half and _FOOTNOTE_START_OCR.match(page[i]):
                    page = page[:i]
                    break
        # 3. paragraphs
        block: list[str] = []
        for ln in page + [""]:
            if _is_junk(ln):
                continue
            if ln.strip():
                block.append(ln)
            elif block:
                tokens.append(Para(_join_lines(block)))
                block = []

    # 4. re-join paragraphs split by a page turn (or by a dropped line of debris)
    merged: list[Token] = []
    last_para_idx: int | None = None
    for tok in tokens:
        if isinstance(tok, Para) and last_para_idx is not None:
            prev = merged[last_para_idx]
            assert isinstance(prev, Para)
            starts_lower = tok.text[:1].islower()
            if starts_lower and not prev.text.endswith(_TERMINAL):
                if prev.text.endswith("-"):
                    joined = prev.text[:-1] + tok.text
                else:
                    joined = prev.text + " " + tok.text
                merged[last_para_idx] = Para(joined)
                continue
        merged.append(tok)
        if isinstance(tok, Para):
            last_para_idx = len(merged) - 1
    return merged
