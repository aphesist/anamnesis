// Review: resurfaced items with one-tap "useful / not" (sent as ratings for the
// resurfacing trial), plus a pointer to link suggestions.
import { h, clear, fmtDate } from '../lib/dom.js';
import { store } from '../lib/store.js';
import { api } from '../lib/api.js';

let pending = 0;
export function reviewList(ctx, { onEmpty } = {}) {
  const list = h('ul.list');
  const load = async () => {
    clear(list).append(h('li.soft', 'gathering…'));
    let items = [];
    try { items = (await api.review()).items || []; } catch { items = []; }
    pending = items.length;
    clear(list);
    if (!items.length) { list.append(h('li.empty', 'nothing to review today.')); onEmpty?.(); return; }
    for (const r of items) {
      const li = h('li.review-item',
        h('button.item-title', { type: 'button', onclick: () => ctx.open(r.item.id) }, r.item.title),
        h('div.reason', r.reason),
        h('div.meta', `${r.item.kind} · ${fmtDate(r.item.created_at)}`));
      const rate = h('div.rate', { role: 'group', 'aria-label': 'was this worth seeing again?' },
        h('button', { type: 'button', onclick: () => send(true) }, 'useful'),
        h('button', { type: 'button', onclick: () => send(false) }, 'not'));
      li.append(rate);
      async function send(useful) {
        li.classList.add('done');
        rate.replaceWith(h('div.meta', useful ? 'marked useful. thank you.' : 'marked not useful.'));
        try { await api.rate(r.resurface_id, useful); pending = Math.max(0, pending - 1); } catch { ctx.toast('the rating did not save.'); }
      }
      li.addEventListener('mouseenter', () => { const i = store.index.get(r.item.id); if (i != null) ctx.brain.setHover(i); });
      li.addEventListener('mouseleave', () => ctx.brain.setHover(-1));
      list.append(li);
    }
  };
  load();
  list.reload = load;
  return list;
}

export default {
  id: 'review',
  title: 'review',
  badge: () => pending || '',
  mount(el, ctx) {
    el.append(h('p.meta', { style: { margin: '0 0 6px' } }, 'a few things worth seeing again. some are picked by the algorithm, some at random; your answers tell which works better.'), reviewList(ctx));
  },
};
