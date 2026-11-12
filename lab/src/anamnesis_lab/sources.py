"""Source records with copyright reasoning, and a cached downloader.

The bar for a demo brain is "public domain in both the United States and Brazil":

- United States: a work published before 1931 is public domain as of 2026
  (95-year term for 1926–1930 works has run out; earlier works expired sooner).
- Brazil: Lei 9.610/98, art. 41 — rights last seventy years counted from 1 January
  of the year after the author's death. So a person who died in year *d* is public
  domain from 1 January of *d* + 71. Editors and translators are authors of their own
  contributions, so every person on the edition is checked, not just the author.
"""

from __future__ import annotations

import datetime as dt
import gzip
import hashlib
import io
import os
import time
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

USER_AGENT = "anamnesis-lab/0.1 (demo-brain builder)"


@dataclass(frozen=True)
class Person:
    name: str
    role: str  # "author" | "editor" | "translator"
    born: int | None
    died: int
    basis: str  # where the dates come from

    def brazil_pd_from(self) -> int:
        return self.died + 71


@dataclass
class Source:
    id: str
    title: str
    author: str
    persons: list[Person]
    published: int  # year of the edition used
    url: str  # where the text is downloaded from
    page_url: str  # human-facing page for the colophon
    fmt: str  # "pg" (clean e-text with wrapper) | "ocr" (archive.org djvu text) | "fixture"
    kind: str  # "journal" | "letters" | "essays" | "book"
    edition: str
    host: str  # "Project Gutenberg" | "Internet Archive" | "bundled fixture"
    parse: dict = field(default_factory=dict)
    notes: str = ""
    # A scan of the same edition, used as each note's ``source_url``. Notes must not
    # carry a distributor's trademark, so e-text pages are only named in the colophon.
    cite_url: str = ""

    @property
    def note_url(self) -> str:
        return self.cite_url or self.page_url

    def us_status(self) -> str:
        if self.fmt == "fixture":
            return "original fixture text written for the lab (Apache-2.0)"
        if self.published <= 1930:
            return f"public domain (published {self.published}, before 1931)"
        return f"NOT CLEAR: published {self.published}"

    def brazil_status(self, today: dt.date | None = None) -> str:
        if self.fmt == "fixture":
            return "original fixture text written for the lab (Apache-2.0)"
        today = today or dt.date.today()
        last = max(p.died for p in self.persons)
        pd_from = last + 71
        if pd_from <= today.year:
            who = max(self.persons, key=lambda p: p.died)
            return f"public domain since {pd_from}-01-01 (last contributor {who.name} died {who.died}; Lei 9.610/98 art. 41)"
        return f"NOT CLEAR until {pd_from}-01-01"

    def is_clear(self, today: dt.date | None = None) -> bool:
        return not self.us_status().startswith("NOT") and not self.brazil_status(today).startswith("NOT")

    def licence_record(self) -> dict:
        return {
            "id": self.id,
            "title": self.title,
            "author": self.author,
            "edition": self.edition,
            "published": self.published,
            "host": self.host,
            "page_url": self.page_url,
            "cite_url": self.note_url,
            "persons": [
                {"name": p.name, "role": p.role, "born": p.born, "died": p.died, "basis": p.basis}
                for p in self.persons
            ],
            "us": self.us_status(),
            "brazil": self.brazil_status(),
            "notes": self.notes,
        }


def fetch(url: str, cache_dir: Path, name: str | None = None, retries: int = 3) -> str:
    """Download ``url`` once into ``cache_dir`` and return it as text (utf-8)."""
    cache_dir.mkdir(parents=True, exist_ok=True)
    name = name or hashlib.sha1(url.encode()).hexdigest()[:16]
    path = cache_dir / name
    if path.exists() and path.stat().st_size > 0:
        return path.read_text(encoding="utf-8", errors="replace")
    last_err: Exception | None = None
    for attempt in range(retries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept-Encoding": "gzip"})
            with urllib.request.urlopen(req, timeout=180) as resp:
                data = resp.read()
                if resp.headers.get("Content-Encoding") == "gzip":
                    data = gzip.GzipFile(fileobj=io.BytesIO(data)).read()
            tmp = path.with_suffix(".part")
            tmp.write_bytes(data)
            os.replace(tmp, path)
            return data.decode("utf-8", errors="replace")
        except Exception as e:  # network errors: retry with backoff
            last_err = e
            time.sleep(2 * (attempt + 1))
    raise RuntimeError(f"download failed for {url}: {last_err}")
