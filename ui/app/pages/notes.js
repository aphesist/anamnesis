// Notes: a filterable list and the reader (with provenance and link suggestions).
import { h, clear, fmtDate } from '../lib/dom.js';
import { store, on, kindOf, regionOf, SPEAKERS, showBucket } from '../lib/store.js';
import { renderReader } from '../lib/reader.js';

const LIMIT = 400;
export default {
  mount(el, ctx) {
    const q = h('input', { type: 'search', placeholder: 'filter by title', 'aria-label': 'filter notes by title' });
    const kind = h('select.quiet', { 'aria-label': 'kind' });
    const who = h('select.quiet', { 'aria-label': 'who wrote it' }, h('option', { value: '' }, 'anyone'), ...SPEAKERS.map((s, k) => h('option', { value: k }, s === 'you' ? 'you' : s)));
    const area = h('select.quiet', { 'aria-label': 'area' });
    const count = h('div.meta');
    const list = h('ul.list', { 'aria-label': 'notes' });
    const reader = h('article.reader', { 'aria-label': 'reader', tabindex: '-1' }, h('p.empty', 'choose something to read.'));
    const left = h('div.notes-list', h('div.filters', q, h('div.btnrow', kind, who, area), count), list);
    el.append(left, reader);
    let current = null;

    const fillSelects = () => {
      clear(kind).append(h('option', { value: '' }, 'every kind'), ...store.cloud.kinds.map((k, i) => h('option', { value: i }, k)));
      clear(area).append(h('option', { value: '' }, 'every area'), h('option', { value: '-1' }, 'unsorted'), ...store.cloud.regions.map((r, i) => h('option', { value: i }, r.label)));
    };
    const render = () => {
      if (!store.cloud) return;
      const c = store.cloud;
      const needle = q.value.trim().toLowerCase();
      const out = [];
      for (let i = store.n - 1; i >= 0; i--) {
        if (kind.value !== '' && c.kind[i] !== +kind.value) continue;
        if (who.value !== '' && c.speaker[i] !== +who.value) continue;
        if (area.value !== '' && c.region[i] !== +area.value) continue;
        if (needle && !c.titles[i].toLowerCase().includes(needle)) continue;
        out.push(i);
      }
      out.sort((a, b) => c.t[b] - c.t[a]);
      count.textContent = out.length > LIMIT ? `showing ${LIMIT} of ${out.length}` : `${out.length} items`;
      clear(list);
      for (const i of out.slice(0, LIMIT)) {
        const r = regionOf(i);
        list.append(h('li', h('button', { type: 'button', 'aria-current': String(c.ids[i] === current), onclick: () => open(c.ids[i]) },
          h('div.item-title', c.titles[i]),
          h('div.meta', `${kindOf(i)} · ${SPEAKERS[c.speaker[i]]} · ${fmtDate(c.t[i])}${r ? ' · ' + r.label : ''}`))));
      }
    };
    const open = (id) => {
      current = id;
      for (const b of list.querySelectorAll('button[aria-current]')) b.setAttribute('aria-current', 'false');
      renderReader(reader, id, ctx).then(() => reader.focus({ preventScroll: true }));
      render();
    };
    let t = 0;
    q.addEventListener('input', () => { clearTimeout(t); t = setTimeout(render, 150); });
    for (const s of [kind, who, area]) s.addEventListener('change', render);
    on('open-item', (id) => open(id));
    on('cloud-ready', () => { fillSelects(); render(); });
    if (store.cloud) { fillSelects(); render(); }
    void showBucket;
  },
};
