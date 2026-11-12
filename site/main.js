/* anamnesis · anamnesis.aphes.ist. No framework, no build step. The aphes.ist runtime (vendor/aphesis/bundle.js,
   window.Aphesis) draws the ground and the marks, set up as the site's shell sets them up, kept quiet:
   1. the fog: one Aphesis.Fog on a fixed canvas, low, calm behind the words on screen and around the brain, and
      drifting out of the column once the hero has gone by
   2. the header: the lab kit's receding header (vendor/lab-kit-v1), with anamnesis's icon beside the name and the
      eye (the theme switch, a wave from it). The tab's icon is the eye too, as on aphes.ist
   3. the cursor, the tooltips, the letter waves, the brushed strokes under the actions
   4. the brain, loaded after the first paint (hero/brain.js); the still image inside #brain stays until its first
      frame is on screen, and for good if WebGL2 is missing
   A reader who asks for less motion gets the same page held still: one frame of fog, a still brain, no boil. */
(function () {
  'use strict';
  const A = window.Aphesis;
  const $ = (s, r) => (r || document).querySelector(s);
  const $$ = (s, r) => Array.from((r || document).querySelectorAll(s));
  const ROOT = document.documentElement;
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const phoneQ = matchMedia('(max-width: 720px)');

  if (!A) return; /* without the runtime the page is plain words and pictures, which is enough */

  /* ---------- the fog: low, so the brain and the words lead ---------- */
  const HERO = $('.hero'), BRAIN = $('#brain');
  const look = () => (phoneQ.matches ? { density: 0.22, light: 0.1 } : { density: 0.28, light: 0.14 });
  const fog = A.Fog($('#fog'), Object.assign({ clear: 0 }, look()));
  /* a still fog: let it draw once (and again when something changes), then hold */
  let holdT = 0;
  function hold(ms) { if (!still) return; fog.start(); clearTimeout(holdT); holdT = setTimeout(() => fog.stop(), ms); }
  hold(400);
  document.addEventListener('visibilitychange', () => { if (document.hidden) fog.stop(); else if (!still) fog.start(); else hold(400); });
  phoneQ.addEventListener('change', () => { fog.set(look()); hold(400); });

  /* calm zones: the brain, and the words on screen (twelve at most, the fog's limit) */
  const ZONED = '.title, .hero-text > .lede, .question, .hero .acts, .hero-meta, .head, .does > li, .os > li, .sec > .note, .sec > .lede, .sec > .acts, .plain li, .code, .sub, #pictures > .small, .shot figcaption, .foot-cols, .made, .notices';
  const zoned = $$(ZONED);
  let zoneKey = '', zq = 0;
  function zones() {
    zq = 0;
    const H = innerHeight, list = [];
    const br = BRAIN.getBoundingClientRect();
    if (br.bottom > 0 && br.top < H) list.push({ el: BRAIN, pad: 0 });
    for (const el of zoned) {
      const r = el.getBoundingClientRect();
      if (r.bottom > -40 && r.top < H + 40 && r.width) list.push({ el, pad: 16 });
      if (list.length === 12) break;
    }
    const key = list.map((z) => (z.el === BRAIN ? 'b' : zoned.indexOf(z.el))).join(',');
    if (key !== zoneKey) { zoneKey = key; fog.zones(list); }
    fog.set({ clear: HERO.getBoundingClientRect().bottom < H * 0.45 ? 1 : 0 });
    hold(250);
  }
  const queueZones = () => { if (!zq) zq = requestAnimationFrame(zones); };
  addEventListener('scroll', queueZones, { passive: true });
  addEventListener('resize', queueZones);
  zones();

  /* ---------- the header (the lab kit's: the φ, the name, the parts, the eye) ----------
     the project's own icon beside its name: six small linked rings, three lit with stipple, as if being recalled.
     Hovering the name stirs its stipple (.aph-icon); pressing gathers it */
  const project = $('.lab-project');
  $('.project-icon').innerHTML = A.icon('anamnesis', 0);
  project.addEventListener('pointerdown', () => project.classList.add('aph-gather'));
  project.addEventListener('pointerleave', () => project.classList.remove('aph-gather'));

  /* the eye: open in light, closed in dark; the switch is a blink, and the theme spreads as a wave from it (drawn as
     aphes.ist's shell draws it) */
  const EYE = {
    open: A.brush(A.arc(12, 15.5, 9, -160, -20, 40, 7), 1.7, 31) + A.brush(A.arc(12, 8.5, 9, 20, 160, 40, 7), 1.7, 33),
    iris: A.stipple(A.inC(12, 12, 3.1), 36, 260, 0.95),
    closed: (() => { let d = A.brush(A.arc(12, 8, 9, 25, 155, 40, 6), 1.8, 37);
      [-0.55, 0, 0.55].forEach((k, i) => { const x = 12 + k * 9.5; d += A.brush(A.line(x, 14.2 + Math.abs(k) * -1.2, x + k * 1.8, 17.2 + Math.abs(k) * -1, 10), 1.2, 41 + i, 0.35, 0.05); }); return d; })(),
  };
  const eye = $('#eye'), O = $('#iopen'), I = $('#iris'), C = $('#iclosed');
  O.setAttribute('d', EYE.open); I.setAttribute('d', EYE.iris); C.setAttribute('d', EYE.closed);
  const sysDark = matchMedia('(prefers-color-scheme: dark)');
  const isDark = () => (ROOT.dataset.theme ? ROOT.dataset.theme === 'dark' : sysDark.matches);
  function themeIcon() {
    const dark = isDark();
    O.style.opacity = dark ? 0 : 1; O.style.transform = dark ? 'scaleY(.08)' : 'none';
    I.style.opacity = dark ? 0 : 1; I.style.transform = dark ? 'scaleY(.05)' : 'none';
    C.style.opacity = dark ? 1 : 0; C.style.transform = dark ? 'none' : 'scaleY(.2) translateY(-6px)';
    eye.dataset.tip = dark ? 'closed · open the eye' : 'open · close the eye';
    eye.setAttribute('aria-pressed', String(dark));
    hold(1200);
  }
  themeIcon(); sysDark.addEventListener('change', themeIcon);
  eye.addEventListener('click', () => {
    const r = eye.getBoundingClientRect(), dark = isDark();
    const apply = () => { ROOT.dataset.theme = dark ? 'light' : 'dark'; themeIcon(); };
    if (still) { apply(); return; }
    A.themeShift(apply, { origin: [r.left + r.width / 2, r.top + r.height / 2] });
  });

  /* the tab's icon: the eye, open while the page is seen, closed while it is hidden or the window is left */
  if (typeof favicons === 'function') {
    const FAV = favicons(A, { light: '#ab0d19', dark: '#d43031' }); /* rubedo, light and dark (tokens.json) */
    const link = $('#favicon');
    const fav = (seen) => { const h = seen ? FAV.open : FAV.closed; if (link.getAttribute('href') !== h) link.setAttribute('href', h); };
    fav(!document.hidden && document.hasFocus());
    document.addEventListener('visibilitychange', () => fav(!document.hidden));
    addEventListener('blur', () => fav(false));
    addEventListener('focus', () => fav(!document.hidden));
  }

  /* ---------- the hand: cursor, tooltips, letters, the strokes under the actions ---------- */
  A.cursor();
  A.tooltip();
  $$('.act-w, .lab-nav a').forEach((el) => A.letters(el)); /* the actions and the header's parts */
  let seed = 70;
  const acts = $$('.act').map((a) => ({ a, seed: seed += 3, w: 0 }));
  function strokes() {
    for (const s of acts) {
      const w = Math.round(s.a.querySelector('.act-w').getBoundingClientRect().width);
      if (!w || w === s.w) continue;
      s.w = w;
      let svg = s.a.querySelector('svg');
      if (!svg) { svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.setAttribute('aria-hidden', 'true'); svg.innerHTML = '<path/>'; s.a.appendChild(svg); }
      svg.setAttribute('viewBox', `0 0 ${w} 6`);
      svg.firstChild.setAttribute('d', A.brush(A.line(1, 3.2, w - 1, 2.6, Math.max(12, Math.round(w / 4))), 1.6, s.seed, 0.4, 0.08));
    }
  }
  (document.fonts ? document.fonts.ready : Promise.resolve()).then(() => { strokes(); queueZones(); });
  addEventListener('resize', strokes);
  strokes();

  /* ---------- install: say which one is this computer ---------- */
  const nav = navigator.userAgent.toLowerCase();
  const ua = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || nav).toLowerCase();
  const os = /win/.test(ua) ? 'windows' : /mac/.test(ua) && !/iphone|ipad/.test(nav) ? 'mac' : /linux|x11/.test(ua) && !/android/.test(nav) ? 'linux' : '';
  if (os) { const li = $(`.os [data-os="${os}"]`); if (li) { li.querySelector('.yours').hidden = false; li.parentNode.prepend(li); } }

  /* ---------- the copy button on the build block ---------- */
  $$('[data-copy]').forEach((btn) => {
    const source = document.getElementById(btn.dataset.copy), status = document.getElementById(btn.getAttribute('aria-describedby'));
    const word = btn.querySelector('.act-w');
    let timer = 0;
    const set = (t) => { word.removeAttribute('data-split'); word.textContent = t; A.letters(word); strokes(); };
    btn.addEventListener('click', async () => {
      const text = source.innerText.trim();
      let ok = false;
      try { await navigator.clipboard.writeText(text); ok = true; } catch (e) {
        const r = document.createRange(); r.selectNodeContents(source); const s = getSelection(); s.removeAllRanges(); s.addRange(r);
        try { ok = document.execCommand('copy'); } catch (e2) { ok = false; }
      }
      set(ok ? 'copied' : 'select it, then copy');
      if (status) status.textContent = ok ? 'copied to the clipboard' : 'could not copy; the text is selected, copy it by hand';
      clearTimeout(timer);
      timer = setTimeout(() => { set('copy'); if (status) status.textContent = ''; }, 2200);
    });
  });

  /* ---------- the brain ---------- */
  function hasWebGL2() {
    try {
      const gl = document.createElement('canvas').getContext('webgl2');
      if (!gl) return false;
      const lose = gl.getExtension('WEBGL_lose_context'); if (lose) lose.loseContext();
      return true;
    } catch (e) { return false; }
  }
  async function brain() {
    if (!BRAIN || !hasWebGL2()) return;
    try {
      const mod = await import('./hero/brain.js');
      mod.mount(BRAIN, { reducedMotion: still, labels: !phoneQ.matches, zoom: phoneQ.matches ? 1.15 : 1 });
    } catch (e) { /* the still image stays */ }
  }
  const afterPaint = (fn) => requestAnimationFrame(() => requestAnimationFrame(() => ('requestIdleCallback' in window ? requestIdleCallback(fn, { timeout: 2500 }) : setTimeout(fn, 200))));
  if (document.readyState === 'complete') afterPaint(brain); else addEventListener('load', () => afterPaint(brain), { once: true });
})();
