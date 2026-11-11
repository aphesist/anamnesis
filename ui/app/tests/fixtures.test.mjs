// The mock fixtures must keep the exact http api shapes from docs/contracts.md.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const load = (n) => JSON.parse(readFileSync(new URL(`../mock/${n}`, import.meta.url), 'utf8'));

test('cloud.json is columnar and consistent', () => {
  const c = load('cloud.json');
  const n = c.ids.length;
  assert.ok(n >= 2000);
  assert.equal(c.xyz.length, n * 3);
  for (const k of ['region', 'kind', 'speaker', 't', 'titles']) assert.equal(c[k].length, n, k);
  assert.ok(c.regions.length >= 8 && c.regions.length <= 12);
  for (const r of c.regions) {
    for (const k of ['id', 'label', 'anatomy', 'size', 'centroid', 'born_at']) assert.ok(k in r, k);
    assert.equal(r.label, r.label.toLowerCase());
  }
  assert.ok(c.region.every((r) => r >= -1 && r < c.regions.length));
  assert.ok(c.kind.every((k) => k >= 0 && k < c.kinds.length));
  assert.ok(c.speaker.every((s) => s === 0 || s === 1 || s === 2));
  assert.ok(c.links.every(([i, j, kind]) => i < n && j < n && typeof kind === 'string'));
  assert.ok(new Set(c.ids).size === n);
});

test('timeline.json series line up with its months', () => {
  const t = load('timeline.json');
  const m = t.months.length;
  for (const r of t.by_region) assert.equal(r.counts.length, m);
  for (const k of ['you', 'assistants', 'other']) assert.equal(t.by_speaker[k].length, m);
  for (const k of ['decision', 'open_question', 'plan', 'rejection']) assert.equal(t.threads_by_month[k].length, m);
  assert.ok(t.days.every((d) => /^\d{4}-\d{2}-\d{2}$/.test(d.date) && d.count > 0));
});

test('lineage.json has a split and a merge', () => {
  const types = load('lineage.json').runs.flatMap((r) => r.transitions.map((x) => x.type));
  assert.ok(types.includes('split'));
  assert.ok(types.includes('merge'));
  assert.ok(types.includes('birth'));
});

test('backends.json includes a grey claude cli entry with a notice', () => {
  const b = load('backends.json');
  const cli = b.backends.find((x) => x.id === 'claude-cli');
  assert.equal(cli.terms, 'grey');
  assert.ok(cli.notice);
  assert.ok(b.routing.chat);
});

test('eval.json has the four metrics with n, estimate, interval, status and basis', () => {
  const e = load('eval.json');
  for (const k of ['links', 'retrieval', 'resurfacing', 'growth']) {
    const m = e[k];
    assert.ok(Number.isFinite(m.n) && Number.isFinite(m.estimate) && m.interval.length === 2, k);
    assert.ok(['measuring', 'enough data'].includes(m.status));
    assert.ok(['measured', 'modelled', 'hypothesis'].includes(m.basis));
  }
});

test('threads.json uses the model enums', () => {
  for (const t of load('threads.json').threads) {
    assert.ok(['decision', 'open_question', 'plan', 'rejection'].includes(t.kind));
    assert.ok(['open', 'resolved', 'abandoned', 'superseded'].includes(t.status));
  }
});
