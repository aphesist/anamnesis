// anamnesis theme runtime (ES module, no dependencies).
//
//   import * as theme from './theme/theme.js';
//   await theme.init();                       // apply the stored (or default) theme as early as possible
//   theme.applyTheme('paper');                // switch theme by id
//   theme.setMode('light' | 'dark' | 'system');
//   theme.setToken('--accent', 'oklch(0.6 0.2 30)', 'dark');   // any token, per mode or 'all'
//   window.addEventListener('anamnesis:themechange', (e) => renderer.readColours(theme.tokenRGB));
//
// The ui reads only the css custom properties listed in docs/contracts.md. This module sets them by loading
// dist/<id>.css (generated from the design tokens), then layering user choices on top in one <style>:
// token overrides, fonts, type scale and logo. The html element carries the switches:
//   data-theme="light|dark"   data-motion="full|reduced"   data-cursor="custom|system"   data-scrollbars="hidden|system"
//
// Choices persist in localStorage (via storage.js) and, when the anamnesis server is there, in
// PUT /api/settings under the `theme` key. Fonts and logos the user uploads live in IndexedDB.

import { kv, blobs, blobToDataUrl } from './storage.js';
import { compileTheme, parseColor, overridesToCss } from './tokens.js';

export const EVENT = 'anamnesis:themechange';
const BASE = new URL('./', import.meta.url);
const url = (p) => new URL(p, BASE).href;
const doc = () => globalThis.document;
const root = () => doc()?.documentElement;

const DEFAULT_STATE = {
  id: 'aphesis',
  // 'theme': the theme's own default (aphesis: always dark; a theme whose manifest says
  // "modeFollowsSystem": true follows the os). 'system' | 'light' | 'dark' are explicit choices.
  mode: 'theme',
  modeV: 2,
  motion: null, // null until the person chooses: then 'full' | 'reduced'
  cursor: null, // null follows the theme's behaviours
  scrollbars: null,
  typeScale: null,
  overrides: { all: {}, light: {}, dark: {} },
  fonts: {}, // role -> { family, stack?, source: 'builtin' | 'upload' | 'url', key?, url?, name? }
  logo: {}, // 'mark' | 'wordmark' -> { source: 'upload' | 'url', key?, url?, name? }
};

let state = structuredCloneSafe(DEFAULT_STATE);
let index = null; // dist/index.json
let current = null; // { id, manifest, json, behaviours }
let objectUrls = [];
let systemQuery = null;
let motionQuery = null;
let serverOk = null; // null unknown, true reachable, false absent
let options = { server: 'auto', base: BASE.href };

function structuredCloneSafe(o) {
  return JSON.parse(JSON.stringify(o));
}

// ---------------------------------------------------------------- public api

/**
 * Applies the stored theme. Call once, as early as possible (before first paint if you can).
 * opts.server: true | false | 'auto' (auto: use /api/settings when the page is served over http).
 * opts.defaultTheme: theme id when nothing is stored.
 */
export async function init(opts = {}) {
  options = { ...options, ...opts };
  const stored = kv.get('theme');
  state = { ...structuredCloneSafe(DEFAULT_STATE), ...(stored ?? {}) };
  if (!stored && opts.defaultTheme) state.id = opts.defaultTheme;
  if (!stored && useServer()) {
    const remote = await serverGet();
    if (remote?.theme) state = { ...state, ...sanitizeRemote(remote.theme) };
  }
  migrateMode();
  watchSystem();
  await applyTheme(state.id, { persist: false, reason: 'init' });
  return getState();
}

/** the current state (a copy) plus what is resolved from it */
export function getState() {
  return {
    ...structuredCloneSafe(state),
    resolvedMode: resolvedMode(),
    resolvedMotion: resolvedMotion(),
    resolvedCursor: resolvedBehaviour('cursor'),
    resolvedScrollbars: resolvedBehaviour('scrollbars'),
    motionChosen: state.motion != null,
  };
}

