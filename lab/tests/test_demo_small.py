import json

import pytest
import yaml

from anamnesis_lab.demo import build_theme
from anamnesis_lab.gutenberg import assert_clean
from anamnesis_lab.links import extract_links


@pytest.fixture(scope="module")
def bundle(tmp_path_factory):
    out = tmp_path_factory.mktemp("bundle")
    manifest = build_theme("transcendentalist", out, out / "cache", small=True)
    return out, manifest


def test_small_build_counts(bundle):
    out, m = bundle
    c = m["counts"]
    assert 250 <= c["notes"] <= 350
    assert set(c["by_author"]) >= {"Henry David Thoreau", "Ralph Waldo Emerson"}
    assert c["by_origin"]["owner_written"] > 0 and c["by_origin"]["external"] > 0
    assert c["links_to_earlier_notes"] > 0
    assert m["small"] is True
    assert json.loads((out / "manifest.json").read_text())["counts"] == c
    assert (out / "COLOPHON.md").exists() and (out / "SOURCES.md").exists()
    assert not (out / "cache").exists()  # --small never downloads


def test_every_note_has_frontmatter_and_no_distributor_marks(bundle):
    out, _ = bundle
    files = list((out / "notes").rglob("*.md"))
    names = {p.stem for p in files}
    for p in files:
        text = p.read_text(encoding="utf-8")
        assert_clean(text, str(p))
        _, fm, body = text.split("---\n", 2)
        meta = yaml.safe_load(fm)
        for key in ("title", "created", "tags", "author", "source_url", "origin"):
            assert key in meta, (p, key)
        assert meta["origin"] in ("owner_written", "external")
        for t in extract_links(body):
            assert t in names, (p, t)


def test_dates_and_links_are_plausible(bundle):
    out, m = bundle
    first, last = m["date_range"]["day_precision_entries"]
    assert first >= "1836-01-01" and last <= "1856-01-01"
    thoreau = "\n".join(p.read_text() for p in (out / "notes" / "henry-david-thoreau").rglob("*.md"))
    assert "[[Ralph Waldo Emerson|Emerson]]" in thoreau or "|Emerson]]" in thoreau
    assert "G. B. Emerson's Report" in thoreau and "[[Ralph Waldo Emerson|G. B." not in thoreau
    emerson = "\n".join(p.read_text() for p in (out / "notes" / "ralph-waldo-emerson").rglob("*.md"))
    assert "Henry Thoreau]]" in emerson
    assert "editors' note" not in emerson  # bracketed editorial notes are dropped
    assert "foot of the page" not in emerson  # page-foot notes are dropped


def test_pragmatist_small_builds(tmp_path):
    m = build_theme("pragmatist", tmp_path, tmp_path / "cache", small=True)
    assert m["counts"]["by_kind"]["letter"] >= 25
    assert "partial" in m
    letters = list((tmp_path / "notes" / "william-james" / "letter").glob("*.md"))
    assert all("skipped" not in p.read_text() for p in letters)
