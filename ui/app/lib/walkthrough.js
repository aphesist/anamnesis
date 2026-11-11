// Guided tour: a few steps, each pointing at one part. Keyboard: → / enter next, ← back, esc ends.
import { h } from './dom.js';
import { theme } from './tokens.js';

const STEPS = [
  { sel: '#brain', title: 'the brain', text: 'every dot is something you wrote, kept or asked. near dots mean near things. drag to turn it, scroll to come closer, click a dot to read it.' },
  { sel: '.ask', title: 'ask or search', text: 'type to search; end with a question mark to ask. found dots light up. ⌘k from anywhere.' },
  { sel: '.legend', title: 'function (anatomy)', text: 'the regions, named in the manner of anatomy. the names are ornament: nearness is the meaning. click one to light it.' },
  { sel: '.panel-view', title: 'view', text: 'morph from anatomy to layout, group by what things are, split them apart, hide what you do not want to see.', panel: 'view' },
  { sel: '.timebar', title: 'time', text: 'drag back to see the brain as it was. grow replays it: regions are born, split and merge as they did.' },
  { sel: '.panel-when', title: 'when you wrote', text: 'activity by area, decisions and questions over time, the days you wrote. select any mark to light its notes.', panel: 'when' },
  { sel: '.panel-chat', title: 'chat', text: 'ask with the model you chose. the dots it cites light up, so you can check every claim.', panel: 'chat' },
  { sel: '.panel-eval', title: 'evaluation', text: 'how well anamnesis is doing, measured on your own use. each number says whether it is measured, modelled or a hypothesis.', panel: 'eval' },
];

export function startWalkthrough(ctx) {
  let k = 0;
  const ring = h('div.walk-ring');
  const box = h('div.walk', { role: 'dialog', 'aria-modal': 'false', 'aria-labelledby': 'walk-title', tabindex: '-1' });
  document.body.append(ring, box);
  const end = () => { ring.remove(); box.remove(); document.removeEventListener('keydown', key, true); };
  const show = () => {
    const s = STEPS[k];
    if (s.panel) ctx.layout.open(s.panel);
    if (ctx.store.view.page !== 'brain') ctx.go('brain');
    requestAnimationFrame(() => {
      const target = document.querySelector(s.sel);
      const r = target ? target.getBoundingClientRect() : { left: innerWidth / 2 - 100, top: innerHeight / 2 - 50, width: 200, height: 100 };
      Object.assign(ring.style, { left: r.left - 6 + 'px', top: r.top - 6 + 'px', width: r.width + 12 + 'px', height: r.height + 12 + 'px' });
      box.replaceChildren(
        h('h3#walk-title', s.title), h('p', { style: { margin: '0 0 10px' } }, s.text),
        h('div.btnrow',
          h('button.textbtn', { type: 'button', disabled: k === 0, onclick: () => { k--; show(); } }, 'back'),
          h('button.textbtn.strong', { type: 'button', onclick: () => { if (k < STEPS.length - 1) { k++; show(); } else end(); } }, k < STEPS.length - 1 ? 'next' : 'done'),
          h('button.textbtn', { type: 'button', onclick: end }, 'end'),
          h('span.meta', `${k + 1} / ${STEPS.length}`)));
      const bw = 320;
      let x = r.left + r.width + 16, y = r.top;
      if (x + bw > innerWidth - 10) x = Math.max(10, r.left - bw - 16);
      if (r.width > innerWidth * 0.5) { x = r.left + r.width / 2 - bw / 2; y = r.top + 60; }
      y = Math.min(Math.max(10, y), innerHeight - 200);
      Object.assign(box.style, { left: x + 'px', top: y + 'px' });
      box.focus({ preventScroll: true });
    });
  };
  const key = (e) => {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); end(); }
    else if ((e.key === 'ArrowRight' || e.key === 'Enter') && box.contains(document.activeElement) && document.activeElement === box) { e.preventDefault(); if (k < STEPS.length - 1) { k++; show(); } else end(); }
    else if (e.key === 'ArrowLeft' && document.activeElement === box && k > 0) { e.preventDefault(); k--; show(); }
  };
  document.addEventListener('keydown', key, true);
  void theme;
  show();
}
