// Left sidebar: ask or search, navigation, + add, more, and the mini brain.
import { h, clear } from '../lib/dom.js';
import { icon } from '../lib/icons.js';
import { store, on, BIT, LIT, setBit, indicesWith } from '../lib/store.js';
import { theme, rgba } from '../lib/tokens.js';
import { api } from '../lib/api.js';
import { shortId } from '../lib/layout.js';
import { gradeControl } from '../lib/grade.js';

const QUESTION = /\?\s*$|^(what|why|how|when|where|who|which|is|are|do|does|did|can|could|should|would|will|tell|summari[sz]e|explain|find me)\b/i;

export default {
  id: 'sidebar',
  title: 'sidebar',
  mount(el, ctx) {
    // the theme's mark, drawn as a mask in --seal (.anm-mark from ui/theme/theme.css)
    const mark = h('span.mark.anm-mark', { 'aria-hidden': 'true' });
    const markUrl = getComputedStyle(document.documentElement).getPropertyValue('--mark').trim();
    if (!markUrl || markUrl === 'none') mark.classList.add('fallback');
    const word = h('div.wordmark', mark, h('span.anm-display', 'anamnesis'));

    // ---------- ask or search ----------
    const input = h('input', { type: 'search', placeholder: 'ask or search', 'aria-label': 'ask or search', autocomplete: 'off', role: 'combobox', 'aria-expanded': 'false', 'aria-controls': 'ask-results', 'aria-autocomplete': 'list' });
    const results = h('div.ask-results#ask-results', { role: 'listbox', hidden: true, 'aria-label': 'search results' });
    // until the embedding model is ready (a first download, a slow load), search matches words only
    const mode = h('div.meta.ask-mode', { hidden: true, 'aria-live': 'polite' });
    const ask = h('div.ask', input, h('span.kbd', { 'aria-hidden': 'true' }, '⌘k'), results, mode);
    let modeTimer = 0;
    const checkMode = async () => {
      clearTimeout(modeTimer);
      let st;
      try { st = await api.status(); } catch { return; }
      const e = st?.embedder || {};
      const ready = e.status === 'ready' && e.id;
      mode.hidden = !!ready;
      mode.textContent = e.fallback ? 'search: words only · the local model could not load' : 'search: words only until the model is ready';
      if (!ready && !e.fallback && el.isConnected !== false) modeTimer = setTimeout(checkMode, 4000);
    };
    checkMode();
    on('brain-switched', checkMode);
    let timer = 0, hits = [], sel = -1, seq = 0;
    const close = () => { results.hidden = true; input.setAttribute('aria-expanded', 'false'); sel = -1; };
    const renderHits = (q) => {
      clear(results);
      if (QUESTION.test(q)) results.append(h('button', { type: 'button', role: 'option', onclick: () => { close(); ctx.ask(q); } }, h('span.soft', 'ask: '), h('span.hit-title', q)));
      hits.forEach((hit, k) => results.append(h('div.hit-row', { role: 'presentation' },
        h('button', { type: 'button', role: 'option', id: `hit-${k}`, onclick: () => { close(); ctx.open(hit.item_id); } },
          h('div.hit-title', hit.title), h('div.meta', hit.snippet)),
        gradeControl(q, hit.item_id, ctx))));
      if (hits.length) results.append(h('div.meta.hit-hint', 'tap 0–3 to say how relevant a hit is; that is how search gets scored.'));
      if (!hits.length && !QUESTION.test(q)) results.append(h('div.empty', { style: { padding: '4px 12px' } }, 'nothing yet. press enter to ask instead.'));
      results.hidden = false; input.setAttribute('aria-expanded', 'true');
    };
    const runSearch = async (q) => {
      const my = ++seq;
      if (q.trim().length < 2) { close(); ctx.clearLight(); return; }
      try {
        const r = await api.search(q, 30);
        if (my !== seq) return;
        hits = r.hits || [];
        const idx = hits.map((x) => store.index.get(x.item_id)).filter((i) => i != null);
        ctx.light(idx, `search: ${q}`, { bit: BIT.SEARCH, fly: false });
        renderHits(q);
      } catch (e) { console.warn(e); }
    };
    input.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(() => runSearch(input.value), 220); });
    input.addEventListener('keydown', (e) => {
      const opts = [...results.querySelectorAll('[role=option]')];
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (!opts.length) return;
        sel = (sel + (e.key === 'ArrowDown' ? 1 : -1) + opts.length) % opts.length;
        opts.forEach((o, k) => o.setAttribute('aria-selected', String(k === sel)));
        input.setAttribute('aria-activedescendant', opts[sel].id || '');
        opts[sel].scrollIntoView({ block: 'nearest' });
      } else if (e.key === 'Enter') {
        e.preventDefault();
        const q = input.value.trim();
        if (sel >= 0 && opts[sel]) { opts[sel].click(); return; }
        if (!q) return;
        if (QUESTION.test(q) || !hits.length) { close(); ctx.ask(q); } else { close(); const idx = indicesWith(BIT.SEARCH); if (idx.length) ctx.brain.focusOn(idx); }
      } else if (e.key === 'Escape') { close(); }
    });
    // grading a hit moves focus into the list; only close when focus leaves both
    const maybeClose = (e) => { if (results.contains(e.relatedTarget) || e.relatedTarget === input) return; setTimeout(() => { if (!results.contains(document.activeElement) && document.activeElement !== input) close(); }, 180); };
    input.addEventListener('blur', maybeClose);
    results.addEventListener('focusout', maybeClose);

    // ---------- nav ----------
    const navItems = [['brain', 'brain'], ['notes', 'notes'], ['write', 'write'], ['today', 'today']];
    const nav = h('div.nav', { role: 'list' });
    const navBtns = navItems.map(([page, label]) => {
      const b = h('button', { type: 'button', role: 'listitem', 'data-page': page, onclick: () => ctx.go(page) }, icon(page), h('span', label));
      nav.append(b);
      return b;
    });
    on('page', (p) => navBtns.forEach((b) => (b.dataset.page === p ? b.setAttribute('aria-current', 'page') : b.removeAttribute('aria-current'))));

    // ---------- + add and more ----------
    const addBtn = h('button.side-btn', { type: 'button', 'aria-haspopup': 'true', 'aria-expanded': 'false' }, icon('add'), h('span', '+ add'));
    const moreBtn = h('button.side-btn', { type: 'button', 'aria-haspopup': 'true', 'aria-expanded': 'false' }, icon('more'), h('span', 'more'));
    let openMenu = null;
    const closeMenu = () => { if (openMenu) { openMenu.menu.remove(); openMenu.btn.setAttribute('aria-expanded', 'false'); openMenu.btn.focus(); openMenu = null; } };
    const showMenu = (btn, build) => {
      if (openMenu && openMenu.btn === btn) { closeMenu(); return; }
      closeMenu();
      const menu = h('div.menu', { role: 'menu' });
      build(menu);
      const r = btn.getBoundingClientRect();
      menu.style.left = r.right + 14 + 'px';
      menu.style.top = Math.min(r.top - 8, innerHeight - 380) + 'px';
      document.body.append(menu);
      btn.setAttribute('aria-expanded', 'true');
      openMenu = { btn, menu };
      menu.querySelector('button, input, select')?.focus();
      menu.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') { e.stopPropagation(); closeMenu(); }
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          const items = [...menu.querySelectorAll('button, a, input, select')];
          const k = items.indexOf(document.activeElement);
          items[(k + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus(); e.preventDefault();
        }
      });
    };
    document.addEventListener('pointerdown', (e) => { if (openMenu && !openMenu.menu.contains(e.target) && !openMenu.btn.contains(e.target)) closeMenu(); });

    // open "+ add" with a kind chosen and the path field focused (the demo banner's "import a folder…")
    ctx.openAdd = (k = 'folder') => {
      if (!openMenu || openMenu.btn !== addBtn) addBtn.click();
      const sel = openMenu?.menu.querySelector('select');
      if (sel) sel.value = k;
      openMenu?.menu.querySelector('input')?.focus();
    };
    addBtn.addEventListener('click', () => showMenu(addBtn, (m) => {
      m.append(h('button', { type: 'button', role: 'menuitem', onclick: () => { closeMenu(); ctx.go('write'); } }, 'a new note'));
      const kind = h('select.quiet', { 'aria-label': 'what to add' },
        ...[['folder', 'a folder or vault'], ['obsidian', 'an obsidian vault'], ['pdf', 'a pdf folder'], ['rss', 'a feed'], ['web', 'a link']].map(([v, l]) => h('option', { value: v }, l)));
      const where = h('input', { type: 'text', placeholder: 'a folder’s path, or a url', 'aria-label': 'path or url', style: { width: '100%', background: 'transparent', border: 0, borderBottom: '1px solid var(--line)', padding: '4px 0', outline: 'none' } });
      const go = h('button.textbtn.strong', { type: 'button', onclick: async () => {
        const v = where.value.trim(); if (!v) { where.focus(); return; }
        const isUrl = /^https?:/.test(v);
        try {
          await api.post('/api/sources', { connector: kind.value, name: v.split(/[\\/]/).filter(Boolean).pop() || v, config: isUrl ? { url: v } : { path: v } });
          ctx.toast('added. it will appear as it is read.');
          ctx.followGrowth?.();
        } catch (e) { ctx.toast(e.serverMessage ? 'not added: ' + e.serverMessage : 'could not add that source.'); }
        closeMenu();
      } }, 'add');
      const more = h('button.textbtn', { type: 'button', onclick: () => { closeMenu(); ctx.openSettings('sources'); } }, 'all kinds…');
      m.append(h('h4', 'capture'), kind, where, h('div.btnrow', { style: { marginTop: '6px' } }, go, more));
    }));

    moreBtn.addEventListener('click', () => showMenu(moreBtn, (m) => {
      const item = (label, fn) => h('button', { type: 'button', role: 'menuitem', onclick: () => { closeMenu(); fn(); } }, label);
      m.append(
        h('h4', 'settings'),
        item('connections', () => ctx.openSettings('connections')),
        item('sources', () => ctx.openSettings('sources')),
        item('privacy · what left this machine', () => ctx.openSettings('privacy')),
        item('appearance', () => ctx.openSettings('appearance')),
        item(document.documentElement.dataset.theme === 'light' ? 'eye closed (dark)' : 'eye open (light)', () => {
          document.documentElement.dataset.theme = document.documentElement.dataset.theme === 'light' ? 'dark' : 'light';
          window.dispatchEvent(new CustomEvent('anamnesis:themechange'));
        }),
        item(theme().motion ? 'reduce motion' : 'full motion', () => {
          document.documentElement.style.setProperty('--motion', theme().motion ? '0' : '1');
          window.dispatchEvent(new CustomEvent('anamnesis:themechange'));
        }),
        item('evaluation', () => ctx.layout.open('eval')),
        h('h4', 'layout'));
      for (const p of ctx.layout.config.panels) {
        if (shortId(p.id) === 'sidebar' || !ctx.layout.registry[shortId(p.id)]) continue;
        const hidden = p.visible === false || (ctx.layout.place(p).startsWith('slot') && p.open === false);
        m.append(item(`${hidden ? 'show' : 'hide'} ${ctx.layout.title(p)}`, () => ctx.layout.setVisible(p.id, hidden)));
      }
      m.append(item('reset layout', () => ctx.layout.reset()), h('h4', 'data'), h('a', { href: (new URLSearchParams(location.search).get('api') || '') + '/api/export', role: 'menuitem', download: 'anamnesis-export.jsonl' }, 'export everything'));
    }));

    // ---------- mini brain ----------
    const mini = h('canvas.mini-brain', { role: 'img', 'aria-label': 'mini brain: a small turning view of the cloud with the current selection lit', tabindex: '0', title: 'back to the brain' });
    mini.addEventListener('click', () => ctx.go('brain'));
    mini.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); ctx.go('brain'); } });
    const miniCap = h('div.mini-caption', h('span', 'mini brain'), h('span.data', ''));
    const foot = h('div.side-foot', window.__anamnesisMock ? 'demo data · nothing leaves this machine' : 'local · nothing leaves this machine unasked');

    el.classList.add('sidebar');
    el.append(word, ask, nav, h('div.side-actions', addBtn, moreBtn), h('div.side-spacer'), mini, miniCap, foot);
    Object.assign(el.style, { display: 'contents' });

    let sample = [], yaw = 0, last = 0;
    const build = () => {
      const n = store.n; const k = Math.max(1, Math.floor(n / 1500));
      sample = []; for (let i = 0; i < n; i += k) sample.push(i);
      miniCap.lastChild.textContent = `${n} dots`;
    };
    on('cloud-ready', build);
    const draw = (now) => {
      requestAnimationFrame(draw);
      const t = theme();
      if (now - last < (t.motion ? 66 : 500)) return;
      const dt = (now - last) / 1000; last = now;
      if (t.motion) yaw += dt * 0.35;
      const A = ctx.brain.A; if (!A || !sample.length) return;
      const dpr = Math.min(2, devicePixelRatio || 1);
      const w = mini.clientWidth, hh = mini.clientHeight;
      if (!w) return;
      if (mini.width !== Math.round(w * dpr)) { mini.width = Math.round(w * dpr); mini.height = Math.round(hh * dpr); }
      const g = mini.getContext('2d');
      g.setTransform(dpr, 0, 0, dpr, 0, 0);
      g.clearRect(0, 0, w, hh);
      const c = Math.cos(yaw), s = Math.sin(yaw);
      const sc = Math.min(w, hh) * 0.36;
      const dot = rgba(t.colors.dot, 0.5), hot = rgba(t.colors['dot-hot'], 1);
      const f = store.flags;
      const anyLit = sample.some((i) => f[i] & LIT);
      for (const i of sample) {
        if (f[i] & BIT.FILTERED) continue;
        const x = A[i * 3], y = A[i * 3 + 1], z = A[i * 3 + 2];
        const rx = x * c + z * s, rz = -x * s + z * c;
        const lit = f[i] & LIT;
        const depth = 0.55 + 0.45 * (rz + 1.3) / 2.6;
        g.globalAlpha = lit ? 1 : (anyLit ? 0.25 : 0.65) * depth;
        g.fillStyle = lit ? hot : dot;
        const r = lit ? 1.3 : 0.7;
        g.fillRect(w / 2 + rx * sc - r / 2, hh / 2 - y * sc - r / 2, r, r);
      }
      g.globalAlpha = 1;
    };
    requestAnimationFrame(draw);
  },
};
