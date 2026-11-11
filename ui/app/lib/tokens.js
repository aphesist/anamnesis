// Reads the theme's css custom properties (docs/contracts.md "theme tokens")
// and turns colours into linear-ish rgb arrays for the shaders and 2d canvases.
// Re-read on `anamnesis:themechange` and when <html data-theme> changes.

const probe = document.createElement('canvas');
probe.width = probe.height = 1;
const pctx = probe.getContext('2d', { willReadFrequently: true });

export function cssVar(name, el = document.documentElement) {
  return getComputedStyle(el).getPropertyValue(name).trim();
}

// Any css colour the browser understands (oklch included) -> [r,g,b,a] in 0..1.
export function parseColor(str, fallback = [1, 1, 1, 1]) {
  if (!str) return fallback;
  pctx.clearRect(0, 0, 1, 1);
  pctx.fillStyle = '#000';
  pctx.fillStyle = str;
  pctx.fillRect(0, 0, 1, 1);
  const d = pctx.getImageData(0, 0, 1, 1).data;
  if (d[3] === 0 && !/transparent/.test(str)) return fallback;
  return [d[0] / 255, d[1] / 255, d[2] / 255, d[3] / 255];
}

const COLOR_TOKENS = ['--bg', '--ink', '--ink-soft', '--line', '--fog', '--accent', '--accent-text', '--seal', '--dot', '--dot-dim', '--dot-hot', '--thread', '--region-label'];

// The theming package's runtime (ui/theme/theme.js), when loaded: its
// tokenRGB() gives gamma-encoded and linear values for the renderer.
let themeApi = null;
export function useThemeApi(t) { themeApi = t; refresh(); }
const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4));

export function readTheme() {
  const c = {}, lin = {};
  for (const t of COLOR_TOKENS) {
    const k = t.slice(2);
    let rgb = null;
    if (themeApi) { try { const v = themeApi.tokenRGB(t); if (v) { rgb = [v.r, v.g, v.b, v.alpha ?? 1]; lin[k] = v.linear; } } catch { rgb = null; } }
    if (!rgb) { rgb = parseColor(cssVar(t)); lin[k] = rgb.slice(0, 3).map(toLinear); }
    c[k] = rgb;
  }
  const m = parseFloat(cssVar('--motion'));
  const attr = document.documentElement.dataset.motion;
  const motion = attr === 'reduced' ? 0 : attr === 'full' ? 1 : Number.isFinite(m) ? m : (matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 1);
  const lum = 0.2126 * c.bg[0] + 0.7152 * c.bg[1] + 0.0722 * c.bg[2];
  const durMid = parseFloat(cssVar('--dur-mid')) || 600;
  return { colors: c, linear: lin, motion, light: lum > 0.5, durMid: /ms/.test(cssVar('--dur-mid')) || durMid > 10 ? durMid : durMid * 1000 };
}

let current = null;
const subs = new Set();
export const theme = () => current || (current = readTheme());
export function onTheme(fn) { subs.add(fn); return () => subs.delete(fn); }
function refresh() {
  current = readTheme();
  document.documentElement.classList.toggle('reduced', current.motion === 0);
  for (const fn of subs) fn(current);
}
window.addEventListener('anamnesis:themechange', () => requestAnimationFrame(refresh));
new MutationObserver(refresh).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'data-motion', 'data-theme-id', 'style'] });
matchMedia('(prefers-reduced-motion: reduce)').addEventListener?.('change', refresh);
export const refreshTheme = refresh;

// Categorical hues for areas, derived from the accent/dot tokens so a theme
// change carries through: same lightness, rotated hue, low chroma (stipple, not paint).
export function areaColor(i, alpha = 1) {
  // themes may set --area-1 … --area-8; past eight, or without them, rotate a calm hue
  const k = (i % 8) + 1;
  const v = i < 8 ? cssVar(`--area-${k}`) : '';
  if (v) { const c = parseColor(v); return `rgba(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0},${alpha})`; }
  const hue = (i * 137.508 + 210) % 360;
  return `oklch(0.78 0.07 ${hue.toFixed(1)} / ${alpha})`;
}
export const rgba = (c, a = 1) => `rgba(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0},${a})`;
