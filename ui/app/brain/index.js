// The 3D stipple brain: WebGL2 points + filament threads, orbit camera,
// layout morphing, CPU screen-space picking and projected html labels.
import { OrbitCamera } from './camera.js';
import { POINT_VS, POINT_FS, LINK_VS, LINK_FS, SHELL_VS, SHELL_FS } from './shaders.js';
import { normalise, anatomy, groupsFor, spheres, split, hash, shell } from './layouts.js';
import { store, on, emit, BIT, LIT, groupKey, regionOf, fromNorm } from '../lib/store.js';
import { theme, onTheme } from '../lib/tokens.js';
import { h } from '../lib/dom.js';

const TEXW = 1024;
const SEGMENTS = 18;

function compile(gl, type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src); gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) + '\n' + src.split('\n').map((l, i) => i + 1 + ': ' + l).join('\n'));
  return s;
}
function program(gl, vs, fs) {
  const p = gl.createProgram();
  gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
  const u = {};
  const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
  for (let i = 0; i < n; i++) { const info = gl.getActiveUniform(p, i); const name = info.name.replace(/\[0\]$/, ''); u[name] = gl.getUniformLocation(p, info.name); }
  return { p, u };
}

export class Brain {
  constructor(canvas, overlay) {
    this.canvas = canvas;
    this.overlay = overlay;
    this.cam = new OrbitCamera();
    this.cam.shift = [0.1, 0.06];
    this.morph = 0; this.morphTarget = 0;
    this.lt = 1;
    this.glow = new Float32Array(64); this.glowTarget = new Float32Array(64);
    this.time = 0; this.last = performance.now();
    this.dirty = true;
    this.hover = -1;
    this.labels = [];
    this.gridVersion = -1;
    this.onHover = () => {}; this.onOpen = () => {}; this.onRegion = () => {};
    this.ok = false;
    try { this.initGL(); this.ok = true; } catch (e) { console.warn('webgl2 unavailable:', e); this.fail(e); }
    this.bindInput();
    on('flags', () => this.uploadFlags());
    onTheme(() => { this.readColors(); this.dirty = true; });
    this.readColors();
    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(canvas);
    requestAnimationFrame((t) => this.frame(t));
  }

  fail(e) {
    this.canvas.replaceWith(h('div.brain-fallback', { role: 'note' },
      h('p', 'the brain needs webgl2, which this window did not offer.'),
      h('p.soft', 'the list view, notes, charts and chat all still work.'),
      h('small.soft', String(e && e.message || e).slice(0, 120))));
  }

