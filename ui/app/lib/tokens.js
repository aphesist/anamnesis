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

export function readTheme() {
  const c = {};
  for (const t of COLOR_TOKENS) c[t.slice(2)] = parseColor(cssVar(t));
  const m = parseFloat(cssVar('--motion'));
  const motion = Number.isFinite(m) ? m : (matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 1);
  const lum = 0.2126 * c.bg[0] + 0.7152 * c.bg[1] + 0.0722 * c.bg[2];
  const durMid = parseFloat(cssVar('--dur-mid')) || 600;
  return { colors: c, motion, light: lum > 0.5, durMid: /ms/.test(cssVar('--dur-mid')) || durMid > 10 ? durMid : durMid * 1000 };
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
new MutationObserver(refresh).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'style', 'class'].slice(0, 2) });
matchMedia('(prefers-reduced-motion: reduce)').addEventListener?.('change', refresh);
export const refreshTheme = refresh;

// Categorical hues for areas, derived from the accent/dot tokens so a theme
// change carries through: same lightness, rotated hue, low chroma (stipple, not paint).
export function areaColor(i, alpha = 1) {
  const hue = (i * 137.508 + 210) % 360;
  return `oklch(0.78 0.07 ${hue.toFixed(1)} / ${alpha})`;
}
export const rgba = (c, a = 1) => `rgba(${(c[0] * 255) | 0},${(c[1] * 255) | 0},${(c[2] * 255) | 0},${a})`;