/** themes available: built-in (dist/index.json) plus custom ones the user imported */
export async function listThemes() {
  const idx = await loadIndex();
  const custom = Object.values(kv.get('themes.custom', {}) ?? {}).map((t) => ({
    id: t.id,
    name: t.name,
    description: t.description ?? 'imported theme',
    defaultMode: t.defaultMode,
    custom: true,
    preview: t.preview,
  }));
  return [...idx.themes, ...custom];
}

/** switches to a theme by id (built-in or imported) */
export async function applyTheme(id, { persist = true, reason = 'theme' } = {}) {
  const idx = await loadIndex();
  const custom = kv.get('themes.custom', {}) ?? {};
  let entry = idx.themes.find((t) => t.id === id);
  if (!entry && !custom[id]) {
    console.warn(`[theme] unknown theme "${id}", using aphesis`);
    id = 'aphesis';
    entry = idx.themes.find((t) => t.id === id) ?? idx.themes[0];
  }
  state.id = id;
  if (entry) {
    await setStylesheet(url('dist/' + entry.css));
    setCustomCss('');
    let manifest = {};
    try {
      manifest = await (await fetch(url(`themes/${id}/theme.json`))).json();
    } catch {
      /* served without the theme folder: the css alone still works */
    }
    let values = null;
    try {
      values = await (await fetch(url('dist/' + (entry.values ?? `${id}.json`)))).json();
    } catch {
      /* fine: user fonts then fall back to serif */
    }
    current = { id, manifest, behaviours: entry.behaviours ?? manifest.behaviours ?? {}, fontStacks: fontStacksOf(values?.values?.dark) };
  } else {
    const t = custom[id];
    setCustomCss(t.css);
    await setStylesheet(null);
    current = { id, manifest: t, behaviours: t.behaviours ?? {}, fontStacks: t.fontStacks ?? {} };
  }
  await loadOptionalAssets();
  await restoreUserAssets();
  applySwitches();
  writeOverrides();
  if (persist) save();
  emit(reason);
}

export function setMode(mode) {
  if (!MODES.includes(mode)) throw new Error('mode is theme, system, light or dark');
  state.mode = mode;
  state.modeV = 2;
  applySwitches();
  save();
  emit('mode');
}

/** 'full' | 'reduced' | null (null: follow the os until the person chooses) */
export function setMotion(motion) {
  state.motion = motion === 'reduced' || motion === 'full' ? motion : null;
  applySwitches();
  save();
  emit('motion');
}

export function setCursor(cursor) {
  state.cursor = cursor === 'custom' || cursor === 'system' ? cursor : null;
  applySwitches();
  save();
  emit('cursor');
}

export function setScrollbars(v) {
  state.scrollbars = v === 'hidden' || v === 'system' ? v : null;
  applySwitches();
  save();
  emit('scrollbars');
}

export function setTypeScale(n) {
  state.typeScale = n == null ? null : Math.min(2, Math.max(0.6, Number(n) || 1));
  writeOverrides();
  save();
  emit('type');
}

/** overrides any token. mode: 'all' | 'light' | 'dark'. value null removes the override. */
export function setToken(name, value, mode = 'all') {
  const n = name.startsWith('--') ? name : '--' + name;
  const bucket = (state.overrides[mode] ??= {});
  if (value == null || value === '') delete bucket[n];
  else bucket[n] = String(value);
  writeOverrides();
  save();
  emit('token');
}

/** replaces all overrides at once ({ all, light, dark }) */
export function setOverrides(overrides) {
  state.overrides = { all: {}, light: {}, dark: {}, ...(overrides ?? {}) };
  writeOverrides();
  save();
  emit('token');
}

export function clearOverrides() {
  setOverrides(null);
}

/** reads a token's current value (a css string) */
export function readToken(name) {
  const r = root();
  if (!r) return '';
  return getComputedStyle(r).getPropertyValue(name.startsWith('--') ? name : '--' + name).trim();
}

/**
 * A colour token as numbers for webgl: { r, g, b } gamma-encoded 0..1, linear [r, g, b], alpha, hex.
 * Handles oklch / oklab / rgb / hex directly and anything else through a 1px canvas.
 */
