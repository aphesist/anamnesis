// Builds the panels from the layout config owned by the theming package
// (ui/theme/layout.js, schema in ui/theme/layouts/layout.schema.json):
//   { panels: [{ id, label, region: left|right|bottom|overlay, order, open, visible, anchor?, size? }] }
// and rebuilds on `anamnesis:layoutchange`. Without the theme package it falls
// back to the embedded default in ../layout-default.js (same schema).
//
// Where things go:
//   left                       -> the sidebar column (only the nav lives there)
//   right                      -> the tabbed right dock (one open at a time)
//   bottom                     -> the time scrubber sits on the stage; anything else in the bottom dock
//   overlay, anchor bottom-left-> the corner slot on the brain (the legend)
//   overlay, other anchors     -> a floating sheet over the stage (right / left / center), one at a time
// Floating panels also get a tab in the right dock so they can be found.
import { DEFAULT_LAYOUT } from '../layout-default.js';
import { h, clear } from './dom.js';
import { emit } from './store.js';

// theme panel ids -> the app's short ids (css classes, registry keys)
const ALIAS = { 'sidebar-left': 'sidebar', 'view-right': 'view', 'legend-bottom-left': 'legend', 'when-you-wrote': 'when', 'timeline-bottom': 'time' };
const FULL = Object.fromEntries(Object.entries(ALIAS).map(([a, b]) => [b, a]));
export const shortId = (id) => ALIAS[id] || id;
export const fullId = (id) => FULL[id] || id;
const FALLBACK_KEY = 'anamnesis.layout';

let themeLayout = null; // the theming package's layout module, when present
export async function loadLayout() {
  try { themeLayout = await import('../../theme/layout.js'); } catch { themeLayout = null; }
  if (themeLayout) {
    try { return await themeLayout.getLayout(); } catch { /* fall through */ }
  }
  try { const s = JSON.parse(localStorage.getItem(FALLBACK_KEY) || 'null'); if (s && Array.isArray(s.panels)) return s; } catch { /* */ }
  return structuredClone(DEFAULT_LAYOUT);
}

export class Layout {
  constructor(config, registry, ctx) {
    this.config = config;
    this.registry = registry; // short id -> { title, tab?, mount(el, ctx), shown?, badge? }
    this.ctx = ctx;
    this.mounted = new Map();
    this.docks = { left: document.getElementById('dock-left'), right: document.getElementById('dock-right'), bottom: document.getElementById('dock-bottom') };
    this.slots = { 'bottom-left': document.getElementById('slot-bottom-left'), bottom: document.getElementById('slot-bottom') };
    this.float = document.getElementById('float');
    if (themeLayout) themeLayout.onLayoutChange((l) => { this.config = l; this.render(); });
  }

  // ---------- model ----------
  panel(id) { return this.config.panels.find((p) => shortId(p.id) === shortId(id)); }
  ensure(id) {
    let p = this.panel(id);
    if (!p) { p = { id: fullId(id), label: this.registry[shortId(id)]?.title || id, region: 'right', order: 99, open: false, visible: true }; this.config.panels.push(p); }
    return p;
  }
  place(p) {
    const s = shortId(p.id);
    if (!this.registry[s] || p.visible === false) return 'hidden';
    if (s === 'sidebar') return 'left';
    if (s === 'time' && (p.region === 'bottom' || p.region === 'overlay')) return 'slot-bottom';
    if (p.region === 'overlay') return p.anchor === 'bottom-left' || p.anchor === 'bottom-right' ? 'slot-bottom-left' : 'float';
    if (p.region === 'bottom') return 'bottom';
    return 'right'; // right, and left for anything but the nav
  }
  inPlace(place) { return this.config.panels.filter((p) => this.place(p) === place).sort((a, b) => (a.order ?? 0) - (b.order ?? 0)); }
  commit() {
    if (themeLayout) { themeLayout.saveLayout(this.config, 'app'); return; } // the change event re-renders
    try { localStorage.setItem(FALLBACK_KEY, JSON.stringify(this.config)); } catch { /* */ }
    this.render();
  }

  el(id) {
    const s = shortId(id);
    if (!this.mounted.has(s)) {
      const body = h('div.panel-body');
      const wrap = h(`div.panel.panel-${s}`, { 'data-panel': s }, body);
      this.mounted.set(s, wrap);
      this.registry[s].mount(body, this.ctx);
    }
    return this.mounted.get(s);
  }
  title(p) { const d = this.registry[shortId(p.id)]; return d?.title || p.label || p.id; }
  tabText(p) { const d = this.registry[shortId(p.id)]; return d?.tab || p.label || d?.title || p.id; }

