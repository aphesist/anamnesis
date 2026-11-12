// anamnesis settings panel for theming: theme picker, token editor, fonts, type scale, motion and behaviours,
// logo, layout editor, export / import. Plain DOM, no framework; the app mounts it wherever it likes.
//
//   import { mountSettings } from './theme/settings.js';
//   const panel = await mountSettings(document.querySelector('#settings'));
//   panel.destroy();
//
// It drives theme.js and layout.js only, so anything it does can also be done from code.

import * as theme from './theme.js';
import * as layout from './layout.js';
import { showMotionDialog } from './motion.js';
import { srgbToOklch, hexToSrgb } from './tokens.js';
import { brush, curve } from './brush.js';

const COLOUR_TOKENS = [
  ['--bg', 'ground'],
  ['--ink', 'ink'],
  ['--ink-soft', 'soft ink'],
  ['--line', 'hairline'],
  ['--fog', 'fog'],
  ['--accent', 'accent'],
  ['--accent-text', 'accent text'],
  ['--seal', 'seal'],
  ['--dot', 'dot'],
  ['--dot-dim', 'dim dot'],
  ['--dot-hot', 'lit dot'],
  ['--thread', 'thread'],
  ['--region-label', 'region name'],
];

const FONT_CHOICES = [
  ['theme', null],
  ['special elite', '"Special Elite", "Courier New", monospace'],
  ['aphesis script', '"Aphesis Script", "Special Elite", serif'],
  ['serif', '"Iowan Old Style", Palatino, Georgia, serif'],
  ['sans', 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif'],
  ['mono', 'ui-monospace, Menlo, Consolas, monospace'],
];

const REGION_LABELS = { left: 'left', right: 'right', bottom: 'bottom', overlay: 'float' };

/** mounts the panel into `host`; returns { el, refresh, destroy } */
export async function mountSettings(host, opts = {}) {
  injectStyle();
  const el = h('div', { class: 'anm-settings', role: 'region', 'aria-label': 'appearance' });
  host.appendChild(el);
  let scope = 'mode'; // colour edits apply to: 'mode' (the current one) | 'all'
  let currentLayout = await layout.getLayout();
  let quiet = false; // set while a control in this panel is being dragged, so it is not rebuilt under the pointer
  const live = (fn) => {
    quiet = true;
    try {
      fn();
    } finally {
      quiet = false;
    }
  };

  const render = async () => {
    const s = theme.getState();
    const themes = await theme.listThemes();
    el.replaceChildren(
      section('theme', themeSection(s, themes)),
      section('colour', colourSection(s)),
      section('type', typeSection(s)),
      section('motion', motionSection(s)),
      section('mark', markSection(s)),
      section('layout', layoutSection(currentLayout)),
      section('keep', shareSection()),
    );
  };

  // ---------------------------------------------------------------- sections

  function themeSection(s, themes) {
    const list = h('div', { class: 'anm-s-themes', role: 'radiogroup', 'aria-label': 'theme' });
    for (const t of themes) {
      const pv = t.preview?.[s.resolvedMode] ?? t.preview?.dark ?? {};
      const b = choice(t.id === s.id, async () => theme.applyTheme(t.id), [
        h('span', { class: 'anm-s-swatches', 'aria-hidden': 'true' }, ['--bg', '--ink', '--accent', '--dot'].map((k) => h('i', { style: `background:${pv[k] ?? 'transparent'}` }))),
        h('span', { class: 'anm-label' }, t.name.toLowerCase()),
        h('span', { class: 'anm-meta' }, t.description ?? ''),
      ]);
      list.appendChild(b);
    }
    const modes = radioRow('light or dark', [
      ['theme', 'the theme’s own'],
      ['system', 'follow the system'],
      ['light', 'eye open'],
      ['dark', 'eye closed'],
    ], s.mode, (v) => theme.setMode(v));
    return [list, modes];
  }

  function colourSection(s) {
    const scopeRow = radioRow('apply to', [
      ['mode', `this mode (${s.resolvedMode === 'light' ? 'eye open' : 'eye closed'})`],
      ['all', 'both modes'],
    ], scope, (v) => {
      scope = v;
      render();
    });
    const grid = h('div', { class: 'anm-s-colours' });
    for (const [name, label] of COLOUR_TOKENS) {
      const c = theme.tokenRGB(name);
      const overridden = s.overrides?.all?.[name] != null || s.overrides?.[s.resolvedMode]?.[name] != null;
      const input = h('input', { type: 'color', value: c?.hex ?? '#000000', 'aria-label': label });
      input.addEventListener('input', () => {
        const [r, g, b] = hexToSrgb(input.value).srgb;
        const [L, C, H] = srgbToOklch(r, g, b);
        const alpha = c && c.alpha < 1 ? ` / ${round(c.alpha, 3)}` : '';
        live(() => theme.setToken(name, `oklch(${round(L, 4)} ${round(C, 4)} ${round(H, 2)}${alpha})`, scope === 'all' ? 'all' : s.resolvedMode));
        reset.hidden = false;
      });
      const reset = h('button', { type: 'button', class: 'anm-meta anm-s-reset', 'aria-label': `reset ${label}` }, 'reset');
      reset.hidden = !overridden;
      reset.addEventListener('click', () => {
        theme.setToken(name, null, 'all');
        theme.setToken(name, null, 'light');
        theme.setToken(name, null, 'dark');
      });
      grid.appendChild(h('label', { class: 'anm-s-colour' }, [input, h('span', { class: 'anm-label' }, label), h('code', { class: 'anm-meta' }, name), reset]));
    }
    const clear = button('reset all colours', () => theme.clearOverrides());
    return [scopeRow, grid, clear];
  }

  function typeSection(s) {
    const out = [];
    for (const role of ['display', 'text', 'data']) {
      const f = s.fonts?.[role];
      const active = !f ? 'theme' : f.source === 'builtin' ? FONT_CHOICES.find(([, v]) => v === f.stack)?.[0] ?? 'custom' : 'upload';
      const row = radioRow(role, FONT_CHOICES.map(([k]) => [k, k]).concat(f && f.source !== 'builtin' ? [['upload', f.name ?? f.family]] : []), active, (v) => {
        const stack = FONT_CHOICES.find(([k]) => k === v)?.[1] ?? null;
        if (v !== 'upload') theme.setFont(role, stack);
      });
      const file = fileButton(`add a font for ${role}`, '.woff2,.woff,.ttf,.otf,font/*', async (file) => {
        try {
          await theme.addFont(role, file);
        } catch (e) {
          note(`that font could not be read (${e.message ?? e}).`);
        }
      });
      row.appendChild(file);
      row.appendChild(h('span', { class: `anm-s-sample anm-s-sample-${role}` }, role === 'data' ? '0123456789 · 1,618' : 'the archive keeps what the fog would erase'));
      out.push(row);
    }
    const scale = h('input', { type: 'range', min: '0.8', max: '1.4', step: '0.05', value: String(s.typeScale ?? 1), 'aria-label': 'type size' });
    const val = h('span', { class: 'anm-meta anm-data' }, `${Math.round((s.typeScale ?? 1) * 100)}%`);
    scale.addEventListener('input', () => {
      val.textContent = `${Math.round(scale.value * 100)}%`;
      live(() => theme.setTypeScale(Number(scale.value)));
    });
    out.push(h('div', { class: 'anm-s-row' }, [h('span', { class: 'anm-label anm-s-key' }, 'size'), scale, val]));
    return out;
  }

  function motionSection(s) {
    return [
      radioRow('motion', [
        ['full', 'keep the motion'],
        ['reduced', 'calm it'],
      ], s.resolvedMotion, (v) => theme.setMotion(v)),
      radioRow('scrollbars', [
        ['hidden', 'hidden'],
        ['system', 'the system’s'],
      ], s.resolvedScrollbars, (v) => theme.setScrollbars(v)),
      button('ask about motion again', () => showMotionDialog()),
    ];
  }

  function markSection(s) {
    const prev = h('div', { class: 'anm-s-marks' }, [h('span', { class: 'anm-mark', role: 'img', 'aria-label': 'mark' }), h('span', { class: 'anm-wordmark', role: 'img', 'aria-label': 'wordmark' })]);
    const rows = ['mark', 'wordmark'].map((kind) =>
      h('div', { class: 'anm-s-row' }, [
        h('span', { class: 'anm-label anm-s-key' }, kind),
        fileButton(`choose a ${kind} (svg or png)`, 'image/svg+xml,image/png,.svg,.png', async (file) => {
          try {
            await theme.setLogo(file, kind);
          } catch (e) {
            note(e.message ?? String(e));
          }
        }),
        s.logo?.[kind] ? button('use the theme’s', () => theme.setLogo(null, kind)) : null,
      ]),
    );
    return [prev, ...rows, h('p', { class: 'anm-meta' }, 'a mark is drawn in the seal colour; a png keeps its own colours where the app shows it as a picture.')];
  }

  function layoutSection(l) {
    const out = [];
    out.push(
      h('div', { class: 'anm-s-row' }, [
        h('span', { class: 'anm-label anm-s-key' }, 'start from'),
        ...layout.PRESETS.map((p) => button(p, async () => setLayout(await layout.resetLayout(p), false))),
      ]),
    );
    for (const region of layout.REGIONS) {
      const panels = layout.panelsIn(l, region, true);
      if (!panels.length) continue;
      const list = h('ol', { class: 'anm-s-panels', 'aria-label': REGION_LABELS[region] });
      for (const p of panels) {
        const vis = h('input', { type: 'checkbox', 'aria-label': `show ${p.label ?? p.id}` });
        vis.checked = p.visible;
        vis.addEventListener('change', () => setLayout(layout.updatePanel(l, p.id, { visible: vis.checked })));
        const open = h('input', { type: 'checkbox', 'aria-label': `${p.label ?? p.id} starts open` });
        open.checked = p.open;
        open.addEventListener('change', () => setLayout(layout.updatePanel(l, p.id, { open: open.checked })));
        const where = radioRow(`${p.label ?? p.id}: where`, layout.REGIONS.map((r) => [r, REGION_LABELS[r]]), p.region, (r) => setLayout(layout.updatePanel(l, p.id, { region: r })), true);
        list.appendChild(
          h('li', { class: 'anm-s-panel' + (p.visible ? '' : ' is-hidden') }, [
            h('label', { class: 'anm-s-check' }, [vis, h('span', { class: 'anm-label' }, p.label ?? p.id)]),
            h('label', { class: 'anm-s-check anm-meta' }, [open, 'open']),
            where,
            h('span', { class: 'anm-s-move' }, [
              iconButton('↑', `move ${p.label ?? p.id} up`, () => setLayout(layout.movePanel(l, p.id, -1))),
              iconButton('↓', `move ${p.label ?? p.id} down`, () => setLayout(layout.movePanel(l, p.id, +1))),
            ]),
          ]),
        );
      }
      out.push(h('h4', { class: 'anm-meta anm-s-region' }, REGION_LABELS[region]), list);
    }
    return out;
  }

  function shareSection() {
    const exp = button('export this look', async () => {
      const data = await theme.exportTheme();
      data.layout = currentLayout;
      download(`anamnesis-theme-${data.base}.json`, JSON.stringify(data, null, 2));
    });
    const imp = fileButton('import a look', 'application/json,.json', async (file) => {
      try {
        const data = JSON.parse(await file.text());
        await theme.importTheme(data);
        if (data.layout) setLayout(data.layout);
        note('imported.');
      } catch (e) {
        note(`that file is not a theme (${e.message ?? e}).`);
      }
    });
    const reset = button('return to the defaults', async () => {
      if (reset.dataset.sure !== '1') {
        reset.dataset.sure = '1';
        reset.firstChild.textContent = 'return to the defaults · sure?';
        return;
      }
      await theme.reset();
      setLayout(await layout.resetLayout('default'), false);
    });
    return [h('div', { class: 'anm-s-row' }, [exp, imp, reset]), h('p', { class: 'anm-meta anm-s-note', role: 'status', 'aria-live': 'polite' })];
  }

  // ---------------------------------------------------------------- helpers

  function setLayout(l, save = true) {
    currentLayout = save ? layout.saveLayout(l) : l;
    opts.onLayoutChange?.(currentLayout);
    render();
  }

  function note(text) {
    const n = el.querySelector('.anm-s-note');
    if (n) n.textContent = text;
  }

  const off = theme.onThemeChange(() => {
    if (!quiet) render();
  });
  await render();
  return {
    el,
    refresh: render,
    destroy() {
      off();
      el.remove();
    },
  };
}

function section(title, children) {
  const id = 'anm-s-' + title;
  return h('section', { class: 'anm-s-section', 'aria-labelledby': id }, [h('h3', { id, class: 'anm-display anm-s-title' }, title), ...[].concat(children).filter(Boolean)]);
}

function choice(checked, onPick, children) {
  const b = h('button', { type: 'button', role: 'radio', 'aria-checked': String(checked), class: 'anm-s-choice' + (checked ? ' is-on' : '') }, children);
  b.addEventListener('click', onPick);
  return b;
}

function radioRow(label, options, value, onPick, compact = false) {
  const group = h('div', { class: 'anm-s-row' + (compact ? ' is-compact' : ''), role: 'radiogroup', 'aria-label': label });
  if (!compact) group.appendChild(h('span', { class: 'anm-label anm-s-key' }, label));
  const buttons = options.map(([v, text]) => {
    const on = v === value;
    const b = h('button', { type: 'button', role: 'radio', 'aria-checked': String(on), tabindex: on ? '0' : '-1', class: 'anm-s-opt anm-label' + (on ? ' is-on' : '') }, [
      text,
      underline(),
    ]);
    b.addEventListener('click', () => onPick(v));
    return b;
  });
  // roving focus with the arrow keys, as a radio group should
  buttons.forEach((b, i) =>
    b.addEventListener('keydown', (e) => {
      const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
      if (!d) return;
      e.preventDefault();
      const n = buttons[(i + d + buttons.length) % buttons.length];
      n.focus();
      n.click();
    }),
  );
  if (!buttons.some((b) => b.tabIndex === 0) && buttons[0]) buttons[0].tabIndex = 0;
  buttons.forEach((b) => group.appendChild(b));
  return group;
}

function button(text, onClick) {
  const b = h('button', { type: 'button', class: 'anm-s-btn anm-label' }, [text, underline()]);
  b.addEventListener('click', onClick);
  return b;
}

function iconButton(text, label, onClick) {
  const b = h('button', { type: 'button', class: 'anm-s-icon', 'aria-label': label }, text);
  b.addEventListener('click', onClick);
  return b;
}

function fileButton(text, accept, onFile) {
  const input = h('input', { type: 'file', accept, class: 'anm-s-file' });
  input.addEventListener('change', () => {
    const f = input.files?.[0];
    if (f) onFile(f);
    input.value = '';
  });
  return h('label', { class: 'anm-s-btn anm-label' }, [input, text, underline()]);
}

let underSeed = 40;
function underline() {
  const seed = underSeed++ % 97;
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'anm-underline');
  svg.setAttribute('viewBox', '0 -2 100 8');
  svg.setAttribute('preserveAspectRatio', 'none');
  svg.setAttribute('aria-hidden', 'true');
  const p = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  p.setAttribute('fill', 'currentColor');
  p.setAttribute('d', brush(curve([1, 2.6], [50, 0.8], [99, 2.2], 36), 2.1, seed, 0.32, 0.12));
  svg.appendChild(p);
  return svg;
}

