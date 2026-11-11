import datetime as dt

import pytest

from anamnesis_lab.dates import chapter_year, parse_date_head, running_head, safe_date, year_only


@pytest.mark.parametrize(
    "line,expected",
    [
        ('_Oct 22._ "What are you doing now?"', (10, 22, None, '"What are you doing now?"')),
        ("_Oct. 24._ Every part of nature teaches", (10, 24, None, "Every part of nature teaches")),
        ("Sept. 16. Met the selectmen of Sudbury, and", (9, 16, None, "Met the selectmen of Sudbury, and")),
        ("_Sept. 11. Wednesday._ The river higher", (9, 11, None, "The river higher")),
        ("Thursday, January 3. N. lat. 37° 53.", (1, 3, None, "N. lat. 37° 53.")),
        ("Fanuary 22.", (1, 22, None, "")),  # OCR reads J as F
        ("Fuly 21 [2].", (7, 21, None, "")),
        ("February 12 (?).", (2, 12, None, "")),
        ("Montreal, April 19.", (4, 19, None, "")),
        ("Concord, March 23, 1835.", (3, 23, 1835, "")),
        ("_June 11, 1851._", (6, 11, 1851, "")),
        ("CAMBRIDGE, _Jan. 30, 1865_.", (1, 30, 1865, "")),
        ("RIVER SOLIMOES (AMAZON), _Sept. 12-15, 1865_.", (9, 12, 1865, "")),
        ("May 26.", (5, 26, None, "")),
    ],
)
def test_parse_date_head(line, expected):
    h = parse_date_head(line)
    assert h is not None
    assert (h.month, h.day, h.year, h.rest) == expected


@pytest.mark.parametrize(
    "line",
    [
        "September 7 was published Nature, Addresses",
        "January 1, 1850, and sheets printed here were sent",
        "January 165.",
        "i saw him, May 4.",
        "On Friday, July 19, Margaret dies on rocks",
        "Mayflowers are out.",
    ],
)
def test_not_a_date_head(line):
    assert parse_date_head(line) is None


def test_running_heads():
    assert running_head("1851] PERAMBULATING THE BOUNDS 5") == (True, 1851)
    assert running_head("1833) | WIND AND CALM -") == (True, 1833)
    assert running_head("4 JOURNAL (Serr. 16") == (True, None)
    assert running_head("6 . JOURNAL [AcE 29") == (True, None)
    assert running_head("1851, the year the selectmen came, was dry and long and hot and slow") == (False, None)


def test_year_lines():
    assert year_only("1837") == 1837
    assert year_only("1845-1846") == 1845
    assert year_only("1837 was a year") is None
    assert chapter_year("SEPTEMBER AND OCTOBER, 1851 (ÆT. 34)") == 1851
    assert chapter_year("1850 (ÆT. 32-33)") == 1850
    assert chapter_year("In 1851 he walked to the cliffs.") is None


def test_safe_date_clamps():
    assert safe_date(1851, 2, 31) == dt.date(1851, 2, 28)
    assert safe_date(1852, 2, 30) == dt.date(1852, 2, 29)