  // ---------- render ----------
  render() {
    const left = this.docks.left;
    for (const p of this.inPlace('left')) if (!left.contains(this.el(p.id))) left.append(this.el(p.id));
    for (const [place, slot] of [['slot-bottom-left', this.slots['bottom-left']], ['slot-bottom', this.slots.bottom]]) {
      for (const p of this.config.panels.filter((x) => this.place(x) === place)) {
        const el = this.el(p.id);
        if (el.parentElement !== slot) slot.append(el);
        el.hidden = p.open === false;
      }
    }
    this.renderDock('right');
    this.renderDock('bottom');
    this.renderFloat();
    for (const p of this.config.panels) if (this.place(p) === 'hidden' && this.mounted.has(shortId(p.id))) this.mounted.get(shortId(p.id)).remove();
    emit('layout', this.config);
  }

  head(p, place) {
    const moveLabel = place === 'right' ? 'move down' : place === 'bottom' ? 'float' : 'dock right';
    return h('div.panel-head',
      h('h2', this.title(p)),
      h('div.panel-tools',
        h('button', { type: 'button', title: 'move this panel', onclick: () => this.move(p.id) }, moveLabel),
        h('button', { type: 'button', title: place === 'float' ? 'close' : 'hide this panel (restore from more)', onclick: () => (place === 'float' ? this.toggle(p.id, false) : this.hide(p.id)) }, place === 'float' ? 'close' : 'hide')));
  }

  renderDock(area) {
    const dock = this.docks[area];
    const panels = this.inPlace(area);
    const floats = area === 'right' ? this.inPlace('float') : [];
    clear(dock);
    dock.hidden = panels.length + floats.length === 0;
    if (dock.hidden) return;
    const open = panels.find((p) => p.open);
    const tabs = h('div.dock-tabs', { role: 'tablist', 'aria-label': area === 'right' ? 'panels' : 'lower panels' });
    const body = h('div.dock-body');
    for (const p of [...panels, ...floats]) {
      const isFloat = floats.includes(p);
      const sel = isFloat ? !!p.open : p === open;
      const tab = h('button' + (isFloat ? '.floating' : ''), {
        type: 'button', role: 'tab', id: `tab-${shortId(p.id)}`, 'aria-selected': String(sel), 'aria-controls': `tabpanel-${shortId(p.id)}`,
        tabindex: sel || (!open && p === panels[0]) ? '0' : '-1', onclick: () => this.toggle(p.id),
      }, this.tabText(p));
      const b = this.registry[shortId(p.id)].badge?.();
      if (b) tab.append(h('span.badge', String(b)));
      tabs.append(tab);
    }
    tabs.addEventListener('keydown', (e) => {
      const list = [...tabs.querySelectorAll('[role=tab]')];
      const k = list.indexOf(document.activeElement);
      if (k < 0) return;
      const n = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? (k + 1) % list.length : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? (k - 1 + list.length) % list.length : -1;
      if (n >= 0) { e.preventDefault(); list[n].focus(); }
    });
    dock.append(tabs, body);
    if (open) {
      body.append(h('div', { role: 'tabpanel', id: `tabpanel-${shortId(open.id)}`, 'aria-labelledby': `tab-${shortId(open.id)}` }, this.head(open, area), this.el(open.id)));
      this.registry[shortId(open.id)].shown?.(this.ctx);
    }
    dock.classList.toggle('open', !!open);
    dock.classList.toggle('collapsed', !open && area === 'right');
    if (area === 'right') dock.style.width = open && !dock.classList.contains('collapsed') ? Math.max(300, Math.min(560, open.size || 340)) + 'px' : '';
    if (!dock.classList.contains('collapsed')) {
      fitTabs(tabs);
      if (!this.tabObserver && typeof ResizeObserver !== 'undefined') {
        this.tabObserver = new ResizeObserver(() => { const t = dock.querySelector('.dock-tabs'); if (t && !dock.classList.contains('collapsed')) fitTabs(t); });
        this.tabObserver.observe(dock);
      }
    }
  }

  renderFloat() {
    const f = this.float;
    const open = this.inPlace('float').find((p) => p.open);
    clear(f);
    f.hidden = !open;
    if (!open) return;
    f.dataset.anchor = open.anchor || 'right';
    f.style.setProperty('--float-size', (open.size || (open.anchor === 'center' ? 680 : 420)) + 'px');
    f.append(h('div', { role: 'dialog', 'aria-modal': 'false', 'aria-label': this.title(open), id: `tabpanel-${shortId(open.id)}` }, this.head(open, 'float'), this.el(open.id)));
    this.registry[shortId(open.id)].shown?.(this.ctx);
  }

