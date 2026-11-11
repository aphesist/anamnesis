// anamnesis ui entry point. No build step: plain es modules.
import { store, setCloud, defaultView, setView, on, emit, setBit, clearBit, clearLit, BIT, LIT, applyFilters, regionOf, titleOf, kindOf, SPEAKERS, indicesWith } from './lib/store.js';
import { api } from './lib/api.js';
import { theme, onTheme, refreshTheme } from './lib/tokens.js';
import { h, clear, fmtDate } from './lib/dom.js';
import { loadLayout, Layout } from './lib/layout.js';

const params = new URLSearchParams(location.search);
if (params.get('mock') === '1') {
  const { installMock } = await import('./mock/mock.js');
  installMock();
}
// The theming package's runtime (fonts, settings, first-launch); optional.
try { await import('../theme/theme.js'); } catch { /* not installed yet */ }
refreshTheme();

const { Brain } = await import('./brain/index.js');
const panels = {
  sidebar: (await import('./panels/sidebar.js')).default,
  view: (await import('./panels/view.js')).default,
  legend: (await import('./panels/legend.js')).default,
  time: (await import('./panels/time.js')).default,
  when: (await import('./panels/when.js')).default,
  chat: (await import('./panels/chat.js')).default,
  threads: (await import('./panels/threads.js')).default,
  review: (await import('./panels/review.js')).default,
  skills: (await import('./panels/skills.js')).default,
  eval: (await import('./panels/eval.js')).default,
};
const pages = {
  notes: (await import('./pages/notes.js')).default,
  write: (await import('./pages/write.js')).default,
  today: (await import('./pages/today.js')).default,
};
const altViews = {
  histogram: (await import('./views/histogram.js')).default,
  grid: (await import('./views/grid.js')).default,
  table: (await import('./views/table.js')).default,
};
const { mountMirror } = await import('./lib/mirror.js');
const { openPeek } = await import('./lib/reader.js');

store.view = defaultView();
const app = document.getElementById('app');
const brain = new Brain(document.getElementById('brain'), document.getElementById('labels'));
window.addEventListener('anamnesis:themechange', () => brain.readColors());

// ---------- shared actions for panels ----------
const toastEl = document.getElementById('toast');
let toastTimer = 0;
const ctx = {
  store, api, brain,
  toast(msg) { toastEl.textContent = msg; toastEl.classList.add('show'); clearTimeout(toastTimer); toastTimer = setTimeout(() => toastEl.classList.remove('show'), 2600); },
  // light points (SELECTED bit), optionally fly to them
  light(indices, label, { bit = BIT.SELECTED, fly = true } = {}) {
    clearBit(BIT.SELECTED | BIT.TIMEWIN | BIT.SEARCH);
    setBit(bit, indices, { label });
    emit('lit', { indices, label });
    if (fly && indices.length && store.view.page === 'brain') brain.focusOn(indices.length > 400 ? indices.filter((_, k) => k % 5 === 0) : indices);
  },
  clearLight() { clearLit(); for (let r = 0; r < 64; r++) brain.setGlow(r, 0); emit('lit', { indices: [], label: '' }); },
  lightRegion(r, { fly = true } = {}) {
    const idx = [];
    for (let i = 0; i < store.n; i++) if (store.cloud.region[i] === r) idx.push(i);
    const reg = store.cloud.regions[r];
    for (let k = 0; k < 64; k++) brain.setGlow(k, 0);
    ctx.light(idx, reg.label, { fly: false });
    if (fly) { if (store.view.page !== 'brain') ctx.go('brain'); brain.focusOn(idx); }
  },
  open(iOrId) {
    const i = typeof iOrId === 'number' ? iOrId : store.index.get(iOrId);
    const id = typeof iOrId === 'number' ? store.cloud.ids[iOrId] : iOrId;
    clearBit(BIT.FOCUS);
    if (i != null && i >= 0) setBit(BIT.FOCUS, [i], { exclusive: false });
    if (store.view.page === 'brain') openPeek(id, ctx);
    else { ctx.go('notes'); emit('open-item', id); }
  },
  openInNotes(id) { ctx.go('notes'); emit('open-item', id); },
  go(page) {
    if (!['brain', 'notes', 'write', 'today'].includes(page)) page = 'brain';
    if (location.hash !== '#/' + page) history.replaceState(null, '', location.pathname + location.search + '#/' + page);
    showPage(page);
  },
  ask(text) { layout.open('chat'); emit('chat-ask', text); },
  setView(patch) { setView(patch); },
  get layout() { return layout; },
  altView(mode) { setView({ mode }); },
};
window.anamnesis = { ctx, store, brain };

