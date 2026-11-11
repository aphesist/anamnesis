// Bottom-left legend: "function (anatomy)" — regions with their ornamental
// anatomical names, content labels and counts. Clicking lights the region.
import { h, clear } from '../lib/dom.js';
import { store, on, BIT } from '../lib/store.js';
import { areaColor } from '../lib/tokens.js';

export default {
  id: 'legend',
  title: 'function (anatomy)',
  mount(el, ctx) {
    el.classList.add('legend');
    const list = h('ol', { 'aria-label': 'regions' });
    el.append(h('h3', 'function ', h('small', '(anatomy)')), list, h('p.note', 'names are ornament; nearness is meaning.'));
    let active = -1;
    const render = () => {
      clear(list);
      if (!store.cloud) return;
      const c = store.cloud;
      const counts = new Uint32Array(c.regions.length);
      let unsorted = 0;
      for (let i = 0; i < store.n; i++) { const r = c.region[i]; if (r >= 0) counts[r]++; else unsorted++; }
      const order = c.regions.map((_, r) => r).sort((a, b) => counts[b] - counts[a]);
      for (const r of order) {
        const reg = c.regions[r];
        const b = h('button', { type: 'button', 'aria-pressed': String(active === r), title: `light ${reg.label}` },
          h('span.sw', { style: { color: areaColor(r) }, 'aria-hidden': 'true' }),
          h('span', reg.label, h('span.anat', reg.anatomy || '')),
          h('span.count', String(counts[r])));
        b.addEventListener('click', () => {
          if (active === r) { active = -1; ctx.clearLight(); render(); return; }
          active = r; ctx.lightRegion(r); render();
        });
        b.addEventListener('mouseenter', () => ctx.brain.setGlow(r, 0.85));
        b.addEventListener('mouseleave', () => ctx.brain.setGlow(r, 0));
        b.addEventListener('focus', () => ctx.brain.setGlow(r, 0.85));
        b.addEventListener('blur', () => ctx.brain.setGlow(r, 0));
        list.append(h('li', b));
      }
      if (unsorted) list.append(h('li', h('button', { type: 'button', onclick: () => {
        const idx = []; for (let i = 0; i < store.n; i++) if (c.region[i] < 0) idx.push(i);
        active = -2; ctx.light(idx, `unsorted · ${idx.length}`);
      } }, h('span.sw', { 'aria-hidden': 'true' }), h('span', 'unsorted', h('span.anat', 'waiting')), h('span.count', String(unsorted)))));
    };
    on('cloud-ready', render);
    if (store.cloud) render();
    on('lit-cleared', () => { if (active !== -1) { active = -1; render(); } });
  },
};
