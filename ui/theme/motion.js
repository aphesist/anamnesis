// First-launch motion choice. anamnesis moves a lot by default (the aphes.ist feel); the first time it opens
// it asks once, with the operating system's reduced-motion setting as the pre-selected answer.
// The answer sets --motion (1 / 0) through theme.setMotion and can be changed later in settings.
//
//   import { askMotionOnce } from './theme/motion.js';
//   await theme.init();
//   await askMotionOnce();            // resolves 'full' | 'reduced'; does nothing if already answered

import * as theme from './theme.js';
import { brush, frame, stippleDensity, curve } from './brush.js';

const COPY = {
  title: 'motion',
  body: 'the brain drifts, turns and breathes. it can also keep still.',
  full: 'keep the motion',
  fullNote: 'drift, turning, camera flights',
  reduced: 'calm it',
  reducedNote: 'still brain, cross-fades only',
  os: 'your system asks for less motion, so calm is chosen.',
  osFull: 'your system has no motion preference.',
  later: 'you can change this later in more · motion.',
  go: 'go on',
};

/** asks once; resolves the choice ('full' | 'reduced'). opts.force asks again (for settings). */
export function askMotionOnce(opts = {}) {
  const s = theme.getState();
  if (s.motionChosen && !opts.force) return Promise.resolve(s.resolvedMotion);
  return showMotionDialog(opts);
}

export function showMotionDialog({ mount } = {}) {
  const d = document;
  const osReduced = theme.osPrefersReducedMotion();
  const current = theme.getState();
  let choice = current.motionChosen ? current.resolvedMotion : osReduced ? 'reduced' : 'full';

  const dlg = d.createElement('dialog');
  dlg.className = 'anm-motion';
  dlg.setAttribute('aria-labelledby', 'anm-motion-title');
  dlg.setAttribute('aria-describedby', 'anm-motion-body');
  dlg.innerHTML = `
    <svg class="anm-motion-frame" aria-hidden="true" focusable="false"><path fill="currentColor"/></svg>
    <form method="dialog" class="anm-motion-inner">
      <h2 id="anm-motion-title" class="anm-display anm-heading">${COPY.title}</h2>
      <p id="anm-motion-body" class="anm-lede">${COPY.body}</p>
      <div class="anm-motion-choices" role="radiogroup" aria-label="${COPY.title}">
        ${choiceHtml('full', COPY.full, COPY.fullNote)}
        ${choiceHtml('reduced', COPY.reduced, COPY.reducedNote)}
      </div>
      <p class="anm-meta">${osReduced ? COPY.os : COPY.osFull} ${COPY.later}</p>
      <button class="anm-label anm-motion-go" value="go">${COPY.go}<svg class="anm-underline" viewBox="0 -2 60 8" preserveAspectRatio="none" aria-hidden="true"><path fill="currentColor" d="${brushUnder(60, 21)}"/></svg></button>
    </form>`;
  injectStyle();
  (mount ?? d.body).appendChild(dlg);

  const radios = [...dlg.querySelectorAll('[role="radio"]')];
  const select = (v, focus) => {
    choice = v;
    for (const r of radios) {
      const on = r.dataset.value === v;
      r.setAttribute('aria-checked', String(on));
      r.tabIndex = on ? 0 : -1;
      if (on && focus) r.focus();
    }
    dlg.dataset.choice = v;
  };
  radios.forEach((r) => {
    r.addEventListener('click', () => select(r.dataset.value, true));
    r.addEventListener('keydown', (e) => {
      if (['ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft'].includes(e.key)) {
        e.preventDefault();
        select(choice === 'full' ? 'reduced' : 'full', true);
      } else if (e.key === ' ' || e.key === 'Enter') {
        e.preventDefault();
        select(r.dataset.value, true);
        if (e.key === 'Enter') dlg.querySelector('.anm-motion-go').click();
      }
    });
  });
  select(choice, false);

  const drawFrame = () => {
    const box = dlg.getBoundingClientRect();
    const svg = dlg.querySelector('.anm-motion-frame');
    const w = Math.max(10, box.width), h = Math.max(10, box.height);
    svg.setAttribute('viewBox', `-12 -12 ${w + 24} ${h + 24}`);
    svg.querySelector('path').setAttribute('d', frame(w, h, 11, 7, 22));
  };

  return new Promise((resolve) => {
    dlg.addEventListener('close', () => {
      theme.setMotion(choice); // escape and the button both keep the selected answer
      dlg.remove();
      resolve(choice);
    });
    dlg.showModal();
    drawFrame();
    radios.find((r) => r.dataset.value === choice)?.focus();
  });
}

