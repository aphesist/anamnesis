// anamnesis theme tokens: a small, dependency-free compiler from W3C Design Tokens
// (Format Module 2025.10) JSON to CSS custom properties. Runs in node (scripts/build-tokens.mjs)
// and in the browser (theme.js, for imported themes and user overrides).
//
// What it understands:
// - groups and tokens (`$value`, `$type`, `$description`, `$extensions`, `$deprecated`), with `$type`
//   inherited from the nearest ancestor group;
// - curly-brace aliases (`"{color.neutral.975}"`), chained, with cycle detection;
// - types: color (2025.10 object form, or a CSS string), dimension, duration, cubicBezier, number,
//   fontFamily, fontWeight; composite types are ignored by the CSS output.
// - two anamnesis extensions on groups, under `$extensions["ist.anamnesis"]`:
//     `flatten: true`  the group's name is not part of the css name (role.bg -> --bg)
//     `emit: false`    the group's tokens are primitives: resolvable as aliases, never emitted
//
// The css name of a token is `--` + its path joined with `-` (minus flattened groups).

export const EXT = 'ist.anamnesis';

/** deep merge of token documents; later documents win, objects merge, everything else replaces */
export function mergeTokens(...docs) {
  const out = {};
  for (const doc of docs) if (doc) mergeInto(out, doc);
  return out;
}

function mergeInto(target, src) {
  for (const [k, v] of Object.entries(src)) {
    if (k === '$value' || !isPlainObject(v) || !isPlainObject(target[k])) {
      target[k] = clone(v);
    } else {
      mergeInto(target[k], v);
    }
  }
  return target;
}

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function clone(v) {
  return v === undefined ? v : JSON.parse(JSON.stringify(v));
}

/**
 * Walks a token document. Returns a Map path -> { path, type, value, description, emit, cssName }.
 * `value` is still raw (may be an alias).
 */
export function flattenTokens(doc) {
  const tokens = new Map();
  const walk = (node, path, inherited) => {
    const type = node.$type ?? inherited.type;
    const ext = node.$extensions?.[EXT] ?? {};
    const emit = ext.emit === false ? false : inherited.emit;
    const flat = ext.flatten === true;
    if ('$value' in node) {
      const nameParts = path.filter((_, i) => !inherited.flattened.has(i));
      tokens.set(path.join('.'), {
        path: path.join('.'),
        type,
        value: node.$value,
        description: node.$description,
        emit,
        cssName: '--' + nameParts.join('-'),
      });
      return;
    }
    for (const [key, child] of Object.entries(node)) {
      if (key.startsWith('$') || !isPlainObject(child)) continue;
      if (/[{}.]/.test(key)) throw new Error(`invalid token name "${key}" at ${path.join('.') || '(root)'}`);
      const flattened = new Set(inherited.flattened);
      if (flat) flattened.add(path.length - 1);
      walk(child, [...path, key], { type, emit, flattened });
    }
  };
  // the root's own flags apply to nothing; groups below decide
  for (const [key, child] of Object.entries(doc)) {
    if (key.startsWith('$') || !isPlainObject(child)) continue;
    walk(child, [key], { type: doc.$type, emit: true, flattened: new Set() });
  }
  return tokens;
}

const ALIAS = /^\{([^{}]+)\}$/;

/** resolves aliases; returns Map path -> { ...token, value (resolved), type (resolved) } */
export function resolveTokens(tokens) {
  const out = new Map();
  const resolving = new Set();
  const resolve = (path) => {
    if (out.has(path)) return out.get(path);
    const t = tokens.get(path);
    if (!t) throw new Error(`unknown token reference {${path}}`);
    if (resolving.has(path)) throw new Error(`alias cycle at {${path}}`);
    resolving.add(path);
    let value = t.value;
    let type = t.type;
    const m = typeof value === 'string' ? value.match(ALIAS) : null;
    if (m) {
      const target = resolve(m[1]);
      value = target.value;
      type = type ?? target.type;
    } else {
      value = resolveNested(value, resolve);
    }
    resolving.delete(path);
    const r = { ...t, value, type };
    out.set(path, r);
    return r;
  };
  for (const path of tokens.keys()) resolve(path);
  return out;
}

