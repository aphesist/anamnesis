// The aphes.ist-style cursor: a disc in difference blend, the negative of whatever it covers. Small at rest,
// full over links and buttons (data-cursor=".55" for small controls), tightening on press. Fine pointers only.
// It shows only while <html data-cursor="custom">; theme.js sets that from the theme and the person's choice,
// so the system cursor comes back the moment they turn it off in settings.

let el = null;
let raf = 0;
let stopFns = [];

export function startCursor() {
  const d = globalThis.document;
  if (!d || el) return stop;
  if (!globalThis.matchMedia?.('(hover: hover) and (pointer: fine)').matches) return stop;
  el = d.createElement('div');
  el.className = 'anm-cursor';
  el.setAttribute('aria-hidden', 'true');
  d.body.appendChild(el);
  d.documentElement.classList.add('anm-has-cursor');
  let x = -100, y = -100, tx = -100, ty = -100, s = 0.2, ts = 0.2, pressed = false;
  const target = (node) => {
    const hit = node?.closest?.('[data-cursor], a, button, [role="button"], [role="radio"], [role="switch"], label, input, textarea, summary');
    if (!hit) return 0.2;
    const v = parseFloat(hit.getAttribute?.('data-cursor'));
    return Number.isFinite(v) ? v : 1;
  };
  const move = (e) => {
    tx = e.clientX;
    ty = e.clientY;
    ts = target(e.target);
    el.classList.add('on');
    // a modal dialog lives in the top layer: the disc must be inside it to stay visible
    const top = d.querySelector('dialog[open]');
    const host = top ?? d.body;
    if (el.parentNode !== host) host.appendChild(el);
  };
  const down = () => (pressed = true);
  const up = () => (pressed = false);
  const leave = () => el.classList.remove('on');
  d.addEventListener('pointermove', move, { passive: true });
  d.addEventListener('pointerdown', down, { passive: true });
  d.addEventListener('pointerup', up, { passive: true });
  d.documentElement.addEventListener('pointerleave', leave);
  const tick = () => {
    const calm = d.documentElement.getAttribute('data-motion') === 'reduced';
    const k = calm ? 1 : 0.32; // reduced motion: no trailing, the disc sits on the pointer
    x += (tx - x) * k;
    y += (ty - y) * k;
    s += ((pressed ? ts * 0.78 : ts) - s) * (calm ? 1 : 0.2);
    el.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px) scale(${s.toFixed(3)})`;
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
  stopFns = [
    () => d.removeEventListener('pointermove', move),
    () => d.removeEventListener('pointerdown', down),
    () => d.removeEventListener('pointerup', up),
    () => d.documentElement.removeEventListener('pointerleave', leave),
  ];
  return stop;
}

export function stop() {
  cancelAnimationFrame(raf);
  stopFns.forEach((f) => f());
  stopFns = [];
  el?.remove();
  el = null;
  globalThis.document?.documentElement.classList.remove('anm-has-cursor');
}
