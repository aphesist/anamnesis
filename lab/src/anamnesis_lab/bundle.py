"""Write entries as an importable bundle: Markdown notes with YAML frontmatter + manifest.

Layout::

    <out>/
      notes/<author>/<kind>/<basename>.md   one note per entry (point the folder or
                                            obsidian connector at notes/)
      manifest.json                         counts, date range, links, licences
      SOURCES.md                            edition, contributors, death years, status
      COLOPHON.md                           where the texts came from

Basenames are unique across the bundle so ``[[basename]]`` resolves the way Obsidian does.
"""

from __future__ import annotations

import datetime as dt
import json
import re
import unicodedata
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

from .gutenberg import assert_clean
from .links import extract_links


@dataclass
class Note:
    basename: str
    title: str
    created: dt.date
    author: str
    origin: str
    kind: str
    body: str
    tags: list[str] = field(default_factory=list)
    source_url: str = ""
    source_title: str = ""
    precision: str = "day"
    links: list[str] = field(default_factory=list)

    @property
    def created_rfc3339(self) -> str:
        return f"{self.created.isoformat()}T12:00:00Z"


_BAD_CHARS = re.compile(r'[\\/:*?"<>|#^\[\]{}\n\r\t]')


def clean_basename(s: str, max_len: int = 90) -> str:
    s = unicodedata.normalize("NFC", s)
    s = s.replace("_", "")
    s = _BAD_CHARS.sub("", s)
    s = re.sub(r"\s+", " ", s).strip(" .-")
    if len(s) > max_len:
        s = s[:max_len].rsplit(" ", 1)[0].rstrip(" ,;.-")
    return s or "untitled"


def slug(s: str) -> str:
    s = unicodedata.normalize("NFKD", s).encode("ascii", "ignore").decode()
    s = re.sub(r"[^a-zA-Z0-9]+", "-", s).strip("-").lower()
    return s or "x"


# Abbreviations after which a period does not end a sentence.
_ABBREV = re.compile(
    r"(?:\b(?:Mr|Mrs|Dr|St|Rev|Prof|Gen|Col|Capt|Mt|vol|viz|cf|ib|ibid|etc|i\.e|e\.g|p|a|m|vs|No)|\b[A-Z])\.$"
)


def first_sentence(body: str, max_len: int = 60) -> str:
    """A title from the first sentence of a body: abbreviations and initials don't end it."""
    text = re.sub(r"^>\s?", "", body.strip(), flags=re.M)
    text = re.sub(r"\[\[([^\]|]+\|)?([^\]]+)\]\]", r"\2", text)
    text = text.replace("_", "").replace("\n", " ").strip()
    end = None
    for m in re.finditer(r"[.!?;:—](?=\s|$)", text):
        if m.group(0) == "." and _ABBREV.search(text[: m.end()]):
            continue
        if m.end() < 8:
            continue
        end = m.end()
        break
    s = (text[:end] if end else text).strip().rstrip(".;:—").strip()
    if len(s) > max_len:
        s = s[:max_len].rsplit(" ", 1)[0].rstrip(",;") + "…"
    return s or "untitled"


def _yaml_str(s: str) -> str:
    # JSON strings are valid YAML double-quoted scalars.
    return json.dumps(s, ensure_ascii=False)


def frontmatter(note: Note) -> str:
    lines = [
        "---",
        f"title: {_yaml_str(note.title)}",
        f"created: {note.created_rfc3339}",
        "tags: [" + ", ".join(_yaml_str(t) for t in note.tags) + "]",
        f"author: {_yaml_str(note.author)}",
        f"source_url: {_yaml_str(note.source_url)}",
        f"origin: {note.origin}",
        f"kind: {note.kind}",
        f"date_precision: {note.precision}",
    ]
    if note.source_title:
        lines.append(f"source: {_yaml_str(note.source_title)}")
    lines.append("---")
    return "\n".join(lines) + "\n"


def render(note: Note) -> str:
    return frontmatter(note) + "\n" + note.body.strip() + "\n"


class Basenames:
    """Hands out bundle-unique basenames (case-insensitive, like most file systems)."""

    def __init__(self) -> None:
        self._lower: set[str] = set()

    def reserve(self, name: str, alt: str | None = None) -> str:
        base = clean_basename(name)
        if alt and base.lower() in self._lower:
            base = clean_basename(alt)
        cand, i = base, 2
        while cand.lower() in self._lower:
            cand = f"{base} ({i})"
            i += 1
        self._lower.add(cand.lower())
        return cand


def write_bundle(
    out: Path,
    notes: list[Note],
    *,
    theme: str,
    owner_label: str,
    sources: list[dict],
    colophon: str,
    sources_md: str,
    small: bool,
    extra: dict | None = None,
) -> dict:
    """Write notes and metadata; return the manifest (also written to manifest.json)."""
    notes_dir = out / "notes"
    if notes_dir.exists():
        for p in sorted(notes_dir.rglob("*.md"), reverse=True):
            p.unlink()
    out.mkdir(parents=True, exist_ok=True)
    by_name = {n.basename: n for n in notes}
    link_pairs = 0
    later = 0
    for n in notes:
        n.links = [t for t in extract_links(n.body) if t in by_name]
        for t in n.links:
            link_pairs += 1
            if by_name[t].created < n.created:
                later += 1
        path = notes_dir / slug(n.author) / n.kind / f"{n.basename}.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        text = render(n)
        assert_clean(text, str(path))
        path.write_text(text, encoding="utf-8")

    dated = [n for n in notes if n.kind in ("journal", "letter") and n.precision == "day"]
    manifest = {
        "theme": theme,
        "owner": owner_label,
        "small": small,
        "built_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
        "notes_dir": "notes",
        "counts": {
            "notes": len(notes),
            "by_author": dict(Counter(n.author for n in notes).most_common()),
            "by_kind": dict(Counter(n.kind for n in notes).most_common()),
            "by_origin": dict(Counter(n.origin for n in notes).most_common()),
            "by_precision": dict(Counter(n.precision for n in notes).most_common()),
            "by_decade": dict(sorted(Counter(f"{n.created.year // 10 * 10}s" for n in notes).items())),
            "links": link_pairs,
            "links_to_earlier_notes": later,
            "words": sum(len(n.body.split()) for n in notes),
        },
        "date_range": {
            "all": [min(n.created for n in notes).isoformat(), max(n.created for n in notes).isoformat()] if notes else None,
            "day_precision_entries": [min(n.created for n in dated).isoformat(), max(n.created for n in dated).isoformat()] if dated else None,
        },
        "sources": sources,
        "licence": (
            "texts: public domain in the United States and Brazil (see SOURCES.md); "
            "builder code and generated index/hub notes: Apache-2.0"
        ),
        "note": "links between authors are explicit references found by a heuristic; the demo's 'links made later' is a demonstration, not a measurement",
    }
    if extra:
        manifest.update(extra)
    # COLOPHON and SOURCES are metadata about the bundle and may name where texts came from.
    (out / "COLOPHON.md").write_text(colophon, encoding="utf-8")
    (out / "SOURCES.md").write_text(sources_md, encoding="utf-8")
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return manifest
