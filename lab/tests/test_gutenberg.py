import pytest

from anamnesis_lab.gutenberg import assert_clean, strip_gutenberg

RAW = """﻿The Project Gutenberg eBook of Walking\r
\r
This eBook is for the use of anyone anywhere in the United States and\r
most other parts of the world at no cost. See www.gutenberg.org.\r
\r
Title: Walking\r
\r
*** START OF THE PROJECT GUTENBERG EBOOK WALKING ***\r
\r
Produced by a volunteer and the Online Distributed Proofreading Team\r
at http://www.pgdp.net\r
\r
Transcriber's Note:\r
\r
  Obvious typographical errors have been corrected.\r
\r
WALKING\r
\r
I wish to speak a word for Nature, for absolute freedom and wildness.\r
\r
It was so many years before Gutenberg invented printing.\r
\r
This file was made available by Project Gutenberg volunteers.\r
\r
*** END OF THE PROJECT GUTENBERG EBOOK WALKING ***\r
\r
Section 1. General Terms of Use and Redistributing Project Gutenberg\r
electronic works.\r
"""


def test_strip_removes_header_footer_credits_and_mentions():
    out = strip_gutenberg(RAW)
    assert out.startswith("WALKING")
    assert "absolute freedom" in out
    assert "Produced by" not in out
    assert "Transcriber" not in out
    assert "Section 1" not in out
    assert "\r" not in out and "﻿" not in out
    assert_clean(out)


def test_inventor_of_printing_is_kept():
    out = strip_gutenberg(RAW)
    assert "before Gutenberg invented printing" in out


@pytest.mark.parametrize(
    "text",
    ["Thanks to Project Gutenberg.", "see https://www.gutenberg.org/ebooks/1", "the Project Gutenberg-tm License"],
)
def test_assert_clean_rejects_mentions(text):
    with pytest.raises(ValueError):
        assert_clean(text)


def test_strip_without_markers_keeps_text():
    assert strip_gutenberg("Just a plain text.\n") == "Just a plain text.\n"
