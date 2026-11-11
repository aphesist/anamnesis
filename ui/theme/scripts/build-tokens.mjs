#!/usr/bin/env node
// Compiles the design tokens to css: tokens/base.tokens.json + themes/<id>/{tokens,light,dark}.tokens.json
// -> dist/<id>.css (light and dark via [data-theme]) and dist/<id>.json (resolved values, hex and linear rgb
// for the webgl renderer), plus dist/index.json (the theme list) and the placeholder marks in marks/.
//
// usage: node ui/theme/scripts/build-tokens.mjs           write everything
//        node ui/theme/scripts/build-tokens.mjs --check   exit 1 if the committed output is stale
// No dependencies; node 18+.

import { readFileSync, writeFileSync, readdirSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { compileTheme } from '../tokens.js';
import { phiPlaceholder, toSvg } from '../brush.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = join(root, 'dist');
const check = process.argv.includes('--check');
const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));
const rel = (from, to) => relative(from, to).split(sep).join('/');

const outputs = new Map(); // path -> content

const base = readJson(join(root, 'tokens', 'base.tokens.json'));
const themesDir = join(root, 'themes');
const index = { generated: 'scripts/build-tokens.mjs', themes: [] };

for (const id of readdirSync(themesDir).sort()) {
  const dir = join(themesDir, id);
  const manifestPath = join(dir, 'theme.json');
  if (!existsSync(manifestPath)) continue;
  const m = readJson(manifestPath);
  if (m.id !== id) throw new Error(`themes/${id}/theme.json: id "${m.id}" must match its folder`);
  const load = (f) => (f && existsSync(join(dir, f)) ? readJson(join(dir, f)) : undefined);
  const theme = {
    id,
    name: m.name,
    defaultMode: m.defaultMode,
    base,
    common: load(m.tokens),
    modes: { light: load(m.modes?.light), dark: load(m.modes?.dark) },
    fontFaces: (m.fonts ?? []).map((f) => ({ ...f, src: rel(dist, resolve(dir, f.src)) })),
    marks: {
      mark: m.marks?.mark?.fallback ? rel(dist, resolve(dir, m.marks.mark.fallback)) : undefined,
      wordmark: m.marks?.wordmark?.fallback ? rel(dist, resolve(dir, m.marks.wordmark.fallback)) : undefined,
    },
  };
  const { css, json } = compileTheme(theme);
  json.description = m.description;
  json.behaviours = m.behaviours ?? {};
  outputs.set(join(dist, `${id}.css`), css);
  outputs.set(join(dist, `${id}.json`), JSON.stringify(json, null, 2) + '\n');
  index.themes.push({
    id,
    name: m.name,
    description: m.description,
    defaultMode: theme.defaultMode === 'light' ? 'light' : 'dark',
    css: `${id}.css`,
    values: `${id}.json`,
    manifest: `../themes/${id}/theme.json`,
    behaviours: m.behaviours ?? {},
    // swatches for the picker, so it needs no extra request
    preview: Object.fromEntries(
      ['light', 'dark'].map((mode) => [
        mode,
        Object.fromEntries(['--bg', '--ink', '--accent', '--dot'].map((k) => [k, json.modes[mode][k]?.hex])),
      ]),
    ),
  });
}
outputs.set(join(dist, 'index.json'), JSON.stringify(index, null, 2) + '\n');

// placeholder marks, drawn by brush.js (deterministic)
outputs.set(
  join(root, 'marks', 'phi-placeholder.svg'),
  toSvg(phiPlaceholder(), { viewBox: '0 0 100 140', fill: '#c8262e', label: 'φ' }).replace(' style="overflow:visible"', '') + '\n',
);
outputs.set(
  join(root, 'marks', 'wordmark-placeholder.svg'),
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 360 64" role="img" aria-label="anamnesis">` +
    `<text x="0" y="46" font-family="Georgia, 'Times New Roman', serif" font-style="italic" font-size="48" letter-spacing="1" fill="#c8262e">anamnesis</text></svg>\n`,
);

let stale = 0;
for (const [p, content] of outputs) {
  const now = existsSync(p) ? readFileSync(p, 'utf8') : null;
  if (now === content) continue;
  if (check) {
    console.error(`stale: ${rel(root, p)}`);
    stale++;
  } else {
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, content);
    console.log(`wrote ${rel(root, p)}`);
  }
}
if (check && stale) {
  console.error(`${stale} generated file(s) out of date: run node ui/theme/scripts/build-tokens.mjs`);
  process.exit(1);
}
if (check) console.log('generated theme files are up to date');
