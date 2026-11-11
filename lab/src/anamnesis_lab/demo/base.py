"""The generic demo-brain pipeline shared by every theme.

source text → strip wrapper → paragraphs → dated entries / sections → notes with
frontmatter → explicit cross-author wikilinks → bundle on disk.
"""

from __future__ import annotations

import datetime as dt
import logging
import re
from collections import defaultdict
from dataclasses import dataclass, field
from importlib import resources
from pathlib import Path

from ..bundle import Basenames, Note, first_sentence, slug, write_bundle
from ..gutenberg import strip_gutenberg
from ..journal import (
    Entry,
    JournalOptions,
    SectionOptions,
    looks_like_quotation,
    split_journal,
    split_long,
    split_sections,
)
from ..links import LinkRule, apply_links
from ..sources import Source, fetch
from ..textclean import Para, ocr_paragraphs, pg_paragraphs, slice_text

log = logging.getLogger("anamnesis_lab.demo")

_EDITORIAL = re.compile(r"^\s*(?:\[|\((?:From|Continued)\b)")


@dataclass
class Hub:
    """A short note about a person, written for the lab; target of name links."""

    title: str
    author: str  # whose hub this is (used to set its date to their first entry)
    body: str
    tags: list[str] = field(default_factory=list)


@dataclass
class Theme:
    id: str
    title: str
    owner_label: str
    sources: list[Source]
    small_sources: list[Source]
    link_rules: list[LinkRule]
    hubs: list[Hub]
    author_tags: dict[str, str]
    partial: bool = False
    partial_note: str = ""

    def sources_for(self, small: bool) -> list[Source]:
        return self.small_sources if small else self.sources


def fixture_text(theme_id: str, name: str) -> str:
    return (resources.files("anamnesis_lab") / "fixtures" / theme_id / name).read_text(encoding="utf-8")


def load_entries(src: Source, cache_dir: Path, theme_id: str) -> list[Entry]:
    """Download (or read the fixture), clean and split one source into entries."""
    p = dict(src.parse)
    if src.fmt == "fixture":
        raw = fixture_text(theme_id, src.url)
        wrapper = p.pop("wrapper", "pg")
    else:
        raw = fetch(src.url, cache_dir, name=f"{src.id}.txt")
        wrapper = src.fmt
    start = p.pop("start", None)
    start_nth = p.pop("start_nth", 1)
    stop = p.pop("stop", None)
    drop = [tuple(x) for x in p.pop("drop", [])]
    year_min = p.pop("year_min", 1600)
    year_max = p.pop("year_max", 1930)
    date_after = p.pop("date_after", None)
    max_chars = p.pop("max_chars", 4000)

    if wrapper == "pg":
        text = slice_text(strip_gutenberg(raw), start, stop, start_nth, drop)
        tokens = pg_paragraphs(text)
    elif wrapper == "ocr":
        text = slice_text(raw.replace("\r\n", "\n"), start, stop, start_nth, drop)
        tokens = ocr_paragraphs(text, year_min, year_max)
    else:
        raise ValueError(f"unknown wrapper {wrapper!r}")

    # Editors' bracketed notes and notebook labels are not the author's words.
    tokens = [t for t in tokens if not (isinstance(t, Para) and _EDITORIAL.match(t.text))]

    if src.kind in ("journal", "letters"):
        kind = "letter" if src.kind == "letters" else "journal"
        opt = JournalOptions(author=src.author, source_id=src.id, year_min=year_min, year_max=year_max, kind=kind, **p)
        entries = split_journal(tokens, opt)
        if date_after:
            cutoff = dt.date.fromisoformat(date_after)
            entries = [e for e in entries if e.date > cutoff]
        for e in entries:
            if looks_like_quotation(e):
                e.origin = "external"
                e.tags.append("quotation")
        entries = [e for e in entries if len(e.body) >= 25]
    else:
        work = p.pop("work", src.title)
        p.pop("sections", None)
        date = dt.date(p.pop("year", src.published), 1, 1)
        opt = SectionOptions(author=src.author, source_id=src.id, work=work, date=date, kind="essay", **p)
        entries = split_sections(tokens, opt)
        for e in entries:
            e.tags.append(f"work/{slug(work)}")
    return split_long(entries, max_chars=max_chars)