// aliases may also sit inside composite values (e.g. a color component, a typography field)
function resolveNested(v, resolve) {
  if (typeof v === 'string') {
    const m = v.match(ALIAS);
    return m ? resolve(m[1]).value : v;
  }
  if (Array.isArray(v)) return v.map((x) => resolveNested(x, resolve));
  if (isPlainObject(v)) {
    const o = {};
    for (const [k, x] of Object.entries(v)) o[k] = resolveNested(x, resolve);
    return o;
  }
  return v;
}

// ---------------------------------------------------------------- values -> css

const GENERIC_FAMILIES = new Set([
  'serif', 'sans-serif', 'monospace', 'cursive', 'fantasy', 'system-ui', 'ui-serif', 'ui-sans-serif',
  'ui-monospace', 'ui-rounded', 'emoji', 'math', 'fangsong',
]);

const num = (n) => {
  if (typeof n !== 'number') return String(n);
  return String(Math.round(n * 1e5) / 1e5);
};

/** converts one resolved token value to a css string, or null if the type has no css form */
export function toCss(type, value) {
  if (value == null) return null;
  switch (type) {
    case 'color':
      return colorToCss(value);
    case 'dimension':
    case 'duration':
      if (isPlainObject(value)) return num(value.value) + value.unit;
      return String(value);
    case 'cubicBezier':
      return Array.isArray(value) ? `cubic-bezier(${value.map(num).join(', ')})` : String(value);
    case 'number':
    case 'fontWeight':
      return typeof value === 'number' ? num(value) : String(value);
    case 'fontFamily': {
      const list = Array.isArray(value) ? value : [value];
      return list.map((f) => (GENERIC_FAMILIES.has(f) ? f : `"${String(f).replace(/"/g, '')}"`)).join(', ');
    }
    case 'string':
      return String(value);
    default:
      return typeof value === 'string' || typeof value === 'number' ? String(value) : null;
  }
}

/** DTCG 2025.10 colour object (or css string) -> css colour string */
export function colorToCss(c) {
  if (typeof c === 'string') return c;
  const comps = (c.components ?? []).map((x) => (x === 'none' ? 'none' : num(x)));
  const alpha = c.alpha != null && c.alpha !== 1 ? ` / ${num(c.alpha)}` : '';
  switch (c.colorSpace) {
    case 'oklch':
    case 'oklab':
    case 'lab':
    case 'lch':
      return `${c.colorSpace}(${comps.join(' ')}${alpha})`;
    case 'hsl':
      return `hsl(${comps[0]} ${comps[1]}% ${comps[2]}%${alpha})`;
    case 'hwb':
      return `hwb(${comps[0]} ${comps[1]}% ${comps[2]}%${alpha})`;
    case 'srgb':
      return `color(srgb ${comps.join(' ')}${alpha})`;
    default:
      return `color(${c.colorSpace} ${comps.join(' ')}${alpha})`;
  }
}

// ---------------------------------------------------------------- colour maths (oklch <-> srgb)

const clamp01 = (x) => Math.min(1, Math.max(0, x));
const toGamma = (x) => (x <= 0.0031308 ? 12.92 * x : 1.055 * Math.pow(x, 1 / 2.4) - 0.055);
const toLinear = (x) => (x <= 0.04045 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4));