export function tokenRGB(name) {
  const v = readToken(name);
  const p = parseColor(v) ?? canvasColor(v);
  if (!p) return null;
  return { r: p.srgb[0], g: p.srgb[1], b: p.srgb[2], linear: p.linear, alpha: p.alpha, hex: p.hex, css: v };
}

/**
 * Sets the face for a role ('display' | 'text' | 'data').
 * font: a css family stack string (e.g. 'Georgia, serif'), or null to restore the theme's.
 */
export function setFont(role, font) {
  checkRole(role);
  if (font == null) delete state.fonts[role];
  else state.fonts[role] = { source: 'builtin', stack: String(font) };
  writeOverrides();
  save();
  emit('font');
}

/**
 * Adds a font the person provides and uses it for a role.
 * source: a File/Blob (stored in IndexedDB) or { url } (a local path the app can fetch).
 */
export async function addFont(role, source, { family } = {}) {
  checkRole(role);
  const name = source?.name ?? source?.url ?? 'font';
  const fam = family || 'user ' + String(name).replace(/\.[a-z0-9]+$/i, '').replace(/[^a-zA-Z0-9 _-]/g, '').slice(0, 40);
  if (source?.url) {
    await registerFont(fam, `url("${encodeURI(source.url)}")`);
    state.fonts[role] = { source: 'url', family: fam, url: source.url, name };
  } else if (source instanceof Blob) {
    const key = `font.${role}.${Date.now().toString(36)}`;
    const persisted = await blobs.put(key, source);
    await registerFont(fam, await source.arrayBuffer());
    const prev = state.fonts[role];
    if (prev?.key) blobs.delete(prev.key);
    state.fonts[role] = { source: 'upload', family: fam, key, name, persisted };
  } else {
    throw new Error('addFont needs a File/Blob or { url }');
  }
  writeOverrides();
  save();
  emit('font');
  return fam;
}

/**
 * Sets a custom logo. kind: 'mark' | 'wordmark'. source: File/Blob (svg or png), { url }, or null to restore.
 */
export async function setLogo(source, kind = 'mark') {
  if (kind !== 'mark' && kind !== 'wordmark') throw new Error('kind is mark or wordmark');
  const prev = state.logo[kind];
  if (prev?.key) await blobs.delete(prev.key);
  if (source == null) {
    delete state.logo[kind];
  } else if (source.url) {
    state.logo[kind] = { source: 'url', url: source.url };
  } else if (source instanceof Blob) {
    if (!/^image\/(svg\+xml|png|webp|jpeg)$/.test(source.type)) throw new Error('a logo is an svg or png image');
    const key = `logo.${kind}.${Date.now().toString(36)}`;
    const persisted = await blobs.put(key, source);
    state.logo[kind] = { source: 'upload', key, name: source.name, type: source.type, persisted, _url: track(URL.createObjectURL(source)) };
  }
  writeOverrides();
  save();
  emit('logo');
}

/** the logo url currently in effect (for an <img>) */
export function markUrl(kind = 'mark') {
  const v = readToken(kind === 'wordmark' ? '--wordmark' : '--mark');
  const m = v.match(/url\(["']?([^"')]+)["']?\)/);
  return m ? m[1] : null;
}

/** everything needed to recreate the current look elsewhere, as a json-serialisable object */
export async function exportTheme() {
  const out = {
    format: 'anamnesis-theme',
    version: 1,
    exported: new Date().toISOString(),
    base: state.id,
    mode: state.mode,
    motion: state.motion,
    cursor: state.cursor,
    scrollbars: state.scrollbars,
    typeScale: state.typeScale,
    overrides: state.overrides,
    fonts: {},
    logo: {},
  };
  const custom = kv.get('themes.custom', {})?.[state.id];
  if (custom) out.theme = { id: custom.id, name: custom.name, defaultMode: custom.defaultMode, tokens: custom.tokens };
  for (const [role, f] of Object.entries(state.fonts)) {
    const e = { family: f.family, stack: f.stack, source: f.source, url: f.url, name: f.name };
    if (f.key) {
      const b = await blobs.get(f.key);
      if (b) e.data = await blobToDataUrl(b);
    }
    out.fonts[role] = e;
  }
  for (const [kind, l] of Object.entries(state.logo)) {
    const e = { source: l.source, url: l.url, name: l.name };
    if (l.key) {
      const b = await blobs.get(l.key);
      if (b) e.data = await blobToDataUrl(b);
    }
    out.logo[kind] = e;
  }
  return out;
}

