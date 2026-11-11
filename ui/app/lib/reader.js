// The reader: one item with its provenance badge, body, links, threads and
// link suggestions (accept / dismiss). Used by the notes page and the peek drawer.
import { h, clear, fmtDate } from './dom.js';
import { renderMarkdown } from './md.js';
import { api } from './api.js';
import { store, BIT, setBit, clearBit } from './store.js';

const ORIGIN = {
  owner_written: ['you', 'written by you'],
  owner_spoken: ['you', 'spoken by you'],
  owner_annotation: ['you', 'your annotation'],
  owner_in_chat: ['you', 'you, in a chat'],
  ai_generated: ['assistants', 'written by an assistant'],
  ai_edited: ['assistants', 'yours, edited by an assistant'],
  external: ['other', "someone else's text"],
};
export const originInfo = (o) => ORIGIN[o] || ['other', o || 'unknown'];

export async function renderReader(el, id, ctx, { compact = false } = {}) {
  clear(el).append(h('p.soft', 'opening…'));
  let data;
  try { data = await api.item(id); } catch (e) { clear(el).append(h('p.empty', 'this item could not be opened.')); return; }
  const { item, links_out = [], links_in = [], threads = [], region } = data;
  const [who, whoText] = originInfo(item.origin);
  const titleOf = (iid) => { const i = store.index.get(iid); return i != null ? store.cloud.titles[i] : iid.slice(-6).toLowerCase(); };
  const itemLink = (iid) => h('button.textbtn.keep-case', { type: 'button', onclick: () => (compact ? ctx.open(iid) : renderReader(el, iid, ctx)) }, titleOf(iid));

  const body = h('div.reader-body', { html: renderMarkdown(item.body) });
  body.addEventListener('click', async (e) => {
    const a = e.target.closest('a.wikilink'); if (!a) return;
    e.preventDefault();
    const t = a.dataset.title.toLowerCase();
    const i = store.cloud ? store.cloud.titles.findIndex((x) => x.toLowerCase() === t) : -1;
    if (i >= 0) renderReader(el, store.cloud.ids[i], ctx, { compact });
    else { const r = await api.search(a.dataset.title, 1).catch(() => ({ hits: [] })); if (r.hits[0]) renderReader(el, r.hits[0].item_id, ctx, { compact }); else ctx.toast('no note by that name yet.'); }
  });

  const prov = h('div.provenance', { 'aria-label': 'provenance' },
    h(`span.badge.${who}`, whoText),
    h('span', `${item.kind}`),
    h('span', 'source: ', h('span.keep-case', item.meta?.source_name || item.external_id || item.source_id)),
    h('span', `written ${fmtDate(item.created_at)}`),
    h('span', `captured ${fmtDate(item.captured_at, true)}`),
    region ? h('button.textbtn', { type: 'button', onclick: () => { const r = store.cloud.regions.findIndex((x) => x.id === region.id); if (r >= 0) ctx.lightRegion(r); } }, `in ${region.label}`) : h('span', 'unsorted'));

  const side = h('div.reader-side');
  const all = [...links_out.map((l) => ['→', l.to, l.kind]), ...links_in.map((l) => ['←', l.from, l.kind])];
  if (all.length) side.append(h('section', h('h3', 'links'), h('ul.list', ...all.slice(0, 24).map(([dir, iid, kind]) => h('li', h('span.soft', dir + ' '), itemLink(iid), h('span.meta', '  ' + kind.replace('_', ' ')))))));
  if (threads.length) side.append(h('section', h('h3', 'threads in this note'), h('ul.list', ...threads.map((t) => h('li', h('span.kind-tag', t.kind.replace('_', ' ')), ' ', h('span.keep-case', t.statement), ' ', h(`span.status.${t.status}`, t.status))))));

  // link suggestions
  const sugg = h('section', h('h3', 'perhaps linked'), h('p.soft', { style: { fontSize: '11px', margin: '0' } }, 'looking…'));
  side.append(sugg);
  api.suggestions(item.id).then(({ suggestions = [] }) => {
    const list = h('div');
    if (!suggestions.length) list.append(h('p.empty', 'no suggestions for this one.'));
    for (const s of suggestions) {
      const row = h('div.sugg',
        h('div', itemLink(s.to.id), h('span.score', `  ${s.score.toFixed(2)}`)),
        h('div.btnrow',
          h('button.textbtn.accent', { type: 'button', onclick: () => decide(true) }, 'link'),
          h('button.textbtn', { type: 'button', onclick: () => decide(false) }, 'dismiss')));
      async function decide(accept) {
        row.style.opacity = 0.4;
        try { await api.decide(s.suggestion_id, accept); row.replaceWith(h('p.meta', accept ? `linked to "${s.to.title}".` : 'dismissed. it will count toward the evaluation either way.')); }
        catch { row.style.opacity = 1; ctx.toast('that did not save. try again.'); }
      }
      list.append(row);
    }
    sugg.lastChild.replaceWith(list);
    sugg.append(h('p.meta', { style: { marginTop: '6px' } }, 'some suggestions are held back at random and never shown; that is how anamnesis learns whether its links were already latent.'));
  }).catch(() => sugg.lastChild.replaceWith(h('p.empty', 'suggestions unavailable.')));

  clear(el).append(
    h('h1', item.title || 'untitled'),
    prov, body, side);
  const i = store.index.get(item.id);
  clearBit(BIT.FOCUS);
  if (i != null) setBit(BIT.FOCUS, [i], { exclusive: false });
}

// A drawer on the brain stage, so opening a dot does not leave the brain.
let peek = null;
export function openPeek(id, ctx) {
  if (!peek) {
    const inner = h('div.peek-inner');
    const close = h('button.textbtn', { type: 'button', onclick: closePeek }, 'close');
    const full = h('button.textbtn', { type: 'button', onclick: () => { const cur = peek.dataset.id; closePeek(); ctx.openInNotes(cur); } }, 'open in notes');
    peek = h('aside.peek.no-orbit', { 'aria-label': 'item', tabindex: '-1' }, h('div.peek-tools', full, close), inner);
    peek.addEventListener('keydown', (e) => { if (e.key === 'Escape') { e.stopPropagation(); closePeek(); } });
    document.getElementById('page-brain').append(peek);
  }
  peek.dataset.id = id;
  peek.hidden = false;
  renderReader(peek.querySelector('.peek-inner'), id, ctx, { compact: true }).then(() => peek.focus({ preventScroll: true }));
}
export function closePeek() {
  if (!peek) return;
  peek.hidden = true;
  clearBit(BIT.FOCUS);
  document.getElementById('brain')?.focus({ preventScroll: true });
}