function choiceHtml(value, label, note) {
  return `<div class="anm-motion-choice" role="radio" tabindex="-1" aria-checked="false" data-value="${value}">
      <svg class="anm-motion-swatch" viewBox="0 0 48 48" aria-hidden="true" focusable="false"><path fill="currentColor" d="${swatch(value)}"/></svg>
      <span class="anm-motion-text"><span class="anm-label">${label}</span><span class="anm-meta">${note}</span></span>
      <svg class="anm-underline" viewBox="0 -2 120 8" preserveAspectRatio="none" aria-hidden="true"><path fill="currentColor" d="${brushUnder(120, value === 'full' ? 4 : 8)}"/></svg>
    </div>`;
}

function brushUnder(w, seed) {
  return brush(curve([1, 2.6], [w / 2, 0.8], [w - 1, 2.2], 40), 2.2, seed, 0.32, 0.12);
}

// a small stipple cloud: loose and drifting for "keep the motion", gathered and still for "calm it"
function swatch(kind) {
  const loose = kind === 'full';
  return stippleDensity(
    (x, y) => {
      const r = Math.hypot(x - 24, y - 24);
      const ring = loose ? Math.exp(-((r - 15) ** 2) / 40) + 0.25 * Math.exp(-(r * r) / 60) : Math.exp(-(r * r) / 120);
      return ring * (loose ? 0.95 : 0.9);
    },
    { width: 48, height: 48, cell: 2, seed: loose ? 3 : 5 },
  );
}

let styled = false;
function injectStyle() {
  if (styled) return;
  styled = true;
  const st = document.createElement('style');
  st.id = 'anm-motion-style';
  st.textContent = `
  .anm-motion { position: fixed; inset: 0; margin: auto; width: min(520px, calc(100vw - 32px)); max-height: calc(100vh - 32px);
    overflow: visible; color: var(--ink); text-transform: lowercase; }
  .anm-motion[open] { animation: anm-motion-in calc(var(--dur-long) * 1) var(--ease) both; }
  .anm-motion-frame { position: absolute; left: -12px; top: -12px; width: calc(100% + 24px); height: calc(100% + 24px);
    color: var(--ink-soft); opacity: .85; pointer-events: none; overflow: visible; }
  .anm-motion-inner { position: relative; padding: var(--space-5) var(--space-5) var(--space-4); display: grid; gap: var(--space-4);
    background: color-mix(in oklab, var(--bg) 92%, transparent); }
  .anm-motion h2 { margin: 0; color: var(--ink); }
  .anm-motion p { margin: 0; }
  .anm-motion-choices { display: grid; gap: var(--space-2); }
  .anm-motion-choice { position: relative; display: grid; grid-template-columns: 48px 1fr; align-items: center; gap: var(--space-3);
    padding: var(--space-2) var(--space-2) var(--space-3); cursor: pointer; color: var(--ink-soft);
    transition: color var(--dur-short) var(--ease); }
  .anm-motion-choice .anm-motion-text { display: grid; gap: var(--space-1); }
  .anm-motion-choice .anm-label { color: inherit; }
  .anm-motion-choice .anm-underline { position: absolute; left: 60px; right: 0; bottom: 0; width: auto; opacity: 0;
    transition: opacity var(--dur-mid) var(--ease); color: var(--accent); }
  .anm-motion-choice[aria-checked="true"] { color: var(--ink); }
  .anm-motion-choice[aria-checked="true"] .anm-underline { opacity: 1; }
  .anm-motion-choice:hover { color: var(--ink); }
  /* focus is shown by the label turning accent and the underline, not by a box */
  .anm-motion-choice:focus-visible { outline: none; }
  .anm-motion-choice:focus-visible .anm-label { color: var(--accent-text); }
  .anm-motion-swatch { width: 48px; height: 48px; color: var(--fog); opacity: .72; }
  .anm-motion-choice[aria-checked="true"] .anm-motion-swatch { color: var(--accent); opacity: 1; }
  .anm-motion[data-choice="full"] .anm-motion-choice[data-value="full"] .anm-motion-swatch { animation: anm-motion-drift 5.2s ease-in-out infinite; }
  .anm-motion-go { justify-self: end; position: relative; padding: var(--space-2) 0; color: var(--ink); }
  .anm-motion-go .anm-underline { position: absolute; left: 0; right: 0; bottom: 0; color: var(--ink-soft); }
  .anm-motion-go:hover, .anm-motion-go:focus-visible { color: var(--accent-text); }
  @keyframes anm-motion-in { from { opacity: 0; filter: blur(10px); transform: translateY(14px); } to { opacity: 1; filter: none; transform: none; } }
  @keyframes anm-motion-drift { 0%,100% { transform: rotate(0) scale(1); } 50% { transform: rotate(24deg) scale(1.06); } }
  :root[data-motion="reduced"] .anm-motion[open] { animation-duration: var(--dur-short); }
  `;
  document.head.appendChild(st);
}

export { COPY as MOTION_COPY };