/**
 * Imports a theme exported by exportTheme(), or a full theme: { format, theme: { id, name, defaultMode,
 * tokens: { common?, light?, dark? } } } whose tokens are W3C design tokens layered over the base tokens.
 */
export async function importTheme(data) {
  if (typeof data === 'string') data = JSON.parse(data);
  if (!data || data.format !== 'anamnesis-theme') throw new Error('not an anamnesis theme file');
  let id = data.base ?? 'aphesis';
  if (data.theme?.tokens) id = await registerTheme(data.theme);
  state.mode = MODES.includes(data.mode) ? data.mode : state.mode;
  if (data.motion !== undefined) state.motion = data.motion === 'reduced' || data.motion === 'full' ? data.motion : state.motion;
  if (data.cursor !== undefined) state.cursor = data.cursor ?? null;
  if (data.scrollbars !== undefined) state.scrollbars = data.scrollbars ?? null;
  state.typeScale = data.typeScale ?? null;
  state.overrides = { all: {}, light: {}, dark: {}, ...(data.overrides ?? {}) };
  state.fonts = {};
  for (const [role, f] of Object.entries(data.fonts ?? {})) {
    if (!['display', 'text', 'data'].includes(role)) continue;
    try {
      if (f.data) await addFont(role, await (await fetch(f.data)).blob(), { family: f.family });
      else if (f.url) await addFont(role, { url: f.url }, { family: f.family });
      else if (f.stack) state.fonts[role] = { source: 'builtin', stack: f.stack };
    } catch (e) {
      console.warn('[theme] font not imported', role, e);
    }
  }
  state.logo = {};
  for (const [kind, l] of Object.entries(data.logo ?? {})) {
    try {
      if (l.data) {
        const blob = await (await fetch(l.data)).blob();
        await setLogo(new File([blob], l.name ?? kind, { type: blob.type }), kind);
      } else if (l.url) await setLogo({ url: l.url }, kind);
    } catch (e) {
      console.warn('[theme] logo not imported', kind, e);
    }
  }
  await applyTheme(id, { reason: 'import' });
}

/**
 * Registers a full theme from design tokens (stored locally). Returns its id.
 * theme = { id, name, description?, defaultMode, tokens: { common?, light?, dark? }, behaviours? }
 */
export async function registerTheme(theme) {
  const base = await (await fetch(url('tokens/base.tokens.json'))).json();
  const id = 'custom-' + String(theme.id ?? theme.name ?? 'theme').toLowerCase().replace(/[^a-z0-9-]/g, '-').replace(/^custom-/, '');
  const { css, json } = compileTheme({
    id,
    name: theme.name,
    defaultMode: theme.defaultMode,
    base,
    common: theme.tokens?.common,
    modes: { light: theme.tokens?.light, dark: theme.tokens?.dark },
    marks: { mark: url('marks/phi-placeholder.svg'), wordmark: url('marks/wordmark-placeholder.svg') },
    fontFaces: [{ family: 'Special Elite', src: url('fonts/SpecialElite-Regular.woff2') }],
  });
  const preview = Object.fromEntries(
    ['light', 'dark'].map((m) => [m, Object.fromEntries(['--bg', '--ink', '--accent', '--dot'].map((k) => [k, json.modes[m][k]?.hex]))]),
  );
  const all = kv.get('themes.custom', {}) ?? {};
  all[id] = { id, name: theme.name ?? id, description: theme.description, defaultMode: json.defaultMode, tokens: theme.tokens, css, preview, behaviours: theme.behaviours, fontStacks: fontStacksOf(json.values.dark) };
  kv.set('themes.custom', all);
  return id;
}