def build(theme: Theme, out: Path, cache_dir: Path, *, small: bool = False) -> dict:
    """Build the theme into ``out`` and return the manifest."""
    sources = theme.sources_for(small)
    for s in sources:
        if not s.is_clear():
            raise RuntimeError(f"{s.id}: not clear in both countries: us={s.us_status()} brazil={s.brazil_status()}")

    names = Basenames()
    notes: list[Note] = []
    per_source: dict[str, list[Entry]] = {}
    for s in sources:
        log.info("loading %s", s.id)
        per_source[s.id] = load_entries(s, cache_dir, theme.id)
        log.info("  %d entries", len(per_source[s.id]))

    # Reserve names in priority order: hubs, then work index notes, then essays, then entries.
    hub_names = {h.title: names.reserve(h.title) for h in theme.hubs}
    # A work split into several sections gets an index note named after it; a single
    # essay (Walking) is itself the note named after the work.
    work_index: dict[str, str] = {}
    for s in sources:
        if s.kind in ("essays", "book") and len({e.title for e in per_source[s.id]}) > 1:
            work_index[s.id] = names.reserve(s.parse.get("work", s.title))

    src_by_id = {s.id: s for s in sources}
    section_names: dict[str, list[str]] = defaultdict(list)
    for s in sources:
        for e in per_source[s.id]:
            note = _entry_note(e, s, theme, names)
            notes.append(note)
            if s.kind in ("essays", "book"):
                section_names[s.id].append(note.basename)

    # Work index notes (generated by the lab; external origin).
    for sid, base in work_index.items():
        s = src_by_id[sid]
        items = "\n".join(f"- [[{n}]]" for n in section_names[sid])
        body = (
            f"_{s.parse.get('work', s.title)}_ by {s.author} ({s.parse.get('year', s.published)}).\n\n"
            f"sections in this brain:\n\n{items}\n"
        )
        notes.append(
            Note(
                basename=base, title=s.parse.get("work", s.title), created=dt.date(s.parse.get("year", s.published), 1, 1),
                author=s.author, origin="external", kind="index", body=body,
                tags=[theme.author_tags.get(s.author, "other"), "index"], source_url=s.note_url,
                source_title=s.edition, precision="year",
            )
        )

    # Hub notes: dated to their person's first entry so links to them are "later".
    first_by_author: dict[str, dt.date] = {}
    for n in notes:
        if n.kind in ("journal", "letter"):
            d = first_by_author.get(n.author)
            first_by_author[n.author] = min(d, n.created) if d else n.created
    for h in theme.hubs:
        created = first_by_author.get(h.author) or min(n.created for n in notes)
        notes.append(
            Note(
                basename=hub_names[h.title], title=h.title, created=created, author="anamnesis lab",
                origin="external", kind="hub", body=h.body, tags=["hub", *h.tags], precision="year",
            )
        )

    # Hub notes list the person's works; unlink the ones this build does not contain.
    targets = {n.basename for n in notes}
    for n in notes:
        if n.kind == "hub":
            n.body = re.sub(r"\[\[([^\]|]+)\]\]", lambda m: m.group(0) if m.group(1) in targets else f"_{m.group(1)}_", n.body)

    # Explicit cross-author references → wikilinks. A person's name in a dated entry
    # points at that person's own entry of the same day when there is one (both wrote
    # about the same walk or visit); otherwise at the person's hub note.
    targets = {n.basename for n in notes}
    hub_author = {hub_names[h.title]: h.author for h in theme.hubs}
    same_day: dict[tuple[str, dt.date], str] = {}
    for n in notes:
        if n.kind in ("journal", "letter") and n.precision == "day":
            same_day.setdefault((n.author, n.created), n.basename)
    for n in notes:
        if n.kind in ("index", "hub"):
            continue

        def resolve(target: str, n: Note = n) -> str:
            person = hub_author.get(target)
            if person and n.precision == "day":
                return same_day.get((person, n.created), target)
            return target

        n.body, n.links = apply_links(n.body, n.author, theme.link_rules, targets, resolve)

    notes.sort(key=lambda n: (n.created, n.author, n.basename))
    manifest = write_bundle(
        out,
        notes,
        theme=theme.id,
        owner_label=theme.owner_label,
        sources=[s.licence_record() for s in sources],
        colophon=colophon(theme, sources),
        sources_md=sources_markdown(theme, sources),
        small=small,
        extra={"partial": theme.partial_note} if theme.partial else None,
    )
    return manifest


