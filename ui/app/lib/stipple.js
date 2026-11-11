// Stipple drawing for 2d charts: density is drawn as dots, never as solid fills.
// Dot positions come from a seeded hash, so redraws do not flicker.

export function hash2(a, b) {
  let x = Math.imul(a ^ 0x27d4eb2d, 0x165667b1) ^ Math.imul(b + 0x9e3779b9, 0x85ebca6b);
  x ^= x >>> 15; x = Math.imul(x, 0x2c1b3c6d); x ^= x >>> 12; x = Math.imul(x, 0x297a2d39); x ^= x >>> 15;
  return (x >>> 0) / 4294967296;
}

export function setup(canvas) {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const w = canvas.clientWidth || canvas.width, h = canvas.clientHeight || canvas.height;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) { canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr); }
  const g = canvas.getContext('2d');
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, w, h);
  return { g, w, h };
}

// Fill a rectangle with dots: density in [0,1] ≈ share of a full dot grid at `pitch` px.
export function stippleRect(g, x, y, w, h, density, color, seed = 1, pitch = 2.6, r = 0.75) {
  if (density <= 0 || w <= 0 || h <= 0) return;
  g.fillStyle = color;
  const nx = Math.max(1, Math.round(w / pitch)), ny = Math.max(1, Math.round(h / pitch));
  for (let j = 0; j < ny; j++) for (let i = 0; i < nx; i++) {
    const k = hash2(seed * 131 + i, j * 977 + seed);
    if (k > density) continue;
    const jx = hash2(i + 7, j + seed * 3) - 0.5, jy = hash2(j + 11, i + seed * 5) - 0.5;
    g.fillRect(x + (i + 0.5 + jx * 0.8) * (w / nx) - r / 2, y + (j + 0.5 + jy * 0.8) * (h / ny) - r / 2, r, r);
  }
}

// A brush-like hairline through points, with slight wobble.
export function hairline(g, pts, color, width = 0.8, seed = 1) {
  if (pts.length < 2) return;
  g.strokeStyle = color; g.lineWidth = width; g.lineCap = 'round'; g.lineJoin = 'round';
  g.beginPath();
  pts.forEach(([x, y], k) => {
    const wob = (hash2(k, seed) - 0.5) * 0.6;
    if (k === 0) g.moveTo(x, y + wob);
    else {
      const [px, py] = pts[k - 1];
      g.quadraticCurveTo(px, py, (px + x) / 2, (py + y) / 2 + wob);
    }
  });
  const last = pts[pts.length - 1];
  g.lineTo(last[0], last[1]);
  g.stroke();
}

// Stacked streams: series [{values:[n per bucket], color}] -> stipple bands.
// Returns hit(x, y) -> { s, k } (series index, bucket index) or null.
export function stackedStreams(canvas, series, { pad = [6, 4, 14, 4], dim = null, lit = null } = {}) {
  const { g, w, h } = setup(canvas);
  const m = series[0]?.values.length || 0;
  if (!m) return () => null;
  const tot = new Float32Array(m);
  for (const s of series) s.values.forEach((v, k) => { tot[k] += v; });
  const max = Math.max(1, ...tot);
  const X = (k) => pad[3] + (k / Math.max(1, m - 1)) * (w - pad[1] - pad[3]);
  const H = h - pad[0] - pad[2];
  const base = new Float32Array(m);
  const bands = [];
  // smooth a little for a stream look
  series.forEach((s, si) => {
    const lo = Array.from(base), hi = s.values.map((v, k) => base[k] + v);
    hi.forEach((v, k) => { base[k] = v; });
    bands.push({ lo, hi });
    const bw = (w - pad[1] - pad[3]) / Math.max(1, m - 1);
    const faded = dim && dim(si);
    for (let k = 0; k < m; k++) {
      const v = s.values[k]; if (!v) continue;
      const y0 = pad[0] + H * (1 - hi[k] / max), y1 = pad[0] + H * (1 - lo[k] / max);
      const isLit = lit && lit(si, k);
      stippleRect(g, X(k) - bw / 2, y0, bw, y1 - y0, isLit ? 0.85 : faded ? 0.12 : 0.42, s.color, si * 97 + k + 1, 2.4, isLit ? 1.1 : 0.8);
    }
    hairline(g, hi.map((v, k) => [X(k), pad[0] + H * (1 - v / max)]), s.color, 0.55, si + 3);
  });
  return (x, y) => {
    const k = Math.round(((x - pad[3]) / (w - pad[1] - pad[3])) * (m - 1));
    if (k < 0 || k >= m) return null;
    const v = max * (1 - (y - pad[0]) / H);
    for (let si = 0; si < bands.length; si++) if (v >= bands[si].lo[k] && v <= bands[si].hi[k]) return { s: si, k };
    return { s: -1, k };
  };
}