export function oklabToLinearSrgb(L, a, b) {
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.291485548 * b;
  const l = l_ ** 3, m = m_ ** 3, s = s_ ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

export function linearSrgbToOklab(r, g, b) {
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

/** oklch (L 0..1, C, H degrees) -> { srgb: [0..1]x3 gamma-encoded, linear: [0..1]x3 } (clipped) */
export function oklchToSrgb(L, C, H) {
  const h = ((Number(H) || 0) * Math.PI) / 180;
  const lin = oklabToLinearSrgb(L, C * Math.cos(h), C * Math.sin(h)).map(clamp01);
  return { linear: lin, srgb: lin.map(toGamma).map(clamp01) };
}

/** srgb (0..1 gamma-encoded) -> [L, C, H] */
export function srgbToOklch(r, g, b) {
  const [L, a, bb] = linearSrgbToOklab(toLinear(r), toLinear(g), toLinear(b));
  const C = Math.sqrt(a * a + bb * bb);
  let H = (Math.atan2(bb, a) * 180) / Math.PI;
  if (H < 0) H += 360;
  return [L, C, C < 1e-4 ? 0 : H];
}

export function hexToSrgb(hex) {
  let h = hex.replace('#', '').trim();
  if (h.length === 3 || h.length === 4) h = [...h].map((x) => x + x).join('');
  const n = (i) => parseInt(h.slice(i, i + 2), 16) / 255;
  return { srgb: [n(0), n(2), n(4)], alpha: h.length === 8 ? n(6) : 1 };
}

export function srgbToHex([r, g, b]) {
  const x = (v) => Math.round(clamp01(v) * 255).toString(16).padStart(2, '0');
  return '#' + x(r) + x(g) + x(b);
}

/**
 * Parses a css colour string we can handle without a browser (oklch, oklab, hex, rgb(a), DTCG object).
 * Returns { srgb, linear, alpha, hex } or null.
 */
export function parseColor(input) {
  if (input == null) return null;
  if (isPlainObject(input)) {
    const c = input.components ?? [];
    const alpha = input.alpha ?? 1;
    if (input.colorSpace === 'oklch') return finish(oklchToSrgb(+c[0], +c[1], c[2] === 'none' ? 0 : +c[2]).srgb, alpha);
    if (input.colorSpace === 'oklab') {
      const lin = oklabToLinearSrgb(+c[0], +c[1], +c[2]).map(clamp01);
      return finish(lin.map(toGamma), alpha);
    }
    if (input.colorSpace === 'srgb') return finish(c.map(Number), alpha);
    if (input.hex) return finish(hexToSrgb(input.hex).srgb, alpha);
    return null;
  }
  const s = String(input).trim().toLowerCase();
  if (s.startsWith('#')) {
    const { srgb, alpha } = hexToSrgb(s);
    return finish(srgb, alpha);
  }
  const fn = s.match(/^(oklch|oklab|rgba?)\(\s*([^)]*)\)$/);
  if (!fn) return null;
  const [body, a] = fn[2].split('/').map((x) => x.trim());
  const parts = body.split(/[\s,]+/).filter(Boolean);
  const alpha = a != null ? pct(a, 1) : fn[1] === 'rgba' && parts.length === 4 ? pct(parts.pop(), 1) : 1;
  if (fn[1] === 'oklch') {
    return finish(oklchToSrgb(pct(parts[0], 1), pct(parts[1], 0.4), parts[2] === 'none' ? 0 : parseFloat(parts[2])).srgb, alpha);
  }
  if (fn[1] === 'oklab') {
    const lin = oklabToLinearSrgb(pct(parts[0], 1), pct(parts[1], 0.4), pct(parts[2], 0.4)).map(clamp01);
    return finish(lin.map(toGamma), alpha);
  }
  return finish(parts.slice(0, 3).map((p) => pct(p, 255) / 255), alpha);
}

function pct(s, scale) {
  s = String(s);
  return s.endsWith('%') ? (parseFloat(s) / 100) * scale : parseFloat(s);
}

function finish(srgb, alpha) {
  const g = srgb.map(clamp01);
  return { srgb: g, linear: g.map(toLinear), alpha, hex: srgbToHex(g) };
}

// ---------------------------------------------------------------- compile

/**
 * Compiles one merged token document to { vars: { '--bg': 'oklch(...)', ... }, meta: { '--bg': {type, hex?} } }.
 */
export function compileVars(doc) {
  const resolved = resolveTokens(flattenTokens(doc));
  const vars = {};
  const meta = {};
  for (const t of resolved.values()) {
    if (!t.emit) continue;
    const css = toCss(t.type, t.value);
    if (css == null) continue;
    vars[t.cssName] = css;
    const m = { type: t.type, path: t.path };
    if (t.description) m.description = t.description;
    if (t.type === 'color') {
      const p = parseColor(t.value);
      if (p) {
        m.hex = p.hex;
        m.alpha = p.alpha;
        m.linear = p.linear.map((x) => Math.round(x * 1e4) / 1e4);
      }
    }
    meta[t.cssName] = m;
  }
  return { vars, meta };
}

/**
 * Compiles a theme (base doc + common overrides + per-mode overrides) into CSS text.
 *
 * theme = {
 *   id, name, defaultMode: 'dark' | 'light',
 *   base, common, modes: { light: doc, dark: doc },
 *   fontFaces: [{ family, src, weight?, style?, display? }],   // src relative to the css file
 *   marks: { mark, wordmark },                                // urls relative to the css file
 * }
 * Returns { css, json } where json holds the resolved values per mode (for the webgl renderer and settings).
 */
export function compileTheme(theme) {
  const modes = {};
  for (const mode of ['light', 'dark']) {
    const doc = mergeTokens(theme.base, theme.common, theme.modes?.[mode]);
    modes[mode] = compileVars(doc);
  }
  const names = new Set([...Object.keys(modes.light.vars), ...Object.keys(modes.dark.vars)]);
  const shared = {};
  const perMode = { light: {}, dark: {} };
  for (const n of names) {
    const l = modes.light.vars[n];
    const d = modes.dark.vars[n];
    if (l === d) shared[n] = l;
    else {
      if (l != null) perMode.light[n] = l;
      if (d != null) perMode.dark[n] = d;
    }
  }
  if (theme.marks?.mark) shared['--mark'] = `url("${theme.marks.mark}")`;
  if (theme.marks?.wordmark) shared['--wordmark'] = `url("${theme.marks.wordmark}")`;

  const def = theme.defaultMode === 'light' ? 'light' : 'dark';
  const other = def === 'light' ? 'dark' : 'light';
  const block = (sel, vars, extra = '') => {
    const lines = Object.entries(vars).map(([k, v]) => `  ${k}: ${v};`);
    if (extra) lines.unshift(`  ${extra}`);
    return `${sel} {\n${lines.join('\n')}\n}\n`;
  };

  let css = `/* generated from design tokens by ui/theme/scripts/build-tokens.mjs — edit the .tokens.json files, not this. */\n`;
  css += `/* theme: ${theme.id}${theme.name ? ' (' + theme.name + ')' : ''} */\n\n`;
  for (const f of theme.fontFaces ?? []) {
    css += `@font-face {\n  font-family: "${f.family}";\n  src: url("${f.src}") format("${f.format ?? 'woff2'}");\n`;
    css += `  font-weight: ${f.weight ?? 400};\n  font-style: ${f.style ?? 'normal'};\n  font-display: ${f.display ?? 'swap'};\n}\n\n`;
  }
  css += block(':root', shared, `--theme-id: "${theme.id}";`);
  css += '\n' + block(`:root, :root[data-theme="${def}"]`, perMode[def], `color-scheme: ${def};`);
  css += '\n' + block(`:root[data-theme="${other}"]`, perMode[other], `color-scheme: ${other};`);
  css += `\n/* without a stored choice, follow the system */\n@media (prefers-color-scheme: ${other}) {\n`;
  css += block(`:root:not([data-theme])`, perMode[other], `color-scheme: ${other};`)
    .split('\n')
    .map((l) => (l ? '  ' + l : l))
    .join('\n');
  css += `}\n`;
  css += `\n/* reduced motion: the same theme, calmer. --motion is the switch the app reads. */\n`;
  css += `:root[data-motion="reduced"] {\n  --motion: 0;\n  --dur-mid: var(--dur-short);\n  --dur-long: var(--dur-short);\n}\n`;

  const json = {
    id: theme.id,
    name: theme.name,
    defaultMode: def,
    modes: { light: modes.light.meta, dark: modes.dark.meta },
    values: { light: modes.light.vars, dark: modes.dark.vars },
  };
  return { css, json };
}

/**
 * Compiles a flat override map ({ '--accent': 'oklch(...)' }) into css.
 * overrides = { all?: {...}, light?: {...}, dark?: {...} }
 * The selectors outweigh the generated theme css (0,2,1 vs 0,2,0), so overrides always win.
 */
export function overridesToCss(overrides) {
  if (!overrides) return '';
  const safe = (v) => String(v).replace(/[;{}<>]/g, '');
  const name = (k) => (k.startsWith('--') ? k : '--' + k).replace(/[^a-zA-Z0-9_-]/g, '');
  const block = (sel, o) => {
    const entries = Object.entries(o ?? {});
    if (!entries.length) return '';
    return `${sel} {\n${entries.map(([k, v]) => `  ${name(k)}: ${safe(v)};`).join('\n')}\n}\n`;
  };
  return (
    block('html:root:is([data-theme], :not([data-theme]))', overrides.all) +
    block('html:root[data-theme="light"]', overrides.light) +
    block('html:root[data-theme="dark"]', overrides.dark)
  );
}
