// anamnesis layout config: which panels exist, where (left / right / bottom / overlay), their order and whether
// they start open. ui/app builds its panels from getLayout(); the settings panel edits it with the helpers here.
//
//   import { getLayout, onLayoutChange } from './theme/layout.js';
//   const layout = await getLayout();            // the saved layout, or layouts/default.json
//   onLayoutChange((layout) => app.rebuildPanels(layout));

import { kv } from './storage.js';

export const EVENT = 'anamnesis:layoutchange';
export const REGIONS = ['left', 'right', 'bottom', 'overlay'];
export const ANCHORS = ['top-left', 'top-right', 'bottom-left', 'bottom-right', 'left', 'right', 'center'];
export const PRESETS = ['default', 'minimal'];
const BASE = new URL('./', import.meta.url);

const presetCache = new Map();

/** loads a preset from layouts/<id>.json */
export async function loadPreset(id = 'default') {
  if (!PRESETS.includes(id)) throw new Error(`unknown layout preset "${id}"`);
  if (!presetCache.has(id)) {
    const r = await fetch(new URL(`layouts/${id}.json`, BASE));
    presetCache.set(id, normalizeLayout(await r.json()));
  }
  return structuredClone(presetCache.get(id));
}

/** the person's layout, or the default preset */
export async function getLayout() {
  const saved = kv.get('layout');
  if (saved) {
    try {
      return normalizeLayout(saved);
    } catch {
      /* a broken saved layout falls back to the default */
    }
  }
  return loadPreset('default');
}

export function saveLayout(layout, reason = 'edit') {
  const l = normalizeLayout(layout);
  kv.set('layout', l);
  try {
    globalThis.dispatchEvent?.(new CustomEvent(EVENT, { detail: { layout: structuredClone(l), reason } }));
  } catch {
    /* no window */
  }
  return l;
}

export async function resetLayout(preset = 'default') {
  kv.remove('layout');
  return saveLayout(await loadPreset(preset), 'reset');
}

export function onLayoutChange(fn) {
  const h = (e) => fn(e.detail.layout, e.detail.reason);
  globalThis.addEventListener?.(EVENT, h);
  return () => globalThis.removeEventListener?.(EVENT, h);
}

/**
 * Validates and normalises a layout: known regions only, unique ids, orders compacted 0..n per region.
 * Throws on anything that cannot be a layout.
 */
export function normalizeLayout(layout) {
  if (!layout || typeof layout !== 'object' || !Array.isArray(layout.panels)) throw new Error('a layout has a panels array');
  const seen = new Set();
  const panels = [];
  for (const p of layout.panels) {
    if (!p || typeof p.id !== 'string' || !p.id) throw new Error('every panel has an id');
    if (seen.has(p.id)) throw new Error(`duplicate panel "${p.id}"`);
    seen.add(p.id);
    const region = REGIONS.includes(p.region) ? p.region : 'overlay';
    const out = {
      ...p,
      region,
      order: Number.isFinite(p.order) ? p.order : 0,
      open: p.open !== false,
      visible: p.visible !== false,
    };
    if (region === 'overlay' && !ANCHORS.includes(out.anchor)) out.anchor = 'center';
    if (region !== 'overlay') delete out.anchor;
    panels.push(out);
  }
  // compact orders per region, keeping relative order (ties by original position)
  for (const r of REGIONS) {
    panels
      .map((p, i) => ({ p, i }))
      .filter(({ p }) => p.region === r)
      .sort((a, b) => a.p.order - b.p.order || a.i - b.i)
      .forEach(({ p }, k) => (p.order = k));
  }
  return { ...layout, version: 1, panels };
}

/** panels of one region, in order (visible only unless all = true) */
export function panelsIn(layout, region, all = false) {
  return layout.panels.filter((p) => p.region === region && (all || p.visible)).sort((a, b) => a.order - b.order);
}

/** returns a new layout with one panel changed (patch: region, open, visible, anchor, size) */
export function updatePanel(layout, id, patch) {
  const l = structuredClone(layout);
  const p = l.panels.find((x) => x.id === id);
  if (!p) throw new Error(`no panel "${id}"`);
  if (patch.region && patch.region !== p.region) {
    // moving to another region puts it last there
    p.order = Math.max(-1, ...l.panels.filter((x) => x.region === patch.region).map((x) => x.order)) + 1;
  }
  Object.assign(p, patch);
  return normalizeLayout(l);
}

/** returns a new layout with a panel moved up (-1) or down (+1) inside its region */
export function movePanel(layout, id, delta) {
  const l = structuredClone(layout);
  const p = l.panels.find((x) => x.id === id);
  if (!p) throw new Error(`no panel "${id}"`);
  const list = panelsIn(l, p.region, true);
  const i = list.indexOf(p);
  const j = Math.min(list.length - 1, Math.max(0, i + delta));
  if (i === j) return normalizeLayout(l);
  const other = list[j];
  [p.order, other.order] = [other.order, p.order];
  return normalizeLayout(l);
}
