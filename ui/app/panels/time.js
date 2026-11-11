// The timeline scrubber ("time"): sets the upper bound of the time window the
// shader shows, and replays growth — points appear in the order they were
// written, and region births, splits and merges from /api/lineage animate.
import { h, fmtDate } from '../lib/dom.js';
import { store, on, setView } from '../lib/store.js';
import { theme, rgba } from '../lib/tokens.js';
import { api } from '../lib/api.js';
import { setup, stippleRect } from '../lib/stipple.js';

export default {
  id: 'time',
  title: 'time',
  mount(el, ctx) {
    const play = h('button.play', { type: 'button', 'aria-label': 'replay growth' }, '▷ grow');
    const canvas = h('canvas', { 'aria-hidden': 'true' });
    const range = h('input', { type: 'range', min: 0, max: 1000, step: 1, value: 1000, 'aria-label': 'as of' });
    const marks = h('div.marks');
    const now = h('div.now', { 'aria-live': 'off' }, '');
    const track = h('div.track', canvas, marks, range);
    el.classList.add('timebar');
    el.append(play, track, now);
    const caption = document.getElementById('caption');
    let lineage = [], fired = new Set(), playing = false, raf = 0;

    const label = () => {
      const t = store.view.time[1];
      const sec = store.tMin + (store.tMax - store.tMin) * t;
      now.textContent = t >= 0.999 ? 'now' : fmtDate(sec);
      range.setAttribute('aria-valuetext', t >= 0.999 ? 'now' : 'as of ' + fmtDate(sec));
    };
    const drawHist = () => {
      if (!store.cloud) return;
      const { g, w, h: H } = setup(canvas);
      const B = 120, counts = new Uint32Array(B);
      for (let i = 0; i < store.n; i++) counts[Math.min(B - 1, Math.floor(store.tNorm[i] * B))]++;
      const max = Math.max(1, ...counts);
      const c = theme().colors;
      const t = store.view.time[1];
      for (let k = 0; k < B; k++) {
        const x = (k / B) * w, hh = (H - 16) * Math.sqrt(counts[k] / max);
        const past = (k + 0.5) / B <= t;
        stippleRect(g, x, H - 14 - hh, w / B, hh, past ? 0.55 : 0.14, rgba(past ? c.dot : c['ink-soft'], past ? 0.95 : 0.6), k + 1, 1.8, 0.8);
      }
    };
    const placeMarks = () => {
      marks.textContent = '';
      for (const run of lineage) {
        const tt = (Date.parse(run.at) / 1000 - store.tMin) / (store.tMax - store.tMin);
        if (tt < 0 || tt > 1) continue;
        const kinds = [...new Set(run.transitions.map((x) => x.type).filter((x) => x !== 'survive'))];
        const m = h('button.mark', { type: 'button', style: { left: (tt * 100).toFixed(2) + '%' }, title: describe(run), onclick: () => seek(tt + 0.0005, true) }, kinds.join(' · '));
        m.dataset.t = tt;
        marks.append(m);
      }
      syncMarks();
    };
    const syncMarks = () => { for (const m of marks.children) m.classList.toggle('past', +m.dataset.t <= store.view.time[1]); };

    const regionName = (run, id) => (run.regions.find((r) => r.id === id) || store.cloud.regions.find((r) => r.id === id) || { label: '…' }).label;
    function describe(run) {
      return run.transitions.filter((x) => x.type !== 'survive').map((x) => {
        if (x.type === 'birth') return `born: ${regionName(run, x.region)}`;
        if (x.type === 'death') return `faded: ${regionName(run, x.region)}`;
        if (x.type === 'split') return `split: ${regionName(run, x.from)} → ${x.into.map((i) => regionName(run, i)).join(', ')}`;
        if (x.type === 'merge') return `merged: ${x.from.map((i) => regionName(run, i)).join(' + ')} → ${regionName(run, x.into)}`;
        return x.type;
      }).join('; ');
    }
    const regionIndex = (id) => store.cloud.regions.findIndex((r) => r.id === id);
    function fire(run) {
      if (fired.has(run.run_id)) return;
      fired.add(run.run_id);
      const text = describe(run);
      caption.textContent = text;
      caption.style.opacity = 1;
      clearTimeout(fire.t); fire.t = setTimeout(() => { caption.style.opacity = 0; }, 2600);
      for (const x of run.transitions) {
        const ids = x.type === 'split' ? x.into : x.type === 'merge' ? [x.into] : x.type === 'birth' ? [x.region] : [];
        for (const id of ids) ctx.brain.flashRegion(regionIndex(id));
      }
    }
    function seek(t, user) {
      t = Math.max(0, Math.min(1, t));
      setView({ time: [store.view.time[0], t] });
      range.value = Math.round(t * 1000);
      for (const run of lineage) {
        const tt = (Date.parse(run.at) / 1000 - store.tMin) / (store.tMax - store.tMin);
        if (tt > t) fired.delete(run.run_id);
        else if (playing || user) { if (tt > t - (playing ? 0.02 : 0.004)) fire(run); else fired.add(run.run_id); }
      }
      label(); drawHist(); syncMarks();
    }
    range.addEventListener('input', () => { stop(); seek(+range.value / 1000, true); });
    function stop() { playing = false; cancelAnimationFrame(raf); play.textContent = '▷ grow'; play.setAttribute('aria-pressed', 'false'); }
    play.addEventListener('click', () => {
      if (playing) { stop(); return; }
      playing = true; play.textContent = '❙❙ pause'; play.setAttribute('aria-pressed', 'true');
      fired = new Set();
      let t = store.view.time[1] >= 0.999 ? 0 : store.view.time[1];
      if (theme().motion === 0) {
        // reduced motion: step through the lineage runs instead of sweeping
        const steps = [...lineage.map((r) => (Date.parse(r.at) / 1000 - store.tMin) / (store.tMax - store.tMin)), 1].filter((x) => x > t).sort((a, b) => a - b);
        const step = () => { if (!playing || !steps.length) { stop(); return; } seek(steps.shift() + 0.001); raf = setTimeout(step, 1800); };
        step();
        return;
      }
      let last = performance.now();
      const tick = (nowT) => {
        const dt = (nowT - last) / 1000; last = nowT;
        t += dt / 16;
        seek(t);
        if (t >= 1) { stop(); seek(1); return; }
        raf = requestAnimationFrame(tick);
      };
      raf = requestAnimationFrame(tick);
    });

    const onCloud = async () => {
      label(); drawHist();
      try { lineage = ((await api.lineage()).runs || []).sort((a, b) => Date.parse(a.at) - Date.parse(b.at)); } catch { lineage = []; }
      placeMarks();
    };
    on('cloud-ready', onCloud);
    if (store.cloud) onCloud();
    on('view', (p) => { if ('time' in p && !playing) { range.value = Math.round(store.view.time[1] * 1000); label(); drawHist(); syncMarks(); } });
    on('theme', drawHist);
    new ResizeObserver(drawHist).observe(canvas);
    ctx.timeSeek = seek;
  },
};
