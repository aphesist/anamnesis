"""Strip Project Gutenberg wrappers so the remaining text is plain public-domain text.

The licence page says the trademark terms stop applying once the licence and every
reference to the project's name are removed; credit then belongs in a colophon only.
:func:`strip_gutenberg` removes the header, the footer licence, producer credits,
transcriber notes and any remaining line naming the project, and
:func:`assert_clean` is a hard check the builders run on every output file.
"""

from __future__ import annotations

import re

_START = re.compile(r"^\s*\*{3}\s*START OF (?:THE|THIS) PROJECT GUTENBERG.*$", re.I | re.M)
_END = re.compile(r"^\s*\*{3}\s*END OF (?:THE|THIS) PROJECT GUTENBERG.*$", re.I | re.M)
# Very old files use a different marker for the end of the text.
_END_OLD = re.compile(r"^\s*End of (?:the )?Project Gutenberg.*$", re.I | re.M)
# The project's name, its site, its trademark forms and its proofreading site. The bare
# word "Gutenberg" is allowed: Thoreau writes about the inventor of printing.
_MENTION = re.compile(
    r"project\s+gutenberg|gutenberg\.org|gutenberg[-\s]*(?:tm|™)|gutenberg\s+literary|gutenberg\s+e-?books?"
    r"|gutenberg\s+license|pgdp\.net|distributed\s+proofread|pglaf",
    re.I,
)
_CREDITS = re.compile(r"^\s*(?:Produced by|E-?text prepared by|Transcribed by)\b", re.I)
_TRANSCRIBER = re.compile(r"^\s*\[?\s*Transcriber['’]s Notes?\b", re.I)


def _paragraphs(text: str) -> list[str]:
    return re.split(r"\n[ \t]*\n", text)


def strip_gutenberg(text: str) -> str:
    """Return the body of a Project Gutenberg plain-text file with all PG matter removed."""
    text = text.replace("\r\n", "\n").replace("﻿", "")
    m = _START.search(text)
    if m:
        text = text[m.end():]
    m = _END.search(text) or _END_OLD.search(text)
    if m:
        text = text[: m.start()]

    kept: list[str] = []
    skipping_transcriber = False
    for para in _paragraphs(text):
        stripped = para.strip()
        if not stripped:
            kept.append(para)
            continue
        if _CREDITS.match(stripped):
            continue
        if _TRANSCRIBER.match(stripped):
            # A transcriber's note runs until the next blank-line-separated paragraph
            # that is not indented (the list of retained spellings is indented).
            skipping_transcriber = True
            continue
        if skipping_transcriber:
            if para.startswith((" ", "\t")):
                continue
            skipping_transcriber = False
        if _MENTION.search(para):
            # Drop just the offending lines; if nothing meaningful remains, drop it all.
            lines = [ln for ln in para.split("\n") if not _MENTION.search(ln)]
            if sum(len(ln.strip()) for ln in lines) < 20:
                continue
            para = "\n".join(lines)
        kept.append(para)
    out = "\n\n".join(kept)
    out = re.sub(r"\n{3,}", "\n\n\n", out)
    return out.strip("\n") + "\n"


def assert_clean(text: str, where: str = "") -> None:
    """Raise if any trace of the Project Gutenberg name survives."""
    m = _MENTION.search(text)
    if m:
        ctx = text[max(0, m.start() - 40) : m.end() + 40].replace("\n", " ")
        raise ValueError(f"project-gutenberg mention survived in {where or 'text'}: …{ctx}…")
