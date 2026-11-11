// One-tap relevance grades (0–3) for a search hit or a chat citation, posted as
// POST /api/eval/label { query, item_id, grade }. These labels are what the retrieval
// evaluation is scored on, so the control is quiet but always there.
import { h } from './dom.js';
import { api } from './api.js';

export const GRADES = [
  [0, 'not relevant'],
  [1, 'barely'],
  [2, 'relevant'],
  [3, 'exactly'],
];

// Labels given in this session, so a re-rendered list shows what was already graded.
const given = new Map();
const key = (q, id) => q.trim().toLowerCase() + '\u0000' + id;

export function gradeControl(query, itemId, { toast } = {}) {
  const wrap = h('span.grade', { role: 'group', 'aria-label': 'how relevant is this?' });
  const btns = GRADES.map(([g, label]) => h('button', {
    type: 'button', 'data-g': String(g), title: `${g} · ${label}`, 'aria-label': `relevance ${g}: ${label}`,
    'aria-pressed': 'false',
    onclick: async (e) => {
      e.stopPropagation();
      e.preventDefault();
      const prev = given.get(key(query, itemId));
      set(g);
      try { await api.post('/api/eval/label', { query, item_id: itemId, grade: g }); given.set(key(query, itemId), g); }
      catch { set(prev); toast?.('that label did not save.'); }
    },
  }));
  const set = (g) => {
    for (const b of btns) {
      const n = +b.dataset.g;
      b.setAttribute('aria-pressed', String(g != null && n === g));
      b.classList.toggle('on', g != null && n <= g);
    }
    wrap.classList.toggle('done', g != null);
  };
  wrap.append(...btns);
  set(given.get(key(query, itemId)));
  return wrap;
}
