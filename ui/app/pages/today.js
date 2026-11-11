// Today: today's captures, the daily review, and questions still open.
import { h, clear, fmtDate } from '../lib/dom.js';
import { store, on, kindOf, SPEAKERS } from '../lib/store.js';
import { reviewList } from '../panels/review.js';
import { api } from '../lib/api.js';

export default {
  mount(el, ctx) {
    const caps = h('ul.list');
    const capHead = h('p.meta');
    const open = h('ul.list');
    el.append(
      h('div.today-col', h('h2', 'today'), capHead, caps),
      h('div.today-col', h('h2', 'worth seeing again'), reviewList(ctx), h('h2', { style: { marginTop: '32px' } }, 'questions still open'), open));
    const render = () => {
      if (!store.cloud) return;
      const c = store.cloud;
      const now = Date.now() / 1000;
      let from = now - 86400;
      let idx = [];
      for (let i = 0; i < store.n; i++) if (c.t[i] >= from) idx.push(i);
      if (!idx.length) {
        // nothing today: show the most recent day written
        const lastDay = new Date(store.tMax * 1000).toISOString().slice(0, 10);
        for (let i = 0; i < store.n; i++) if (new Date(c.t[i] * 1000).toISOString().slice(0, 10) === lastDay) idx.push(i);
        capHead.textContent = `nothing captured yet today. the last day you wrote was ${fmtDate(store.tMax)}:`;
      } else capHead.textContent = `${idx.length} captured since yesterday.`;
      idx.sort((a, b) => c.t[b] - c.t[a]);
      clear(caps);
      for (const i of idx.slice(0, 60)) caps.append(h('li', h('button.item-title', { type: 'button', onclick: () => ctx.openInNotes(c.ids[i]) }, c.titles[i]), h('div.meta', `${kindOf(i)} · ${SPEAKERS[c.speaker[i]]}`)));
    };
    const loadOpen = async () => {
      let th = [];
      try { th = (await api.threads('open_question', 'open')).threads || []; } catch { /* */ }
      th.sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at));
      clear(open);
      if (!th.length) open.append(h('li.empty', 'no open questions.'));
      for (const t of th.slice(0, 12)) open.append(h('li', h('button.item-title', { type: 'button', onclick: () => ctx.openInNotes(t.item_id) }, t.statement), h('div.meta', `open since ${fmtDate(t.created_at)}`)));
    };
    on('cloud-ready', render);
    render(); loadOpen();
  },
};
