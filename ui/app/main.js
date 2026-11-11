// anamnesis ui entry point. No build step: plain es modules.
import { store, setCloud, defaultView, setView, on, emit, setBit, clearBit, clearLit, BIT, LIT, applyFilters, regionOf, titleOf, kindOf, SPEAKERS, indicesWith } from './lib/store.js';
import { api } from './lib/api.js';
import { theme, onTheme, refreshTheme, useThemeApi } from './lib/tokens.js';
import { h, clear, fmtDate } from './lib/dom.js';
import { loadLayout, Layout } from './lib/layout.js';

const params = new URLSearchParams(location.search);
if (params.get('mock') === '1') {
  const { installMock } = await import('./mock/mock.js');
  installMock();
}
// The theming package's runtime (tokens, fonts, cursor, first-launch, settings).
// Optional: without it the app runs on fallback-tokens.css.
let themeApi = null;
try {
  themeApi = await import('../theme/theme.js');
  await themeApi.init();
  const fb = document.getElementById('fallback-tokens');
  if (fb) fb.disabled = true;
  useThemeApi(themeApi);
  try { (await import('../theme/cursor.js')).startCursor(); } catch (e) { console.warn('cursor', e); }
} catch (e) {
  console.warn('ui/theme not loaded, using fallback tokens', e);
  themeApi = null;
}
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
  go(page, { replace = false } = {}) {
    if (!PAGES.includes(page)) page = 'brain';
    // each page change is a history entry, so back and forward walk between pages
    if (routeOf(location.hash) !== page || !location.hash) history[replace ? 'replaceState' : 'pushState'](null, '', location.pathname + location.search + '#' + page);
    showPage(page);
  },
  ask(text) { layout.open('chat'); emit('chat-ask', text); },
  setView(patch) { setView(patch); },
  get layout() { return layout; },
  altView(mode) { setView({ mode }); },
};
window.anamnesis = { ctx, store, brain, theme: themeApi };

