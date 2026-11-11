"""Curate a compact demo brain from a full demo build.

The full transcendentalist build is ~9,200 notes and ~40 MB of Markdown: too much to keep
in the repository or inside the binary. ``curate`` picks a deterministic subset that still
shows everything the app does:

- the person hub notes (targets of the name links);
- every journal entry that carries a cross-author ``[[wikilink]]``, and the entries those
  links point at (both authors writing about the same day);
- dated journal entries spread across the whole timeline: every year of each author,
  evenly through the year, preferring clean, mid-length entries;
- a handful of essay sections per work (``origin: external``), so "when you wrote" shows
  two voices, and the work index notes;
- ``COLOPHON.md`` and ``SOURCES.md`` at the root, outside ``notes/``.

Links whose target is not selected are dropped (the alias text stays, a list line holding
only a dropped link goes). The same input and parameters always give the same output, byte
for byte: there is no randomness and no timestamp.
"""

from __future__ import annotations

import json
import math
import re
import shutil
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

from ..gutenberg import assert_clean
from ..links import extract_links

_LINK = re.compile(r"\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]")
_LIST_LINK = re.compile(r"^\s*-\s*\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|[^\]]*)?\]\]\s*$")
_PART = re.compile(r", part (\d+)$")
_CLEAN_CHARS = re.compile(r"[A-Za-z\s.,;:!?'’‘“”\"()—–-]")


@dataclass
class Parsed:
    rel: str  # path relative to the notes folder, with forward slashes
    basename: str
    meta: dict
    head: str  # frontmatter block, verbatim (with its --- fences)
    body: str
    links: list[str] = field(default_factory=list)

    @property
    def kind(self) -> str:
        return str(self.meta.get("kind", ""))

    @property
    def author(self) -> str:
        return str(self.meta.get("author", ""))

    @property
    def created(self) -> str:
        return str(self.meta.get("created", ""))

    @property
    def year(self) -> int:
        return int(self.created[:4]) if self.created[:4].isdigit() else 0

    @property
    def size(self) -> int:
        return len((self.head + self.body).encode("utf-8"))

    @property
    def work(self) -> str:
        for t in self.meta.get("tags", []):
            if isinstance(t, str) and t.startswith("work/"):
                return t[5:]
        return ""


def _value(raw: str):
    raw = raw.strip()
    if raw.startswith(("\"", "[")):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return raw
    return raw


def parse_note(path: Path, root: Path) -> Parsed:
    text = path.read_text(encoding="utf-8")
    meta: dict = {}
    head, body = "", text
    if text.startswith("---\n"):
        end = text.find("\n---\n", 4)
        if end > 0:
            head = text[: end + 5]
            body = text[end + 5 :]
            for line in text[4:end].split("\n"):
                k, sep, v = line.partition(":")
                if sep:
                    meta[k.strip()] = _value(v)
    return Parsed(
        rel=path.relative_to(root).as_posix(), basename=path.stem, meta=meta, head=head, body=body,
        links=extract_links(body),
    )


def quality(n: Parsed) -> float:
    """Share of 'clean prose' characters in the body and title (ocr noise scores low)."""
    text = n.body.strip()
    if not text:
        return 0.0
    body_q = len(_CLEAN_CHARS.findall(text)) / len(text)
    title = str(n.meta.get("title", ""))
    words = re.findall(r"[A-Za-z]{3,}", title)
    title_q = 1.0 if len(words) >= 2 else 0.85
    return body_q * title_q


def _even(cands: list[Parsed], k: int) -> list[Parsed]:
    """``k`` items spread evenly through a (date-ordered) list."""
    if k >= len(cands):
        return list(cands)
    return [cands[int((i + 0.5) * len(cands) / k)] for i in range(k)]


def strip_dangling(body: str, keep: set[str]) -> str:
    lines = [ln for ln in body.split("\n") if not ((m := _LIST_LINK.match(ln)) and m.group(1).strip() not in keep)]
    body = "\n".join(lines)

    def sub(m: re.Match[str]) -> str:
        if m.group(1).strip() in keep:
            return m.group(0)
        return m.group(2) if m.group(2) is not None else m.group(1)

    return _LINK.sub(sub, body)


