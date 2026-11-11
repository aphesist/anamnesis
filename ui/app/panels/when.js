// "when you wrote": activity by area over time (stacked stipple streams),
// lifecycle marks (decisions, open questions, plans, rejections), a calendar of
// days written (stipple density) and monthly volume by speaker. Selecting any
// mark lights its items in the brain; dragging across the streams brushes the
// time window, which the shader applies as a uniform range.
import { h, clear, fmtDate } from '../lib/dom.js';
import { store, on, setView, BIT, SPEAKERS } from '../lib/store.js';
import { theme, rgba, areaColor } from '../lib/tokens.js';
import { api } from '../lib/api.js';
import { stackedStreams, calendar, columns, monthAxis } from '../lib/stipple.js';

const TH_KINDS = [['decision', 'decisions'], ['open_question', 'open questions'], ['plan', 'plans'], ['rejection', 'rejections']];
const ymOf = (sec) => new Date(sec * 1000).toISOString().slice(0, 7);

export default {
  id: 'when',
  title: 'when you wrote',
  tab: 'when',
  mount(el, ctx) {
    let tl = null, threads = [], monthIdx = null, sel = null;
    const mkChart = (title, cls = '') => {
      const canvas = h('canvas' + cls, { tabindex: '0', role: 'img' });
      const keys = h('div.keys');
      const wrap = h('div.chart', h('h4', title), canvas, keys);
      return { wrap, canvas, keys };
    };
    const streams = mkChart('activity by area');
    const life = mkChart('decisions · questions · plans · rejections');
    const cal = mkChart('days written', '.heat');
    const spk = mkChart('monthly volume by speaker');
    const windowNote = h('div.meta');
    const tools = h('div.btnrow', { style: { marginBottom: '6px' } }, windowNote,
      h('button.textbtn', { type: 'button', onclick: () => { setView({ time: [0, 1] }); ctx.timeSeek?.(1); render(); } }, 'clear the time window'));
    el.append(tools, h('div.charts', streams.wrap, life.wrap, cal.wrap, spk.wrap),
      h('p.meta', { style: { marginTop: '8px' } }, 'select a mark to light its notes · drag across the streams to set a time window · arrow keys and enter work on every chart'));

    const lightWhere = (pred, label) => {
      const idx = [];
      for (let i = 0; i < store.n; i++) if (pred(i)) idx.push(i);
      sel = label;
      ctx.light(idx, label, { bit: BIT.TIMEWIN, fly: false });
      render();
      return idx;
    };

    let hitStreams = () => null, hitLife = () => null, hitCal = () => null, hitSpk = () => null;
    const cursor = { streams: { s: 0, k: 0 }, life: { s: 0, k: 0 }, spk: { s: 0, k: 0 }, cal: 0 };
    let selKey = null; // what is selected, for highlighting in the charts

    function render() {
      if (!tl || !store.cloud) return;
      const c = theme().colors;
      const months = tl.months;
      const m = months.length;
      const regs = store.cloud.regions;
      const rIndex = new Map(regs.map((r, k) => [r.id, k]));
      const t0 = store.view.time[0], t1 = store.view.time[1];
      windowNote.textContent = t0 > 0 || t1 < 1 ? `window: ${fmtDate(store.tMin + t0 * (store.tMax - store.tMin))} – ${fmtDate(store.tMin + t1 * (store.tMax - store.tMin))}` : 'all time';

      // (a) streams by region
      const series = tl.by_region.map((br) => ({ values: br.counts, color: areaColor(rIndex.get(br.region_id) ?? 0, 0.95), r: rIndex.get(br.region_id) }));
      hitStreams = stackedStreams(streams.canvas, series, {
        lit: (si, k) => selKey && selKey.chart === 'streams' && selKey.s === si && (selKey.k == null || selKey.k === k),
        dim: (si) => selKey && selKey.chart === 'streams' && selKey.s !== si,
      });
      monthAxis(streams.canvas, months, rgba(c['ink-soft'], 0.8));
      drawWindow(streams.canvas, m);
      clear(streams.keys).append(...series.map((s, si) => h('button', { type: 'button', style: { color: s.color }, onclick: () => selectStream(si, null) }, h('i'), h('span', { style: { color: 'var(--ink-soft)' } }, regs[s.r]?.label || '?'))));
      streams.canvas.setAttribute('aria-label', `activity by area over ${m} months. arrow keys move, enter lights.`);

      // (b) lifecycle
      const ls = TH_KINDS.map(([k], si) => ({ values: tl.threads_by_month[k] || new Array(m).fill(0), color: [rgba(c.accent, 0.95), rgba(c.dot, 0.95), rgba(c['dot-hot'], 0.9), rgba(c.seal, 0.95)][si] }));
      hitLife = columns(life.canvas, ls, { lit: (si, k) => selKey && selKey.chart === 'life' && selKey.s === si && (selKey.k == null || selKey.k === k) });
      monthAxis(life.canvas, months, rgba(c['ink-soft'], 0.8));
      clear(life.keys).append(...TH_KINDS.map(([k, label], si) => h('button', { type: 'button', style: { color: ls[si].color }, onclick: () => selectLife(si, null) }, h('i'), h('span', { style: { color: 'var(--ink-soft)' } }, label))));
      life.canvas.setAttribute('aria-label', 'decisions, open questions, plans and rejections per month.');

      // (c) calendar
      const litDay = selKey && selKey.chart === 'cal' ? selKey.iso : null;
      const litFn = litDay ? Object.assign((iso) => iso === litDay, { color: rgba(c['dot-hot'], 1) }) : null;
      hitCal = calendar(cal.canvas, tl.days, { color: rgba(c.dot, 0.95), soft: rgba(c['ink-soft'], 0.9), lit: litFn });
      const total = tl.days.reduce((a, d) => a + d.count, 0);
      clear(cal.keys).append(h('span', `${tl.days.length} days written · ${total} items · denser dots, more written`));
      cal.canvas.setAttribute('aria-label', `calendar of days written: ${tl.days.length} days. arrow keys move by day and week, enter lights that day.`);

      // (d) speakers
      const ss = [['you', c['dot-hot']], ['assistants', c.accent], ['other', c['ink-soft']]].map(([k, col]) => ({ values: tl.by_speaker[k] || new Array(m).fill(0), color: rgba(col, 0.95) }));
      hitSpk = columns(spk.canvas, ss, { stacked: true, lit: (si, k) => selKey && selKey.chart === 'spk' && selKey.s === si && (selKey.k == null || selKey.k === k) });
      monthAxis(spk.canvas, months, rgba(c['ink-soft'], 0.8));
      clear(spk.keys).append(...SPEAKERS.map((label, si) => h('button', { type: 'button', style: { color: ss[si].color }, onclick: () => selectSpk(si, null) }, h('i'), h('span', { style: { color: 'var(--ink-soft)' } }, label))));
      spk.canvas.setAttribute('aria-label', 'monthly volume by speaker: you, assistants, other.');
    }

    function drawWindow(canvas, m) {
      const t0 = store.view.time[0], t1 = store.view.time[1];
      if (t0 <= 0 && t1 >= 1) return;
      const g = canvas.getContext('2d');
      const w = canvas.clientWidth, H = canvas.clientHeight;
      const x0 = 4 + t0 * (w - 8), x1 = 4 + t1 * (w - 8);
      g.fillStyle = rgba(theme().colors.bg, 0.55);
      g.fillRect(0, 0, x0, H); g.fillRect(x1, 0, w - x1, H);
      g.strokeStyle = rgba(theme().colors.accent, 0.8); g.setLineDash([2, 3]); g.lineWidth = 1;
      g.beginPath(); g.moveTo(x0, 0); g.lineTo(x0, H - 12); g.moveTo(x1, 0); g.lineTo(x1, H - 12); g.stroke(); g.setLineDash([]);
      void m;
    }

    const monthOfPoint = () => {
      if (monthIdx) return monthIdx;
      const map = new Map(tl.months.map((ym, k) => [ym, k]));
      monthIdx = Int16Array.from(store.cloud.t, (t) => map.get(ymOf(t)) ?? -1);
      return monthIdx;
    };
    function selectStream(si, k) {
      const br = tl.by_region[si]; if (!br) return;
      const r = store.cloud.regions.findIndex((x) => x.id === br.region_id);
      const mi = monthOfPoint();
      selKey = { chart: 'streams', s: si, k };
      lightWhere((i) => store.cloud.region[i] === r && (k == null || mi[i] === k), `${store.cloud.regions[r]?.label}${k == null ? '' : ', ' + tl.months[k]}`);
    }
    function selectLife(si, k) {
      const kind = TH_KINDS[si][0];
      const ids = new Set(threads.filter((t) => t.kind === kind && (k == null || t.created_at.slice(0, 7) === tl.months[k])).map((t) => t.item_id));
      selKey = { chart: 'life', s: si, k };
      const idx = lightWhere((i) => ids.has(store.cloud.ids[i]), `${TH_KINDS[si][1]}${k == null ? '' : ', ' + tl.months[k]}`);
      if (!idx.length) ctx.toast('the notes behind those marks are not in this brain view.');
    }
    function selectSpk(si, k) {
      const mi = monthOfPoint();
      selKey = { chart: 'spk', s: si, k };
      lightWhere((i) => store.cloud.speaker[i] === si && (k == null || mi[i] === k), `${SPEAKERS[si]}${k == null ? '' : ', ' + tl.months[k]}`);
    }
    function selectDay(iso) {
      selKey = { chart: 'cal', iso };
      lightWhere((i) => new Date(store.cloud.t[i] * 1000).toISOString().slice(0, 10) === iso, `written ${iso}`);
    }

    // pointer: click selects; drag on the streams brushes a time window
    const local = (e, cv) => { const r = cv.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
    let brush = null;
    streams.canvas.addEventListener('pointerdown', (e) => { brush = { x0: local(e, streams.canvas)[0], moved: false }; streams.canvas.setPointerCapture(e.pointerId); });
    streams.canvas.addEventListener('pointermove', (e) => {
      if (!brush) return;
      const x = local(e, streams.canvas)[0];
      if (Math.abs(x - brush.x0) > 6) {
        brush.moved = true;
        const w = streams.canvas.clientWidth;
        const a = Math.max(0, Math.min(1, (Math.min(x, brush.x0) - 4) / (w - 8))), b = Math.max(0, Math.min(1, (Math.max(x, brush.x0) - 4) / (w - 8)));
        // months -> normalised time of the cloud
        const toT = (f) => { const k = f * (tl.months.length - 1); const ym = tl.months[Math.round(k)]; const sec = Date.parse(ym + '-01T00:00:00Z') / 1000; return Math.max(0, Math.min(1, (sec - store.tMin) / (store.tMax - store.tMin))); };
        setView({ time: [toT(a), Math.max(toT(a) + 0.01, toT(b) + 0.03)] });
        render();
      }
    });
    streams.canvas.addEventListener('pointerup', (e) => {
      if (brush && !brush.moved) { const hit = hitStreams(...local(e, streams.canvas)); if (hit && hit.s >= 0) selectStream(hit.s, hit.k); }
      brush = null;
    });
    life.canvas.addEventListener('click', (e) => { const hit = hitLife(...local(e, life.canvas)); if (hit && hit.s >= 0) selectLife(hit.s, hit.k); });
    spk.canvas.addEventListener('click', (e) => { const hit = hitSpk(...local(e, spk.canvas)); if (hit && hit.s >= 0) selectSpk(hit.s, hit.k); });
    cal.canvas.addEventListener('click', (e) => { const hit = hitCal(...local(e, cal.canvas)); if (hit && hit.n) selectDay(hit.iso); });

    // keyboard on each chart
    const keyNav = (canvas, name, S, select) => canvas.addEventListener('keydown', (e) => {
      const cur = cursor[name], m = tl.months.length;
      if (e.key === 'ArrowRight') cur.k = Math.min(m - 1, cur.k + 1);
      else if (e.key === 'ArrowLeft') cur.k = Math.max(0, cur.k - 1);
      else if (e.key === 'ArrowUp') cur.s = Math.min(S() - 1, cur.s + 1);
      else if (e.key === 'ArrowDown') cur.s = Math.max(0, cur.s - 1);
      else if (e.key === 'Enter' || e.key === ' ') { select(cur.s, cur.k); e.preventDefault(); return; }
      else return;
      e.preventDefault();
      select(cur.s, cur.k);
    });
    keyNav(streams.canvas, 'streams', () => tl.by_region.length, selectStream);
    keyNav(life.canvas, 'life', () => 4, selectLife);
    keyNav(spk.canvas, 'spk', () => 3, selectSpk);
    cal.canvas.addEventListener('keydown', (e) => {
      const d = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 1, ArrowUp: -1 }[e.key];
      const days = tl.days;
      if (d) { e.preventDefault(); cursor.cal = Math.max(0, Math.min(days.length - 1, cursor.cal + (e.key.includes('Up') || e.key.includes('Down') ? d * 7 : d))); selectDay(days[cursor.cal].date); }
      if (e.key === 'Enter') selectDay(days[cursor.cal].date);
    });

    const load = async () => {
      try { [tl, threads] = await Promise.all([api.timeline(), api.threads().then((r) => r.threads || [])]); } catch (e) { clear(el).append(h('p.empty', 'the timeline could not be read.')); return; }
      cursor.cal = tl.days.length - 1;
      cursor.streams.k = cursor.life.k = cursor.spk.k = tl.months.length - 1;
      monthIdx = null;
      render();
    };
    on('cloud-ready', load);
    if (store.cloud) load();
    on('theme', render);
    on('lit-cleared', () => { selKey = null; render(); });
    new ResizeObserver(() => render()).observe(el);
    this._render = render;
  },
  shown() { requestAnimationFrame(() => this._render?.()); },
};