export function removeTheme(id) {
  const all = kv.get('themes.custom', {}) ?? {};
  delete all[id];
  kv.set('themes.custom', all);
}

/** resets everything to the defaults (keeps imported themes) */
export async function reset() {
  for (const f of Object.values(state.fonts)) if (f.key) await blobs.delete(f.key);
  for (const l of Object.values(state.logo)) if (l.key) await blobs.delete(l.key);
  const motion = state.motion;
  state = structuredCloneSafe(DEFAULT_STATE);
  state.motion = motion; // the motion answer stays: it was asked once
  await applyTheme(state.id, { reason: 'reset' });
}

export function onThemeChange(fn) {
  const h = (e) => fn(e.detail);
  globalThis.addEventListener?.(EVENT, h);
  return () => globalThis.removeEventListener?.(EVENT, h);
}

// ---------------------------------------------------------------- internals

function fontStacksOf(vars) {
  if (!vars) return {};
  return { display: vars['--font-display'], text: vars['--font-text'], data: vars['--font-data'] };
}

function checkRole(role) {
  if (!['display', 'text', 'data'].includes(role)) throw new Error('font role is display, text or data');
}

async function loadIndex() {
  if (index) return index;
  try {
    index = await (await fetch(url('dist/index.json'))).json();
  } catch {
    index = { themes: [{ id: 'aphesis', name: 'aphes.ist', css: 'aphesis.css', defaultMode: 'dark', behaviours: { cursor: 'custom', scrollbars: 'hidden' } }] };
  }
  return index;
}

function setStylesheet(href) {
  const d = doc();
  if (!d) return Promise.resolve();
  let link = d.getElementById('anamnesis-theme-css');
  if (!href) {
    link?.remove();
    return Promise.resolve();
  }
  if (link && link.href === href) return Promise.resolve();
  const next = d.createElement('link');
  next.rel = 'stylesheet';
  next.href = href;
  return new Promise((resolve) => {
    const done = () => {
      // swap only once the new sheet has loaded, so there is no unstyled frame
      if (link && link !== next) link.remove();
      next.id = 'anamnesis-theme-css';
      resolve();
    };
    next.onload = done;
    next.onerror = done;
    (link?.parentNode ?? d.head).insertBefore(next, link ? link.nextSibling : null);
  });
}

function styleEl(id) {
  const d = doc();
  if (!d) return null;
  let el = d.getElementById(id);
  if (!el) {
    el = d.createElement('style');
    el.id = id;
    d.head.appendChild(el);
  }
  return el;
}

function setCustomCss(css) {
  const el = styleEl('anamnesis-theme-custom');
  if (el) el.textContent = css ?? '';
  // keep the overrides after it
  const o = doc()?.getElementById('anamnesis-theme-overrides');
  if (o && el) doc().head.appendChild(o);
}

// optional theme assets (the aphesis theme's Aphesis Script and φ): used when present, silently absent otherwise.
// The fetch script writes assets/fetched.json; one request for it tells us what is there, so a missing asset
// costs one 404 instead of one per file.
const assetLists = new Map();
async function fetchedAssets(themeBase) {
  if (assetLists.has(themeBase)) return assetLists.get(themeBase);
  let names = new Set();
  try {
    const r = await fetch(new URL('assets/fetched.json', themeBase), { cache: 'no-cache' });
    if (r.ok) names = new Set(Object.keys(await r.json()).map((f) => 'assets/' + f));
  } catch {
    /* none */
  }
  assetLists.set(themeBase, names);
  return names;
}