def _entry_note(e: Entry, s: Source, theme: Theme, names: Basenames) -> Note:
    author_tag = theme.author_tags.get(e.author, "other")
    if e.kind in ("journal", "letter"):
        # A headed entry keeps its heading across parts; an unheaded part is named by its own text.
        title = f"{e.title}, part {e.part}" if (e.title and e.part > 1) else (e.title or first_sentence(e.body))
        when = e.date.isoformat() if e.precision == "day" else str(e.date.year)
        base = names.reserve(f"{when} {author_tag} {title}")
        tags = [author_tag, e.kind, f"{e.date.year // 10 * 10}s", *e.tags]
        display = f"{title}"
    else:
        work = s.parse.get("work", s.title)
        title = e.title or first_sentence(e.body)
        if s.parse.get("sections") == "chapters":
            title = f"{work} · {title}"
        if e.part > 1:
            title = f"{title}, part {e.part}"
        base = names.reserve(title, alt=f"{title} ({work})")
        tags = [author_tag, "essay", *e.tags]
        display = title
    return Note(
        basename=base, title=display, created=e.date, author=e.author, origin=e.origin, kind=e.kind,
        body=e.body, tags=tags, source_url=s.note_url, source_title=s.edition, precision=e.precision,
    )


def colophon(theme: Theme, sources: list[Source]) -> str:
    hosts = sorted({s.host for s in sources})
    lines = [
        f"# colophon — {theme.title}",
        "",
        f"a demo brain for anamnesis. the demo owner is \"{theme.owner_label}\": journal entries and letters are",
        "marked `owner_written`, essays and commonplace-book quotations `external`, so the speaker charts have variety.",
        "",
        "the texts are public domain in the united states and in brazil (see SOURCES.md for each edition,",
        "its contributors and their death years). they were downloaded, stripped of every distributor's licence",
        "header, footer and trademark, split into dated entries, and written as markdown notes by the anamnesis lab.",
        "",
        "texts courtesy of: " + "; ".join(hosts) + ".",
        "thanks to the volunteers who transcribed and scanned them. any errors of splitting, dating or ocr are the lab's.",
        "",
        "the `[[links]]` between authors mark places where one names the other or one of the other's works.",
        "they were found by a simple heuristic. \"links made later\" in this demo is a demonstration, not a measurement.",
    ]
    if theme.partial:
        lines += ["", f"this theme is partial: {theme.partial_note}"]
    return "\n".join(lines) + "\n"


def sources_markdown(theme: Theme, sources: list[Source]) -> str:
    out = [f"# SOURCES — {theme.title}", ""]
    for s in sources:
        out.append(f"## {s.title}")
        out.append("")
        out.append(f"- edition: {s.edition}")
        out.append(f"- text from: {s.host} — {s.page_url}")
        for p in s.persons:
            born = f"{p.born}–" if p.born else "†"
            out.append(f"- {p.role}: {p.name} ({born}{p.died}); dates from {p.basis}")
        out.append(f"- united states: {s.us_status()}")
        out.append(f"- brazil: {s.brazil_status()}")
        if s.notes:
            out.append(f"- notes: {s.notes}")
        out.append("")
    return "\n".join(out)
