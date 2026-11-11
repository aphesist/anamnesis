import datetime as dt

import yaml

from anamnesis_lab.bundle import Basenames, Note, clean_basename, first_sentence, render
from anamnesis_lab.links import LinkRule, apply_links, extract_links


def test_frontmatter_is_valid_yaml_with_required_fields():
    n = Note(
        basename="1851-09-16 thoreau Met the selectmen", title='Met the "selectmen": of Sudbury',
        created=dt.date(1851, 9, 16), author="Henry David Thoreau", origin="owner_written", kind="journal",
        body="Met the selectmen of Sudbury.", tags=["thoreau", "journal", "1850s"],
        source_url="https://archive.org/details/x", source_title="Journal (1906)",
    )
    text = render(n)
    assert text.startswith("---\n")
    _, fm, body = text.split("---\n", 2)
    meta = yaml.safe_load(fm)
    assert meta["title"] == 'Met the "selectmen": of Sudbury'
    assert meta["created"] == dt.datetime(1851, 9, 16, 12, 0, tzinfo=dt.timezone.utc)
    assert meta["tags"] == ["thoreau", "journal", "1850s"]
    assert meta["author"] == "Henry David Thoreau"
    assert meta["source_url"] == "https://archive.org/details/x"
    assert meta["origin"] == "owner_written"
    assert meta["date_precision"] == "day"
    assert body.strip() == "Met the selectmen of Sudbury."


def test_basenames_are_unique_and_safe():
    b = Basenames()
    assert b.reserve("Nature") == "Nature"
    assert b.reserve("nature") == "nature (2)"
    assert b.reserve("Nature", alt="Nature (Essays, Second Series)") == "Nature (Essays, Second Series)"
    assert clean_basename('a/b: c? "d" [e] #f') == "ab c d e f"


def test_first_sentence_skips_abbreviations():
    assert first_sentence("P. M. — To Cliffs. Then home.") == "P. M. — To Cliffs"
    assert first_sentence("Mr. Emerson came by. We walked.") == "Mr. Emerson came by"
    assert first_sentence("[[Henry David Thoreau|Henry]] walked with me today.") == "Henry walked with me today"
    assert len(first_sentence("word " * 40)) <= 61


def test_apply_and_extract_links():
    rules = [
        LinkRule(r"(?<!G\. B\. )\bEmerson\b(?![’']s Report)", "Ralph Waldo Emerson", frozenset({"T"})),
        LinkRule(r"\bSelf-Reliance\b", "Self-Reliance", frozenset({"T"})),
    ]
    targets = {"Ralph Waldo Emerson", "Self-Reliance"}
    body, found = apply_links("G. B. Emerson's Report on trees. Later Emerson read Self-Reliance.", "T", rules, targets)
    assert found == ["Ralph Waldo Emerson", "Self-Reliance"]
    assert "G. B. Emerson's Report" in body
    assert "[[Ralph Waldo Emerson|Emerson]] read [[Self-Reliance]]" in body
    assert extract_links(body) == ["Ralph Waldo Emerson", "Self-Reliance"]
    # other authors are not linked, missing targets are skipped
    assert apply_links("Emerson", "E", rules, targets)[1] == []
    assert apply_links("Emerson", "T", rules, set())[1] == []
    # a resolver can redirect the target for this note
    body, found = apply_links("Emerson came.", "T", rules, targets | {"1850-01-02 emerson x"}, lambda t: "1850-01-02 emerson x")
    assert found == ["1850-01-02 emerson x"]
