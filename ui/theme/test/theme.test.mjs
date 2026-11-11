// run: node --test ui/theme/test/*.test.mjs
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { flattenTokens, resolveTokens, compileVars, compileTheme, parseColor, srgbToOklch, oklchToSrgb, toCss, overridesToCss } from '../tokens.js';
import { brush, stipple, arc, line, MJ, phiPlaceholder, frame } from '../brush.js';
import { normalizeLayout, movePanel, updatePanel, panelsIn } from '../layout.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const json = (p) => JSON.parse(readFileSync(join(root, p), 'utf8'));

const CONTRACT_VARS = [
  '--bg', '--ink', '--ink-soft', '--line', '--fog', '--accent', '--accent-text', '--seal',
  '--dot', '--dot-dim', '--dot-hot', '--thread', '--region-label',
  '--font-display', '--font-text', '--font-data', '--type-scale',
  '--ease', '--dur-short', '--dur-mid', '--dur-long', '--motion',
  ...Array.from({ length: 9 }, (_, i) => `--space-${i + 1}`),
  '--mark', '--wordmark',
];

test('aliases resolve, chained, and cycles are reported', () => {
  const doc = {
    color: { $type: 'color', a: { $value: { colorSpace: 'oklch', components: [0.5, 0.1, 30] } }, b: { $value: '{color.a}' } },
    role: { $type: 'color', $extensions: { 'ist.anamnesis': { flatten: true } }, accent: { $value: '{color.b}' } },
  };
  const r = resolveTokens(flattenTokens(doc));
  assert.deepEqual(r.get('role.accent').value.components, [0.5, 0.1, 30]);
  assert.equal(r.get('role.accent').cssName, '--accent');
  assert.throws(() => resolveTokens(flattenTokens({ x: { $type: 'number', a: { $value: '{x.b}' }, b: { $value: '{x.a}' } } })), /cycle/);
  assert.throws(() => resolveTokens(flattenTokens({ x: { $type: 'number', a: { $value: '{x.nope}' } } })), /unknown/);
});

test('emit:false groups are aliases only', () => {
  const { vars } = compileVars({
    color: { $type: 'color', $extensions: { 'ist.anamnesis': { emit: false } }, red: { $value: '#ff0000' } },
    accent: { $type: 'color', $value: '{color.red}' },
  });
  assert.deepEqual(Object.keys(vars), ['--accent']);
  assert.equal(vars['--accent'], '#ff0000');
});

test('value types map to css', () => {
  assert.equal(toCss('dimension', { value: 4, unit: 'px' }), '4px');
  assert.equal(toCss('duration', { value: 350, unit: 'ms' }), '350ms');
  assert.equal(toCss('cubicBezier', [0.2, 0.7, 0.2, 1]), 'cubic-bezier(0.2, 0.7, 0.2, 1)');
  assert.equal(toCss('fontFamily', ['Special Elite', 'serif']), '"Special Elite", serif');
  assert.equal(toCss('color', { colorSpace: 'oklch', components: [0.57, 0.2, 26], alpha: 0.5 }), 'oklch(0.57 0.2 26 / 0.5)');
});

test('colour maths round-trips and matches known points', () => {
  assert.equal(parseColor('oklch(1 0 0)').hex, '#ffffff');
  assert.equal(parseColor('oklch(0 0 0)').hex, '#000000');
  assert.equal(parseColor('#c8262e').hex, '#c8262e');
  assert.equal(parseColor('rgb(255 0 0)').hex, '#ff0000');
  // pure srgb red is about oklch(0.628 0.258 29.2)
  const [L, C, H] = srgbToOklch(1, 0, 0);
  assert.ok(Math.abs(L - 0.628) < 0.002 && Math.abs(C - 0.2577) < 0.002 && Math.abs(H - 29.23) < 0.1, `${L} ${C} ${H}`);
  const back = oklchToSrgb(L, C, H).srgb;
  assert.ok(back.every((v, i) => Math.abs(v - [1, 0, 0][i]) < 1e-3));
});

test('every theme defines every contract variable in both modes', () => {
  const base = json('tokens/base.tokens.json');
  for (const id of readdirSync(join(root, 'themes'))) {
    const m = json(`themes/${id}/theme.json`);
    const load = (f) => (f ? json(`themes/${id}/${f}`) : undefined);
    const { css, json: out } = compileTheme({
      id, base, defaultMode: m.defaultMode, common: load(m.tokens),
      modes: { light: load(m.modes.light), dark: load(m.modes.dark) },
      marks: { mark: 'x.svg', wordmark: 'y.svg' },
    });
    for (const v of CONTRACT_VARS) {
      if (v === '--mark' || v === '--wordmark') {
        assert.ok(css.includes(`${v}: url(`), `${id} ${v}`);
        continue;
      }
      for (const mode of ['light', 'dark']) assert.ok(out.values[mode][v] != null, `${id}/${mode} lacks ${v}`);
    }
    assert.match(css, /:root\[data-theme="light"\]/);
    assert.match(css, /:root\[data-theme="dark"\]/);
    assert.match(css, /:root\[data-motion="reduced"\]/);
  }
});

