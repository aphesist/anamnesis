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
  const linkRow = ([dir, iid, kind]) => h('li', h('span.soft', dir + ' '), itemLink(iid), h('span.meta', '  ' + String(kind).replace('_', ' ')));
  const linkList = h('ul.list', ...all.slice(0, 24).map(linkRow));
  if (!all.length) linkList.append(h('li.empty.no-links', 'no links yet.'));
  side.append(h('section.links', h('h3', 'links'), linkList, linkPicker(item, ctx, (hit) => {
    linkList.querySelector('.no-links')?.remove();
    linkList.prepend(linkRow(['→', hit.item_id, 'manual']));
  })));
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
    prov, privacyRow(item, ctx), body, side);
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

// Privacy for one item: local only / cloud ok / ask, via PUT /api/items/:id { privacy }.
export const PRIVACY_WORDS = { local_only: 'local only', cloud_ok: 'cloud ok', ask: 'ask' };
const PRIVACY_HINT = { local_only: 'never sent to a model that is not on this machine', cloud_ok: 'may go to the model you chose', ask: 'the chat asks before it leaves' };
function privacyRow(item, ctx) {
  let cur = item.privacy || 'cloud_ok';
  const hint = h('span.meta', PRIVACY_HINT[cur] || '');
  const g = h('div.choice.privacy-choice', { role: 'radiogroup', 'aria-label': 'privacy of this item' });
  const sync = () => { for (const b of g.children) { const on = b.dataset.v === cur; b.setAttribute('aria-checked', String(on)); b.tabIndex = on ? 0 : -1; } hint.textContent = PRIVACY_HINT[cur] || ''; };
  for (const [v, label] of Object.entries(PRIVACY_WORDS)) {
    g.append(h('button', { type: 'button', role: 'radio', 'data-v': v, onclick: async () => {
      if (v === cur) return;
      const prev = cur; cur = v; sync();
      try {
        const r = await api.setPrivacy(item.id, v);
        const got = r && (r.privacy || r.item?.privacy);
        if (got && got !== v) throw new Error('not taken');
        item.privacy = v;
        ctx.toast(`this item is now ${label}.`);
      } catch { cur = prev; sync(); ctx.toast('privacy did not change: the server would not take it.'); }
    } }, label));
  }
  g.addEventListener('keydown', (e) => {
    const list = [...g.children]; const k = list.findIndex((b) => b.dataset.v === cur);
    const n = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? (k + 1) % list.length : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? (k - 1 + list.length) % list.length : -1;
    if (n >= 0) { e.preventDefault(); list[n].click(); list[n].focus(); }
  });
  sync();
  return h('div.privacy-row', h('span.lab', 'privacy'), g, hint);
}

// "link to…": a small search picker that makes a manual link (POST /api/links).
function linkPicker(item, ctx, onLinked) {
  const wrap = h('div.link-picker');
  const open = h('button.textbtn', { type: 'button', 'aria-expanded': 'false' }, 'link to…');
  const box = h('div.link-box', { hidden: true });
  const input = h('input.field', { type: 'search', placeholder: 'find a note to link', 'aria-label': 'find a note to link', autocomplete: 'off' });
  const results = h('ul.list.link-results', { role: 'listbox', 'aria-label': 'notes to link' });
  box.append(input, results);
  open.addEventListener('click', () => { box.hidden = !box.hidden; open.setAttribute('aria-expanded', String(!box.hidden)); if (!box.hidden) input.focus(); });
  let timer = 0, seq = 0;
  input.addEventListener('input', () => {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const q = input.value.trim(); const my = ++seq;
      if (q.length < 2) { clear(results); return; }
      let hits = [];
      try { hits = (await api.search(q, 10)).hits || []; } catch { hits = []; }
      if (my !== seq) return;
      clear(results);
      const seen = new Set([item.id]);
      for (const hit of hits) {
        if (seen.has(hit.item_id)) continue; seen.add(hit.item_id);
        results.append(h('li', h('button.textbtn.keep-case', { type: 'button', role: 'option', onclick: async () => {
          try { await api.link(item.id, hit.item_id); ctx.toast(`linked to "${hit.title}". it counts as a link you made by hand.`); onLinked(hit); input.value = ''; clear(results); box.hidden = true; open.setAttribute('aria-expanded', 'false'); }
          catch (e) { ctx.toast(e.serverMessage || 'that link did not save.'); }
        } }, hit.title || hit.item_id.slice(-6))));
      }
      if (!results.children.length) results.append(h('li.empty', 'nothing by that name.'));
    }, 200);
  });
  input.addEventListener('keydown', (e) => { if (e.key === 'Escape') { e.stopPropagation(); box.hidden = true; open.setAttribute('aria-expanded', 'false'); open.focus(); } });
  wrap.append(open, box);
  return wrap;
}