async function loadOptionalAssets() {
  const m = current?.manifest ?? {};
  const themeBase = url(`themes/${current.id}/`);
  current.assets = {};
  const wantsAssets = (m.optionalFonts ?? []).length || Object.values(m.marks ?? {}).some((x) => x?.src);
  const present = wantsAssets ? await fetchedAssets(themeBase) : new Set();
  const exists = (src) => present.has(src.replace(/^\.\//, ''));
  for (const f of m.optionalFonts ?? []) {
    const href = new URL(f.src, themeBase).href;
    if (!exists(f.src)) continue;
    try {
      await registerFont(f.family, `url("${href}")`, { weight: f.weight ?? '400' });
    } catch {
      /* not a usable font file: the stack falls back */
    }
  }
  for (const kind of ['mark', 'wordmark']) {
    const spec = m.marks?.[kind];
    if (!spec) continue;
    if (spec.src) {
      const href = new URL(spec.src, themeBase).href;
      if (exists(spec.src)) {
        current.assets[kind] = href;
        continue;
      }
    }
    // absolute url: css url() inside a custom property must not depend on the stylesheet it was declared in
    if (spec.fallback) current.assets[kind] = new URL(spec.fallback, themeBase).href;
  }
}

const registered = new Set();
async function registerFont(family, src, descriptors = {}) {
  const key = family + '|' + (typeof src === 'string' ? src : src.byteLength);
  if (registered.has(key)) return;
  if (!globalThis.FontFace || !doc()?.fonts) return;
  const face = new FontFace(family, src, { display: 'swap', ...descriptors });
  await face.load();
  doc().fonts.add(face);
  registered.add(key);
}

async function restoreUserAssets() {
  for (const [role, f] of Object.entries(state.fonts)) {
    try {
      if (f.source === 'upload' && f.key) {
        const b = await blobs.get(f.key);
        if (b) await registerFont(f.family, await b.arrayBuffer());
        else delete state.fonts[role];
      } else if (f.source === 'url' && f.url) {
        await registerFont(f.family, `url("${encodeURI(f.url)}")`);
      }
    } catch {
      delete state.fonts[role];
    }
  }
  for (const [kind, l] of Object.entries(state.logo)) {
    if (l.source === 'upload' && l.key && !l._url) {
      const b = await blobs.get(l.key);
      if (b) l._url = track(URL.createObjectURL(b));
      else delete state.logo[kind];
    }
  }
}

function track(u) {
  objectUrls.push(u);
  return u;
}

function writeOverrides() {
  const el = styleEl('anamnesis-theme-overrides');
  if (!el) return;
  const all = { ...(state.overrides?.all ?? {}) };
  if (current?.assets?.mark) all['--mark'] ??= `url("${current.assets.mark}")`;
  if (current?.assets?.wordmark) all['--wordmark'] ??= `url("${current.assets.wordmark}")`;
  for (const kind of ['mark', 'wordmark']) {
    const l = state.logo[kind];
    const u = l?._url ?? l?.url;
    if (u) all['--' + kind] = `url("${u}")`;
  }
  if (state.typeScale != null) all['--type-scale'] = String(state.typeScale);
  // a user face goes in front of the theme's own stack, so glyphs it lacks still fall back
  for (const [role, f] of Object.entries(state.fonts)) {
    const themeStack = current?.fontStacks?.[role] ?? 'serif';
    all['--font-' + role] = f.source === 'builtin' ? f.stack : `"${f.family}", ${themeStack}`;
  }
  let css = '';
  css += overridesToCss({ all, light: state.overrides?.light, dark: state.overrides?.dark });
  el.textContent = css;
  doc()?.head.appendChild(el); // always last, so it wins over a theme sheet inserted later
}

const MODES = ['theme', 'system', 'light', 'dark'];

// Before modeV 2 the default was 'system', stored even when nobody chose it: read that as the
// theme's own default, so the aphes.ist theme opens dark whatever the os prefers.
function migrateMode() {
  if (state.modeV !== 2) {
    if (state.mode === 'system' || !MODES.includes(state.mode)) state.mode = 'theme';
    state.modeV = 2;
  }
}

function themeDefaultMode() {
  return current?.manifest?.defaultMode === 'light' ? 'light' : 'dark';
}

function resolvedMode() {
  if (state.mode === 'light' || state.mode === 'dark') return state.mode;
  if (state.mode !== 'system' && !current?.manifest?.modeFollowsSystem) return themeDefaultMode();
  const mq = globalThis.matchMedia?.('(prefers-color-scheme: light)');
  if (mq?.matches) return 'light';
  const mqd = globalThis.matchMedia?.('(prefers-color-scheme: dark)');
  if (mqd?.matches) return 'dark';
  return themeDefaultMode();
}

export function osPrefersReducedMotion() {
  return !!globalThis.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

function resolvedMotion() {
  if (state.motion) return state.motion;
  return osPrefersReducedMotion() ? 'reduced' : 'full';
}

function resolvedBehaviour(name) {
  if (state[name]) return state[name];
  const b = current?.behaviours?.[name];
  if (name === 'cursor') return b === 'custom' ? 'custom' : 'system';
  return b === 'hidden' ? 'hidden' : 'system';
}

function applySwitches() {
  const r = root();
  if (!r) return;
  r.setAttribute('data-theme', resolvedMode());
  r.setAttribute('data-motion', resolvedMotion());
  r.setAttribute('data-cursor', resolvedBehaviour('cursor'));
  r.setAttribute('data-scrollbars', resolvedBehaviour('scrollbars'));
  r.setAttribute('data-theme-id', state.id);
}

function watchSystem() {
  if (systemQuery || !globalThis.matchMedia) return;
  systemQuery = matchMedia('(prefers-color-scheme: dark)');
  systemQuery.addEventListener?.('change', () => {
    if (state.mode !== 'system' && !(state.mode === 'theme' && current?.manifest?.modeFollowsSystem)) return;
    applySwitches();
    emit('system');
  });
  motionQuery = matchMedia('(prefers-reduced-motion: reduce)');
  motionQuery.addEventListener?.('change', () => {
    if (state.motion) return;
    applySwitches();
    emit('system');
  });
}

function emit(reason) {
  const detail = { ...getState(), reason };
  try {
    globalThis.dispatchEvent?.(new CustomEvent(EVENT, { detail }));
  } catch {
    /* no window */
  }
}

function persistable() {
  const s = structuredCloneSafe(state);
  for (const l of Object.values(s.logo ?? {})) delete l._url;
  return s;
}

function save() {
  kv.set('theme', persistable());
  scheduleServerSave();
}

// ---------------------------------------------------------------- optional server sync

function useServer() {
  if (options.server === false) return false;
  if (options.server === true) return serverOk !== false;
  const proto = globalThis.location?.protocol;
  return (proto === 'http:' || proto === 'https:') && serverOk !== false;
}

async function serverGet() {
  try {
    const r = await fetch('/api/settings', { headers: { accept: 'application/json' } });
    if (!r.ok) {
      serverOk = false;
      return null;
    }
    serverOk = true;
    return await r.json();
  } catch {
    serverOk = false;
    return null;
  }
}

let saveTimer = null;
function scheduleServerSave() {
  if (!useServer()) return;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    const current = await serverGet();
    if (!current || typeof current !== 'object') return;
    const s = persistable();
    // uploaded files stay on this device; the server keeps references only
    const theme = { ...s, fonts: Object.fromEntries(Object.entries(s.fonts).filter(([, f]) => f.source !== 'upload')), logo: Object.fromEntries(Object.entries(s.logo).filter(([, l]) => l.source === 'url')) };
    try {
      await fetch('/api/settings', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        // only the keys this module owns; `theme` is sent whole, merged over what the
        // server holds so fields written elsewhere are kept
        body: JSON.stringify({ theme: { ...(isPlainObject(current.theme) ? current.theme : {}), ...theme }, motion: resolvedMotion() }),
      });
    } catch {
      serverOk = false;
    }
  }, 600);
}

const isPlainObject = (v) => v != null && typeof v === 'object' && !Array.isArray(v);

function sanitizeRemote(t) {
  const out = {};
  for (const k of ['id', 'mode', 'modeV', 'motion', 'cursor', 'scrollbars', 'typeScale', 'overrides', 'fonts', 'logo']) if (k in t) out[k] = t[k];
  return out;
}

let canvas = null;
function canvasColor(v) {
  try {
    canvas ??= doc().createElement('canvas');
    canvas.width = canvas.height = 1;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = '#000';
    ctx.fillStyle = v;
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
    return parseColor(`rgb(${r} ${g} ${b} / ${a / 255})`);
  } catch {
    return null;
  }
}
