// Make the page's static icons from anamnesis's own icon (vendor/aphesis/anamnesis.svg, from the aphesis repository's
// design-system branch: six small linked rings, three lit with stipple, as if being recalled).
//
//   NODE_PATH=$(npm root -g) node site/tools/make_icons.mjs      (needs Playwright with Chromium)
//
// Writes into site/:
//   favicon.svg           the tab's icon: the drawing in rubedo, the dark theme's where the browser is dark
//   favicon.ico           16, 32 and 48: the same, on a transparent ground (browsers without svg icons, bookmarks)
//   apple-touch-icon.png  180 x 180: rubedo (dark theme) on the dark ground, with a margin
//   icon-512.png          512 x 512: the same, for the manifest
// The rasters are drawn by the browser from the svg itself, so they match the tab's icon exactly.
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const { chromium } = createRequire(import.meta.url)('playwright'); // require honours NODE_PATH; import does not
const SITE = join(dirname(fileURLToPath(import.meta.url)), '..');
const RED = { light: '#ab0d19', dark: '#d43031' }; // rubedo (tokens.json)
const GROUND = '#050404'; // n-975, the dark ground
const paths = [...readFileSync(join(SITE, 'vendor/aphesis/anamnesis.svg'), 'utf8').matchAll(/ d="([^"]*)"/g)].map((m) => m[1]);
const svg = (fill, box = '1 1 22 22') => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box}">${fill}${paths.map((d) => `<path d="${d}"/>`).join('')}</svg>`;

writeFileSync(join(SITE, 'favicon.svg'),
  svg(`<style>path{fill:${RED.light}}@media (prefers-color-scheme:dark){path{fill:${RED.dark}}}</style>`) + '\n');

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 1 });
async function png(size, { ground = null, margin = 0 } = {}) {
  await page.setViewportSize({ width: size, height: size });
  const inner = Math.round(size * (1 - 2 * margin));
  await page.setContent(`<html><body style="margin:0;width:${size}px;height:${size}px;display:grid;place-items:center;background:${ground || 'transparent'}">
    <div style="width:${inner}px;height:${inner}px">${svg(`<style>path{fill:${RED.dark}}</style>`).replace('<svg ', '<svg width="100%" height="100%" ')}</div></body></html>`);
  return page.screenshot({ omitBackground: !ground, type: 'png' });
}
writeFileSync(join(SITE, 'apple-touch-icon.png'), await png(180, { ground: GROUND, margin: 0.14 }));
writeFileSync(join(SITE, 'icon-512.png'), await png(512, { ground: GROUND, margin: 0.14 }));

// favicon.ico: PNG images inside an ICO container (16, 32, 48)
const imgs = [];
for (const s of [16, 32, 48]) imgs.push([s, await png(s)]);
const head = Buffer.alloc(6 + 16 * imgs.length);
head.writeUInt16LE(0, 0); head.writeUInt16LE(1, 2); head.writeUInt16LE(imgs.length, 4);
let offset = head.length;
imgs.forEach(([s, data], i) => {
  const o = 6 + 16 * i;
  head.writeUInt8(s % 256, o); head.writeUInt8(s % 256, o + 1); head.writeUInt8(0, o + 2); head.writeUInt8(0, o + 3);
  head.writeUInt16LE(1, o + 4); head.writeUInt16LE(32, o + 6); head.writeUInt32LE(data.length, o + 8); head.writeUInt32LE(offset, o + 12);
  offset += data.length;
});
writeFileSync(join(SITE, 'favicon.ico'), Buffer.concat([head, ...imgs.map(([, d]) => d)]));
await browser.close();
console.log('icons written to', SITE);