test('role text pairs keep readable contrast in every theme', () => {
  // WCAG relative luminance from the resolved linear rgb
  const lum = (lin) => 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
  const ratio = (a, b) => {
    const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
    return (x + 0.05) / (y + 0.05);
  };
  for (const f of readdirSync(join(root, 'dist')).filter((f) => f.endsWith('.json') && f !== 'index.json')) {
    const t = json(`dist/${f}`);
    for (const mode of ['light', 'dark']) {
      const m = t.modes[mode];
      const bg = m['--bg'].linear;
      assert.ok(ratio(m['--ink'].linear, bg) >= 7, `${t.id}/${mode} ink`);
      assert.ok(ratio(m['--ink-soft'].linear, bg) >= 4.5, `${t.id}/${mode} ink-soft ${ratio(m['--ink-soft'].linear, bg).toFixed(2)}`);
      assert.ok(ratio(m['--accent-text'].linear, bg) >= 4.5, `${t.id}/${mode} accent-text ${ratio(m['--accent-text'].linear, bg).toFixed(2)}`);
    }
  }
});

test('committed dist is up to date with the tokens', () => {
  execFileSync(process.execPath, [join(root, 'scripts', 'build-tokens.mjs'), '--check'], { stdio: 'pipe' });
});

test('overrides outweigh the theme and strip unsafe characters', () => {
  const css = overridesToCss({ all: { accent: 'red; } body { x: y' }, dark: { '--bg': '#000' } });
  assert.match(css, /html:root:is\(\[data-theme\], :not\(\[data-theme\]\)\) \{\n  --accent: red  body  x: y;/);
  assert.match(css, /html:root\[data-theme="dark"\] \{\n  --bg: #000;/);
});

test('brush and stipple are deterministic and well formed', () => {
  const a = brush(arc(12, 12, 8, 0, Math.PI), 1.8, 7);
  assert.equal(a, brush(arc(12, 12, 8, 0, Math.PI), 1.8, 7));
  assert.notEqual(a, brush(arc(12, 12, 8, 0, Math.PI), 1.8, 8));
  assert.match(a, /^M[\d.\- ]+(Q[\d.\- ]+)+Z$/);
  assert.ok(!/NaN|Infinity/.test(a + frame(200, 100) + brush(line(0, 0, 0, 0, 4), 2, 1)));
  const s = stipple((x, y) => (x - 12) ** 2 + (y - 12) ** 2 < 64, 3, 800, 0.6);
  assert.ok(s.split('M').length > 50);
  assert.equal(s, stipple((x, y) => (x - 12) ** 2 + (y - 12) ** 2 < 64, 3, 800, 0.6));
  assert.equal(MJ(0), 0);
  assert.equal(MJ(1), 1);
  assert.ok(Math.abs(MJ(0.5) - 0.5) < 1e-9);
  assert.ok(phiPlaceholder().length > 1000);
});

test('layout presets are valid and complete', () => {
  const required = ['sidebar-left', 'view-right', 'legend-bottom-left', 'when-you-wrote', 'timeline-bottom', 'chat', 'review', 'threads', 'eval', 'skills'];
  for (const f of ['default.json', 'minimal.json']) {
    const l = normalizeLayout(json(`layouts/${f}`));
    assert.deepEqual(l.panels.map((p) => p.id).sort(), [...required].sort(), f);
  }
  const d = normalizeLayout(json('layouts/default.json'));
  assert.equal(d.panels.find((p) => p.id === 'legend-bottom-left').anchor, 'bottom-left');
  assert.throws(() => normalizeLayout({ panels: [{ id: 'a' }, { id: 'a' }] }), /duplicate/);
});

test('layout edits: move, change region, hide', () => {
  let l = normalizeLayout(json('layouts/default.json'));
  const right = panelsIn(l, 'right', true).map((p) => p.id);
  l = movePanel(l, right[1], -1);
  assert.deepEqual(panelsIn(l, 'right', true).map((p) => p.id).slice(0, 2), [right[1], right[0]]);
  l = updatePanel(l, 'chat', { region: 'left' });
  assert.equal(panelsIn(l, 'left').at(-1).id, 'chat');
  assert.equal(l.panels.find((p) => p.id === 'chat').anchor, undefined);
  l = updatePanel(l, 'review', { visible: false });
  assert.ok(!panelsIn(l, 'right').some((p) => p.id === 'review'));
  assert.ok(panelsIn(l, 'right', true).some((p) => p.id === 'review'));
});
