"""Demo-brain builders. Each theme turns public-domain texts into an importable bundle."""

from __future__ import annotations

from pathlib import Path

from .base import Theme, build


def themes() -> dict[str, Theme]:
    from . import pragmatist, transcendentalist

    return {t.id: t for t in (transcendentalist.THEME, pragmatist.THEME)}


def build_theme(name: str, out: Path, cache_dir: Path, *, small: bool = False) -> dict:
    """Build theme ``name`` into ``out``; downloads go to ``cache_dir`` (not used with ``small``)."""
    return build(themes()[name], out, cache_dir, small=small)


__all__ = ["Theme", "build", "build_theme", "themes"]