  // ---------- actions ----------
  toggle(id, force) {
    const p = this.ensure(id);
    if (p.visible === false) p.visible = true;
    const place = this.place(p);
    const want = force ?? !p.open;
    if (place === 'right' || place === 'bottom' || place === 'float') for (const q of this.inPlace(place)) q.open = false;
    p.open = want;
    const tabFocused = document.activeElement?.getAttribute('role') === 'tab';
    this.commit();
    if (!themeLayout) { /* rendered synchronously in commit */ }
    if (tabFocused) requestAnimationFrame(() => document.getElementById(`tab-${shortId(id)}`)?.focus({ preventScroll: true }));
  }
  open(id) { this.toggle(id, true); }
  move(id) {
    const p = this.ensure(id);
    const place = this.place(p);
    if (place === 'right') p.region = 'bottom';
    else if (place === 'bottom') { p.region = 'overlay'; p.anchor = p.anchor && p.anchor !== 'bottom-left' ? p.anchor : 'center'; }
    else p.region = 'right';
    for (const q of this.inPlace(this.place(p))) q.open = false;
    p.open = true;
    this.commit();
  }
  hide(id) { const p = this.ensure(id); p.visible = false; p.open = false; this.commit(); this.ctx.toast(`${this.title(p)} hidden. bring it back from more → layout.`); }
  setVisible(id, v) { const p = this.ensure(id); p.visible = v; if (v && this.place(p) !== 'slot-bottom-left' && this.place(p) !== 'slot-bottom') { for (const q of this.inPlace(this.place(p))) q.open = false; } p.open = v; this.commit(); }
  async reset() {
    if (themeLayout) { await themeLayout.resetLayout('default'); return; }
    try { localStorage.removeItem(FALLBACK_KEY); } catch { /* */ }
    this.config = structuredClone(DEFAULT_LAYOUT); this.render();
  }
  set(config) { if (config && Array.isArray(config.panels)) { this.config = config; this.commit(); } }
  get() { return structuredClone(this.config); }
}

// Keep a tab strip on one line: tabs that do not fit move into a "more" menu at the end
// (the selected tab always stays visible). Re-run on resize.
export function fitTabs(tabs) {
  if (!tabs || !tabs.isConnected) return;
  tabs.querySelector('.tab-more')?.remove();
  tabs.querySelector('.tab-menu')?.remove();
  const all = [...tabs.querySelectorAll('[role=tab]')];
  for (const t of all) t.hidden = false;
  if (tabs.scrollWidth <= tabs.clientWidth + 1) return;
  const more = h('button.tab-more', { type: 'button', 'aria-haspopup': 'true', 'aria-expanded': 'false', title: 'more panels' }, '···');
  tabs.append(more);
  const hidden = [];
  for (let k = all.length - 1; k >= 0 && tabs.scrollWidth > tabs.clientWidth + 1; k--) {
    if (all[k].getAttribute('aria-selected') === 'true') continue;
    all[k].hidden = true;
    hidden.unshift(all[k]);
  }
  if (!hidden.length) { more.remove(); return; }
  more.dataset.count = String(hidden.length);
  if (hidden.some((t) => t.querySelector('.badge'))) more.classList.add('has-badge');
  const menu = h('div.tab-menu', { role: 'menu', hidden: true });
  for (const t of hidden) menu.append(h('button', { type: 'button', role: 'menuitem', onclick: () => { menu.hidden = true; t.click(); } }, t.textContent));
  const close = () => { menu.hidden = true; more.setAttribute('aria-expanded', 'false'); };
  more.addEventListener('click', () => { menu.hidden = !menu.hidden; more.setAttribute('aria-expanded', String(!menu.hidden)); if (!menu.hidden) menu.querySelector('button')?.focus(); });
  menu.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { e.stopPropagation(); close(); more.focus(); }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { const b = [...menu.children]; const k = b.indexOf(document.activeElement); b[(k + (e.key === 'ArrowDown' ? 1 : b.length - 1)) % b.length]?.focus(); e.preventDefault(); }
  });
  menu.addEventListener('focusout', (e) => { if (!menu.contains(e.relatedTarget) && e.relatedTarget !== more) close(); });
  tabs.append(menu);
}
