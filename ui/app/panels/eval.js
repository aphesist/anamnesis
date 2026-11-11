// Evaluation: latent links share, retrieval, resurfacing and growth — each with
// n, point estimate, interval, status and whether it is measured, modelled or a hypothesis.
import { h, clear } from '../lib/dom.js';
import { api } from '../lib/api.js';
import { theme, rgba } from '../lib/tokens.js';
import { setup, stippleRect } from '../lib/stipple.js';
import { on } from '../lib/store.js';

const ORDER = [['links', 'latent links'], ['retrieval', 'retrieval'], ['resurfacing', 'resurfacing'], ['growth', 'growth']];
const fmt = (x, unit) => (x == null || isNaN(x) ? '—' : unit === 'share' ? `${Math.round(x * 100)}%` : (Math.abs(x) < 10 ? (+x).toFixed(2) : Math.round(x)));

export default {
  id: 'eval',
  title: 'evaluation',
  tab: 'eval',
  mount(el) {
    const wrap = h('div');
    el.append(h('p.meta', { style: { margin: '0 0 4px' } }, 'measured on your own use, on this machine. intervals are 95%. "measuring" means not enough data to say yet.'), wrap);
    let data = null;
    const draw = () => {
      if (!data) return;
      clear(wrap);
      for (const [key, fallback] of ORDER) {
        const m = data[key];
        if (!m) continue;
        const basis = (m.basis || m.label || 'measured').toLowerCase();
        const status = m.status || 'measuring';
        const est = m.estimate ?? m.point ?? m.value;
        const iv = m.interval || m.ci || [];
        const canvas = h('canvas', { 'aria-hidden': 'true' });
        wrap.append(h('section.eval-card', { 'aria-label': m.title || fallback },
          h('div.btnrow', { style: { justifyContent: 'space-between' } }, h('h3', m.title || fallback), h(`span.basis.${basis}`, basis)),
          m.question ? h('p.q', m.question) : null,
          h('div.btnrow', h('span.est', fmt(est, m.unit)), h('span.ci', iv.length ? `[${fmt(iv[0], m.unit)} – ${fmt(iv[1], m.unit)}]` : ''), h('span.meta', m.unit && m.unit !== 'share' ? m.unit : '')),
          canvas,
          h('div.btnrow', h('span.meta.data', `n ${m.n ?? 0}`), h(`span.meta.${status === 'enough data' ? 'st-enough' : 'st-measuring'}`, status)),
          m.detail ? h('p.meta', { style: { margin: '4px 0 0' } }, m.detail) : null));
        requestAnimationFrame(() => drawInterval(canvas, est, iv, m.unit));
      }
    };
    const drawInterval = (canvas, est, iv, unit) => {
      const { g, w, h: H } = setup(canvas);
      const c = theme().colors;
      let lo = unit === 'share' || (iv[0] >= 0 && iv[1] <= 1) ? 0 : Math.min(0, iv[0] ?? 0) - 0.1;
      let hi = unit === 'share' || (iv[0] >= 0 && iv[1] <= 1) ? 1 : Math.max(iv[1] ?? 1, est ?? 1) * 1.1;
      if (lo < 0 && hi > 0 && unit !== 'share') { const m = Math.max(-lo, hi); lo = -m; hi = m; }
      const X = (v) => 4 + ((v - lo) / (hi - lo || 1)) * (w - 8);
      g.fillStyle = rgba(c['ink-soft'], 0.5);
      for (let x = 4; x < w - 4; x += 4) g.fillRect(x, H / 2, 0.8, 0.8);
      if (lo < 0) { g.fillStyle = rgba(c['ink-soft'], 0.8); g.fillRect(X(0), H / 2 - 6, 0.8, 12); }
      if (iv.length) stippleRect(g, X(iv[0]), H / 2 - 5, X(iv[1]) - X(iv[0]), 10, 0.5, rgba(c.accent, 0.9), 7, 2, 0.9);
      if (est != null) { g.fillStyle = rgba(c['dot-hot'], 1); g.beginPath(); g.arc(X(est), H / 2, 2.6, 0, Math.PI * 2); g.fill(); }
    };
    (async () => {
      try { data = await api.eval(); draw(); } catch { wrap.append(h('p.empty', 'the evaluation could not be read.')); }
    })();
    on('theme', draw);
  },
};