// Calendar heatmap as stipple density. days: [{date, count}] (sparse).
export function calendar(canvas, days, { color, soft, lit = null, weeks = null }) {
  const { g, w, h } = setup(canvas);
  if (!days.length) return () => null;
  const map = new Map(days.map((d) => [d.date, d.count]));
  const last = new Date(days[days.length - 1].date + 'T00:00:00Z');
  const nW = weeks || Math.min(160, Math.max(10, Math.floor((w - 20) / 7.5)));
  const cell = Math.min((w - 20) / nW, (h - 14) / 7);
  const end = new Date(last); end.setUTCDate(end.getUTCDate() + (6 - end.getUTCDay()));
  const start = new Date(end); start.setUTCDate(start.getUTCDate() - nW * 7 + 1);
  const max = Math.max(1, ...days.map((d) => d.count));
  const cells = [];
  g.fillStyle = soft; g.font = '9px ' + getComputedStyle(canvas).fontFamily;
  let lastMonth = -1;
  for (let k = 0; k < nW * 7; k++) {
    const d = new Date(start); d.setUTCDate(start.getUTCDate() + k);
    const col = Math.floor(k / 7), row = d.getUTCDay();
    const iso = d.toISOString().slice(0, 10);
    const n = map.get(iso) || 0;
    const x = 18 + col * cell, y = 12 + row * cell;
    if (row === 0 && d.getUTCMonth() !== lastMonth) { lastMonth = d.getUTCMonth(); if (d.getUTCDate() <= 7) { g.fillStyle = soft; g.fillText(['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'][lastMonth], x, 8); } }
    // an empty day is one faint dot; a full day is dense stipple
    const isLit = lit && lit(iso);
    if (n) stippleRect(g, x + 0.5, y + 0.5, cell - 1.2, cell - 1.2, isLit ? 1 : 0.15 + 0.85 * Math.sqrt(n / max), isLit ? lit.color : color, k + 1, 1.6, 0.85);
    else { g.fillStyle = soft; g.globalAlpha = 0.35; g.fillRect(x + cell / 2 - 0.4, y + cell / 2 - 0.4, 0.8, 0.8); g.globalAlpha = 1; }
    cells.push({ x, y, iso, n });
  }
  g.fillStyle = soft;
  ['s', 'm', 't', 'w', 't', 'f', 's'].forEach((l, r) => { if (r % 2) g.fillText(l, 4, 12 + r * cell + cell * 0.8); });
  return (x, y) => cells.find((c) => x >= c.x && x < c.x + cell && y >= c.y && y < c.y + cell) || null;
}

// Grouped stipple columns: series [{values, color}] side by side per bucket.
export function columns(canvas, series, { pad = [6, 4, 14, 4], lit = null, stacked = false } = {}) {
  const { g, w, h } = setup(canvas);
  const m = series[0]?.values.length || 0;
  if (!m) return () => null;
  const max = Math.max(1, ...(stacked ? Array.from({ length: m }, (_, k) => series.reduce((a, s) => a + s.values[k], 0)) : series.flatMap((s) => s.values)));
  const bw = (w - pad[1] - pad[3]) / m;
  const H = h - pad[0] - pad[2];
  const S = series.length;
  for (let k = 0; k < m; k++) {
    let acc = 0;
    series.forEach((s, si) => {
      const v = s.values[k]; if (!v) return;
      const hh = H * (v / max);
      const x = stacked ? pad[3] + k * bw + bw * 0.12 : pad[3] + k * bw + (si / S) * bw * 0.9;
      const ww = stacked ? bw * 0.76 : (bw * 0.9) / S;
      const y = pad[0] + H - hh - acc;
      if (stacked) acc += hh;
      const isLit = lit && lit(si, k);
      stippleRect(g, x, y, ww, hh, isLit ? 0.95 : 0.5, s.color, si * 1000 + k + 1, 1.9, isLit ? 1.1 : 0.85);
    });
  }
  return (x, y) => {
    const k = Math.floor((x - pad[3]) / bw);
    if (k < 0 || k >= m) return null;
    if (stacked) {
      let acc = pad[0] + H;
      for (let si = 0; si < S; si++) { const hh = H * (series[si].values[k] / max); if (y <= acc && y >= acc - hh) return { s: si, k }; acc -= hh; }
      return { s: -1, k };
    }
    const si = Math.floor(((x - pad[3] - k * bw) / (bw * 0.9)) * S);
    return { s: Math.min(S - 1, Math.max(0, si)), k };
  };
}

// Month axis ticks along the bottom.
export function monthAxis(canvas, months, color, pad = [6, 4, 14, 4]) {
  const g = canvas.getContext('2d');
  const w = canvas.clientWidth, h = canvas.clientHeight;
  g.fillStyle = color; g.font = '9px ' + getComputedStyle(canvas).fontFamily;
  const m = months.length;
  months.forEach((ym, k) => {
    if (!ym.endsWith('-01') && k !== 0) return;
    const x = pad[3] + (k / Math.max(1, m - 1)) * (w - pad[1] - pad[3]);
    g.fillText(ym.slice(0, 4), Math.min(w - 24, x), h - 2);
  });
}
