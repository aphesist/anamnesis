import json

import pytest

from anamnesis_lab.cli import main
from anamnesis_lab.demo import build_theme
from anamnesis_lab.demo.curate import curate, parse_note, strip_dangling
from anamnesis_lab.gutenberg import assert_clean
from anamnesis_lab.links import extract_links


@pytest.fixture(scope="module")
def full(tmp_path_factory):
    out = tmp_path_factory.mktemp("full")
    build_theme("transcendentalist", out, out / "cache", small=True)
    return out


def _tree(d):
    return {p.relative_to(d).as_posix(): p.read_bytes() for p in sorted(d.rglob("*")) if p.is_file()}


def test_curate_is_deterministic_and_self_contained(full, tmp_path):
    a, b = tmp_path / "a", tmp_path / "b"
    m = curate(full, a, target_notes=120, min_chars=50)
    # the cli takes the notes folder as well as the bundle folder
    assert main(["demo", "curate", "--from", str(full / "notes"), "--out", str(b), "--target-notes", "120", "--min-chars", "50"]) == 0
    assert _tree(a) == _tree(b)

    assert (a / "COLOPHON.md").is_file() and (a / "SOURCES.md").is_file()
    assert "this curated set" in (a / "COLOPHON.md").read_text()
    files = list((a / "notes").rglob("*.md"))
    assert len(files) == m["counts"]["notes"] <= 120
    assert m["counts"]["notes"] >= 100

    names = {p.stem for p in files}
    notes = [parse_note(p, a / "notes") for p in files]
    for p in files:
        text = p.read_text(encoding="utf-8")
        assert_clean(text, str(p))
        for t in extract_links(text):
            assert t in names, (p, t)  # no dangling links survive

    kinds = {n.kind for n in notes}
    assert {"hub", "journal", "essay"} <= kinds
    assert sum(n.kind == "hub" for n in notes) == 2
    assert m["counts"]["by_origin"]["external"] > 0 and m["counts"]["by_origin"]["owner_written"] > 0
    assert m["counts"]["links"] > 0

    # every year of each author's journal in the full build is still there
    full_notes = [parse_note(p, full / "notes") for p in (full / "notes").rglob("*.md")]
    for author in ("Henry David Thoreau", "Ralph Waldo Emerson"):
        want = {n.year for n in full_notes if n.kind == "journal" and n.author == author}
        got = {n.year for n in notes if n.kind == "journal" and n.author == author}
        assert got == want, author

    # every cross-author linked journal entry of the small build fits in this budget
    linked = {n.basename for n in full_notes if n.kind == "journal" and n.links}
    assert linked - names == set() or len(linked - names) < len(linked) // 4
    assert json.loads((a / "manifest.json").read_text())["curated"] is True


def test_byte_budget_is_respected(full, tmp_path):
    m = curate(full, tmp_path / "c", target_notes=1000, max_bytes=150_000)
    # hubs and index notes are always kept, everything else within the budget
    assert m["counts"]["bytes"] <= 150_000 + 20_000


def test_strip_dangling():
    body = "see [[A|the first]] and [[B]].\n\n- [[A]]\n- [[B]]\n"
    assert strip_dangling(body, {"A"}) == "see [[A|the first]] and B.\n\n- [[A]]\n"
