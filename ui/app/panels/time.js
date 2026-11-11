// The timeline scrubber ("time"): sets the upper bound of the time window the
// shader shows, and replays growth — points appear in the order they were
// written, and region births, splits and merges from /api/lineage animate.
import { h, fmtDate } from '../lib/dom.js';
import { store, on, setView, toNorm, fromNorm } from '../lib/store.js';
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
      const sec = fromNorm(t);
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
    // Lineage marks. Runs that fall close together on the track (a re-import, several mind runs in
    // one evening) would print over each other, so neighbours within a label's width are drawn as
    // one count glyph whose tooltip lists every run.
    const WORDS = { birth: 'born', death: 'faded', split: 'split', merge: 'merged' };
    const summary = (runs) => {
      const n = {};
      for (const run of runs) for (const x of run.transitions) if (x.type !== 'survive') n[x.type] = (n[x.type] || 0) + 1;
      return Object.entries(n).map(([k, v]) => (v > 1 ? `${v} ` : '') + (WORDS[k] || k)).join(' · ');
    };
    const placeMarks = () => {
      marks.textContent = '';
      const W = marks.clientWidth || track.clientWidth || 400;
      const runs = lineage.map((run) => ({ run, tt: toNorm(Date.parse(run.at) / 1000) }))
        .filter((x) => x.tt >= 0 && x.tt <= 1 && x.run.transitions.some((t) => t.type !== 'survive'));
      const groups = [];
      for (const r of runs) {
        const g = groups[groups.length - 1];
        const x = r.tt * W;
        const textW = (g ? summary(g.runs.map((y) => y.run)).length : 0) * 6 + 24;
        if (g && x - g.x0 < Math.max(textW, 40)) { g.runs.push(r); g.x1 = x; } else groups.push({ x0: x, x1: x, runs: [r] });
      }
      for (const g of groups) {
        const tt = g.runs[g.runs.length - 1].tt, all = g.runs.map((y) => y.run);
        const many = all.length > 1;
        const text = many ? `${all.length} changes` : summary(all) || 'change';
        const title = all.map((run) => `${fmtDate(Date.parse(run.at) / 1000)}: ${describe(run)}`).join('\n');
        const short = title.length > 900 ? title.slice(0, 900) + '…' : title;
        const m = h('button.mark' + (many ? '.cluster' : ''), {
          type: 'button', style: { left: (((g.x0 + g.x1) / 2 / W) * 100).toFixed(2) + '%' }, title: short,
          'aria-label': many ? `${all.length} lineage changes; go to the first` : short,
          onclick: () => seek(g.runs[0].tt + 0.0005, true),
        }, text);
        m.dataset.t = tt;
        // keep the label inside the track
        const pct = (g.x0 + g.x1) / 2 / W;
        if (pct > 0.9) m.classList.add('end'); else if (pct < 0.1) m.classList.add('start');
        marks.append(m);
      }
      syncMarks();
    };
    const syncMarks = () => { for (const m of marks.children) m.classList.toggle('past', +m.dataset.t <= store.view.time[1]); };

    const regionName = (run, id) => (run.regions.find((r) => r.id === id) || store.cloud.regions.find((r) => r.id === id) || { label: '…' }).label;
    function describe(run) {
      const t = run.transitions.filter((x) => x.type !== 'survive');
      // a first run is a wall of births: name a few and count the rest
      const births = t.filter((x) => x.type === 'birth');
      if (births.length > 4) {
        const rest = t.filter((x) => x.type !== 'birth');
        return [`${births.length} regions born (${births.slice(0, 3).map((x) => regionName(run, x.region)).join('; ')}; …)`, ...rest.map(one)].join('; ');
      }
      return t.map(one).join('; ');
      function one(x) {
        if (x.type === 'birth') return `born: ${regionName(run, x.region)}`;
        if (x.type === 'death') return `faded: ${regionName(run, x.region)}`;
        if (x.type === 'split') return `split: ${regionName(run, x.from)} → ${x.into.map((i) => regionName(run, i)).join(', ')}`;
        if (x.type === 'merge') return `merged: ${x.from.map((i) => regionName(run, i)).join(' + ')} → ${regionName(run, x.into)}`;
        return x.type;
      }
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
        const tt = toNorm(Date.parse(run.at) / 1000);
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
        const steps = [...lineage.map((r) => toNorm(Date.parse(r.at) / 1000)), 1].filter((x) => x > t).sort((a, b) => a - b);
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
    let lastW = 0;
    new ResizeObserver(() => { drawHist(); const W = marks.clientWidth; if (W && Math.abs(W - lastW) > 20) { lastW = W; placeMarks(); } }).observe(canvas);
    ctx.timeSeek = seek;
  },
};