function download(name, text) {
  const a = h('a', { href: URL.createObjectURL(new Blob([text], { type: 'application/json' })), download: name });
  document.body.appendChild(a);
  a.click();
  setTimeout(() => {
    URL.revokeObjectURL(a.href);
    a.remove();
  }, 1000);
}

function round(n, d) {
  const k = 10 ** d;
  return Math.round(n * k) / k;
}

function h(tag, attrs = {}, children = []) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null) continue;
    if (k === 'class') el.className = v;
    else if (k === 'value' && tag === 'input') el.value = v;
    else el.setAttribute(k, v);
  }
  for (const c of [].concat(children)) {
    if (c == null) continue;
    el.appendChild(typeof c === 'string' ? document.createTextNode(c) : c);
  }
  return el;
}

let styled = false;
function injectStyle() {
  if (styled) return;
  styled = true;
  const st = document.createElement('style');
  st.id = 'anm-settings-style';
  st.textContent = `
  .anm-settings { display: grid; gap: var(--space-6); color: var(--ink); text-transform: lowercase; max-width: 760px; }
  .anm-s-section { display: grid; gap: var(--space-3); padding-top: var(--space-3); border-top: 1px solid var(--line); }
  .anm-s-title { margin: 0 0 var(--space-1); font-size: calc(28px * var(--type-scale, 1)); color: var(--ink); }
  .anm-s-row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-4); }
  .anm-s-row.is-compact { gap: var(--space-1) var(--space-3); }
  .anm-s-key { min-width: 7.5em; color: var(--ink-soft); }
  .anm-s-opt, .anm-s-btn { position: relative; padding: var(--space-1) 0 var(--space-2); color: var(--ink-soft);
    transition: color var(--dur-short) var(--ease); cursor: pointer; white-space: nowrap; }
  .anm-s-opt .anm-underline, .anm-s-btn .anm-underline { position: absolute; left: 0; right: 0; bottom: 0; width: 100%;
    opacity: 0; transition: opacity var(--dur-mid) var(--ease); }
  .anm-s-btn .anm-underline { opacity: .5; }
  .anm-s-opt:hover, .anm-s-btn:hover, .anm-s-btn:focus-within { color: var(--ink); }
  .anm-s-opt.is-on { color: var(--ink); }
  .anm-s-opt.is-on .anm-underline { opacity: 1; color: var(--accent); }
  .anm-s-btn:hover .anm-underline { color: var(--accent); opacity: 1; }
  .anm-s-file { position: absolute; width: 1px; height: 1px; opacity: 0; overflow: hidden; }
  .anm-s-btn:focus-within { outline: 1px solid var(--accent); outline-offset: 4px; }
  .anm-s-themes { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: var(--space-3); }
  .anm-s-choice { display: grid; gap: var(--space-1); text-align: left; padding: var(--space-3) 0; color: var(--ink-soft);
    border-bottom: 1px solid var(--line); transition: color var(--dur-short) var(--ease), border-color var(--dur-short) var(--ease); }
  .anm-s-choice.is-on { color: var(--ink); border-bottom-color: var(--accent); }
  .anm-s-choice:hover { color: var(--ink); }
  .anm-s-swatches { display: flex; gap: 3px; }
  .anm-s-swatches i { width: 14px; height: 14px; outline: 1px solid var(--line); }
  .anm-s-colours { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--space-2) var(--space-4); }
  .anm-s-colour { display: grid; grid-template-columns: 28px 1fr auto; grid-template-rows: auto auto; column-gap: var(--space-2); align-items: center; cursor: pointer; }
  .anm-s-colour input { grid-row: span 2; width: 28px; height: 28px; padding: 0; border: 0; background: none; cursor: pointer; }
  .anm-s-colour input::-webkit-color-swatch-wrapper { padding: 0; }
  .anm-s-colour input::-webkit-color-swatch { border: 1px solid var(--line); border-radius: 0; }
  .anm-s-colour input::-moz-color-swatch { border: 1px solid var(--line); border-radius: 0; }
  .anm-s-colour code { grid-column: 2; font-family: var(--font-data); }
  .anm-s-reset { grid-row: span 2; color: var(--accent-text); }
  .anm-s-sample { flex-basis: 100%; color: var(--ink-soft); font-size: calc(18px * var(--type-scale, 1)); }
  .anm-s-sample-display { font-family: var(--font-display); font-variation-settings: "slnt" var(--type-display-slant, 0); font-size: calc(26px * var(--type-scale, 1)); }
  .anm-s-sample-text { font-family: var(--font-text); }
  .anm-s-sample-data { font-family: var(--font-data); font-variant-numeric: tabular-nums; }
  .anm-s-marks { display: flex; align-items: flex-end; gap: var(--space-5); }
  .anm-s-panels { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--space-1); }
  .anm-s-panel { display: grid; grid-template-columns: minmax(10em, 1fr) auto auto auto; align-items: center; gap: var(--space-3);
    padding: var(--space-1) 0; border-bottom: 1px dotted var(--line); }
  .anm-s-panel.is-hidden .anm-label { color: var(--ink-soft); text-decoration: line-through; text-decoration-color: var(--line); }
  .anm-s-check { display: inline-flex; align-items: center; gap: var(--space-2); cursor: pointer; }
  .anm-s-region { margin: var(--space-2) 0 0; font-weight: normal; }
  .anm-s-move { display: inline-flex; gap: var(--space-1); }
  .anm-s-icon { width: 24px; height: 24px; color: var(--ink-soft); font-family: var(--font-data); }
  .anm-s-icon:hover { color: var(--accent-text); }
  @media (max-width: 560px) { .anm-s-panel { grid-template-columns: 1fr auto; } }
  `;
  document.head.appendChild(st);
}