def select(
    notes: list[Parsed],
    *,
    target_notes: int = 1100,
    max_bytes: int = 3_900_000,
    essays_per_work: int = 6,
    min_per_year: int = 2,
    min_chars: int = 400,
) -> list[Parsed]:
    """The curated subset of ``notes`` (deterministic), in the input's path order."""
    notes = sorted(notes, key=lambda n: n.rel)
    by_name = {n.basename: n for n in notes}
    chosen: dict[str, Parsed] = {}
    used = 0

    def take(n: Parsed, *, force: bool = False) -> bool:
        nonlocal used
        if n.basename in chosen:
            return True
        if not force and (used + n.size > max_bytes or len(chosen) >= target_notes):
            return False
        chosen[n.basename] = n
        used += n.size
        return True

    # 1. person hubs
    for n in notes:
        if n.kind == "hub":
            take(n, force=True)

    # 2. cross-author linked journal entries and the entries they point at, shortest first
    #    so a few huge ocr entries cannot crowd the rest out of the budget
    linked = [n for n in notes if n.kind == "journal" and n.links and quality(n) >= 0.9]
    linked.sort(key=lambda n: (n.size, n.rel))
    link_budget = max_bytes * 45 // 100
    for n in linked:
        targets = [by_name[t] for t in n.links if t in by_name and by_name[t].kind == "journal"]
        extra = sum(t.size for t in targets if t.basename not in chosen)
        if used + n.size + extra > link_budget:
            break
        take(n)
        for t in targets:
            take(t)

    # 3. essay sections: a few per work, spread through it, then the work's index note
    works: dict[str, list[Parsed]] = defaultdict(list)
    for n in notes:
        if n.kind == "essay" and n.work and n.size <= 7000 and quality(n) >= 0.9:
            works[n.work].append(n)
    for work in sorted(works):
        secs = works[work]
        firsts = [s for s in secs if not _PART.search(s.basename)] or secs
        for s in _even(firsts, essays_per_work):
            take(s)
    for n in notes:
        if n.kind == "index" and any(t in chosen for t in n.links):
            take(n, force=True)

    # 4. dated journal entries spread over every (author, year), evenly through the year
    pool: dict[tuple[str, int], list[Parsed]] = defaultdict(list)
    for n in notes:
        if n.kind != "journal" or n.basename in chosen:
            continue
        if n.meta.get("origin") != "owner_written":
            continue
        if not (min_chars <= len(n.body) <= 6000) or quality(n) < 0.93:
            continue
        pool[(n.author, n.year)].append(n)
    for k in pool:
        pool[k].sort(key=lambda n: (n.created, n.meta.get("date_precision") != "day", n.rel))
    room = max(0, target_notes - len(chosen))
    weight = {k: math.sqrt(len(v)) for k, v in pool.items()}
    total_w = sum(weight.values()) or 1.0
    quota = {k: min(len(pool[k]), max(min_per_year, round(room * weight[k] / total_w))) for k in pool}
    # trim the largest quotas until the plan fits the note target
    while sum(quota.values()) > room and any(q > min_per_year for q in quota.values()):
        k = max(quota, key=lambda x: (quota[x], x))
        quota[k] -= 1
    # take round by round (every bucket's first pick, then every second…) so that, if the
    # byte budget runs out, every year still has its entries
    picks = {k: _even(pool[k], quota[k]) for k in sorted(pool)}
    for r in range(max((len(v) for v in picks.values()), default=0)):
        for k in sorted(picks):
            if r < len(picks[k]):
                take(picks[k][r])

    return [n for n in notes if n.basename in chosen]


