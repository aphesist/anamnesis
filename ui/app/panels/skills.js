// Skills library: reusable recipes (items of kind skill). Run sends one to chat.
import { h, clear } from '../lib/dom.js';
import { api } from '../lib/api.js';
import { renderMarkdown } from '../lib/md.js';

export default {
  id: 'skills',
  title: 'skills',
  mount(el, ctx) {
    const list = h('ul.list');
    el.append(h('p.meta', { style: { margin: '0 0 6px' } }, 'recipes you can run with your chosen model. they are notes too, and harnesses see them as prompts.'), list);
    (async () => {
      let skills = [];
      try { skills = (await api.skills()).skills || []; } catch { /* */ }
      if (!skills.length) { list.append(h('li.empty', 'no skills yet. write a note of kind skill.')); return; }
      for (const s of skills) {
        const body = h('div.skill-body', { hidden: true, html: renderMarkdown(s.body) });
        const toggle = h('button.item-title', { type: 'button', 'aria-expanded': 'false', onclick: () => { body.hidden = !body.hidden; toggle.setAttribute('aria-expanded', String(!body.hidden)); } }, s.title);
        list.append(h('li', toggle, body, h('div.btnrow',
          h('button.textbtn.accent', { type: 'button', onclick: () => ctx.ask(`run the skill "${s.title}":\n\n${s.body}`) }, 'run'),
          h('button.textbtn', { type: 'button', onclick: () => ctx.openInNotes(s.id) }, 'open'))));
      }
    })();
    void clear;
  },
};
