// Shared app state: the cloud, a per-point state bitmask that drives all
// highlighting in the brain (uploaded to the gpu as a texture), and view settings.

export const BIT = {
  FILTERED: 1,  // excluded by "show" / "hide unsorted": nearly invisible
  HOVER: 2,
  SELECTED: 4,  // lit by legend, charts, grid, histogram
  CITED: 8,     // cited by the chat
  SEARCH: 16,   // search hit
  TIMEWIN: 32,  // lit by a time mark in "when you wrote"
  FOCUS: 64,    // the item open in the reader
};
export const LIT = BIT.SELECTED | BIT.CITED | BIT.SEARCH | BIT.TIMEWIN | BIT.FOCUS;

const listeners = new Map();
export function on(ev, fn) { if (!listeners.has(ev)) listeners.set(ev, new Set()); listeners.get(ev).add(fn); return () => listeners.get(ev).delete(fn); }
export function emit(ev, data) { for (const fn of listeners.get(ev) || []) { try { fn(data); } catch (e) { console.error(e); } } }

export const store = {
  cloud: null,
  n: 0,
  flags: new Uint8Array(0),
  index: new Map(),       // id -> point index
  tNorm: new Float32Array(0),
  tMin: 0, tMax: 1,
  view: null,
  litLabel: '',           // a words-description of what is lit, for the dom mirror
  demo: false,
};

export const defaultView = () => ({
  mode: 'brain',          // brain | histogram | grid | table
  page: 'brain',          // brain | notes | write | today
  morph: 0,               // 0 = anatomy, 1 = layout
  groupBy: 'region',      // kind | region | source | speaker
  layout: 'spheres',      // spheres | split
  order: 'time',          // time | size | recency | open
  hideUnsorted: false,
  show: { notes: true, records: true, chats: true },
  turning: true,
  speed: 0.35,
  time: [0, 1],           // normalised time window (scrubber)
  filaments: true,
  outline: true,           // ornamental brain outline in the anatomy form
});

export function setCloud(cloud) {
  store.cloud = cloud;
  const n = cloud.ids.length;
  store.n = n;
  store.flags = new Uint8Array(n);
  store.index = new Map(cloud.ids.map((id, i) => [id, i]));
  let lo = Infinity, hi = -Infinity;
  for (const t of cloud.t) { if (t < lo) lo = t; if (t > hi) hi = t; }
  if (!(hi > lo)) hi = lo + 1;
  store.tMin = lo; store.tMax = hi;
  store.tNorm = Float32Array.from(cloud.t, (t) => (t - lo) / (hi - lo));
  emit('cloud', cloud);
}

let flagsQueued = false;
function flagsChanged() {
  if (flagsQueued) return;
  flagsQueued = true;
  queueMicrotask(() => { flagsQueued = false; emit('flags'); });
}

export function clearBit(bit) {
  const f = store.flags;
  const m = ~bit & 0xff;
  for (let i = 0; i < f.length; i++) f[i] &= m;
  flagsChanged();
}
export function setBit(bit, indices, { exclusive = true, label } = {}) {
  if (exclusive) clearBit(bit);
  const f = store.flags;
  for (const i of indices) if (i >= 0 && i < f.length) f[i] |= bit;
  if (label !== undefined) store.litLabel = label;
  flagsChanged();
}
export function setBitWhere(bit, pred, opts) {
  const idx = [];
  for (let i = 0; i < store.n; i++) if (pred(i)) idx.push(i);
  setBit(bit, idx, opts);
  return idx;
}
export function indicesWith(mask) {
  const out = [];
  const f = store.flags;
  for (let i = 0; i < f.length; i++) if (f[i] & mask) out.push(i);
  return out;
}
export const anyLit = () => { const f = store.flags; for (let i = 0; i < f.length; i++) if (f[i] & LIT) return true; return false; };
export function clearLit() { clearBit(BIT.SELECTED | BIT.TIMEWIN | BIT.SEARCH | BIT.CITED); store.litLabel = ''; emit('lit-cleared'); }

export function setView(patch) {
  Object.assign(store.view, patch);
  emit('view', patch);
}

// Helpers over the cloud.
export const kindOf = (i) => store.cloud.kinds[store.cloud.kind[i]] || 'note';
export const regionOf = (i) => { const r = store.cloud.region[i]; return r >= 0 ? store.cloud.regions[r] : null; };
export const titleOf = (i) => store.cloud.titles[i] || '';
const RECORD_KINDS = new Set(['article', 'pdf', 'book', 'clip', 'feed']);
// "show" buckets: notes = owner-written, records = imported external items, chats = the app's own chats.
export function showBucket(i) {
  const k = kindOf(i);
  if (k === 'chat') return 'chats';
  if (store.cloud.speaker[i] === 2 || RECORD_KINDS.has(k)) return 'records';
  return 'notes';
}
// The cloud has no source column; derive a coarse source from the kind.
const SOURCE_OF_KIND = { note: 'vault', journal: 'vault', quote: 'vault', skill: 'vault', article: 'papers', pdf: 'papers', book: 'papers', chat: 'chats', clip: 'web clips', feed: 'feeds' };
export const sourceOf = (i) => (store.cloud.sources && store.cloud.source ? store.cloud.sources[store.cloud.source[i]] : SOURCE_OF_KIND[kindOf(i)] || 'other');
export const SPEAKERS = ['you', 'assistants', 'other'];

// Group key for "group by".
export function groupKey(i, by) {
  if (by === 'kind') return kindOf(i);
  if (by === 'source') return sourceOf(i);
  if (by === 'speaker') return SPEAKERS[store.cloud.speaker[i]] || 'other';
  const r = regionOf(i);
  return r ? r.label : 'unsorted';
}

export function applyFilters() {
  const v = store.view, c = store.cloud, f = store.flags;
  for (let i = 0; i < store.n; i++) {
    let out = !v.show[showBucket(i)];
    if (v.hideUnsorted && c.region[i] < 0) out = true;
    if (out) f[i] |= BIT.FILTERED; else f[i] &= ~BIT.FILTERED;
  }
  flagsChanged();
}
