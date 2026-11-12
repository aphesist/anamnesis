"""Make the page's static icons from the closed Φ, the way aphes.ist makes its own (site/tools/make_icons.py on the
aphesis repo's `site` branch).

  python3 site/tools/make_icons.py

Writes into site/:
  favicon.ico           16, 32 and 48: the Φ in red on a transparent ground (browsers without SVG icons, bookmarks)
  apple-touch-icon.png  180 x 180: the Φ in red on the dark ground (n-975), with a margin
  icon-512.png          512 x 512: the same, for the manifest
  site.webmanifest      the name and the two colours, from the tokens

The Φ is vendor/aphesis/phi-closed.d: the path the seal engine (the aphesis repo's `mark` branch, engine/model.js and
engine/core.js) draws for the closed Φ in the corner, saved once so this tool needs no engine. Its polygons are
filled with the nonzero rule at 8x and brought down, so the edges are soft at every size. Red is rubedo in the dark
theme, which reads on light and dark grounds. The tab icon itself is the eye (favicon.svg and favicon-closed.svg,
made by vendor/aphesis/favicon.js), swapped by main.js as the page is seen or left, as aphes.ist does.
Needs numpy and Pillow.
"""
import json, math, os, re
import numpy as np
from PIL import Image

D = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
V = os.path.join(D, 'vendor', 'aphesis')


def oklch_hex(s):
    L, C, h = map(float, re.findall(r'[\d.]+', s)[:3])
    a, b = C * math.cos(math.radians(h)), C * math.sin(math.radians(h))
    l, m, s_ = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3, (L - 0.1055613458 * a - 0.0638541728 * b) ** 3, (L - 0.0894841775 * a - 1.2914855480 * b) ** 3
    rgb = (4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s_, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s_,
           -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s_)
    enc = lambda c: 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055
    return '#' + ''.join('%02x' % round(max(0, min(1, enc(c))) * 255) for c in rgb)


def color(name, theme, tok):
    v = {t['name']: t['value'] for t in tok['color']['tokens']}[name][theme]
    m = re.match(r'\{([\w-]+)\}', v)
    return color(m.group(1), theme, tok) if m else oklch_hex(v)


def phi_polygons():
    d = open(os.path.join(V, 'phi-closed.d')).read()
    polys, cur, mode = [], None, 'x'
    for tok in d.replace('M', ' M ').replace('L', ' L ').replace('Z', ' Z ').split():
        if tok == 'M':
            cur = []; polys.append(cur); mode = 'x'
        elif tok in ('L', 'Z'):
            mode = 'x'
        elif mode == 'x':
            x = float(tok); mode = 'y'
        else:
            cur.append((x, float(tok))); mode = 'x'
    return [p for p in polys if len(p) > 2]


def fill_nonzero(polys, S):
    """fill polygons with the nonzero rule (as the browser fills the seal's path), on an S x S grid"""
    rows = [[] for _ in range(S)]
    for p in polys:
        P = np.array(p + [p[0]], dtype=np.float64)
        for (x0, y0), (x1, y1) in zip(P[:-1], P[1:]):
            if y0 == y1:
                continue
            w = 1 if y1 > y0 else -1
            lo, hi = (y0, y1) if y0 < y1 else (y1, y0)
            ys = np.arange(max(0, int(np.ceil(lo - 0.5))), min(S, int(np.ceil(hi - 0.5))))
            if not len(ys):
                continue
            xs = x0 + (ys + 0.5 - y0) * (x1 - x0) / (y1 - y0)
            for y, x in zip(ys, xs):
                rows[y].append((x, w))
    m = np.zeros((S, S), dtype=np.uint8)
    for y, r in enumerate(rows):
        if not r:
            continue
        r.sort(); wind = 0
        for (xa, wa), (xb, _) in zip(r, r[1:] + [(S, 0)]):
            wind += wa
            if wind:
                m[y, max(0, int(round(xa))):max(0, min(S, int(round(xb))))] = 255
    return Image.fromarray(m, 'L')


def mask(polys, size, margin, ss=8):
    xs = [x for p in polys for x, _ in p]; ys = [y for p in polys for _, y in p]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    S = size * ss; box = S * (1 - 2 * margin); k = box / max(x1 - x0, y1 - y0)
    ox, oy = (S - (x1 - x0) * k) / 2 - x0 * k, (S - (y1 - y0) * k) / 2 - y0 * k
    return fill_nonzero([[(ox + x * k, oy + y * k) for x, y in p] for p in polys], S).resize((size, size), Image.LANCZOS)


def rgb(h):
    return tuple(int(h[i:i + 2], 16) for i in (1, 3, 5))


def main():
    tok = json.load(open(os.path.join(V, 'tokens.json')))
    red, ground = rgb(color('rubedo', 'dark', tok)), rgb(color('n-975', 'dark', tok))
    polys = phi_polygons()

    def tile(size, margin, bg):
        m = mask(polys, size, margin)
        im = Image.new('RGBA', (size, size), bg + (255,) if bg else (0, 0, 0, 0))
        im.paste(Image.new('RGBA', (size, size), red + (255,)), (0, 0), m)
        return im

    tile(180, 0.17, ground).convert('RGB').save(os.path.join(D, 'apple-touch-icon.png'), optimize=True)
    tile(512, 0.17, ground).convert('RGB').save(os.path.join(D, 'icon-512.png'), optimize=True)
    ico = [tile(s, 0.04, None) for s in (16, 32, 48)]
    ico[2].save(os.path.join(D, 'favicon.ico'), sizes=[(16, 16), (32, 32), (48, 48)], append_images=ico[:2])
    manifest = {
        'name': 'anamnesis', 'short_name': 'anamnesis', 'start_url': '/', 'display': 'standalone',
        'background_color': color('n-975', 'dark', tok), 'theme_color': color('n-975', 'dark', tok),
        'icons': [{'src': '/icon-512.png', 'sizes': '512x512', 'type': 'image/png'},
                  {'src': '/apple-touch-icon.png', 'sizes': '180x180', 'type': 'image/png'}],
    }
    open(os.path.join(D, 'site.webmanifest'), 'w').write(json.dumps(manifest, indent=1) + '\n')
    print('icons written to', D)


if __name__ == '__main__':
    main()
