"""Read a demo bundle (or any folder of Markdown notes with the same frontmatter) back in."""

from __future__ import annotations

import datetime as dt
import json
import re
from dataclasses import dataclass, field
from pathlib import Path

from ..links import extract_links

_FM = re.compile(r"^---\n(.*?)\n---\n?(.*)$", re.S)


@dataclass
class BNote:
    basename: str
    path: str
    title: str
    created: dt.datetime
    author: str
    origin: str
    kind: str
    body: str
    tags: list[str] = field(default_factory=list)
    links: list[str] = field(default_factory=list)

    def plain_text(self) -> str:
        """Body with link markup reduced to its visible text, so link targets don't leak."""
        t = re.sub(r"\[\[[^\]|]+\|([^\]]+)\]\]", r"\1", self.body)
        return re.sub(r"\[\[([^\]]+)\]\]", r"\1", t)


def _value(raw: str):
    raw = raw.strip()
    if raw.startswith("[") and raw.endswith("]"):
        inner = raw[1:-1].strip()
        if not inner:
            return []
        return json.loads(raw) if '"' in inner else [x.strip() for x in inner.split(",")]
    if raw.startswith('"'):
        return json.loads(raw)
    return raw


def parse_note(text: str) -> tuple[dict, str]:
    """Minimal frontmatter reader for the flat key: value form the builders write."""
    m = _FM.match(text)
    if not m:
        return {}, text
    meta = {}
    for line in m.group(1).splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            meta[k.strip()] = _value(v)
    return meta, m.group(2)


def read_bundle(root: Path) -> list[BNote]:
    notes_dir = root / "notes" if (root / "notes").is_dir() else root
    out = []
    for p in sorted(notes_dir.rglob("*.md")):
        meta, body = parse_note(p.read_text(encoding="utf-8"))
        if "created" not in meta:
            continue
        created = dt.datetime.fromisoformat(str(meta["created"]).replace("Z", "+00:00"))
        out.append(
            BNote(
                basename=p.stem, path=str(p.relative_to(notes_dir)), title=str(meta.get("title", p.stem)),
                created=created, author=str(meta.get("author", "")), origin=str(meta.get("origin", "")),
                kind=str(meta.get("kind", "")), body=body, tags=list(meta.get("tags", [])),
                links=extract_links(body),
            )
        )
    return out
