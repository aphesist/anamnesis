// A dom list that mirrors the brain for screen readers and keyboard users:
// regions (with counts) and whatever is currently lit, as real buttons.
import { h, clear, fmtDate } from './dom.js';
import { store, on, LIT, BIT, indicesWith } from './store.js';

export function mountMirror(el, ctx) {
  const regions = h('ul', { 'aria-label': 'regions' });
  const litHead = h('h3', { style: { fontWeight: 'normal', fontSize: '11px', letterSpacing: '0.12em' } }, 'lit');
  const lit = h('ul', { 'aria-label': 'lit items' });
  const status = h('p.sr-only', { 'aria-live': 'polite' });
  el.append(h('h2', { style: { fontWeight: 'normal', fontSize: '13px' } }, 'the brain as a list'), regions, litHead, lit, status);
  const renderRegions = () => {
    clear(regions);
    if (!store.cloud) return;
    store.cloud.regions.forEach((r, k) => regions.append(h('li', h('button', { type: 'button', onclick: () => ctx.lightRegion(k) }, `${r.label} (${r.anatomy || 'no anatomy'}) — ${r.size} items`))));
  };
  let queued = false;
  const renderLit = () => {
    if (queued) return; queued = true;
    setTimeout(() => {
      queued = false;
      clear(lit);
      const idx = indicesWith(LIT & ~BIT.FOCUS);
      litHead.textContent = idx.length ? `lit: ${store.litLabel || ''} (${idx.length})` : 'nothing lit';
      for (const i of idx.slice(0, 200)) lit.append(h('li', h('button', { type: 'button', onclick: () => ctx.open(i) }, `${store.cloud.titles[i]} — ${store.cloud.kinds[store.cloud.kind[i]]}, ${fmtDate(store.cloud.t[i])}`)));
      if (idx.length > 200) lit.append(h('li', `and ${idx.length - 200} more`));
      status.textContent = idx.length ? `${idx.length} items lit${store.litLabel ? ': ' + store.litLabel : ''}` : '';
    }, 250);
  };
  on('cloud-ready', renderRegions);
  on('flags', renderLit);
}
