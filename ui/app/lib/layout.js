// Builds the docks from a layout config (see layout-default.js for the shape).
// Users can move a panel between docks or hide it; the result is saved in
// localStorage and exposed as window.anamnesis.layout for the settings ui.
import { DEFAULT_LAYOUT } from '../layout-default.js';
import { h, clear } from './dom.js';
import { emit } from './store.js';

const KEY = 'anamnesis.layout.v1';
const AREAS = ['right', 'bottom', 'hidden'];

export async function loadLayout() {
  let base = DEFAULT_LAYOUT;
  try {
    const r = await fetch(new URL('../../theme/layouts/default.json', import.meta.url));
    if (r.ok) { const j = await r.json(); if (j && Array.isArray(j.panels)) base = j; }
  } catch { /* the theme package may not be there yet */ }
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem(KEY) || 'null'); } catch { saved = null; }
  const merged = structuredClone(base);
  if (saved && Array.isArray(saved.panels)) {
    for (const sp of saved.panels) {
      const p = merged.panels.find((x) => x.id === sp.id);
      if (p) Object.assign(p, sp);
    }
  }
  // any known panel missing from a theme's layout still exists, hidden
  for (const dp of DEFAULT_LAYOUT.panels) if (!merged.panels.find((p) => p.id === dp.id)) merged.panels.push({ ...dp, area: 'hidden', open: false });
  return merged;
}

export class Layout {
  constructor(config, registry, ctx) {
    this.config = config;
    this.registry = registry;   // id -> { title, mount(el, ctx), badge?() }
    this.ctx = ctx;
    this.mounted = new Map();   // id -> element
    this.docks = {
      left: document.getElementById('dock-left'),
      right: document.getElementById('dock-right'),
      bottom: document.getElementById('dock-bottom'),
    };
    this.slots = { 'bottom-left': document.getElementById('slot-bottom-left'), bottom: document.getElementById('slot-bottom') };
  }
  save() {
    try { localStorage.setItem(KEY, JSON.stringify({ panels: this.config.panels.map(({ id, area, order, open, slot }) => ({ id, area, order, open, slot })) })); } catch { /* private mode */ }
  }
  panel(id) { return this.config.panels.find((p) => p.id === id); }
  inArea(area) { return this.config.panels.filter((p) => p.area === area && this.registry[p.id]).sort((a, b) => (a.order ?? 0) - (b.order ?? 0)); }

  // Mount once, then move the element around: panels keep their state.
  el(id) {
    if (!this.mounted.has(id)) {
      const def = this.registry[id];
      const body = h('div.panel-body');
      const wrap = h(`div.panel.panel-${id}`, { 'data-panel': id }, body);
      this.mounted.set(id, wrap);
      def.mount(body, this.ctx);
    }
    return this.mounted.get(id);
  }

  render() {
    // left: the sidebar (and anything else placed there) stacked
    const left = this.docks.left;
    for (const p of this.inArea('left')) if (!left.contains(this.el(p.id))) left.append(this.el(p.id));
    // overlays
    for (const p of this.config.panels.filter((x) => x.area === 'overlay' && this.registry[x.id])) {
      const slot = this.slots[p.slot] || this.slots.bottom;
      const el = this.el(p.id);
      if (el.parentElement !== slot) slot.append(el);
      el.hidden = p.open === false;
    }
    this.renderDock('right');
    this.renderDock('bottom');
    for (const p of this.config.panels) if (p.area === 'hidden' && this.mounted.has(p.id)) this.mounted.get(p.id).remove();
    emit('layout', this.config);
  }

  renderDock(area) {
    const dock = this.docks[area];
    const panels = this.inArea(area);
    clear(dock);
    dock.hidden = panels.length === 0;
    if (!panels.length) return;
    let open = panels.find((p) => p.open);
    const tabs = h('div.dock-tabs', { role: 'tablist', 'aria-label': area === 'right' ? 'panels' : 'time panels' });
    const body = h('div.dock-body');
    for (const p of panels) {
      const def = this.registry[p.id];
      const tab = h('button', {
        type: 'button', role: 'tab', id: `tab-${p.id}`, 'aria-selected': String(p === open), 'aria-controls': `tabpanel-${p.id}`,
        tabindex: p === open || (!open && p === panels[0]) ? '0' : '-1',
        onclick: () => this.toggle(p.id),
      }, def.tab || def.title);
      const b = def.badge && def.badge();
      if (b) tab.append(h('span.badge', String(b)));
      tabs.append(tab);
    }
    tabs.addEventListener('keydown', (e) => {
      const list = [...tabs.querySelectorAll('[role=tab]')];
      const k = list.indexOf(document.activeElement);
      if (k < 0) return;
      let n = -1;
      if (e.key === 'ArrowRight' || e.key === 'ArrowDown') n = (k + 1) % list.length;
      if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') n = (k - 1 + list.length) % list.length;
      if (n >= 0) { e.preventDefault(); list[n].focus(); }
    });
    dock.append(tabs, body);
    if (open) {
      const def = this.registry[open.id];
      const el = this.el(open.id);
      const head = h('div.panel-head',
        h('h2', def.title),
        h('div.panel-tools',
          h('button', { type: 'button', title: 'move this panel', onclick: () => this.move(open.id) }, area === 'right' ? 'move down' : 'move right'),
          h('button', { type: 'button', title: 'hide this panel (restore from more)', onclick: () => this.hide(open.id) }, 'hide')));
      const tp = h('div', { role: 'tabpanel', id: `tabpanel-${open.id}`, 'aria-labelledby': `tab-${open.id}` }, head, el);
      body.append(tp);
      this.registry[open.id].shown?.(this.ctx);
    }
    dock.classList.toggle('open', !!open);
    dock.classList.toggle('collapsed', !open && area === 'right');
  }

  toggle(id, force) {
    const p = this.panel(id);
    if (!p) return;
    if (p.area === 'hidden') { p.area = 'right'; }
    const want = force ?? !p.open;
    if (p.area === 'overlay') { p.open = want; this.save(); this.render(); return; }
    for (const q of this.inArea(p.area)) q.open = false;
    p.open = want;
    this.save();
    this.render();
    if (want) requestAnimationFrame(() => document.getElementById(`tab-${id}`)?.focus({ preventScroll: true }));
  }
  open(id) { this.toggle(id, true); }
  move(id) {
    const p = this.panel(id);
    const next = p.area === 'right' ? 'bottom' : 'right';
    for (const q of this.inArea(next)) q.open = false;
    p.area = next; p.open = true;
    this.save(); this.render();
  }
  hide(id) { const p = this.panel(id); p.area = 'hidden'; p.open = false; this.save(); this.render(); this.ctx.toast(`${this.registry[id].title} hidden. bring it back from more → layout.`); }
  show(id, area = 'right') { const p = this.panel(id); if (p.area === 'hidden') p.area = area; this.toggle(id, true); }
  reset() { try { localStorage.removeItem(KEY); } catch { /* */ } this.config = structuredClone(DEFAULT_LAYOUT); this.render(); }
  set(config) { if (config && Array.isArray(config.panels)) { this.config = config; this.save(); this.render(); } }
  get() { return structuredClone(this.config); }
}
export { AREAS };
