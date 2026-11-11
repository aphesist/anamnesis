"""Date parsing for 19th-century journal headings, tolerant of common OCR errors."""

from __future__ import annotations

import calendar
import datetime as dt
import re
from dataclasses import dataclass

# Month names and abbreviations, plus OCR misreadings seen in the archive.org scans
# (a capital J is often read as F: "Fanuary", "Fune", "Fuly").
MONTHS: dict[str, int] = {}
for _i, _name in enumerate(calendar.month_name):
    if _name:
        MONTHS[_name.lower()] = _i
for _abbr, _i in {
    "jan": 1, "feb": 2, "mar": 3, "apr": 4, "jun": 6, "jul": 7, "aug": 8,
    "sep": 9, "sept": 9, "oct": 10, "nov": 11, "dec": 12,
    "fanuary": 1, "fan": 1, "fune": 6, "fuly": 7, "januaiy": 1, "febru-ary": 2,
}.items():
    MONTHS[_abbr] = _i

_MONTH_ALT = "|".join(sorted((re.escape(k) for k in MONTHS), key=len, reverse=True))
_WEEKDAYS = "monday|tuesday|wednesday|thursday|friday|saturday|sunday"

# A date heading at the start of a paragraph. Accepts an optional prefix of up to three
# capitalised words ending in a comma ("Concord,", "Saturday evening,", "Montreal,"),
# the month, the day, an optional year, and small editorial decorations such as
# "[2]" or "(?)". The heading must end with a period or the end of the line, so a
# sentence like "September 7 was published Nature" is not taken for a date.
_DATE_HEAD = re.compile(
    rf"""^\s*_?
    (?:(?:(?-i:\(?[A-Z][A-Za-z]+\)?)\.?\s?){{1,4}}(?:(?:evening|morning|afternoon|night|noon)\s?)?,\s+_?)?
    (?P<month>{_MONTH_ALT})\.?\s+
    (?P<day>\d{{1,2}})(?:st|nd|rd|th|d)?
    (?:\s*(?:\[\d+\]|\(\?\)|\[\?\]))?
    (?:\s*[-–]\s*\d{{1,2}})?
    (?:,\s*(?P<year>1[6-9]\d\d))?
    \s*(?:\.\s*_?|_?\.|_)\s*(?P<rest>.*)$
    |
    ^\s*_?
    (?:(?:(?-i:\(?[A-Z][A-Za-z]+\)?)\.?\s?){{1,4}}(?:(?:evening|morning|afternoon|night|noon)\s?)?,\s+_?)?
    (?P<month2>{_MONTH_ALT})\.?\s+
    (?P<day2>\d{{1,2}})(?:st|nd|rd|th|d)?
    (?:\s*(?:\[\d+\]|\(\?\)|\[\?\]))?
    (?:,\s*(?P<year2>1[6-9]\d\d))?
    \s*$
    """,
    re.I | re.X,
)

_LEADING_WEEKDAY = re.compile(rf"^(?:{_WEEKDAYS})\.?[_.]*\s*", re.I)


@dataclass(frozen=True)
class DateHead:
    month: int
    day: int
    year: int | None
    rest: str


def parse_date_head(line: str) -> DateHead | None:
    """Parse a journal date heading such as ``_Oct. 24._ Every part…`` or ``Thursday, January 3.``.

    Returns ``None`` when the line does not start with a plausible date.
    """
    m = _DATE_HEAD.match(line)
    if not m:
        return None
    month_s = m.group("month") or m.group("month2")
    day_s = m.group("day") or m.group("day2")
    year_s = m.group("year") or m.group("year2")
    rest = (m.group("rest") or "").strip()
    month = MONTHS.get(month_s.lower().rstrip("."))
    if month is None:
        return None
    day = int(day_s)
    if not 1 <= day <= 31:
        return None
    # A capitalised prefix word must not be the start of an ordinary sentence:
    # "May 4." is fine, but "On May" style prefixes are only accepted with a comma.
    rest = _LEADING_WEEKDAY.sub("", rest).lstrip("_. ")
    return DateHead(month=month, day=day, year=int(year_s) if year_s else None, rest=rest)


def safe_date(year: int, month: int, day: int) -> dt.date:
    """Build a date, clamping the day to the month's length (OCR may give "Feb. 31")."""
    last = calendar.monthrange(year, month)[1]
    return dt.date(year, month, min(day, last))


_YEAR_ONLY = re.compile(r"^\s*(1[6-9]\d\d)(?:\s*[-–]\s*1?[6-9]?\d\d)?\s*\.?\s*$")
_RUNNING_HEAD_ODD = re.compile(r"^\s*(1[6-9]\d\d)\s*[\]\)\}|!]")
_RUNNING_HEAD_EVEN = re.compile(r"^\s*\d{1,3}\s*[.,]?\s*(?:JOURNAL|LETTERS)\b")


def year_only(line: str) -> int | None:
    """``1837`` or ``1845-1846`` on a line of its own → first year."""
    m = _YEAR_ONLY.match(line)
    return int(m.group(1)) if m else None


def running_head(line: str) -> tuple[bool, int | None]:
    """Detect printed running heads in OCR text.

    Odd pages carry ``1851] PERAMBULATING THE BOUNDS 5`` (the year is useful),
    even pages ``4 JOURNAL [Sept. 16``. Returns (is_running_head, year_or_None).
    """
    m = _RUNNING_HEAD_ODD.match(line)
    if m and len(line.strip()) < 70:
        return True, int(m.group(1))
    if _RUNNING_HEAD_EVEN.match(line) and len(line.strip()) < 70:
        return True, None
    return False, None


_CHAPTER_YEAR = re.compile(r"\b(1[6-9]\d\d)\b")


def chapter_year(line: str) -> int | None:
    """Chapter headings such as ``SEPTEMBER AND OCTOBER, 1851 (ÆT. 34)`` → 1851.

    Only lines written mostly in capitals count, so a year inside prose is ignored.
    """
    s = line.strip()
    if not s or len(s) > 90:
        return None
    m = _CHAPTER_YEAR.search(s)
    if not m:
        return None
    letters = [c for c in s if c.isalpha()]
    if not letters:
        return int(m.group(1))
    upper = sum(1 for c in letters if c.isupper())
    if upper / len(letters) >= 0.7:
        return int(m.group(1))
    return None
