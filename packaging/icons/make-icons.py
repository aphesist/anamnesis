#!/usr/bin/env python3
"""Render every icon of the app from assets/icon/anamnesis.svg.

- crates/anamnesis-desktop/icons/: 32x32.png, 128x128.png, 128x128@2x.png, icon.png
  (512), icon.ico (16-256) and icon.icns (16-1024), for the tauri bundles and the window;
- packaging/windows/anamnesis.ico (16-256), for the mingw-built installers;
- ui/app/favicon.svg: the same drawing in the theme's accent, darker or lighter with the
  browser's colour scheme.

needs rsvg-convert (librsvg2-bin) and pillow. run from anywhere:
    python3 packaging/icons/make-icons.py
"""

import io
import re
import subprocess
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "assets/icon/anamnesis.svg"
DESKTOP = ROOT / "crates/anamnesis-desktop/icons"
WIN_ICO = ROOT / "packaging/windows/anamnesis.ico"
FAVICON = ROOT / "ui/app/favicon.svg"
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
# the drawing fills about 80% of its square already; keep it as drawn


def render(size: int) -> Image.Image:
    png = subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size), str(SRC)],
        check=True,
        capture_output=True,
    ).stdout
    return Image.open(io.BytesIO(png)).convert("RGBA")


def main() -> int:
    if not SRC.exists():
        print(f"missing {SRC}", file=sys.stderr)
        return 1
    imgs = {s: render(s) for s in sorted(set(ICO_SIZES + [512, 1024]))}
    DESKTOP.mkdir(parents=True, exist_ok=True)
    imgs[32].save(DESKTOP / "32x32.png")
    imgs[128].save(DESKTOP / "128x128.png")
    imgs[256].save(DESKTOP / "128x128@2x.png")
    imgs[512].save(DESKTOP / "icon.png")
    ico = [imgs[s] for s in ICO_SIZES]
    for path in (DESKTOP / "icon.ico", WIN_ICO):
        ico[-1].save(path, format="ICO", sizes=[(s, s) for s in ICO_SIZES], append_images=ico[:-1])
    imgs[1024].save(DESKTOP / "icon.icns", format="ICNS")

    svg = SRC.read_text()
    svg = re.sub(r'fill="#ab0d19"', 'fill="currentColor"', svg)
    style = (
        "<style>svg{color:#ab0d19}"
        "@media (prefers-color-scheme:dark){svg{color:#d43031}}</style>"
    )
    svg = re.sub(r"(<svg[^>]*>)", r"\1" + style, svg, count=1)
    FAVICON.write_text(svg)
    for p in sorted(DESKTOP.iterdir()):
        print(p.relative_to(ROOT))
    print(WIN_ICO.relative_to(ROOT))
    print(FAVICON.relative_to(ROOT))
    return 0


if __name__ == "__main__":
    sys.exit(main())
