from anamnesis_lab.textclean import Para, WeakYear, ocr_paragraphs, pg_paragraphs, slice_text

OCR = """JOURNAL

1851

Sept. 16. Met the selectmen of Sudbury, and found
them exceedingly rough and coun-
trified and more illiterate than usual,
and difficult to deal with on the bounds

1 Probably great bidens.

~,4

4 JOURNAL [Sept. 16

of the town, which they walk every five years.

1852] THE EDGE OF THE MEADOW 5

Sept. 17. Perambulated the Lincoln line.
"""


def test_ocr_paragraphs_heads_footnotes_hyphens_and_page_turns():
    toks = ocr_paragraphs(OCR, 1837, 1862)
    paras = [t.text for t in toks if isinstance(t, Para)]
    years = [t.year for t in toks if isinstance(t, WeakYear)]
    assert "1851" in paras  # a bare year line is kept as a signal, not dropped as debris
    joined = next(p for p in paras if p.startswith("Sept. 16."))
    assert "countrified" in joined  # end-of-line hyphenation undone
    assert joined.endswith("which they walk every five years.")  # re-joined across the page turn
    assert not any("Probably great bidens" in p for p in paras)  # page-foot note dropped
    assert not any("JOURNAL [" in p for p in paras)  # running head dropped
    assert not any("~,4" in p for p in paras)  # OCR debris dropped
    assert years == [1852]
    assert paras[-1] == "Sept. 17. Perambulated the Lincoln line."


def test_pg_paragraphs_verse_and_footnote_marks():
    body = (
        "First line of prose\nwrapped here.[12]\n\n"
        "     He seems to avoid us,\n     To seek something.\n\n"
        "     [12] [A note by the editor.]\n\n"
        "[Illustration: _A view_]\n\n"
        "       *       *       *       *       *\n"
    )
    paras = [t.text for t in pg_paragraphs(body)]
    assert paras[0] == "First line of prose wrapped here."
    assert paras[1] == "> He seems to avoid us,\n> To seek something."
    assert paras[2] == "* * *"
    assert len(paras) == 3


def test_slice_text_nth_stop_and_drop():
    text = "JOURNAL I\ncontents\nJOURNAL I\nbody 1\nEND OF VOLUME I\ntitle page\nTHE JOURNAL OF\nbody 2\nINDEX\nAbbott"
    out = slice_text(text, r"^JOURNAL I$", [r"^INDEX$"], 2, [(r"^END OF VOLUME", r"^THE JOURNAL OF$")])
    assert out.split("\n") == ["JOURNAL I", "body 1", "body 2"]
