"""Heuristic ``[[wikilinks]]`` where one author explicitly names the other or their works.

Only explicit references become links: a name ("Henry Thoreau", "Mr. Emerson") or a
distinctive title ("Self-Reliance", "the Week"). Similarity is never used here, so
these links can serve as ground truth for "links made later" in the demo, which the
app labels as a demonstration, not a measurement.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass, field


@dataclass(frozen=True)
class LinkRule:
    pattern: str
    target: str  # basename of the target note
    # Apply only in notes written by one of these authors.
    authors: frozenset[str] = field(default_factory=frozenset)

    def regex(self) -> re.Pattern[str]:
        return re.compile(self.pattern)


_EXISTING = re.compile(r"\[\[[^\]]*\]\]")


def apply_links(
    body: str,
    author: str,
    rules: list[LinkRule],
    targets: set[str],
    resolve: Callable[[str], str] | None = None,
) -> tuple[str, list[str]]:
    """Wrap the first match of each applicable rule in ``[[target|text]]``.

    Rules whose target is not in ``targets`` (the bundle's basenames) are skipped,
    and text already inside a wikilink is never touched. ``resolve`` may redirect a
    rule's target for this note (e.g. a person's name → that person's entry of the
    same day). Returns the new body and the list of link targets in order of appearance.
    """
    found: list[str] = []
    for rule in rules:
        if rule.authors and author not in rule.authors:
            continue
        target = resolve(rule.target) if resolve else rule.target
        if target not in targets or target in found:
            continue
        protected = [(m.start(), m.end()) for m in _EXISTING.finditer(body)]
        for m in rule.regex().finditer(body):
            if any(a <= m.start() < b for a, b in protected):
                continue
            text = m.group(0)
            link = f"[[{target}]]" if text == target else f"[[{target}|{text}]]"
            body = body[: m.start()] + link + body[m.end():]
            found.append(target)
            break
    return body, found


def extract_links(body: str) -> list[str]:
    """Targets of ``[[target]]`` and ``[[target|alias]]`` links in Markdown text."""
    return [m.group(1).strip() for m in re.finditer(r"\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|[^\]]*)?\]\]", body)]
