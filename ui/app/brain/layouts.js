// Client-side layouts computed from the single xyz set the api gives.
//
// anatomy: a rank-preserving radial remap of the semantic layout into an
//   analytic brain silhouette (two hemispheres, a sagittal groove, cerebellum,
//   stem). Directions are kept, so neighbourhoods survive; only radii change.
//   Shape is ornament; nearness is meaning.
// spheres: each group packed into its own sphere, ordered from core to rim.
// split:   each group keeps its own (anatomy) shape, pulled apart into a grid.
//
// If the server ever sends `layouts: { anatomy: [..], spheres: [..] }` in
// /api/cloud, those are used instead (see brain/index.js).

const hash = (i) => { let x = Math.imul(i ^ 0x9e3779b9, 0x85ebca6b); x ^= x >>> 13; x = Math.imul(x, 0xc2b2ae35); x ^= x >>> 16; return (x >>> 0) / 4294967296; };

// Normalise: centre on the mean, scale so the 92nd percentile radius is 1.
export function normalise(xyz, n) {
  const out = new Float32Array(n * 3);
  let cx = 0, cy = 0, cz = 0;
  for (let i = 0; i < n; i++) { cx += xyz[i * 3]; cy += xyz[i * 3 + 1]; cz += xyz[i * 3 + 2]; }
  cx /= n || 1; cy /= n || 1; cz /= n || 1;
  const r = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    out[i * 3] = xyz[i * 3] - cx; out[i * 3 + 1] = xyz[i * 3 + 1] - cy; out[i * 3 + 2] = xyz[i * 3 + 2] - cz;
    r[i] = Math.hypot(out[i * 3], out[i * 3 + 1], out[i * 3 + 2]);
  }
  const sorted = Float32Array.from(r).sort();
  const s = sorted[Math.floor(n * 0.92)] || 1;
  for (let i = 0; i < n * 3; i++) out[i] /= s;
  return out;
}

// Far intersection of a ray from the origin (direction d) with an ellipsoid.
function rayEllipsoid(d, c, a) {
  // solve |(t d - c) / a|^2 = 1
  const dx = d[0] / a[0], dy = d[1] / a[1], dz = d[2] / a[2];
  const cx = c[0] / a[0], cy = c[1] / a[1], cz = c[2] / a[2];
  const A = dx * dx + dy * dy + dz * dz, B = -2 * (dx * cx + dy * cy + dz * cz), C = cx * cx + cy * cy + cz * cz - 1;
  const disc = B * B - 4 * A * C;
  if (disc < 0) return 0;
  return (-B + Math.sqrt(disc)) / (2 * A);
}

// x lateral, y up, z anterior (front). Units: roughly 1 = half the brain's length.
const LOBES = [
  { c: [0.33, 0.1, 0.03], a: [0.42, 0.62, 0.98] },   // right hemisphere
  { c: [-0.33, 0.1, 0.03], a: [0.42, 0.62, 0.98] },  // left hemisphere
  { c: [0.0, -0.22, 0.32], a: [0.62, 0.36, 0.55] },  // temporal / base mass
  { c: [0.0, -0.48, -0.62], a: [0.5, 0.25, 0.3] },   // cerebellum
  { c: [0.0, -0.62, -0.2], a: [0.13, 0.36, 0.13] },  // stem
];
function brainRadius(d) {
  let t = 0;
  for (const l of LOBES) t = Math.max(t, rayEllipsoid(d, l.c, l.a));
  // gyri: a gentle wobble on the surface
  const g = 1 + 0.03 * Math.sin(d[0] * 17 + d[1] * 11) * Math.sin(d[2] * 15 - d[1] * 9);
  return t * g;
}

