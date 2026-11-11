// Run with: node --test ui/app/tests/
import test from 'node:test';
import assert from 'node:assert/strict';
import { normalise, anatomy, groupsFor, spheres, split, shell } from '../brain/layouts.js';

function cloud(n, seed = 1) {
  let s = seed;
  const r = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  const xyz = new Float32Array(n * 3);
  for (let i = 0; i < n * 3; i++) xyz[i] = (r() - 0.5) * 4 + 7; // off-centre on purpose
  return xyz;
}

test('normalise centres the cloud and scales it to about unit radius', () => {
  const n = 2000, out = normalise(cloud(n), n);
  const c = [0, 0, 0];
  for (let i = 0; i < n; i++) for (let d = 0; d < 3; d++) c[d] += out[i * 3 + d] / n;
  for (const x of c) assert.ok(Math.abs(x) < 1e-3);
  const radii = Array.from({ length: n }, (_, i) => Math.hypot(out[i * 3], out[i * 3 + 1], out[i * 3 + 2])).sort((a, b) => a - b);
  assert.ok(Math.abs(radii[Math.floor(n * 0.92)] - 1) < 0.01);
});

test('anatomy keeps every point finite, inside the silhouette, and preserves radial order', () => {
  const n = 3000, norm = normalise(cloud(n, 7), n), a = anatomy(norm, n);
  assert.equal(a.length, n * 3);
  let maxR = 0;
  for (let i = 0; i < n * 3; i++) assert.ok(Number.isFinite(a[i]));
  for (let i = 0; i < n; i++) maxR = Math.max(maxR, Math.hypot(a[i * 3], a[i * 3 + 1], a[i * 3 + 2]));
  assert.ok(maxR < 1.6, `max radius ${maxR}`);
  // innermost input points stay nearer the core than outermost ones, on average
  const r0 = Array.from({ length: n }, (_, i) => Math.hypot(norm[i * 3], norm[i * 3 + 1], norm[i * 3 + 2]));
  const order = r0.map((r, i) => [r, i]).sort((x, y) => x[0] - y[0]).map(([, i]) => i);
  const mean = (idx) => idx.reduce((s, i) => s + Math.hypot(a[i * 3], a[i * 3 + 1], a[i * 3 + 2]), 0) / idx.length;
  assert.ok(mean(order.slice(0, 300)) < mean(order.slice(-300)));
});

test('groupsFor sorts groups by size and honours the filter', () => {
  const keys = ['a', 'b', 'b', 'c', 'b', 'a', 'x'];
  const g = groupsFor(keys.length, (i) => keys[i], (i) => keys[i] === 'x');
  assert.deepEqual(g.map(([k]) => k), ['b', 'a', 'c']);
  assert.deepEqual(g[0][1], [1, 2, 4]);
});

test('spheres keeps each group inside its own, non-overlapping sphere', () => {
  const n = 1200;
  const keyOf = (i) => (i % 5 === 0 ? 'big' : i % 3 === 0 ? 'mid' : i % 7 === 0 ? 'small' : 'rest');
  const groups = groupsFor(n, keyOf);
  const { pos, centres } = spheres(n, groups, (i) => i);
  const info = groups.map(([key, idx]) => {
    const c = [0, 1, 2].map((d) => idx.reduce((s, i) => s + pos[i * 3 + d], 0) / idx.length);
    const r = Math.max(...idx.map((i) => Math.hypot(pos[i * 3] - c[0], pos[i * 3 + 1] - c[1], pos[i * 3 + 2] - c[2])));
    return { key, c, r };
  });
  for (let a = 0; a < info.length; a++) for (let b = a + 1; b < info.length; b++) {
    const d = Math.hypot(info[a].c[0] - info[b].c[0], info[a].c[1] - info[b].c[1]);
    assert.ok(d > (info[a].r + info[b].r) * 0.9, `${info[a].key} overlaps ${info[b].key}`);
  }
  assert.equal(centres.length, groups.length);
});

test('split lays groups out on a grid and keeps them apart', () => {
  const n = 900, base = anatomy(normalise(cloud(n, 3), n), n);
  const groups = groupsFor(n, (i) => String(i % 6));
  const { pos, centres } = split(n, groups, base);
  assert.equal(centres.length, 6);
  const xs = new Set(centres.map((c) => c.pos[0].toFixed(3)));
  assert.ok(xs.size > 1);
  for (let i = 0; i < n * 3; i++) assert.ok(Number.isFinite(pos[i]));
});

test('the ornamental shell has the requested number of finite points', () => {
  const s = shell(500);
  assert.equal(s.length, 1500);
  for (const x of s) assert.ok(Number.isFinite(x));
});
