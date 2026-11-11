// Small multiples: one small brain (or timeline) per area — an accessible
// overview of the same selection. Enter or click lights that area.
import { h, clear } from '../lib/dom.js';
import { store, LIT, BIT } from '../lib/store.js';
import { theme, rgba } from '../lib/tokens.js';
import { setup, stippleRect } from '../lib/stipple.js';

let mode = 'brains';
export default {
  mount(el, ctx) {
    const choice = h('div.choice', { role: 'radiogroup', 'aria-label': 'draw each area as' });
    const grid = h('div.multiples', { role: 'list' });
    el.append(h('h3', 'grid'), choice, h('div', { style: { height: '10px' } }), grid);
    const sync = () => clear(choice).append(...[['brains', 'small brains'], ['timelines', 'timelines']].map(([v, l]) => h('button', { type: 'button', role: 'radio', 'aria-checked': String(mode === v), onclick: () => { mode = v; sync(); build(); } }, l)));
    sync();
    let figs = [];
    const build = () => {
      clear(grid); figs = [];
      if (!store.cloud) return;
      const c = store.cloud;
      const groups = c.regions.map((r, k) => ({ r: k, label: r.label, anat: r.anatomy, idx: [] }));
      const unsorted = { r: -1, label: 'unsorted', anat: '', idx: [] };
      for (let i = 0; i < store.n; i++) (c.region[i] >= 0 ? groups[c.region[i]] : unsorted).idx.push(i);
      const all = [...groups, unsorted].filter((g) => g.idx.length).sort((a, b) => b.idx.length - a.idx.length);
      for (const g of all) {
        const canvas = h('canvas', { 'aria-hidden': 'true' });
        const lit = h('span.data');
        const fig = h('figure', { tabindex: '0', role: 'listitem', 'aria-label': `${g.label}, ${g.idx.length} items` }, canvas, h('figcaption', h('span', g.label), lit));
        const act = () => (g.r >= 0 ? ctx.lightRegion(g.r, { fly: false }) : ctx.light(g.idx, 'unsorted', { fly: false }));
        fig.addEventListener('click', act);
        fig.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); act(); } });
        grid.append(fig);
        figs.push({ g, canvas, lit });
      }
      requestAnimationFrame(update);
    };
    const update = () => {
      const col = theme().colors, f = store.flags, A = ctx.brain.A, c = store.cloud;
      for (const { g, canvas, lit } of figs) {
        const { g: x, w, h: H } = setup(canvas);
        let nLit = 0;
        for (const i of g.idx) if (f[i] & LIT) nLit++;
        lit.textContent = nLit ? `${nLit}/${g.idx.length}` : String(g.idx.length);
        if (mode === 'brains' && A) {
          // side view of the whole brain, faint, with this area drawn on top
          x.fillStyle = rgba(col['dot-dim'], 0.5);
          for (let i = 0; i < store.n; i += 7) x.fillRect(w / 2 - A[i * 3 + 2] * w * 0.27, H / 2 - A[i * 3 + 1] * H * 0.33, 0.7, 0.7);
          for (const i of g.idx) {
            const on = f[i] & LIT;
            if (f[i] & BIT.FILTERED) continue;
            x.fillStyle = on ? rgba(col['dot-hot'], 1) : rgba(col.dot, nLit ? 0.35 : 0.85);
            const r = on ? 1.4 : 0.9;
            x.fillRect(w / 2 - A[i * 3 + 2] * w * 0.27 - r / 2, H / 2 - A[i * 3 + 1] * H * 0.33 - r / 2, r, r);
          }
        } else {
          const B = 36, all = new Uint16Array(B), on = new Uint16Array(B);
          for (const i of g.idx) { const k = Math.min(B - 1, Math.floor(store.tNorm[i] * B)); all[k]++; if (f[i] & LIT) on[k]++; }
          const max = Math.max(1, ...all);
          for (let k = 0; k < B; k++) {
            const hh = (H - 8) * all[k] / max, lh = (H - 8) * on[k] / max;
            stippleRect(x, (k / B) * w, H - 4 - hh, w / B, hh, 0.4, rgba(col.dot, 0.9), k + g.r * 50 + 60, 1.8, 0.8);
            if (lh) stippleRect(x, (k / B) * w, H - 4 - lh, w / B, lh, 0.95, rgba(col['dot-hot'], 1), k + 7, 1.6, 1);
          }
        }
      }
      void c;
    };
    build();
    this._update = update;
  },
  update() { this._update?.(); },
};