// ---------- pages ----------
// routes: #brain, #notes, #write, #today (the older #/notes form is still read). Other hashes
// (the skip links' #brain-list) are anchors, not routes.
const PAGES = ['brain', 'notes', 'write', 'today'];
function routeOf(hash) {
  const m = String(hash || '').match(/^#\/?(\w+)$/);
  return m && PAGES.includes(m[1]) ? m[1] : null;
}
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
function onRoute() {
  const page = routeOf(location.hash);
  if (page && page !== store.view.page) showPage(page);
  else if (!location.hash && store.view.page !== 'brain') showPage('brain');
}
window.addEventListener('hashchange', onRoute);
window.addEventListener('popstate', onRoute);

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
  document.getElementById('page-brain').classList.toggle('alt', mode !== 'brain');
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

// a floating panel docked to one side of the stage: slide the brain into the space left over
const floatEl = document.getElementById('float');
const baseShift = [...brain.cam.shift];
function syncShift() {
  const stage = document.getElementById('page-brain').getBoundingClientRect();
  let dx = 0;
  const sheet = document.querySelector('.sheet:not([hidden])');
  const side = sheet ? 'right' : floatEl.dataset.anchor || 'right';
  const over = sheet || (!floatEl.hidden ? floatEl : null);
  if (over && stage.width && store.view.page === 'brain') {
    const share = Math.min(0.6, over.getBoundingClientRect().width / stage.width);
    if (side === 'right') dx = -share; else if (side === 'left' || side === 'top-left') dx = share;
  }
  brain.cam.shiftTarget = [baseShift[0] + dx, baseShift[1]];
  brain.dirty = true;
}
new MutationObserver(syncShift).observe(document.getElementById('main'), { attributes: true, attributeFilter: ['hidden', 'data-anchor'], subtree: true, childList: true });
window.addEventListener('resize', syncShift);
on('page', syncShift);

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
showPage(routeOf(location.hash) || 'brain');
renderAlt();

// ---------- the demo brain (GET/POST /api/demo) ----------
const demoBanner = h('div.demo-banner.no-orbit', { role: 'status', hidden: true },
  h('span', 'this is the demo brain'), h('span.soft', ' · '),
  h('button.textbtn.strong', { type: 'button', onclick: () => ctx.setDemo(false) }, 'return to yours'));
document.getElementById('main').append(demoBanner);
let demoInfo = null;
function renderDemo() {
  demoBanner.hidden = !store.demo;
  app.classList.toggle('in-demo', store.demo);
  emit('demo', demoInfo);
}
ctx.setDemo = async (on) => {
  try {
    demoInfo = await api.setDemo(on);
    store.demo = !!demoInfo.active;
  } catch (e) {
    if (e.status === 404 || e.status === 405) {
      // a server without /api/demo: fall back to the bundled fixture, read-only
      store.demo = on;
      if (on) await loadCloud(new URL('./mock/cloud.json', import.meta.url).href); else await loadCloud().catch(() => {});
      renderDemo();
      return;
    }
    ctx.toast(e.serverMessage || (on ? 'the demo brain could not be opened.' : 'your brain could not be reached.'));
    return;
  }
  try { await loadCloud(); } catch { ctx.toast('the brain could not be read.'); }
  emit('brain-switched', store.demo);
  // panels that fetched their data once on mount read the other brain afresh
  for (const k of ['threads', 'review', 'eval', 'skills']) { const el = layout.mounted.get(k); if (el) { el.remove(); layout.mounted.delete(k); } }
  layout.render();
  renderDemo();
  ctx.toast(store.demo ? 'the demo brain. your own is untouched.' : 'back to your brain.');
};
api.demo().then((d) => { demoInfo = d; store.demo = !!d.active; renderDemo(); }).catch(() => { renderDemo(); });

// settings: a sheet over the stage. connections, sources and privacy talk to the server
// (panels/connections.js); appearance is ui/theme's settings panel.
let settingsSheet = null;
const SETTINGS_TABS = [['connections', 'connections'], ['sources', 'sources'], ['privacy', 'privacy'], ['appearance', 'appearance']];
let settingsTab = 'connections';
async function openSettings(tab) {
  if (typeof tab === 'string' && SETTINGS_TABS.some(([k]) => k === tab)) settingsTab = tab;
  if (!settingsSheet) {
    const close = h('button.textbtn.sheet-close', { type: 'button', onclick: () => { settingsSheet.hidden = true; } }, 'close');
    const tabs = h('div.sheet-tabs', { role: 'tablist', 'aria-label': 'settings sections' });
    const body = h('div.sheet-body');
    settingsSheet = h('aside.sheet.no-orbit', { role: 'dialog', 'aria-modal': 'false', 'aria-label': 'settings', tabindex: '-1' },
      h('div.panel-head', h('h2', 'settings'), h('div.panel-tools', close)), tabs, body);
    settingsSheet.addEventListener('keydown', (e) => { if (e.key === 'Escape') { e.stopPropagation(); settingsSheet.hidden = true; } });
    tabs.addEventListener('keydown', (e) => {
      const list = [...tabs.children]; const k = list.indexOf(document.activeElement);
      if (k < 0) return;
      const n = e.key === 'ArrowRight' ? (k + 1) % list.length : e.key === 'ArrowLeft' ? (k - 1 + list.length) % list.length : -1;
      if (n >= 0) { e.preventDefault(); list[n].focus(); list[n].click(); }
    });
    settingsSheet.renderTabs = () => {
      clear(tabs);
      for (const [k, label] of SETTINGS_TABS) tabs.append(h('button', { type: 'button', role: 'tab', id: 'set-tab-' + k, 'aria-selected': String(k === settingsTab), tabindex: k === settingsTab ? '0' : '-1', onclick: () => { settingsTab = k; showTab(); } }, label));
    };
    settingsSheet.body = body;
    document.getElementById('main').append(settingsSheet);
  }
  settingsSheet.hidden = false;
  await showTab();
  settingsSheet.focus({ preventScroll: true });
}
async function showTab() {
  settingsSheet.renderTabs();
  const body = clear(settingsSheet.body);
  body.setAttribute('role', 'tabpanel');
  body.setAttribute('aria-labelledby', 'set-tab-' + settingsTab);
  if (settingsTab === 'appearance') {
    if (!themeApi) { body.append(h('p.empty', 'appearance settings come with the theme package, which is not installed.')); return; }
    const { mountSettings } = await import('../theme/settings.js');
    await mountSettings(body, { onLayoutChange: (l) => { if (l && l.panels) { layout.config = l; layout.render(); } } });
    return;
  }
  const m = await import('./panels/connections.js');
  const fn = { connections: m.mountConnections, sources: m.mountSources, privacy: m.mountPrivacy }[settingsTab];
  await fn(body, ctx);
}
window.addEventListener('anamnesis:opensettings', (e) => openSettings(e.detail?.tab));
ctx.openSettings = openSettings;

// first launch: ask once whether to keep the motion (the theme sets --motion / data-motion)
if (themeApi && !params.has('nomotionask')) {
  import('../theme/motion.js').then((m) => m.askMotionOnce()).then(() => refreshTheme()).catch((e) => console.warn('motion dialog', e));
}
document.documentElement.classList.toggle('reduced', theme().motion === 0);
document.body.dataset.ready = '1';
