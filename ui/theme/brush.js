// anamnesis brush and stipple helpers: SVG path data for brush strokes and stipple fills.
//
// An independent implementation in the spirit of the aphes.ist grammar ("nothing is solid": every mark is
// either a brush stroke or stipple). It is used when the aphes.ist runtime (window.Aphesis) is not loaded,
// and by other themes. The call shapes follow the documented aphes.ist API (brush / arc / line / stipple /
// MJ) so either can be used; the drawing itself is our own and will not match aphes.ist stroke for stroke.
//
// Everything is deterministic for a given seed, so a mark looks the same on every visit.
// Works in node and in the browser (no DOM needed except `toSvg`'s string).

/** small, fast, seedable PRNG (mulberry32) */
export function rng(seed) {
  let a = (seed >>> 0) || 0x9e3779b9;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** minimum-jerk easing: 10t³ − 15t⁴ + 6t⁵ (starts and ends at rest) */
export function MJ(t) {
  t = Math.min(1, Math.max(0, t));
  return t * t * t * (10 - 15 * t + 6 * t * t);
}

/** samples a straight centerline */
export function line(x0, y0, x1, y1, n = 24) {
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    pts.push([x0 + (x1 - x0) * t, y0 + (y1 - y0) * t]);
  }
  return pts;
}

/** samples an arc (angles in radians; ry for an ellipse) */
export function arc(cx, cy, r, a0, a1, n = 48, ry = r) {
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const a = a0 + (a1 - a0) * (i / n);
    pts.push([cx + Math.cos(a) * r, cy + Math.sin(a) * ry]);
  }
  return pts;
}

/** samples a quadratic bezier */
export function curve(p0, p1, p2, n = 32) {
  const pts = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n, u = 1 - t;
    pts.push([u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]]);
  }
  return pts;
}

// 1d value noise along the stroke, smooth, seeded
function noise1(seed) {
  const r = rng(seed);
  const knots = Array.from({ length: 64 }, () => r() * 2 - 1);
  return (x) => {
    const i = Math.floor(x), f = x - i;
    const a = knots[((i % 64) + 64) % 64], b = knots[(((i + 1) % 64) + 64) % 64];
    const s = f * f * (3 - 2 * f);
    return a + (b - a) * s;
  };
}

const f2 = (n) => (Math.round(n * 100) / 100).toString();

/**
 * A brush stroke along `points` as a filled outline (SVG path data).
 * width: the nib's broad width. taper: share of the length that tapers at each end (0 = blunt).
 * wobble: how much the edges drift on their own (0 = clean).
 * opts.nib: nib angle in degrees (38 by default); opts.contrast: thick/thin ratio (2.8).
 */