// ---------- pages ----------
const pageMounted = new Set();
function showPage(page) {
  store.view.page = page;
  app.dataset.page = page;
  for (const p of ['brain', 'notes', 'write', 'today']) document.getElementById('page-' + p).hidden = p !== page;
  if (page !== 'brain' && !pageMounted.has(page)) { pages[page].mount(document.getElementById('page-' + page), ctx); pageMounted.add(page); }
  if (page !== 'brain') pages[page].shown?.(ctx);
  brain.paused = page !== 'brain';
  if (page === 'brain') brain.dirty = true;
  emit('page', page);
}
window.addEventListener('hashchange', () => showPage((location.hash.match(/^#\/(\w+)/) || [])[1] || 'brain'));

// ---------- stage: tooltip, alt views ----------
const tooltip = document.getElementById('tooltip');
brain.onHover = (i, xy) => {
  if (i < 0 || !xy) { tooltip.hidden = true; return; }
  const r = regionOf(i);
  clear(tooltip).append(
    h('div.tt-title', titleOf(i)),
    h('div.tt-meta', `${kindOf(i)} · ${SPEAKERS[store.cloud.speaker[i]]} · ${fmtDate(store.cloud.t[i])}${r ? ' · ' + r.label : ''}`));
  tooltip.hidden = false;
  tooltip.style.left = xy[0] + 'px'; tooltip.style.top = xy[1] + 'px';
};
brain.onOpen = (i) => ctx.open(i);
brain.onRegion = (r) => ctx.lightRegion(r);

const viewsBar = document.querySelector('.stage-views');
const altEl = document.getElementById('alt-view');
for (const mode of ['brain', 'histogram', 'grid', 'table']) {
  viewsBar.append(h('button', { type: 'button', role: 'tab', 'data-mode': mode, 'aria-selected': String(mode === 'brain'), onclick: () => setView({ mode }) }, mode));
}
const litBar = h('div.lit-bar.no-orbit', { hidden: true, 'aria-live': 'polite' });
document.getElementById('page-brain').append(litBar);
function renderLitBar() {
  const lit = indicesWith(LIT & ~BIT.FOCUS).length;
  litBar.hidden = !lit;
  if (!lit) return;
  clear(litBar).append(h('span.dot'), h('span', `${store.litLabel || 'lit'} — ${lit} lit`), h('button.textbtn', { type: 'button', onclick: () => ctx.clearLight() }, 'clear'));
}
on('flags', renderLitBar);

function renderAlt() {
  const mode = store.view.mode;
  for (const b of viewsBar.children) b.setAttribute('aria-selected', String(b.dataset.mode === mode));
  altEl.hidden = mode === 'brain';
  if (mode !== 'brain') { clear(altEl); altViews[mode].mount(altEl, ctx); }
}
on('view', (patch) => {
  if ('mode' in patch) renderAlt();
  if ('morph' in patch) brain.setMorph(store.view.morph, patch.instant);
  if ('groupBy' in patch || 'layout' in patch || 'order' in patch) brain.relayout();
  if ('show' in patch || 'hideUnsorted' in patch) { applyFilters(); brain.relayout(false); }
  if ('turning' in patch || 'speed' in patch) { brain.cam.turning = store.view.turning; brain.cam.speed = store.view.speed; }
  if ('time' in patch || 'filaments' in patch || 'outline' in patch) brain.dirty = true;
});
on('flags', () => { if (store.view.mode !== 'brain' && store.view.mode !== 'table') renderAltSoon(); });
let altQueued = false;
function renderAltSoon() { if (altQueued) return; altQueued = true; requestAnimationFrame(() => { altQueued = false; if (store.view.mode !== 'brain') altViews[store.view.mode].update?.(altEl, ctx); }); }

onTheme((t) => {
  brain.cam.motion = t.motion;
  if (t.motion === 0) { brain.cam.turning = false; } else brain.cam.turning = store.view.turning;
  emit('theme', t);
});

// ---------- keyboard ----------
document.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); document.querySelector('.ask input')?.focus(); return; }
  if (e.key === 'F6') {
    e.preventDefault();
    const zones = [document.getElementById('dock-left'), document.getElementById('brain'), document.getElementById('dock-right'), document.getElementById('dock-bottom')].filter((z) => z && !z.hidden && z.offsetParent !== null);
    const k = zones.findIndex((z) => z.contains(document.activeElement));
    const next = zones[(k + (e.shiftKey ? -1 + zones.length : 1)) % zones.length];
    (next.matches('canvas') ? next : next.querySelector('button, input, [tabindex="0"]'))?.focus();
    return;
  }
  if (e.key === 'Escape' && !e.target.closest('input, textarea, select')) { ctx.clearLight(); emit('escape'); }
});

// ---------- boot ----------
const config = await loadLayout();
const layout = new Layout(config, panels, ctx);
layout.render();
window.anamnesis.layout = layout;
mountMirror(document.getElementById('brain-list'), ctx);

async function loadCloud(source) {
  const cloud = source ? await (await fetch(source)).json() : await api.cloud();
  setCloud(cloud);
  applyFilters();
  brain.setData(cloud);
  brain.relayout(false);
  brain.cam.turning = store.view.turning && theme().motion > 0;
  emit('cloud-ready', cloud);
}
ctx.loadCloud = loadCloud;
try {
  await loadCloud();
} catch (e) {
  console.error(e);
  document.getElementById('caption').textContent = 'the brain could not be reached. try ?mock=1 to see a demo.';
}
showPage((location.hash.match(/^#\/(\w+)/) || [])[1] || 'brain');
renderAlt();
document.documentElement.classList.toggle('reduced', theme().motion === 0);
document.body.dataset.ready = '1';