  initGL() {
    const gl = this.canvas.getContext('webgl2', { antialias: false, premultipliedAlpha: true, alpha: true, preserveDrawingBuffer: false });
    if (!gl) throw new Error('no webgl2 context');
    this.gl = gl;
    this.pts = program(gl, POINT_VS, POINT_FS);
    this.lnk = program(gl, LINK_VS, LINK_FS);
    this.shl = program(gl, SHELL_VS, SHELL_FS);
    this.shellVao = gl.createVertexArray();
    const shellPts = shell(7000);
    this.nShell = shellPts.length / 3;
    gl.bindVertexArray(this.shellVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, shellPts, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    this.emptyVao = gl.createVertexArray();
    this.linkVao = gl.createVertexArray();
    this.tex = {};
    for (const k of ['A', 'P', 'N', 'M']) this.tex[k] = gl.createTexture();
    this.tex.S = gl.createTexture();
    this.canvas.addEventListener('webglcontextlost', (e) => { e.preventDefault(); this.ok = false; });
    this.canvas.addEventListener('webglcontextrestored', () => { try { this.initGL(); this.ok = true; if (store.cloud) this.setData(store.cloud); } catch (e) { this.fail(e); } });
  }

  readColors() {
    // linear colours for the shaders (they encode to srgb on output)
    const c = theme().linear;
    this.colors = c;
    this.light = theme().light;
    this.cam.motion = theme().motion;
    this.dirty = true;
  }

  // ---------- data ----------
  texData(name, data, rows, internal, format, type) {
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.tex[name]);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, TEXW, rows, 0, format, type, data);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  }
  pack4(xyz, w) {
    const rows = this.rows;
    const out = new Float32Array(TEXW * rows * 4);
    for (let i = 0; i < this.n; i++) {
      out[i * 4] = xyz[i * 3]; out[i * 4 + 1] = xyz[i * 3 + 1]; out[i * 4 + 2] = xyz[i * 3 + 2]; out[i * 4 + 3] = w ? w[i] : 0;
    }
    return out;
  }

  setData(cloud) {
    const n = cloud.ids.length;
    this.n = n;
    this.rows = Math.max(1, Math.ceil(n / TEXW));
    const server = cloud.layouts || {};
    const norm = normalise(cloud.xyz, n);
    this.norm = norm;
    this.A = server.anatomy ? Float32Array.from(server.anatomy) : anatomy(norm, n);
    this.seed = Float32Array.from({ length: n }, (_, i) => hash(i * 7919 + 17));
    this.P = Float32Array.from(this.A); this.N = Float32Array.from(this.A);
    this.layoutCentres = [];
    // region anatomy centroids
    const R = cloud.regions.length;
    const cen = new Float32Array(R * 3), cnt = new Uint32Array(R);
    for (let i = 0; i < n; i++) { const r = cloud.region[i]; if (r < 0) continue; cnt[r]++; for (let d = 0; d < 3; d++) cen[r * 3 + d] += this.A[i * 3 + d]; }
    this.regionA = cloud.regions.map((_, r) => { const c = [0, 1, 2].map((d) => cen[r * 3 + d] / Math.max(1, cnt[r])); const l = Math.hypot(...c) || 1; const push = Math.min(1.35, 0.9 / l + 0.35); return c.map((x) => x * Math.max(1.05, push)); });
    if (!this.ok) return;
    const gl = this.gl;
    this.texData('A', this.pack4(this.A, this.seed), this.rows, gl.RGBA32F, gl.RGBA, gl.FLOAT);
    this.texData('P', this.pack4(this.P), this.rows, gl.RGBA32F, gl.RGBA, gl.FLOAT);
    this.texData('N', this.pack4(this.N), this.rows, gl.RGBA32F, gl.RGBA, gl.FLOAT);
    const meta = new Float32Array(TEXW * this.rows * 4);
    for (let i = 0; i < n; i++) { meta[i * 4] = store.tNorm[i]; meta[i * 4 + 1] = cloud.region[i]; meta[i * 4 + 2] = cloud.kind[i]; meta[i * 4 + 3] = cloud.speaker[i]; }
    this.texData('M', meta, this.rows, gl.RGBA32F, gl.RGBA, gl.FLOAT);
    this.flagsTexData = new Uint8Array(TEXW * this.rows);
    this.uploadFlags(true);
    // links: per-vertex t + per-instance (from, to, kind)
    const links = (cloud.links || []).filter((l) => l[0] < n && l[1] < n);
    this.nLinks = links.length;
    gl.bindVertexArray(this.linkVao);
    const tb = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, tb);
    gl.bufferData(gl.ARRAY_BUFFER, Float32Array.from({ length: SEGMENTS + 1 }, (_, k) => k / SEGMENTS), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 1, gl.FLOAT, false, 0, 0);
    const kindIdx = { written: 0, manual: 1, accepted_suggestion: 2 };
    const lb = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, lb);
    // the busier end of each link: threads converging on a hub note are thinned so they do not
    // drown the view (the shader brings a hovered or lit item's own threads back up)
    const deg = new Uint16Array(n);
    for (const l of links) { deg[l[0]]++; deg[l[1]]++; }
    gl.bufferData(gl.ARRAY_BUFFER, Int32Array.from(links.flatMap((l) => [l[0], l[1], kindIdx[l[2]] ?? 0, Math.max(deg[l[0]], deg[l[1]])])), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(1); gl.vertexAttribIPointer(1, 4, gl.INT, 0, 0); gl.vertexAttribDivisor(1, 1);
    gl.bindVertexArray(null);
    this.dirty = true;
    this.buildLabels();
  }

  uploadFlags(full) {
    if (!this.ok || !this.flagsTexData) { this.dirty = true; return; }
    const gl = this.gl;
    this.flagsTexData.set(store.flags);
    let any = 0;
    for (let i = 0; i < store.flags.length; i++) if (store.flags[i] & LIT) { any = 1; break; }
    this.anyLit = any;
    if (full) this.texData('S', this.flagsTexData, this.rows, gl.R8UI, gl.RED_INTEGER, gl.UNSIGNED_BYTE);
    else {
      gl.bindTexture(gl.TEXTURE_2D, this.tex.S);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, TEXW, this.rows, gl.RED_INTEGER, gl.UNSIGNED_BYTE, this.flagsTexData);
    }
    this.gridVersion = -1;
    this.dirty = true;
  }

  // Recompute the "layout" side of the morph for the current view settings.
  relayout(animate = true) {
    if (!store.cloud) return;
    const v = store.view, n = this.n, c = store.cloud;
    const filtered = (i) => (store.flags[i] & BIT.FILTERED) !== 0;
    const groups = groupsFor(n, (i) => groupKey(i, v.groupBy), filtered);
    const deg = this.degree || (this.degree = (() => { const d = new Uint16Array(n); for (const l of c.links || []) { d[l[0]]++; d[l[1]]++; } return d; })());
    const orderKey = v.order === 'links' ? (i) => -deg[i] : v.order === 'kind' ? (i) => c.kind[i] * 1e10 + c.t[i] : v.order === 'recent' ? (i) => -c.t[i] : (i) => c.t[i];
    const res = v.layout === 'split' ? split(n, groups, this.A) : spheres(n, groups, orderKey);
    // points in no group keep their anatomy position
    const inGroup = new Uint8Array(n);
    for (const [, idx] of groups) for (const i of idx) inGroup[i] = 1;
    for (let i = 0; i < n; i++) if (!inGroup[i]) for (let d = 0; d < 3; d++) res.pos[i * 3 + d] = this.A[i * 3 + d];
    // P <- current blend, N <- new, then tween lt 0 -> 1
    const lt = this.lt;
    for (let i = 0; i < n * 3; i++) this.P[i] = this.P[i] + (this.N[i] - this.P[i]) * lt;
    this.N = res.pos;
    this.layoutCentres = res.centres;
    this.lt = animate && theme().motion > 0 && this.morph > 0.01 ? 0 : 1;
    if (this.ok) {
      const gl = this.gl;
      this.texData('P', this.pack4(this.P), this.rows, gl.RGBA32F, gl.RGBA, gl.FLOAT);
      this.texData('N', this.pack4(this.N), this.rows, gl.RGBA32F, gl.RGBA, gl.FLOAT);
    }
    this.buildLabels();
    this.gridVersion = -1;
    this.dirty = true;
    const w = this.canvas.clientWidth, hgt = this.canvas.clientHeight;
    if (w && hgt) this.updateLabels(w, hgt);
  }

  setMorph(m, instant) {
    const wasLayout = this.morphTarget >= 0.5;
    this.morphTarget = m;
    if (instant || theme().motion === 0) this.morph = m;
    const isLayout = m >= 0.5;
    if (isLayout !== wasLayout) {
      // layouts are laid out facing the viewer: come round to the front
      this.cam.sway = isLayout;
      if (isLayout) { const y = Math.round(this.cam.yaw / (2 * Math.PI)) * 2 * Math.PI; this.cam.swayCentre = y; this.cam.swayT = 0; this.cam.flyTo([0, 0, 0], 4.3, { yaw: y, pitch: 0.06, dur: 1100 }); }
      else this.cam.flyTo([0, 0, 0], this.cam.home.dist, { pitch: this.cam.home.pitch, dur: 1100 });
    }
    this.dirty = true;
  }
  setGlow(regionIdx, v) { if (regionIdx >= 0 && regionIdx < 64) this.glowTarget[regionIdx] = v; this.dirty = true; }
  flashRegion(regionIdx) { if (regionIdx >= 0 && regionIdx < 64) { this.glow[regionIdx] = 1; this.glowTarget[regionIdx] = 0; this.dirty = true; } }

  // Current (cpu) position of point i, mirroring the shader without drift.
  pos(i, out = [0, 0, 0]) {
    const m = this.morph, lt = this.lt;
    for (let d = 0; d < 3; d++) {
      const lay = this.P[i * 3 + d] + (this.N[i * 3 + d] - this.P[i * 3 + d]) * lt;
      out[d] = this.A[i * 3 + d] + (lay - this.A[i * 3 + d]) * m;
    }
    return out;
  }

  // Fly to the bounding sphere of some points.
  focusOn(indices, opts = {}) {
    if (!indices.length) return;
    const c = [0, 0, 0], p = [0, 0, 0];
    for (const i of indices) { this.pos(i, p); c[0] += p[0]; c[1] += p[1]; c[2] += p[2]; }
    c[0] /= indices.length; c[1] /= indices.length; c[2] /= indices.length;
    let r = 0;
    for (const i of indices) { this.pos(i, p); r = Math.max(r, Math.hypot(p[0] - c[0], p[1] - c[1], p[2] - c[2])); }
    const dist = indices.length === 1 ? 1.8 : Math.max(indices.length < 12 ? 3.0 : 2.4, Math.min(4.4, r * 2.6 + 0.8));
    this.cam.flyTo(indices.length === 1 ? c : c.map((x) => x * (opts.soft ? 0.3 : 0.85)), opts.dist ?? (opts.soft ? Math.max(dist, 4.3) : dist), opts);
    this.dirty = true;
  }

  // ---------- labels ----------
  buildLabels() {
    if (!store.cloud) return;
    const c = store.cloud, v = store.view;
    for (const l of this.labels) l.el.remove();
    this.labels = [];
    const mk = (text, sub, a, b, kind, extra = {}) => {
      const span = h('span.rl-text', text);
      const el = h('button.region-label', { type: 'button', tabindex: '-1', title: sub || '', style: { opacity: '0' } }, span);
      el.addEventListener('click', (e) => { e.stopPropagation(); if (extra.region != null) this.onRegion(extra.region); else if (extra.group) emit('group-click', extra.group); });
      el.addEventListener('pointerenter', () => { this.labelHover = extra.region ?? null; this.dirty = true; });
      el.addEventListener('pointerleave', () => { this.labelHover = null; this.dirty = true; });
      this.overlay.append(el);
      // "ice · snow · water" -> "ice · snow" when the stage is crowded
      const words = String(text).split(' · ');
      const short = words.length > 2 ? words.slice(0, 2).join(' · ') : text;
      this.labels.push({ el, span, a, b, kind, full: text, short, text, w: text.length * 7.2 + 8, shown: false, ...extra });
    };
    const centreOf = new Map(this.layoutCentres.map((g) => [g.key, g.pos]));
    // region means in the layout, for regions when grouped by something else
    const lay = (r) => {
      const s = [0, 0, 0]; let k = 0;
      for (let i = 0; i < this.n; i++) if (c.region[i] === r) { for (let d = 0; d < 3; d++) s[d] += this.N[i * 3 + d]; k++; }
      return k ? s.map((x) => x / k) : this.regionA[r];
    };
    c.regions.forEach((reg, r) => {
      const b = v.groupBy === 'region' ? centreOf.get(reg.label) || lay(r) : lay(r);
      mk(reg.label, reg.anatomy || '', this.regionA[r], b, v.groupBy === 'region' ? 'both' : 'a', { region: r, born: Date.parse(reg.born_at) / 1000 });
    });
    if (v.groupBy !== 'region') {
      for (const g of this.layoutCentres) mk(g.key, `${g.count} items`, g.pos, g.pos, 'b', { group: g.key, count: g.count });
    } else if (centreOf.has('unsorted')) {
      const g = this.layoutCentres.find((x) => x.key === 'unsorted');
      mk('unsorted', `${g.count} items`, g.pos, g.pos, 'b', { group: 'unsorted', count: g.count });
    }
  }

  // Screen-space decluttering: project every label, rank by region size and nearness to the camera
  // (with a little hysteresis so labels do not flicker as the brain turns), then place greedily:
  // a label is shown only if its box stays inside the stage and clear of panels and of every label
  // already placed. Hovered and selected regions are placed first and always shown. When more than
  // MAX_LABELS regions are visible, labels shrink to their top two keywords.
  updateLabels(w, hgt) {
    const MAX_LABELS = w < 700 ? 8 : 12, PAD = 6, EDGE = 8;
    const vp = this.cam.viewProj, m = this.morph;
    const tmax = fromNorm(store.view.time[1]);
    const obstacles = [];
    const cr = this.canvas.getBoundingClientRect();
    for (const sel of ['#slot-bottom-left', '#slot-bottom', '.stage-views', '.lit-bar:not([hidden])', '.peek:not([hidden])', '.demo-banner:not([hidden])', '.float:not([hidden])', '.sheet:not([hidden])', '#caption']) {
      for (const o of document.querySelectorAll(sel)) {
        if (!o.offsetParent) continue;
        const r = o.getBoundingClientRect(); if (!r.width || !r.height) continue;
        obstacles.push([r.left - cr.left - PAD, r.top - cr.top - PAD, r.right - cr.left + PAD, r.bottom - cr.top + PAD]);
      }
    }
    const hoverPoint = this.hover >= 0 ? store.cloud?.region[this.hover] : -1;
    const lit = store.litLabel;
    const cand = [];
    for (const l of this.labels) {
      let op = l.kind === 'a' ? 1 - m : l.kind === 'b' ? m : 1;
      // a region born after the window closes is not there yet (at "now" every region is)
      if (l.born && l.born > tmax + 1 && store.view.time[1] < 0.999) op = 0;
      const p = [0, 1, 2].map((d) => l.a[d] + (l.b[d] - l.a[d]) * m);
      const x = vp[0] * p[0] + vp[4] * p[1] + vp[8] * p[2] + vp[12];
      const y = vp[1] * p[0] + vp[5] * p[1] + vp[9] * p[2] + vp[13];
      const wc = vp[3] * p[0] + vp[7] * p[1] + vp[11] * p[2] + vp[15];
      if (wc <= 0.05) op = 0;
      l.sx = (x / wc * 0.5 + 0.5) * w; l.sy = (0.5 - y / wc * 0.5) * hgt;
      op *= Math.max(0.6, Math.min(1, 1.7 - wc * 0.2));
      l.op = op;
      const reg = l.region != null ? store.cloud.regions[l.region] : null;
      l.forced = op > 0.05 && l.region != null && (this.glowTarget[l.region] > 0.3 || this.labelHover === l.region || hoverPoint === l.region || (!!lit && reg?.label === lit));
      // bigger and nearer first; labels already showing keep a head start
      const size = reg ? reg.size || 1 : (l.count || 50);
      l.prio = op > 0.15 ? Math.sqrt(size) / Math.max(0.5, wc) * (l.shown ? 1.35 : 1) * (l.kind === 'b' && m > 0.5 ? 1.5 : 1) : -1;
      if (op > 0.15 || l.forced) cand.push(l);
    }
    const crowded = cand.length > MAX_LABELS;
    cand.sort((x, y) => (y.forced - x.forced) || (y.prio - x.prio));
    const placed = [];
    let count = 0;
    for (const l of cand) {
      const text = crowded && !l.forced ? l.short : l.full;
      if (text !== l.text) { l.text = text; l.span.textContent = text; }
      l.mw ??= {}; l.mh ??= 20;
      if (!l.mw[text]) { const r = l.el.getBoundingClientRect(); if (r.width) { l.mw[text] = r.width; l.mh = r.height; } }
      const bw = l.mw[text] || text.length * 8 + 12, bh = l.mh;
      const box = [l.sx - bw / 2 - PAD, l.sy - bh / 2 - PAD / 2, l.sx + bw / 2 + PAD, l.sy + bh / 2 + PAD / 2];
      const inside = box[0] > EDGE && box[1] > EDGE && box[2] < w - EDGE && box[3] < hgt - EDGE;
      const hit = (b) => !(box[2] < b[0] || box[0] > b[2] || box[3] < b[1] || box[1] > b[3]);
      // hovered and selected regions always show (they come first, so the others make way)
      const ok = l.forced || (inside && count < MAX_LABELS && !placed.some(hit) && !obstacles.some(hit));
      l.show = ok;
      if (ok) { placed.push(box); if (!l.forced) count++; }
    }
    for (const l of this.labels) {
      const show = cand.includes(l) && l.show;
      l.shown = show;
      l.el.style.transform = `translate(${(l.sx ?? 0).toFixed(1)}px, ${(l.sy ?? 0).toFixed(1)}px) translate(-50%, -50%)`;
      l.el.style.opacity = show ? (l.forced ? 1 : l.op).toFixed(3) : '0';
      l.el.classList.toggle('lit', !!l.forced);
      l.el.style.pointerEvents = show && l.op > 0.3 ? 'auto' : 'none';
    }
  }

  // ---------- picking (cpu screen-space grid) ----------
  buildGrid(w, hgt) {
    const CELL = 14;
    const gw = Math.ceil(w / CELL) + 1, gh = Math.ceil(hgt / CELL) + 1;
    const n = this.n, vp = this.cam.viewProj, f = store.flags;
    const sx = this.sx || (this.sx = new Float32Array(n)), sy = this.sy || (this.sy = new Float32Array(n)), sz = this.sz || (this.sz = new Float32Array(n));
    if (sx.length !== n) { this.sx = new Float32Array(n); this.sy = new Float32Array(n); this.sz = new Float32Array(n); return this.buildGrid(w, hgt); }
    const cellOf = new Int32Array(n).fill(-1);
    const counts = new Uint32Array(gw * gh + 1);
    const tmin = store.view.time[0], tmax = store.view.time[1];
    const p = [0, 0, 0];
    for (let i = 0; i < n; i++) {
      if (f[i] & BIT.FILTERED) continue;
      const t = store.tNorm[i]; if (t < tmin - 1e-5 || t > tmax + 1e-5) continue;
      if (this.anyLit && !(f[i] & LIT) && this.pickLitOnly) continue;
      this.pos(i, p);
      const wc = vp[3] * p[0] + vp[7] * p[1] + vp[11] * p[2] + vp[15];
      if (wc <= 0.05) continue;
      const x = ((vp[0] * p[0] + vp[4] * p[1] + vp[8] * p[2] + vp[12]) / wc * 0.5 + 0.5) * w;
      const y = (0.5 - (vp[1] * p[0] + vp[5] * p[1] + vp[9] * p[2] + vp[13]) / wc * 0.5) * hgt;
      if (x < 0 || y < 0 || x >= w || y >= hgt) continue;
      sx[i] = x; sy[i] = y; sz[i] = wc;
      const c = ((y / CELL) | 0) * gw + ((x / CELL) | 0);
      cellOf[i] = c; counts[c + 1]++;
    }
    for (let c = 1; c < counts.length; c++) counts[c] += counts[c - 1];
    const items = new Int32Array(counts[counts.length - 1]);
    const fill = counts.slice(0, -1);
    for (let i = 0; i < n; i++) if (cellOf[i] >= 0) items[fill[cellOf[i]]++] = i;
    this.grid = { CELL, gw, gh, start: counts, items };
  }
  pick(x, y) {
    if (!this.n) return -1;
    const w = this.canvas.clientWidth, hgt = this.canvas.clientHeight;
    const ver = this.cam.version * 1e4 + Math.round(this.morph * 1000) + Math.round(this.lt * 7);
    if (this.gridVersion !== ver) { this.buildGrid(w, hgt); this.gridVersion = ver; }
    const g = this.grid; if (!g) return -1;
    const cx = (x / g.CELL) | 0, cy = (y / g.CELL) | 0;
    let best = -1, bd = 11 * 11;
    for (let yy = cy - 1; yy <= cy + 1; yy++) for (let xx = cx - 1; xx <= cx + 1; xx++) {
      if (xx < 0 || yy < 0 || xx >= g.gw || yy >= g.gh) continue;
      const c = yy * g.gw + xx;
      for (let k = g.start[c]; k < g.start[c + 1]; k++) {
        const i = g.items[k];
        const d = (this.sx[i] - x) ** 2 + (this.sy[i] - y) ** 2 + this.sz[i] * 0.5; // prefer nearer dots
        const lit = store.flags[i] & LIT ? -20 : 0;
        if (d + lit < bd) { bd = d + lit; best = i; }
      }
    }
    return best;
  }
  screenOf(i) {
    const p = this.pos(i), vp = this.cam.viewProj, w = this.canvas.clientWidth, hgt = this.canvas.clientHeight;
    const wc = vp[3] * p[0] + vp[7] * p[1] + vp[11] * p[2] + vp[15];
    return [((vp[0] * p[0] + vp[4] * p[1] + vp[8] * p[2] + vp[12]) / wc * 0.5 + 0.5) * w, (0.5 - (vp[1] * p[0] + vp[5] * p[1] + vp[9] * p[2] + vp[13]) / wc * 0.5) * hgt];
  }

  // ---------- input ----------
  bindInput() {
    const el = this.canvas.isConnected ? this.canvas : this.overlay;
    const target = this.overlay.parentElement || el;
    let down = null, moved = 0, lastT = 0;
    const local = (e) => { const r = this.canvas.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
    target.addEventListener('pointerdown', (e) => {
      if (e.target.closest('button, input, a, select, textarea, .no-orbit')) return;
      down = [e.clientX, e.clientY]; moved = 0; lastT = performance.now();
      this.cam.dragging = true; this.cam.flight = null;
      target.setPointerCapture(e.pointerId);
    });
    target.addEventListener('pointermove', (e) => {
      if (down) {
        const now = performance.now();
        const dx = e.clientX - down[0], dy = e.clientY - down[1];
        moved += Math.abs(dx) + Math.abs(dy);
        this.cam.drag(dx, dy, (now - lastT) / 1000);
        down = [e.clientX, e.clientY]; lastT = now;
        this.dirty = true;
        if (moved > 4) this.setHover(-1);
        return;
      }
      if (e.target.closest('.region-label, .no-orbit')) { this.setHover(-1); return; }
      this.pendingHover = local(e);
    });
    const end = (e) => {
      if (!down) return;
      this.cam.dragging = false;
      if (performance.now() - lastT > 80) this.cam.stop();
      down = null;
      if (moved < 5 && e.type === 'pointerup') {
        const [x, y] = local(e);
        const i = this.pick(x, y);
        if (i >= 0) this.onOpen(i);
      }
    };
    target.addEventListener('pointerup', end);
    target.addEventListener('pointercancel', end);
    target.addEventListener('pointerleave', () => { if (!down) this.setHover(-1); });
    target.addEventListener('wheel', (e) => { if (e.target.closest('.no-orbit')) return; e.preventDefault(); this.cam.zoom(e.deltaY * (e.deltaMode ? 30 : 1)); this.dirty = true; }, { passive: false });
    this.canvas.addEventListener('keydown', (e) => {
      // leave ctrl/cmd/alt chords (zoom, reload, quit…) to the window that hosts the ui
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const k = e.key, c = this.cam;
      const step = e.shiftKey ? 0.4 : 0.15;
      if (k === 'ArrowLeft') c.yaw -= step; else if (k === 'ArrowRight') c.yaw += step;
      else if (k === 'ArrowUp') c.pitch = Math.min(1.35, c.pitch + step); else if (k === 'ArrowDown') c.pitch = Math.max(-1.35, c.pitch - step);
      else if (k === '+' || k === '=') c.dist = Math.max(0.6, c.dist * 0.88); else if (k === '-' || k === '_') c.dist = Math.min(12, c.dist * 1.12);
      else if (k === 'Home' || k === '0') c.reset();
      else return;
      e.preventDefault(); this.dirty = true;
    });
  }
  setHover(i) {
    if (i === this.hover) return;
    const f = store.flags;
    if (this.hover >= 0 && this.hover < f.length) f[this.hover] &= ~BIT.HOVER;
    this.hover = i;
    if (i >= 0) f[i] |= BIT.HOVER;
    this.uploadFlags();
    const xy = i >= 0 ? this.screenOf(i) : null;
    this.onHover(i, xy);
  }

  resize() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = this.canvas.clientWidth, hgt = this.canvas.clientHeight;
    if (!w || !hgt) return;
    this.canvas.width = Math.round(w * dpr); this.canvas.height = Math.round(hgt * dpr);
    this.dpr = dpr;
    this.dirty = true;
  }

  // ---------- frame ----------
  frame(now) {
    requestAnimationFrame((t) => this.frame(t));
    const dt = Math.min(0.05, (now - this.last) / 1000);
    this.last = now;
    if (this.paused) return;
    const motion = theme().motion;
    const w = this.canvas.clientWidth, hgt = this.canvas.clientHeight;
    if (!w || !hgt || !this.n) return;
    let animating = this.cam.update(dt, w / hgt);
    if (Math.abs(this.morph - this.morphTarget) > 1e-3) { this.morph += (this.morphTarget - this.morph) * Math.min(1, dt * 6); animating = true; } else this.morph = this.morphTarget;
    if (this.lt < 1) { this.lt = Math.min(1, this.lt + dt / 1.1); animating = true; }
    for (let r = 0; r < 64; r++) {
      const d = this.glowTarget[r] - this.glow[r];
      if (Math.abs(d) > 1e-3) { this.glow[r] += d * Math.min(1, dt * (d > 0 ? 8 : 1.6)); animating = true; } else this.glow[r] = this.glowTarget[r];
    }
    if (motion > 0) { this.time += dt; animating = true; }
    if (this.pendingHover) { const [x, y] = this.pendingHover; this.pendingHover = null; this.setHover(this.pick(x, y)); }
    if (!animating && !this.dirty) return;
    this.dirty = false;
    this.updateLabels(w, hgt);
    if (this.ok) this.draw(w, hgt);
    if (this.hover >= 0) this.onHover(this.hover, this.screenOf(this.hover));
    emit('brain-frame', this);
  }

  setUniforms(prog, hgt) {
    const gl = this.gl, u = prog.u, c = this.colors, v = store.view;
    gl.uniformMatrix4fv(u.uView, false, this.cam.view);
    gl.uniformMatrix4fv(u.uProj, false, this.cam.proj);
    gl.uniform1f(u.uMorph, this.morph); gl.uniform1f(u.uLT, this.lt);
    gl.uniform1f(u.uTime, this.time); gl.uniform1f(u.uMotion, theme().motion);
    gl.uniform1f(u.uTmin, v.time[0]); gl.uniform1f(u.uTmax, v.time[1]);
    gl.uniform1f(u.uAnyLit, this.anyLit ? 1 : 0);
    gl.uniform1f(u.uNear, this.cam.effDist - 1.2); gl.uniform1f(u.uFar, this.cam.effDist + 1.6);
    if (u.uGlow) gl.uniform1fv(u.uGlow, this.glow);
    const tex = [['uA', 'A'], ['uP', 'P'], ['uN', 'N'], ['uM', 'M'], ['uS', 'S']];
    tex.forEach(([name, k], unit) => { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, this.tex[k]); if (u[name]) gl.uniform1i(u[name], unit); });
    const col = (name, key) => { if (u[name]) gl.uniform3f(u[name], c[key][0], c[key][1], c[key][2]); };
    col('cDot', 'dot'); col('cDim', 'dot-dim'); col('cHot', 'dot-hot'); col('cCite', 'accent'); col('cFog', 'fog'); col('cThread', 'thread');
    if (u.uViewH) gl.uniform1f(u.uViewH, hgt * this.dpr);
  }

  draw(w, hgt) {
    const gl = this.gl;
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    if (this.light) gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA); else gl.blendFunc(gl.ONE, gl.ONE);
    const outline = store.view.outline !== false ? (1 - this.morph) : 0;
    if (outline > 0.01) {
      const u = this.shl.u, c = this.colors;
      gl.useProgram(this.shl.p);
      gl.uniformMatrix4fv(u.uView, false, this.cam.view);
      gl.uniformMatrix4fv(u.uProj, false, this.cam.proj);
      gl.uniform1f(u.uViewH, hgt * this.dpr);
      gl.uniform1f(u.uAlpha, outline * (this.light ? 0.45 : 0.6));
      gl.uniform1f(u.uTime, this.time); gl.uniform1f(u.uMotion, theme().motion);
      gl.uniform1f(u.uNear, this.cam.effDist - 1.2); gl.uniform1f(u.uFar, this.cam.effDist + 1.6);
      gl.uniform3f(u.cDim, (c.dot[0] + c['dot-dim'][0]) * 0.5, (c.dot[1] + c['dot-dim'][1]) * 0.5, (c.dot[2] + c['dot-dim'][2]) * 0.5);
      gl.bindVertexArray(this.shellVao);
      gl.drawArrays(gl.POINTS, 0, this.nShell);
    }
    if (store.view.filaments && this.nLinks) {
      gl.useProgram(this.lnk.p);
      this.setUniforms(this.lnk, hgt);
      gl.uniform1f(this.lnk.u.uLinkAlpha, this.light ? 0.05 : 0.04);
      gl.bindVertexArray(this.linkVao);
      gl.drawArraysInstanced(gl.LINE_STRIP, 0, SEGMENTS + 1, this.nLinks);
    }
    gl.useProgram(this.pts.p);
    this.setUniforms(this.pts, hgt);
    gl.uniform1f(this.pts.u.uSize, 0.0088 * Math.max(0.6, Math.min(1.6, Math.sqrt(9000 / Math.max(800, this.n)))));
    if (this.pts.u.uLight) gl.uniform1f(this.pts.u.uLight, this.light ? 1 : 0);
    gl.bindVertexArray(this.emptyVao);
    gl.drawArrays(gl.POINTS, 0, this.n);
    gl.bindVertexArray(null);
  }
}

export { regionOf };
