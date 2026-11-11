// Table builder: a table from the current selection (or everything shown),
// with chosen columns, sortable, exportable as csv or markdown.
import { h, clear, fmtDate } from '../lib/dom.js';
import { store, LIT, BIT, kindOf, regionOf, SPEAKERS, indicesWith } from '../lib/store.js';

const COLS = [
  ['title', 'title', (i) => store.cloud.titles[i]],
  ['area', 'area', (i) => regionOf(i)?.label || 'unsorted'],
  ['type', 'type', (i) => kindOf(i)],
  ['date', 'date', (i) => new Date(store.cloud.t[i] * 1000).toISOString().slice(0, 10)],
  ['origin', 'origin', (i) => SPEAKERS[store.cloud.speaker[i]]],
  ['links', 'links', (i) => linkCount()[i]],
];
let degree = null, degreeFor = null;
function linkCount() {
  if (degreeFor === store.cloud) return degree;
  degree = new Uint16Array(store.n);
  for (const l of store.cloud.links || []) { degree[l[0]]++; degree[l[1]]++; }
  degreeFor = store.cloud;
  return degree;
}
const chosen = new Set(['title', 'area', 'type', 'date', 'origin']);
let sortBy = 'date', desc = true;

export default {
  mount(el, ctx) {
    const lit = indicesWith(LIT & ~BIT.FOCUS);
    const rows = lit.length ? lit : Array.from({ length: store.n }, (_, i) => i).filter((i) => !(store.flags[i] & BIT.FILTERED));
    const cols = h('div.choice', { role: 'group', 'aria-label': 'columns' });
    const tableWrap = h('div');
    const note = h('p.meta', lit.length ? `${lit.length} lit items${store.litLabel ? ' — ' + store.litLabel : ''}` : `nothing lit, so everything shown (${rows.length}). light something first to narrow it.`);
    const exportBtn = (fmt) => h('button.textbtn', { type: 'button', onclick: () => download(fmt) }, fmt);
    el.append(h('h3', 'table'), note, cols, h('div.btnrow', { style: { margin: '8px 0' } }, h('span.meta', 'export:'), exportBtn('csv'), exportBtn('markdown')), tableWrap);
    const syncCols = () => clear(cols).append(...COLS.map(([k, label]) => h('button', { type: 'button', 'aria-pressed': String(chosen.has(k)), onclick: () => { chosen.has(k) ? chosen.delete(k) : chosen.add(k); if (!chosen.size) chosen.add('title'); syncCols(); render(); } }, label)));
    const active = () => COLS.filter(([k]) => chosen.has(k));
    const sorted = () => {
      const col = COLS.find(([k]) => k === sortBy) || COLS[3];
      return [...rows].sort((a, b) => { const x = col[2](a), y = col[2](b); return (x < y ? -1 : x > y ? 1 : 0) * (desc ? -1 : 1); });
    };
    const render = () => {
      const cs = active();
      const head = h('tr', ...cs.map(([k, label]) => h('th', { scope: 'col', 'aria-sort': k === sortBy ? (desc ? 'descending' : 'ascending') : 'none' },
        h('button', { type: 'button', onclick: () => { if (sortBy === k) desc = !desc; else { sortBy = k; desc = k === 'date' || k === 'links'; } render(); } }, label + (k === sortBy ? (desc ? ' ↓' : ' ↑') : '')))));
      const body = sorted().slice(0, 600).map((i) => h('tr', ...cs.map(([k, , get]) => h('td' + (k === 'title' ? '.t' : ''), k === 'title' ? h('button.item-title', { type: 'button', onclick: () => ctx.open(i) }, get(i)) : String(get(i))))));
      clear(tableWrap).append(h('table.tbl', h('thead', head), h('tbody', ...body)));
      if (rows.length > 600) tableWrap.append(h('p.meta', `showing 600 of ${rows.length}; the export has all of them.`));
    };
    const download = (fmt) => {
      const cs = active(), data = sorted();
      let text;
      if (fmt === 'csv') {
        const q = (v) => { const s = String(v); return /[",\n]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s; };
        text = [cs.map(([, l]) => l).join(','), ...data.map((i) => cs.map(([, , g]) => q(g(i))).join(','))].join('\n');
      } else {
        const esc = (v) => String(v).replace(/\|/g, '\\|');
        text = ['| ' + cs.map(([, l]) => l).join(' | ') + ' |', '|' + cs.map(() => ' --- |').join(''), ...data.map((i) => '| ' + cs.map(([, , g]) => esc(g(i))).join(' | ') + ' |')].join('\n');
      }
      const a = h('a', { href: URL.createObjectURL(new Blob([text], { type: fmt === 'csv' ? 'text/csv' : 'text/markdown' })), download: `anamnesis-selection.${fmt === 'csv' ? 'csv' : 'md'}` });
      document.body.append(a); a.click(); a.remove();
      ctx.toast(`exported ${data.length} rows as ${fmt}.`);
    };
    syncCols(); render();
    void fmtDate;
  },
};
