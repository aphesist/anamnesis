// Histogram: a precise 2d companion to the brain. Counts by time, area or kind,
// with the current selection drawn hot on top. Click a bar to light it.
import { h, clear } from '../lib/dom.js';
import { store, LIT, BIT, kindOf, regionOf } from '../lib/store.js';
import { theme, rgba } from '../lib/tokens.js';
import { setup, stippleRect } from '../lib/stipple.js';

let by = 'time';
export default {
  mount(el, ctx) {
    const choice = h('div.choice', { role: 'radiogroup', 'aria-label': 'count by' });
    const canvas = h('canvas', { tabindex: '0', role: 'img', 'aria-label': 'histogram of the selection' });
    const labels = h('div.meta');
    const table = h('table.sr-only');
    el.append(h('h3', 'histogram'), choice, h('div.hist', canvas), labels, table);
    const sync = () => clear(choice).append(...[['time', 'by time'], ['area', 'by area'], ['kind', 'by kind']].map(([v, l]) => h('button', { type: 'button', role: 'radio', 'aria-checked': String(by === v), onclick: () => { by = v; sync(); draw(); } }, l)));
    sync();
    let buckets = [], cursor = 0;
    const draw = () => {
      if (!store.cloud) return;
      const c = store.cloud, n = store.n, f = store.flags;
      const keyOf = by === 'time' ? (i) => Math.min(47, Math.floor(store.tNorm[i] * 48)) : by === 'area' ? (i) => (c.region[i] < 0 ? c.regions.length : c.region[i]) : (i) => c.kind[i];
      const names = by === 'time' ? Array.from({ length: 48 }, (_, k) => { const d = new Date((store.tMin + (k / 48) * (store.tMax - store.tMin)) * 1000); return d.toISOString().slice(0, 7); }) : by === 'area' ? [...c.regions.map((r) => r.label), 'unsorted'] : c.kinds;
      buckets = names.map((name) => ({ name, all: [], lit: 0 }));
      for (let i = 0; i < n; i++) { if (f[i] & BIT.FILTERED) continue; const b = buckets[keyOf(i)]; if (!b) continue; b.all.push(i); if (f[i] & LIT) b.lit++; }
      if (by !== 'time') buckets = buckets.filter((b) => b.all.length).sort((a, b) => b.all.length - a.all.length);
      const { g, w, h: H } = setup(canvas);
      const col = theme().colors;
      const max = Math.max(1, ...buckets.map((b) => b.all.length));
      const bw = (w - 8) / buckets.length;
      buckets.forEach((b, k) => {
        const hh = (H - 40) * (b.all.length / max), lh = (H - 40) * (b.lit / max);
        stippleRect(g, 4 + k * bw + bw * 0.1, H - 30 - hh, bw * 0.8, hh, 0.35, rgba(col.dot, 0.9), k + 3, 2, 0.9);
        if (lh) stippleRect(g, 4 + k * bw + bw * 0.1, H - 30 - lh, bw * 0.8, lh, 0.95, rgba(col['dot-hot'], 1), k + 99, 1.8, 1.1);
        if (k === cursor && document.activeElement === canvas) { g.strokeStyle = rgba(col.accent, 0.9); g.strokeRect(4 + k * bw + 1, 4, bw - 2, H - 34); }
        g.fillStyle = rgba(col['ink-soft'], 0.9); g.font = '9px ' + getComputedStyle(canvas).fontFamily;
        const every = by === 'time' ? 6 : 1;
        if (k % every === 0) { g.save(); g.translate(4 + k * bw + bw / 2, H - 26); if (by !== 'time') g.rotate(0.5); g.fillText(b.name.slice(0, 18), 0, 8); g.restore(); }
      });
      const litTotal = buckets.reduce((a, b) => a + b.lit, 0);
      labels.textContent = litTotal ? `${litTotal} lit of ${buckets.reduce((a, b) => a + b.all.length, 0)} shown` : 'nothing lit — click a bar to light it';
      clear(table).append(h('caption', 'counts'), ...buckets.map((b) => h('tr', h('th', b.name), h('td', String(b.all.length)), h('td', `${b.lit} lit`))));
    };
    const select = (k) => { const b = buckets[k]; if (b) ctx.light(b.all, `${by}: ${b.name}`, { fly: false }); };
    canvas.addEventListener('click', (e) => { const r = canvas.getBoundingClientRect(); const k = Math.floor((e.clientX - r.left - 4) / ((r.width - 8) / buckets.length)); cursor = k; select(k); });
    canvas.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowRight') cursor = Math.min(buckets.length - 1, cursor + 1);
      else if (e.key === 'ArrowLeft') cursor = Math.max(0, cursor - 1);
      else if (e.key === 'Enter' || e.key === ' ') select(cursor);
      else return;
      e.preventDefault(); draw();
    });
    canvas.addEventListener('focus', draw);
    this._draw = draw;
    requestAnimationFrame(draw);
    void kindOf; void regionOf;
  },
  update() { this._draw?.(); },
};