def _counts(notes: list[Parsed], bodies: dict[str, str]) -> dict:
    names = {n.basename for n in notes}
    created = {n.basename: n.created for n in notes}
    links = later = 0
    for n in notes:
        for t in extract_links(bodies[n.basename]):
            if t in names:
                links += 1
                later += created[t] < n.created
    return {
        "notes": len(notes),
        "by_author": dict(sorted(Counter(n.author for n in notes).items())),
        "by_kind": dict(sorted(Counter(n.kind for n in notes).items())),
        "by_origin": dict(sorted(Counter(str(n.meta.get("origin")) for n in notes).items())),
        "by_decade": dict(sorted(Counter(f"{n.year // 10 * 10}s" for n in notes).items())),
        "years_with_journal": {
            a: sorted({n.year for n in notes if n.kind == "journal" and n.author == a})
            for a in sorted({n.author for n in notes if n.kind == "journal"})
        },
        "links": links,
        "links_to_earlier_notes": later,
        "words": sum(len(bodies[n.basename].split()) for n in notes),
    }


def curate(
    src: Path, out: Path, *, target_notes: int = 1100, max_bytes: int = 3_900_000, min_chars: int = 400
) -> dict:
    """Curate ``src`` (a full bundle folder, or its ``notes/``) into ``out``; return the manifest."""
    root = src if (src / "notes").is_dir() else src.parent
    notes_dir = root / "notes" if (root / "notes").is_dir() else src
    files = sorted(notes_dir.rglob("*.md"))
    if not files:
        raise FileNotFoundError(f"no notes under {notes_dir}")
    notes = [parse_note(p, notes_dir) for p in files]
    picked = select(notes, target_notes=target_notes, max_bytes=max_bytes, min_chars=min_chars)
    keep = {n.basename for n in picked}

    out_notes = out / "notes"
    if out_notes.exists():
        shutil.rmtree(out_notes)
    bodies: dict[str, str] = {}
    for n in picked:
        body = strip_dangling(n.body, keep)
        bodies[n.basename] = body
        text = n.head + body
        dest = out_notes / n.rel
        assert_clean(text, str(dest))
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(text, encoding="utf-8", newline="\n")

    source_manifest = {}
    if (root / "manifest.json").is_file():
        source_manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    counts = _counts(picked, bodies)
    counts["bytes"] = sum(len((n.head + bodies[n.basename]).encode("utf-8")) for n in picked)
    manifest = {
        "theme": source_manifest.get("theme", root.name),
        "owner": source_manifest.get("owner", ""),
        "curated": True,
        "notes_dir": "notes",
        "curation": {
            "command": f"anamnesis-lab demo curate --target-notes {target_notes}",
            "target_notes": target_notes,
            "max_bytes": max_bytes,
            "min_chars": min_chars,
            "from_notes": len(notes),
            "from_built_at": source_manifest.get("built_at"),
        },
        "counts": counts,
        "date_range": [min(n.created for n in picked)[:10], max(n.created for n in picked)[:10]],
        "sources": source_manifest.get("sources", []),
        "licence": source_manifest.get(
            "licence", "texts: public domain in the United States and Brazil (see SOURCES.md)"
        ),
        "note": "links between authors are explicit references found by a heuristic; the demo's "
        "'links made later' is a demonstration, not a measurement",
    }
    out.mkdir(parents=True, exist_ok=True)
    colophon = (root / "COLOPHON.md").read_text(encoding="utf-8") if (root / "COLOPHON.md").is_file() else ""
    colophon = colophon.rstrip("\n") + (
        "\n\n## this curated set\n\n"
        f"{len(picked)} of the {len(notes)} notes of a full build, chosen by `anamnesis-lab demo curate`: "
        "the person notes, every journal entry with a link between the two authors (and the entries it "
        "points at), entries spread over every year of both journals, and a few sections of each essay. "
        "links to notes outside the set were removed. the full build is made with "
        "`anamnesis-lab demo transcendentalist` (see docs/demo.md).\n"
    )
    (out / "COLOPHON.md").write_text(colophon, encoding="utf-8", newline="\n")
    if (root / "SOURCES.md").is_file():
        shutil.copyfile(root / "SOURCES.md", out / "SOURCES.md")
    (out / "manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n"
    )
    return manifest
