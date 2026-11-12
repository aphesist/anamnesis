/* anamnesis landing page. No framework, no build step.
   1. the copy button on the build block
   2. the WebGL brain, loaded lazily after first paint, only with WebGL2 and only when motion is welcome;
      otherwise the static image already inside #brain stays. */

/* ---- copy button ---- */
function initCopy() {
  document.querySelectorAll('[data-copy]').forEach((btn) => {
    const source = document.getElementById(btn.dataset.copy);
    const status = document.getElementById(btn.getAttribute('aria-describedby'));
    if (!source) return;
    const label = btn.textContent;
    let timer = 0;
    const say = (state, words) => {
      btn.dataset.state = state;
      btn.textContent = words;
      if (status) status.textContent = state === 'done' ? 'copied to the clipboard' : 'could not copy, select the text and copy it by hand';
      clearTimeout(timer);
      timer = setTimeout(() => { delete btn.dataset.state; btn.textContent = label; if (status) status.textContent = ''; }, 2200);
    };
    const fallback = (text) => {
      const ta = document.createElement('textarea');
      ta.value = text; ta.setAttribute('readonly', ''); ta.style.cssText = 'position:fixed;left:-9999px;top:0;opacity:0';
      document.body.appendChild(ta); ta.select();
      let ok = false;
      try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
      ta.remove();
      return ok;
    };
    btn.addEventListener('click', async () => {
      const text = source.innerText.trim();
      let ok = false;
      try { await navigator.clipboard.writeText(text); ok = true; } catch (e) { ok = fallback(text); }
      say(ok ? 'done' : 'fail', ok ? 'copied' : 'select it');
    });
  });
}

/* ---- the brain ---- */
function hasWebGL2() {
  try {
    const c = document.createElement('canvas');
    const gl = c.getContext('webgl2');
    if (!gl) return false;
    const lose = gl.getExtension('WEBGL_lose_context');
    if (lose) lose.loseContext();
    return true;
  } catch (e) { return false; }
}

function afterFirstPaint(fn) {
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if ('requestIdleCallback' in window) requestIdleCallback(fn, { timeout: 2500 });
    else setTimeout(fn, 200);
  }));
}

async function initBrain() {
  const el = document.getElementById('brain');
  if (!el) return;
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (reduced || !hasWebGL2()) return;
  try {
    const mod = await import('./hero/brain.js');
    await mod.mount(el, { reducedMotion: false, labels: matchMedia('(min-width: 561px)').matches });
  } catch (e) {
    /* the static image stays */
    console.warn('anamnesis: brain not mounted', e);
  }
}

initCopy();
if (document.readyState === 'complete') afterFirstPaint(initBrain);
else window.addEventListener('load', () => afterFirstPaint(initBrain), { once: true });