export function anatomy(norm, n) {
  const out = new Float32Array(n * 3);
  const r = new Float32Array(n);
  for (let i = 0; i < n; i++) r[i] = Math.hypot(norm[i * 3], norm[i * 3 + 1], norm[i * 3 + 2]);
  const order = Array.from({ length: n }, (_, i) => i).sort((a, b) => r[a] - r[b]);
  const rank = new Float32Array(n);
  order.forEach((i, k) => { rank[i] = (k + 0.5) / n; });
  // Directions: a monotone equalisation of azimuth and elevation, blended with
  // the original, so lumpy layouts still fill the silhouette. Order along each
  // angle is kept, so neighbours stay neighbours.
  const az = new Float32Array(n), el = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const l = r[i] || 1e-6;
    az[i] = Math.atan2(norm[i * 3], norm[i * 3 + 2]);
    el[i] = r[i] ? norm[i * 3 + 1] / l : hash(i) * 2 - 1;
  }
  const eqRank = (arr) => { const o = Array.from({ length: n }, (_, i) => i).sort((a, b) => arr[a] - arr[b]); const q = new Float32Array(n); o.forEach((i, k) => { q[i] = (k + 0.5) / n; }); return q; };
  const azR = eqRank(az), elR = eqRank(el);
  const EQ = 0.55;
  const d = [0, 0, 0];
  for (let i = 0; i < n; i++) {
    const a = (1 - EQ) * az[i] + EQ * (azR[i] * 2 - 1) * Math.PI;
    const e = (1 - EQ) * el[i] + EQ * (elR[i] * 2 - 1);
    const ce = Math.sqrt(Math.max(0, 1 - e * e));
    d[0] = Math.sin(a) * ce; d[1] = e; d[2] = Math.cos(a) * ce;
    const R = brainRadius(d);
    // cortex-heavy fill: most dots sit near the surface, a thinner core inside
    const s = 0.18 + 0.82 * Math.pow(rank[i], 0.2);
    let x = d[0] * R * s, y = d[1] * R * s, z = d[2] * R * s;
    // sagittal groove: open a gap between the hemispheres, deeper at the top
    const groove = 0.045 * Math.max(0.2, Math.min(1, (y + 0.3) * 1.6));
    x += Math.sign(x || (hash(i) - 0.5)) * groove;
    // a touch of jitter so the shell is not a perfect surface
    const j = 0.018;
    x += (hash(i * 3 + 1) - 0.5) * j; y += (hash(i * 3 + 2) - 0.5) * j; z += (hash(i * 3 + 3) - 0.5) * j;
    out[i * 3] = x * 1.25; out[i * 3 + 1] = y * 1.25; out[i * 3 + 2] = z * 1.25;
  }
  return out;
}

// Groups: Map key -> array of point indices, in the desired order.
export function groupsFor(n, keyOf, filterOut) {
  const g = new Map();
  for (let i = 0; i < n; i++) {
    if (filterOut && filterOut(i)) continue;
    const k = keyOf(i);
    if (!g.has(k)) g.set(k, []);
    g.get(k).push(i);
  }
  return [...g.entries()].sort((a, b) => b[1].length - a[1].length);
}

// Pack circles of given radii in the xy-plane along a golden spiral.
function packCircles(radii, gap) {
  const placed = [];
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let k = 0; k < radii.length; k++) {
    const r = radii[k];
    if (!placed.length) { placed.push([0, 0, r]); continue; }
    for (let s = 1; s < 4000; s++) {
      const rr = 0.05 * Math.sqrt(s) * 1.6, a = s * golden;
      const x = Math.cos(a) * rr * 1.5, y = Math.sin(a) * rr; // a little wider than tall
      if (placed.every(([px, py, pr]) => Math.hypot(px - x, py - y) > pr + r + gap)) { placed.push([x, y, r]); break; }
    }
    if (placed.length <= k) placed.push([k, 0, r]);
  }
  return placed;
}

