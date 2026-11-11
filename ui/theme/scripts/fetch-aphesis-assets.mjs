#!/usr/bin/env node
// Downloads the aphes.ist theme's own assets (Aphesis Script and the φ mark) from the owner's public
// repository into ui/theme/themes/aphesis/assets/. They are NOT committed to anamnesis: see
// themes/aphesis/assets/NOTICE.md. Without them the theme falls back to Special Elite / serif and a
// stipple placeholder mark, so this script never fails an install: network or http errors only warn.
//
// usage: node ui/theme/scripts/fetch-aphesis-assets.mjs [--force] [--dry-run] [--quiet]
//   ANAMNESIS_SKIP_ASSET_FETCH=1   skip entirely (offline installs, packagers)
//   APHESIS_ASSET_BASE=<url>       another mirror of https://raw.githubusercontent.com/aphesist/aphesis
// No dependencies; node 18+ (global fetch).

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dest = join(root, 'themes', 'aphesis', 'assets');
const base = (process.env.APHESIS_ASSET_BASE || 'https://raw.githubusercontent.com/aphesist/aphesis').replace(/\/$/, '');
const args = new Set(process.argv.slice(2));
const force = args.has('--force');
const dry = args.has('--dry-run');
const quiet = args.has('--quiet');
const log = (...a) => quiet || console.log('[aphesis assets]', ...a);

// what to fetch: branch/path upstream -> file name here
const ASSETS = [
  { file: 'AphesisScript-VF.woff2', from: 'design-system/system/fonts/AphesisScript-VF.woff2', kind: 'font', max: 2_000_000 },
  { file: 'phi.svg', from: 'mark/art/phi.svg', kind: 'mark', max: 200_000 },
];

if (process.env.ANAMNESIS_SKIP_ASSET_FETCH) {
  log('skipped (ANAMNESIS_SKIP_ASSET_FETCH is set); the theme uses its fallbacks.');
  process.exit(0);
}

const manifestPath = join(dest, 'fetched.json');
const manifest = existsSync(manifestPath) ? safeJson(readFileSync(manifestPath, 'utf8')) ?? {} : {};
let fetched = 0;

for (const a of ASSETS) {
  const target = join(dest, a.file);
  if (existsSync(target) && !force) {
    log(`${a.file}: present (use --force to refresh)`);
    continue;
  }
  const src = `${base}/${a.from}`;
  if (dry) {
    log(`${a.file}: would fetch ${src}`);
    continue;
  }
  try {
    const res = await fetch(src, { redirect: 'follow' });
    if (!res.ok) throw new Error(`http ${res.status}`);
    const buf = Buffer.from(await res.arrayBuffer());
    if (buf.length === 0 || buf.length > a.max) throw new Error(`unexpected size ${buf.length}`);
    check(a, buf);
    mkdirSync(dest, { recursive: true });
    writeFileSync(target, buf);
    manifest[a.file] = {
      source: src,
      sha256: createHash('sha256').update(buf).digest('hex'),
      bytes: buf.length,
      fetched: new Date().toISOString(),
    };
    fetched++;
    log(`${a.file}: fetched ${buf.length} bytes`);
  } catch (e) {
    console.warn(`[aphesis assets] ${a.file}: not fetched (${e.message}); the theme will use its fallback.`);
  }
}

if (fetched) writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
log('done. these files are the owner’s: read themes/aphesis/assets/NOTICE.md before redistributing them.');

// light sanity checks so an html error page is never saved as a font or a mark
function check(a, buf) {
  if (a.kind === 'font') {
    const sig = buf.subarray(0, 4).toString('latin1');
    if (sig !== 'wOF2' && sig !== 'wOFF') throw new Error('not a woff/woff2 file');
  }
  if (a.kind === 'mark') {
    const text = buf.toString('utf8');
    if (!/<svg[\s>]/.test(text)) throw new Error('not an svg file');
    if (/<script[\s>]|\son[a-z]+\s*=/i.test(text)) throw new Error('svg contains script; refusing it');
  }
}

function safeJson(s) {
  try {
    return JSON.parse(s);
  } catch {
    return null;
  }
}