export function brush(points, width, seed = 1, taper = 0.22, wobble = 0.08, opts = {}) {
  const n = points.length;
  if (n < 2) return '';
  const nib = ((opts.nib ?? 38) * Math.PI) / 180;
  const thin = 1 / (opts.contrast ?? 2.8);
  const nl = noise1(seed * 7 + 1), nr = noise1(seed * 13 + 5), nc = noise1(seed * 3 + 11);
  // arc length for noise sampling and tapering
  const len = [0];
  for (let i = 1; i < n; i++) len.push(len[i - 1] + Math.hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
  const total = len[n - 1] || 1;
  const freq = 6 / Math.max(width * 4, 1);
  const left = [], right = [];
  for (let i = 0; i < n; i++) {
    const p = points[i];
    const a = points[Math.max(0, i - 1)], b = points[Math.min(n - 1, i + 1)];
    const dx = b[0] - a[0], dy = b[1] - a[1];
    const dl = Math.hypot(dx, dy) || 1;
    const tx = dx / dl, ty = dy / dl;
    const theta = Math.atan2(ty, tx);
    // broad nib: thickest when the stroke runs across the nib, thinnest along it
    const nibK = thin + (1 - thin) * Math.abs(Math.sin(theta - nib));
    const t = len[i] / total;
    let k = 1;
    if (taper > 0) {
      const e = Math.min(t, 1 - t) / taper;
      k = e >= 1 ? 1 : Math.sin((Math.min(1, e) * Math.PI) / 2) ** 0.8;
    }
    const s = len[i] * freq;
    const hw = (width / 2) * nibK * k;
    const wl = hw * (1 + wobble * 2.2 * nl(s));
    const wr = hw * (1 + wobble * 2.2 * nr(s + 17));
    const drift = width * wobble * 0.6 * nc(s * 0.5) * k;
    const nx = -ty, ny = tx;
    const cx = p[0] + nx * drift, cy = p[1] + ny * drift;
    left.push([cx + nx * wl, cy + ny * wl]);
    right.push([cx - nx * wr, cy - ny * wr]);
  }
  const ring = left.concat(right.reverse());
  return smoothClosed(ring);
}

// a closed outline through the points, smoothed with quadratic segments through midpoints
function smoothClosed(pts) {
  const m = pts.length;
  const mid = (i) => {
    const a = pts[i % m], b = pts[(i + 1) % m];
    return [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  };
  const s = mid(0);
  let d = `M${f2(s[0])} ${f2(s[1])}`;
  for (let i = 1; i <= m; i++) {
    const c = pts[i % m], e = mid(i);
    d += `Q${f2(c[0])} ${f2(c[1])} ${f2(e[0])} ${f2(e[1])}`;
  }
  return d + 'Z';
}

/**
 * Stipple: square cells inside a region. `test(x, y)` says whether a point is inside.
 * count: how many tries; size: cell side in box units; box: the square drawing area (24 = icon grid),
 * or [w, h]. Cells sit on a grid of their own size and never overlap.
 */
export function stipple(test, seed = 1, count = 600, size = 0.6, box = 24) {
  const [w, h] = Array.isArray(box) ? box : [box, box];
  const r = rng(seed);
  const taken = new Set();
  let d = '';
  for (let i = 0; i < count; i++) {
    const gx = Math.floor((r() * w) / size), gy = Math.floor((r() * h) / size);
    const key = gx * 100003 + gy;
    if (taken.has(key)) continue;
    const x = gx * size, y = gy * size;
    if (!test(x + size / 2, y + size / 2)) continue;
    taken.add(key);
    d += `M${f2(x)} ${f2(y)}h${f2(size)}v${f2(size)}h${f2(-size)}z`;
  }
  return d;
}

/**
 * Stipple by density: every grid cell lights where density(x, y) (0..1) passes a fixed, dithered threshold.
 * Use for heatmap cells, region fills and fog-like grounds. Cells are square and never antialiased.
 */
export function stippleDensity(density, { width, height, cell = 2, seed = 1 } = {}) {
  let d = '';
  for (let y = 0; y < height; y += cell) {
    for (let x = 0; x < width; x += cell) {
      const v = density(x + cell / 2, y + cell / 2);
      if (v <= 0) continue;
      if (v >= threshold(x / cell, y / cell, seed)) d += `M${f2(x)} ${f2(y)}h${cell}v${cell}h${-cell}z`;
    }
  }
  return d;
}

// interleaved gradient noise: a cheap threshold with blue-ish spectrum, seeded by offset
function threshold(x, y, seed) {
  const v = 52.9829189 * ((0.06711056 * (x + seed * 5.3) + 0.00583715 * (y + seed * 2.1)) % 1);
  return v % 1;
}

/** a brushed underline across `w` units (for links, focused fields, the active tab) */
export function underline(w, seed = 3, width = 1.6, y = 2) {
  const pts = curve([0, y + 0.4], [w / 2, y - 0.8], [w, y + 0.2], 32);
  return brush(pts, width, seed, 0.3, 0.1);
}

/** a frame of four overshooting brush strokes around a w×h box (for dialogs and calm zones) */
export function frame(w, h, seed = 9, width = 2.2, over = 6) {
  const r = rng(seed);
  const j = () => (r() - 0.5) * 3;
  const strokes = [
    line(-over, j(), w + over * 0.6, j(), 40),
    line(w + j(), -over * 0.5, w + j(), h + over, 40),
    line(w + over * 0.4, h + j(), -over * 0.8, h + j(), 40),
    line(j(), h + over * 0.3, j(), -over, 40),
  ];
  // each side its own stroke: thick in the middle, tapering off past the corners, edges drifting
  return strokes.map((p, i) => brush(p, width * (0.8 + r() * 0.5), seed + i * 31, 0.3, 0.22, { contrast: 1.8 })).join('');
}

/** a ring drawn in one sweep with a gap, like a hand closing a circle */
export function ring(cx, cy, r, seed = 5, width = 1.8, ry = r) {
  const start = -Math.PI / 2 + 0.25;
  return brush(arc(cx, cy, r, start, start + Math.PI * 2 - 0.35, 72, ry), width, seed, 0.3, 0.08);
}

/** wraps path data in an svg string; fill defaults to currentColor so css can colour it */
export function toSvg(d, { viewBox = '0 0 24 24', fill = 'currentColor', width, height, label } = {}) {
  const size = (width ? ` width="${width}"` : '') + (height ? ` height="${height}"` : '');
  const a11y = label ? ` role="img" aria-label="${String(label).replace(/"/g, '')}"` : ' aria-hidden="true" focusable="false"';
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}"${size}${a11y} style="overflow:visible"><path d="${d}" fill="${fill}" fill-rule="nonzero"/></svg>`;
}

/**
 * The placeholder mark: a φ made only of stipple (a ring and a stem), drawn here so the app has a mark
 * when the owner's φ is not installed. viewBox 0 0 100 140.
 */
export function phiPlaceholder({ seed = 1618, cell = 2.2 } = {}) {
  const cx = 50, cy = 72, rx = 30, ry = 24;
  const inRing = (x, y) => {
    const e = ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2;
    return e > 0.52 && e < 1.18;
  };
  const inStem = (x, y) => {
    const lean = (y - 70) * -0.06;
    return Math.abs(x - (cx + lean)) < 3.6 && y > 14 && y < 132;
  };
  const r = rng(seed + 1);
  const density = (x, y) => {
    let v = 0;
    if (inRing(x, y)) v = 0.82;
    if (inStem(x, y)) {
      // the stem thins toward its ends, like a tapered stroke
      const t = (y - 14) / 118;
      v = Math.max(v, 0.92 * Math.sin(Math.PI * Math.min(1, Math.max(0, t))) ** 0.35);
    }
    return v ? v * (0.78 + r() * 0.3) : 0;
  };
  return stippleDensity(density, { width: 100, height: 140, cell, seed });
}