// Returns { pos: Float32Array(n*3), centres: [{key, pos:[x,y,z], count}] }.
// Points that are in no group stay at their fallback position (collapsed to the origin).
export function spheres(n, groups, orderKey) {
  const pos = new Float32Array(n * 3);
  const radii = groups.map(([, idx]) => 0.075 * Math.cbrt(idx.length));
  const pack = packCircles(radii, 0.07);
  // fit into a box about 3.2 wide
  let ext = 0;
  pack.forEach(([x, y, r]) => { ext = Math.max(ext, Math.abs(x) * 0.67 + r, Math.abs(y) + r); });
  const s = 1.15 / (ext || 1);
  const golden = Math.PI * (3 - Math.sqrt(5));
  const centres = [];
  groups.forEach(([key, idx], g) => {
    const [cx, cy, R0] = pack[g];
    const R = R0 * s;
    const sorted = orderKey ? [...idx].sort((a, b) => orderKey(a) - orderKey(b)) : idx;
    const m = sorted.length;
    sorted.forEach((i, k) => {
      const u = (k + 0.5) / m;
      const rad = R * Math.cbrt(u);
      const yy = 1 - 2 * ((k * 0.618034 + hash(i) * 0.1) % 1);
      const rr = Math.sqrt(Math.max(0, 1 - yy * yy));
      const a = k * golden;
      pos[i * 3] = cx * s + Math.cos(a) * rr * rad;
      pos[i * 3 + 1] = cy * s + yy * rad;
      pos[i * 3 + 2] = Math.sin(a) * rr * rad;
    });
    centres.push({ key, pos: [cx * s, cy * s + R + 0.06, 0], count: m });
  });
  return { pos, centres };
}

// Split: keep each group's own shape (from `base`), pulled apart into a grid.
export function split(n, groups, base) {
  const pos = new Float32Array(n * 3);
  const G = groups.length;
  const cols = Math.ceil(Math.sqrt(G * 1.6));
  const rows = Math.ceil(G / cols);
  const cell = 3.4 / Math.max(cols, rows * 1.4);
  const centres = [];
  groups.forEach(([key, idx], g) => {
    let cx = 0, cy = 0, cz = 0;
    for (const i of idx) { cx += base[i * 3]; cy += base[i * 3 + 1]; cz += base[i * 3 + 2]; }
    cx /= idx.length; cy /= idx.length; cz /= idx.length;
    let ext = 0;
    for (const i of idx) ext = Math.max(ext, Math.hypot(base[i * 3] - cx, base[i * 3 + 1] - cy, base[i * 3 + 2] - cz));
    const k = (cell * 0.42) / (ext || 1);
    const col = g % cols, row = Math.floor(g / cols);
    const ox = (col - (cols - 1) / 2) * cell, oy = ((rows - 1) / 2 - row) * cell * 0.95;
    for (const i of idx) {
      pos[i * 3] = ox + (base[i * 3] - cx) * k;
      pos[i * 3 + 1] = oy + (base[i * 3 + 1] - cy) * k;
      pos[i * 3 + 2] = (base[i * 3 + 2] - cz) * k;
    }
    centres.push({ key, pos: [ox, oy + cell * 0.46, 0], count: idx.length });
  });
  return { pos, centres };
}

// Ornamental shell: faint dots on the brain's surface, not data. Drawn only in
// the anatomy form, so the silhouette reads even when a brain is small.
export function shell(count = 7000) {
  const out = new Float32Array(count * 3);
  const golden = Math.PI * (3 - Math.sqrt(5));
  const d = [0, 0, 0];
  for (let k = 0; k < count; k++) {
    const y = 1 - 2 * ((k + 0.5) / count);
    const rr = Math.sqrt(1 - y * y), a = k * golden;
    d[0] = Math.cos(a) * rr; d[1] = y; d[2] = Math.sin(a) * rr;
    const R = brainRadius(d) * (0.97 + 0.05 * hash(k * 31 + 5));
    let x = d[0] * R;
    x += Math.sign(x || 1) * 0.045 * Math.max(0.2, Math.min(1, (d[1] * R + 0.3) * 1.6));
    out[k * 3] = x * 1.25; out[k * 3 + 1] = d[1] * R * 1.25; out[k * 3 + 2] = d[2] * R * 1.25;
  }
  return out;
}

export { hash };
