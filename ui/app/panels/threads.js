// Threads: decisions, open questions, plans and rejections, with status changes.
import { h, clear, fmtDate } from '../lib/dom.js';
import { store, BIT } from '../lib/store.js';
import { api } from '../lib/api.js';

const KINDS = [['', 'all'], ['decision', 'decisions'], ['open_question', 'questions'], ['plan', 'plans'], ['rejection', 'rejections']];
const STATUSES = ['open', 'resolved', 'abandoned', 'superseded'];

export default {
  id: 'threads',
  title: 'threads',
  mount(el, ctx) {
    let kind = '', status = '', threads = [];
    const kinds = h('div.choice', { role: 'radiogroup', 'aria-label': 'kind' });
    const stat = h('select.quiet', { 'aria-label': 'status' }, h('option', { value: '' }, 'any status'), ...STATUSES.map((s) => h('option', { value: s }, s)));
    const list = h('ul.list');
    const lightAll = h('button.textbtn', { type: 'button' }, 'light these');
    el.append(kinds, h('div.btnrow', { style: { margin: '8px 0' } }, stat, lightAll), list);
    const renderKinds = () => { clear(kinds).append(...KINDS.map(([v, l]) => h('button', { type: 'button', role: 'radio', 'aria-checked': String(v === kind), onclick: () => { kind = v; renderKinds(); load(); } }, l))); };
    renderKinds();
    stat.addEventListener('change', () => { status = stat.value; load(); });
    lightAll.addEventListener('click', () => {
      const idx = threads.map((t) => store.index.get(t.item_id)).filter((i) => i != null);
      ctx.light(idx, `${KINDS.find((k) => k[0] === kind)[1]} threads`);
    });
    const render = () => {
      clear(list);
      if (!threads.length) { list.append(h('li.empty', 'nothing here.')); return; }
      for (const t of threads) {
        const sel = h('select.quiet', { 'aria-label': `status of: ${t.statement}` }, ...STATUSES.map((s) => h('option', { value: s, selected: s === t.status }, s)));
        sel.addEventListener('change', async () => {
          const prev = t.status;
          try { Object.assign(t, await api.setThread(t.id, sel.value)); ctx.toast(`marked ${sel.value}.`); render(); }
          catch { sel.value = prev; ctx.toast('that did not save.'); }
        });
        const i = store.index.get(t.item_id);
        list.append(h('li',
          h('div', h('span.kind-tag', t.kind.replace('_', ' ')), ' ', h(`span.status.${t.status}`, t.status)),
          h('button.item-title', { type: 'button', onclick: () => { if (i != null) ctx.light([i], t.statement); } }, t.statement),
          t.quote ? h('div.meta.keep-case', `“${t.quote}”`) : null,
          h('div.btnrow', h('span.meta', fmtDate(t.created_at)), i != null ? h('button.textbtn', { type: 'button', onclick: () => ctx.open(t.item_id) }, 'open the note') : null, sel, h('span.meta', t.extractor))));
      }
    };
    async function load() {
      try { threads = (await api.threads(kind, status)).threads || []; } catch { threads = []; }
      threads.sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at));
      render();
    }
    load();
  },
};
