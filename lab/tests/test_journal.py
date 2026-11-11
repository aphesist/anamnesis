import datetime as dt

from anamnesis_lab.journal import (
    Entry,
    JournalOptions,
    SectionOptions,
    heading_title,
    looks_like_quotation,
    split_journal,
    split_long,
    split_sections,
)
from anamnesis_lab.textclean import Para, WeakYear


def P(*texts):
    return [Para(t) for t in texts]


def opts(**kw):
    base = dict(author="A", source_id="s", year_min=1837, year_max=1862)
    base.update(kw)
    return JournalOptions(**base)


def test_dates_years_and_rollover():
    toks = P("1840", "_Nov. 3._ First.", "More of the third.", "_Dec. 30._ Second.", "_Jan. 2._ Third, a new year.")
    es = split_journal(toks, opts())
    assert [e.date for e in es] == [dt.date(1840, 11, 3), dt.date(1840, 12, 30), dt.date(1841, 1, 2)]
    assert es[0].paragraphs == ["First.", "More of the third."]
    assert all(e.precision == "day" for e in es)


def test_explicit_year_and_weak_year_gap():
    toks = P("_June 11, 1846._ One.") + [WeakYear(1850)] + P("_March 2._ After a gap of years.")
    es = split_journal(toks, opts())
    assert [e.date for e in es] == [dt.date(1846, 6, 11), dt.date(1850, 3, 2)]


def test_headings_become_titles():
    toks = P("1837", "SOLITUDE", "_Oct. 22._ To be alone I find it necessary.", "THE FOG", "The prospect is limited.")
    es = split_journal(toks, opts(use_headings=True))
    assert [(e.title, e.date) for e in es] == [("Solitude", dt.date(1837, 10, 22)), ("The Fog", dt.date(1837, 10, 22))]


def test_undated_year_breaks():
    toks = P("1849", "_May 4._ Dated.", "Undated thought.") + [WeakYear(1851)] + P("Another undated thought.")
    es = split_journal(toks, opts(undated_year_breaks=True))
    assert [(e.date, e.precision) for e in es] == [(dt.date(1849, 5, 4), "day"), (dt.date(1851, 1, 1), "year")]


def test_letters_titles_and_editor_narrative():
    toks = P(
        "Introduction by the editor.",
        "_To his Father._",
        "CAMBRIDGE, _Sept. 12, 1865_.",
        "DEAR FATHER,--Great was my joy.",
        "* * *",
        "The editor explains what happened next.",
        "_To Henry James._",
        "_Oct. 1, 1865_.",
        "DEAR HARRY,--Another letter.",
    )
    es = split_journal(toks, opts(kind="letter", year_max=1910, title_regex=r"^_To .+\._$", separator_ends_entry=True))
    assert [(e.title, e.date) for e in es] == [("To his Father", dt.date(1865, 9, 12)), ("To Henry James", dt.date(1865, 10, 1))]
    assert all("editor" not in e.body for e in es)


def test_split_long_keeps_paragraphs_whole():
    e = Entry("A", dt.date(1850, 1, 1), "day", "journal", "s", paragraphs=["x" * 30, "y" * 30, "z" * 30])
    parts = split_long([e], max_chars=65)
    assert [p.part for p in parts] == [1, 2]
    assert parts[0].paragraphs == ["x" * 30, "y" * 30] and parts[1].paragraphs == ["z" * 30]


def test_quotation_detection():
    q = Entry("A", dt.date(1837, 10, 26), "day", "journal", "s",
              paragraphs=["> \"He seems to avoid us,\n> To seek something.\"—_Ibid._"])
    own = Entry("A", dt.date(1837, 10, 26), "day", "journal", "s", paragraphs=["The fog lies on the river."])
    assert looks_like_quotation(q)
    assert not looks_like_quotation(own)


def test_sections_merge_epigraph_and_repeated_title():
    toks = P("I. SELF-RELIANCE", "> a short epigraph", "SELF-RELIANCE", "Trust thyself. " * 40, "II. CIRCLES", "The eye is the first circle. " * 30)
    es = split_sections(toks, SectionOptions(author="E", source_id="s", work="Essays", date=dt.date(1841, 1, 1)))
    assert [e.title for e in es] == ["Self-Reliance", "Circles"]
    assert es[0].paragraphs[0] == "> a short epigraph"
    assert es[0].origin == "external"


def test_heading_title():
    assert heading_title("THE MOULD OUR DEEDS LEAVE") == "The Mould Our Deeds Leave"
    assert heading_title("IX. THE OVER-SOUL.") == "The Over-Soul"
