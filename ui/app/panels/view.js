// The right "view" panel: form, groups, show, motion, walkthrough, demo, reset.
import { h } from '../lib/dom.js';
import { store, on, setView, defaultView } from '../lib/store.js';
import { theme } from '../lib/tokens.js';
import { startWalkthrough } from '../lib/walkthrough.js';

function radios(label, options, get, set) {
  const g = h('div.choice', { role: 'radiogroup', 'aria-label': label });
  const btns = options.map(([value, text]) => h('button', {
    type: 'button', role: 'radio', 'data-value': value,
    onclick: () => { set(value); sync(); },
  }, text));
  const sync = () => btns.forEach((b) => { const on = b.dataset.value === get(); b.setAttribute('aria-checked', String(on)); b.tabIndex = on ? 0 : -1; });
  g.addEventListener('keydown', (e) => {
    const k = btns.findIndex((b) => b.dataset.value === get());
    let n = -1;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') n = (k + 1) % btns.length;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') n = (k - 1 + btns.length) % btns.length;
    if (n >= 0) { e.preventDefault(); btns[n].click(); btns[n].focus(); }
  });
  g.append(...btns);
  sync();
  g.sync = sync;
  return g;
}
function toggle(label, get, set) {
  const b = h('button.toggle', { type: 'button', role: 'switch', onclick: () => { set(!get()); sync(); } }, h('span.tick', { 'aria-hidden': 'true' }), h('span', label));
  const sync = () => b.setAttribute('aria-checked', String(!!get()));
  sync();
  b.sync = sync;
  return b;
}
const sect = (title, open, ...kids) => h('details.sect', { open }, h('summary', title), ...kids);

export default {
  id: 'view',
  title: 'view',
  mount(el, ctx) {
    const v = () => store.view;
    const syncers = [];
    const reg = (x) => { syncers.push(x); return x; };

    // form
    const morph = h('input', { type: 'range', min: 0, max: 1, step: 0.001, value: 0, 'aria-label': 'form: anatomy to layout', 'aria-valuetext': 'anatomy' });
    morph.addEventListener('input', () => {
      const m = +morph.value;
      morph.setAttribute('aria-valuetext', m < 0.15 ? 'anatomy' : m > 0.85 ? 'layout' : `between, ${Math.round(m * 100)}% layout`);
      setView({ morph: m, instant: true });
    });
    on('view', (p) => { if ('morph' in p && !p.instant) morph.value = store.view.morph; });
    const order = reg(radios('order inside groups', [['time', 'by time'], ['links', 'by links'], ['recent', 'newest first'], ['kind', 'by kind']], () => v().order, (x) => setView({ order: x })));
    const form = sect('form', true,
      h('div.row-col', morph, h('div.slider-ends', h('button.textbtn', { type: 'button', onclick: () => { morph.value = 0; setView({ morph: 0 }); } }, 'anatomy'), h('button.textbtn', { type: 'button', onclick: () => { morph.value = 1; setView({ morph: 1 }); } }, 'layout'))),
      h('div.row-col', h('span.lab.soft', 'order'), order),
      h('div.row', h('span.lab', 'table'), h('button.textbtn', { type: 'button', onclick: () => setView({ mode: 'table' }) }, 'build a table from the selection')));

    // groups
    const groupBy = reg(radios('group by', [['kind', 'what it is'], ['region', 'region'], ['source', 'source'], ['speaker', 'speaker']], () => v().groupBy, (x) => { setView({ groupBy: x }); if (store.view.morph < 0.5) { morph.value = 1; setView({ morph: 1 }); } }));
    const layout = reg(radios('layout', [['spheres', 'spheres'], ['split', 'split']], () => v().layout, (x) => { setView({ layout: x }); if (store.view.morph < 0.5) { morph.value = 1; setView({ morph: 1 }); } }));
    const unsorted = reg(toggle('hide unsorted', () => v().hideUnsorted, (x) => setView({ hideUnsorted: x })));
    const groups = sect('groups', true,
      h('div.row-col', h('span.lab.soft', 'group by'), groupBy),
      h('div.row-col', h('span.lab.soft', 'layout'), layout),
      h('div.row', unsorted));

    // show
    const showT = (key, label) => reg(toggle(label, () => v().show[key], (x) => setView({ show: { ...v().show, [key]: x } })));
    const fil = reg(toggle('threads between notes', () => v().filaments, (x) => setView({ filaments: x })));
    const outl = reg(toggle('outline (ornament)', () => v().outline, (x) => setView({ outline: x })));
    const show = sect('show', true,
      h('div.row-col', showT('notes', 'notes (yours)'), showT('records', 'records (imported)'), showT('chats', "the app's own chats"), fil, outl));

    // motion
    const turning = reg(toggle('turning', () => v().turning && theme().motion > 0, (x) => {
      if (x && theme().motion === 0) { ctx.toast('motion is reduced. turn it back on in more → full motion.'); return; }
      setView({ turning: x });
    }));
    const speed = h('input', { type: 'range', min: 0.05, max: 1.5, step: 0.01, value: v().speed, 'aria-label': 'turning speed' });
    speed.addEventListener('input', () => setView({ speed: +speed.value }));
    const motion = sect('motion', true, h('div.row', turning), h('div.row-col', h('span.lab.soft', 'speed'), speed));

    const walk = sect('walkthrough', false,
      h('p.soft', { style: { margin: '0 0 6px', fontSize: '12px' } }, 'a short tour of each part, keyboard first.'),
      h('button.textbtn.strong', { type: 'button', onclick: () => startWalkthrough(ctx) }, 'begin the walkthrough'));

    const demoNote = h('p.soft', { style: { margin: '0 0 6px', fontSize: '12px' } }, 'opens the journals of thoreau and emerson, 1820–1876, as a brain kept apart from yours, so you can try every view without your own notes.');
    const demoBtn = h('button.textbtn.strong', { type: 'button', onclick: () => ctx.setDemo(!store.demo) }, 'show a demo brain');
    const demo = sect('show a demo brain', false, demoNote, demoBtn);
    const syncDemo = (d) => {
      demoBtn.textContent = store.demo ? 'return to my brain' : 'show a demo brain';
      demoBtn.disabled = !store.demo && d && d.available === false;
      if (d && d.available === false && !store.demo) demoNote.textContent = 'no demo bundle was found next to the app, so there is no demo brain to show.';
    };
    on('demo', syncDemo);
    const reset = h('div.row', { style: { marginTop: '10px' } },
      h('button.textbtn', { type: 'button', onclick: () => {
        const keep = { page: store.view.page };
        Object.assign(store.view, defaultView(), keep);
        morph.value = 0;
        setView({ morph: 0, groupBy: store.view.groupBy, show: store.view.show, hideUnsorted: false, turning: true, speed: store.view.speed, time: [0, 1], mode: 'brain', filaments: true });
        speed.value = store.view.speed;
        ctx.clearLight();
        ctx.brain.cam.reset();
        syncers.forEach((s) => s.sync());
        ctx.toast('the view is reset. nothing was deleted.');
      }, style: { whiteSpace: 'nowrap' } }, 'reset the view'),
      h('span.meta', { style: { textAlign: 'right' } }, 'camera, filters, layout — never data'));

    el.append(form, groups, show, motion, walk, demo, reset);
    on('theme', () => syncers.forEach((s) => s.sync()));
  },
};
