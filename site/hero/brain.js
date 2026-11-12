// The landing-page hero: the demo brain as a slowly turning cloud of stipple.
//
// A small, dependency-free WebGL2 module. Contract with the page:
//   const { destroy } = mount(el, { reducedMotion })
// `el` holds an <img> fallback and `data-src` (the packed brain, see
// build/extract.py). The canvas replaces the img only once its first frame has
// rendered; if anything fails, the img stays and nothing else changes.
//
// The look follows the app's default theme (ui/app/brain): micro-stipple
// clusters for the real notes, finer dust around them, a faint ornamental
// shell on the brain's surface, depth fog, colours from the aphes.ist tokens.

const TOKENS = {
  // brain.dot, brain.dot-dim, brain.dot-hot, region-label, accent (ui/theme/dist/aphesis.json)
  dark: { dot: '#9ecaef', dim: '#385e83', hot: '#e1faff', label: '#8ea9bb', accent: '#d43031', bg: '#050404' },
  light: { dot: '#222f3c', dim: '#95a0ab', hot: '#0b4c92', label: '#495766', accent: '#ab0d19', bg: '#f1eded' },
};
const FONT = 'var(--font-display, "Aphesis Script", "Special Elite", "Courier New", monospace)';
const FOV = 0.72, FPS = 60, TURN = 0.075; // radians per second: one turn in about 84 s
const DUST_PER_POINT = 4, SHELL = 9000, MAX_LABELS = 3;

// Ellipsoids of the brain volume in the anatomy frame (crates/anamnesis-mind/src/layout.rs).
const BRAIN = [
  [-0.17, 0.05, 0, 0.4, 0.46, 0.7], [0.17, 0.05, 0, 0.4, 0.46, 0.7],
  [-0.36, -0.2, 0.12, 0.2, 0.18, 0.4], [0.36, -0.2, 0.12, 0.2, 0.18, 0.4],
  [0, -0.34, -0.5, 0.36, 0.16, 0.22],
];
function brainRadius(x, y, z) {
  let best = 0;
  for (const [cx, cy, cz, sx, sy, sz] of BRAIN) {
    const a = (x / sx) ** 2 + (y / sy) ** 2 + (z / sz) ** 2;
    const b = -2 * (x * cx / (sx * sx) + y * cy / (sy * sy) + z * cz / (sz * sz));
    const c = (cx / sx) ** 2 + (cy / sy) ** 2 + (cz / sz) ** 2 - 1;
    const d = b * b - 4 * a * c;
    if (d >= 0) best = Math.max(best, (-b + Math.sqrt(d)) / (2 * a));
  }
  return Math.max(best, 0.2);
}

const hash = (i) => { let x = Math.imul(i ^ 0x9e3779b9, 0x85ebca6b); x ^= x >>> 13; x = Math.imul(x, 0xc2b2ae35); x ^= x >>> 16; return (x >>> 0) / 4294967296; };
const gauss = (i) => Math.sqrt(-2 * Math.log(hash(i) + 1e-9)) * Math.cos(6.2831853 * hash(i + 7919));
const lin = (hex) => [1, 3, 5].map((k) => { const c = parseInt(hex.slice(k, k + 2), 16) / 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; });

const VS = `#version 300 es
precision highp float;
layout(location = 0) in vec4 aP;   // xyz, seed
layout(location = 1) in vec2 aM;   // kind (0 note, 1 dust, 2 shell), region
uniform mat3 uRot;
uniform vec2 uShift;
uniform float uDist, uF, uAspect, uViewH, uSize, uTime, uMotion, uNear, uFar, uIntro;
uniform vec4 uGR, uGA;
uniform vec3 cDot, cDim, cHot;
out vec3 vCol;
out float vA, vSeed, vSize;
void main() {
  float seed = aP.w, kind = aM.x;
  vec3 p = aP.xyz + uMotion * 0.004 * vec3(sin(uTime * 0.53 + seed * 41.0), cos(uTime * 0.47 + seed * 29.0), sin(uTime * 0.41 + seed * 17.0));
  vec3 v = uRot * p;
  float depth = uDist - v.z;
  gl_Position = vec4(v.x * uF / uAspect + uShift.x * depth, v.y * uF + uShift.y * depth, 0.0, depth);
  float fogf = smoothstep(uNear, uFar, depth);
  float glow = kind < 1.5 ? dot(vec4(equal(vec4(aM.y), uGR)), uGA) : 0.0;
  vec3 col = mix(cDot, cDim, fogf * 0.85);
  float alpha = mix(1.0, 0.26, fogf);
  float size = uSize * (0.65 + 0.7 * fract(seed * 7.13));
  if (kind > 1.5) { col = mix(cDot, cDim, 0.5 + 0.4 * fogf); alpha *= 0.8 * (0.55 + 0.45 * fract(seed * 3.7)); size = 0.0052; }
  else if (kind > 0.5) { col = mix(col, cDim, 0.25); alpha *= 0.8; size *= 0.45; }
  col = mix(col, cHot, glow * 0.85);
  alpha = mix(alpha, 1.0, glow * 0.6);
  size *= 1.0 + 0.4 * glow;
  alpha *= 1.0 - uMotion * 0.28 * (0.5 + 0.5 * sin(uTime * (1.3 + fract(seed * 13.7) * 2.1) + seed * 91.0));
  // surfacing: every dot rises out of the ground at its own moment
  alpha *= smoothstep(seed * 0.6, seed * 0.6 + 0.4, uIntro);
  float px = size * uF * uViewH * 0.5 / max(depth, 0.05);
  vSize = clamp(px, 1.0, 40.0);
  vA = alpha * min(1.0, px / 1.2);
  gl_PointSize = vA <= 0.004 ? 0.0 : vSize;
  vCol = col;
  vSeed = seed;
}`;

const FS = `#version 300 es
precision highp float;
in vec3 vCol;
in float vA, vSeed, vSize;
out vec4 frag;
vec3 enc(vec3 c) { return mix(c * 12.92, 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055, step(vec3(0.0031308), c)); }
float h(float n) { return fract(sin(n) * 43758.5453123); }
float h2(vec2 p) { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }
void main() {
  vec2 pc = gl_PointCoord * 2.0 - 1.0;
  float r = length(pc);
  if (r > 1.0) discard;
  float a;
  if (vSize < 7.0) {
    a = smoothstep(1.0, 0.15, r);
  } else {
    // a micro-stipple cluster: 3-6 sub-dots placed by the point's seed
    a = 0.0;
    int k = 3 + int(h(vSeed * 11.0) * 4.0);
    for (int j = 0; j < 6; j++) {
      if (j >= k) break;
      float fj = float(j);
      vec2 o = (vec2(h(vSeed * 3.1 + fj * 1.7), h(vSeed * 5.7 + fj * 2.3)) * 2.0 - 1.0) * 0.55;
      float rr = 0.16 + 0.12 * h(vSeed + fj * 4.1);
      a += smoothstep(rr, rr * 0.35, length(pc - o));
    }
    a = min(1.0, a) * smoothstep(1.0, 0.75, r);
    a *= step(h2(gl_FragCoord.xy + vSeed * 97.0) * 0.6, a); // ink, not paint
  }
  a *= vA;
  if (a < 0.004) discard;
  frag = vec4(enc(vCol) * a, a);
}`;

// Decode the packed brain (see build/extract.py for the layout).
function decode(buf) {
  const dv = new DataView(buf);
  if (dv.getUint32(0, true) !== 0x4e524241 || dv.getUint8(4) !== 1) throw new Error('brain-data: bad header');
  const n = dv.getUint16(6, true), s = dv.getFloat32(8, true) / 32767;
  const pos = new Float32Array(n * 3);
  for (let i = 0; i < n * 3; i++) pos[i] = dv.getInt16(12 + i * 2, true) * s;
  const reg = new Uint8Array(buf, 12 + n * 6, n);
  const o = 12 + n * 7, len = dv.getUint16(o, true);
  const labels = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, o + 2, len)));
  return { n, pos, reg, labels };
}

// Interleaved vertex data: notes, dust around them, the ornamental shell.
function buildPoints({ n, pos, reg }) {
  const total = n * (1 + DUST_PER_POINT) + SHELL;
  const P = new Float32Array(total * 4), M = new Float32Array(total * 2);
  let k = 0;
  // a sagittal groove between the hemispheres, deeper toward the crown
  const put = (x, y, z, seed, kind, r) => {
    const o = k * 4;
    P[o] = x + Math.sign(x || seed - 0.5) * 0.022 * Math.max(0, Math.min(1, (y + 0.05) * 3)); P[o + 1] = y; P[o + 2] = z; P[o + 3] = seed;
    M[k * 2] = kind; M[k * 2 + 1] = r; k++;
  };
  for (let i = 0; i < n; i++) put(pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2], hash(i * 5 + 1), 0, reg[i]);
  for (let i = 0; i < n; i++) for (let j = 0; j < DUST_PER_POINT; j++) {
    const q = 100003 + (i * DUST_PER_POINT + j) * 11, sd = 0.016 + 0.018 * hash(q + 5);
    put(pos[i * 3] + gauss(q) * sd, pos[i * 3 + 1] + gauss(q + 1) * sd, pos[i * 3 + 2] + gauss(q + 2) * sd, hash(q + 3), 1, reg[i]);
  }
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < SHELL; i++) {
    const y = 1 - 2 * ((i + 0.5) / SHELL), rr = Math.sqrt(1 - y * y), a = i * golden;
    const dx = Math.cos(a) * rr, dz = Math.sin(a) * rr;
    // gyri: a gentle wobble on the surface
    const R = brainRadius(dx, y, dz) * (0.97 + 0.04 * hash(i * 31 + 5)) * (1 + 0.035 * Math.sin(dx * 17 + y * 11) * Math.sin(dz * 15 - y * 9));
    put(dx * R, y * R, dz * R, hash(i * 13 + 2), 2, 255);
  }
  return { P, M, count: k };
}

export function mount(el, opts = {}) {
  const still = !!opts.reducedMotion;
  const showLabels = opts.labels !== false;
  const img = el.querySelector('img');
  const win = el.ownerDocument.defaultView || window;
  const canvas = document.createElement('canvas');
  canvas.setAttribute('aria-hidden', 'true');
  Object.assign(canvas.style, { position: 'absolute', inset: '0', width: '100%', height: '100%', display: 'block', opacity: '0', transition: 'opacity 900ms cubic-bezier(0.22, 0.61, 0.36, 1)', pointerEvents: 'none' });
  const overlay = document.createElement('div');
  overlay.setAttribute('aria-hidden', 'true');
  Object.assign(overlay.style, { position: 'absolute', inset: '0', overflow: 'hidden', pointerEvents: 'none' });
  if (getComputedStyle(el).position === 'static') el.style.position = 'relative';

  let gl = null, prog = null, vao = null, bufs = [], U = {};
  let data = null, pts = null, labels = [];
  let alive = true, ready = false, lost = false, raf = 0, visible = true, last = 0, time = 0, intro = still ? 1 : 0;
  let yaw = opts.yaw ?? 1.45, pitch = 0.16, py = 0, pp = 0, tpy = 0, tpp = 0, nextLabel = 1.2, cursor = 0;
  let W = 0, H = 0, dpr = 1, colors = null, light = false, frameMs = 0;
  const cleanups = [];
  const listen = (t, ev, fn, o) => { t.addEventListener(ev, fn, o); cleanups.push(() => t.removeEventListener(ev, fn, o)); };

  const isLight = () => {
    const t = document.documentElement.dataset.theme;
    return t ? t === 'light' : win.matchMedia('(prefers-color-scheme: light)').matches;
  };
  const applyTheme = () => {
    light = isLight();
    const t = TOKENS[light ? 'light' : 'dark'];
    colors = { dot: lin(t.dot), dim: lin(t.dim), hot: lin(t.hot) };
    for (const l of labels) { l.el.style.color = `var(--region-label, ${t.label})`; l.el.style.textShadow = `0 0 3px var(--bg, ${t.bg}), 0 0 8px var(--bg, ${t.bg}), 0 0 16px var(--bg, ${t.bg})`; l.dot.style.color = `var(--accent, ${t.accent})`; }
    kick();
  };

  function compile(type, src) { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return s; }
  // Shaders compile in parallel where the driver allows it, so start-up never stalls the page.
  async function initGL() {
    gl = canvas.getContext('webgl2', { alpha: true, antialias: false, depth: false, stencil: false, premultipliedAlpha: true, powerPreference: 'low-power', preserveDrawingBuffer: !!opts.preserveDrawingBuffer });
    if (!gl) throw new Error('no webgl2');
    await nextFrame(); // context creation and shader compilation land in separate tasks
    if (!alive || gl.isContextLost()) throw new Error('brain: context gone');
    const par = gl.getExtension('KHR_parallel_shader_compile');
    const vs = compile(gl.VERTEX_SHADER, VS), fs = compile(gl.FRAGMENT_SHADER, FS);
    const p = gl.createProgram();
    gl.attachShader(p, vs); gl.attachShader(p, fs); gl.linkProgram(p);
    if (par) while (!gl.getProgramParameter(p, par.COMPLETION_STATUS_KHR)) { await nextFrame(); if (!alive) return; }
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error('brain shader: ' + (gl.getShaderInfoLog(vs) || gl.getShaderInfoLog(fs) || gl.getProgramInfoLog(p)));
    gl.deleteShader(vs); gl.deleteShader(fs);
    prog = p;
    U = {};
    for (let i = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS) - 1; i >= 0; i--) { const name = gl.getActiveUniform(p, i).name; U[name] = gl.getUniformLocation(p, name); }
    vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    bufs = [[pts.P, 0, 4], [pts.M, 1, 2]].map(([arr, loc, size]) => {
      const b = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, arr, gl.STATIC_DRAW);
      gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, size, gl.FLOAT, false, 0, 0);
      return b;
    });
    gl.bindVertexArray(null);
  }

  function buildLabels() {
    for (const [k, L] of data.labels.entries()) {
      const node = document.createElement('span');
      const dot = document.createElement('span');
      dot.textContent = '·';
      Object.assign(dot.style, { marginRight: '5px', opacity: '0.6' });
      const words = L.t.split(' · ');
      node.append(dot, (words.length > 2 ? words.slice(0, 2) : words).join(' · '));
      Object.assign(node.style, { position: 'absolute', left: '0', top: '0', whiteSpace: 'nowrap', fontFamily: FONT, fontSize: '15px', lineHeight: '1', textTransform: 'lowercase', opacity: '0', transition: still ? 'none' : 'opacity 1400ms cubic-bezier(0.22, 0.61, 0.36, 1)', willChange: 'transform, opacity' });
      overlay.append(node);
      // anchor on the brain's surface in the direction of the region's centroid
      const [cx, cy, cz] = L.c, l = Math.hypot(cx, cy, cz) || 1, R = brainRadius(cx / l, cy / l, cz / l) * 1.04;
      labels.push({ el: node, dot, region: k, a: [cx / l * R, cy / l * R, cz / l * R], on: false, until: 0, glow: 0, x: 0, y: 0, z: 0, w: node.textContent.length * 7.5 });
    }
  }

  // camera
  // Framing follows the fallback img as the page's css lays it out (object-fit, object-position),
  // so the swap does not jump: on landscape stages the brain lands exactly where the img drew it.
  // On portrait stages, where a cover-cropped img would cut the brain, it fits the stage instead
  // and only keeps the img's horizontal position. The lens is shifted, the camera not turned.
  let rot = new Float32Array(9), dist = 2.4, F = 1 / Math.tan(FOV / 2), sx = 0, sy = 0;
  let frame0 = { cx: 0.5, cy: 0.5, fw: 1, fh: 1 };
  function measureFrame() {
    let cx = 0.5, cy = 0.5, fw = W, fh = H;
    if (img && opts.centreX == null) {
      const cs = getComputedStyle(img), m = /^(-?[\d.]+)%(?:\s+(-?[\d.]+)%)?/.exec(cs.objectPosition || '');
      const px = m ? +m[1] / 100 : 0.5, py = m && m[2] != null ? +m[2] / 100 : 0.5;
      const iw = img.naturalWidth, ih = img.naturalHeight;
      if (W >= H && iw && ih && (cs.objectFit === 'cover' || cs.objectFit === 'contain')) {
        const k = (cs.objectFit === 'cover' ? Math.max : Math.min)(W / iw, H / ih);
        fw = iw * k; fh = ih * k;
        cx = ((W - fw) * px + fw / 2) / W; cy = ((H - fh) * py + fh / 2) / H;
      } else {
        cx = Math.max(0.3, Math.min(0.7, px));
        fw = W * Math.max(0.75, Math.min(cx, 1 - cx) * 2);
      }
    } else if (opts.centreX != null) cx = opts.centreX;
    frame0 = { cx, cy, fw, fh };
  }
  function camera() {
    const cy = Math.cos(yaw + py), sy_ = Math.sin(yaw + py), cp = Math.cos(pitch + pp), sp = Math.sin(pitch + pp);
    // Rx(pitch) * Ry(yaw), column-major
    rot.set([cy, sp * sy_, -cp * sy_, 0, cp, sp, sy_, -sp * cy, cp * cy]);
    const t = Math.tan(FOV / 2), f = frame0;
    sx = (f.cx - 0.5) * 2; sy = (0.5 - f.cy) * 2;
    // fit the brain in the frame, then scale that frame onto the canvas
    dist = Math.max(0.86 / t, 0.86 / (t * f.fw / f.fh)) * (H / f.fh) / (opts.zoom || 1);
  }
  function project(p, out) {
    const r = rot, x = r[0] * p[0] + r[3] * p[1] + r[6] * p[2], y = r[1] * p[0] + r[4] * p[1] + r[7] * p[2], z = r[2] * p[0] + r[5] * p[1] + r[8] * p[2];
    const d = dist - z;
    out.x = ((x * F / (W / H) / d + sx) * 0.5 + 0.5) * W; out.y = (0.5 - (y * F / d + sy) * 0.5) * H; out.z = z;
  }

  function updateLabels(now) {
    if (!showLabels) return;
    for (const l of labels) project(l.a, l);
    const facing = (l) => l.z > 0.04 && l.x > l.w * 0.6 && l.x < W - l.w * 0.6 && l.y > 20 && l.y < H - 20;
    const clear = (l) => labels.every((o) => !o.on || o === l || Math.abs(o.x - l.x) > (o.w + l.w) * 0.55 || Math.abs(o.y - l.y) > 26);
    const max = W < 600 ? 2 : MAX_LABELS;
    if (still) {
      let shown = 0;
      for (const l of labels) { l.on = shown < max && facing(l) && clear(l); if (l.on) shown++; }
    } else {
      for (const l of labels) if (l.on && (now > l.until || l.z < -0.02)) l.on = false;
      if (time > nextLabel) {
        nextLabel = time + 2.6;
        if (labels.filter((l) => l.on).length < max) for (let k = 0; k < labels.length; k++) {
          const l = labels[(cursor + k) % labels.length];
          if (!l.on && facing(l) && clear(l)) { l.on = true; l.until = now + 5.5; cursor = (cursor + k + 1) % labels.length; break; }
        }
      }
    }
    for (const l of labels) {
      l.el.style.opacity = l.on ? '1' : '0';
      l.el.style.transform = `translate3d(${(l.x).toFixed(1)}px, ${(l.y).toFixed(1)}px, 0) translate(-50%, -50%)`;
    }
  }

  function draw() {
    const now = time;
    camera();
    updateLabels(now);
    const gr = [-1, -1, -1, -1], ga = [0, 0, 0, 0];
    let s = 0;
    for (const l of labels) {
      l.glow += ((l.on ? 1 : 0) - l.glow) * (still ? 1 : Math.min(1, frameDt * 1.6));
      if (l.glow > 0.01 && s < 4) { gr[s] = l.region; ga[s++] = l.glow; }
    }
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    if (light) gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA); else gl.blendFunc(gl.ONE, gl.ONE);
    gl.useProgram(prog);
    gl.uniformMatrix3fv(U.uRot, false, rot);
    gl.uniform2f(U.uShift, sx, sy); gl.uniform1f(U.uDist, dist); gl.uniform1f(U.uF, F); gl.uniform1f(U.uAspect, W / H);
    gl.uniform1f(U.uViewH, H * dpr); gl.uniform1f(U.uSize, 0.0105);
    gl.uniform1f(U.uTime, time); gl.uniform1f(U.uMotion, still ? 0 : 1);
    gl.uniform1f(U.uNear, dist - 0.55); gl.uniform1f(U.uFar, dist + 0.6); gl.uniform1f(U.uIntro, intro);
    gl.uniform4fv(U.uGR, gr); gl.uniform4fv(U.uGA, ga);
    gl.uniform3fv(U.cDot, colors.dot); gl.uniform3fv(U.cDim, colors.dim); gl.uniform3fv(U.cHot, colors.hot);
    gl.bindVertexArray(vao);
    gl.drawArrays(gl.POINTS, 0, pts.count);
    gl.bindVertexArray(null);
  }

  let frameDt = 0;
  function frame(t) {
    raf = 0;
    if (!alive || lost || !visible || !ready) return;
    raf = win.requestAnimationFrame(frame);
    if (still) return;
    if (last && t - last < 1000 / FPS - 2) return; // high-refresh screens: hold to the budget
    frameDt = last ? Math.min(0.05, (t - last) / 1000) : 1 / FPS;
    last = t;
    time += frameDt;
    intro = Math.min(1, intro + frameDt / 2.2);
    yaw += TURN * frameDt;
    py += (tpy - py) * Math.min(1, frameDt * 2.2);
    pp += (tpp - pp) * Math.min(1, frameDt * 2.2);
    const t0 = performance.now();
    draw();
    frameMs = performance.now() - t0;
  }
  // Wake the loop (motion) or draw once (still).
  function kick() {
    if (!alive || lost || !ready || !gl) return;
    if (still) { frameDt = 1; draw(); return; }
    if (!raf && visible) { last = 0; raf = win.requestAnimationFrame(frame); }
  }

  function resize() {
    const w = el.clientWidth, h = el.clientHeight;
    if (!w || !h) return false;
    dpr = Math.min(win.devicePixelRatio || 1, 2);
    W = w; H = h;
    measureFrame();
    canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr);
    for (const l of labels) l.el.style.fontSize = (w < 600 ? 13 : 15) + 'px';
    if (ready) { if (still) kick(); else if (!raf) { frameDt = 0; draw(); } }
    return true;
  }

  const nextFrame = () => new Promise((r) => win.requestAnimationFrame(() => r()));

  function teardown(keepImg) {
    alive = false;
    if (raf) win.cancelAnimationFrame(raf);
    raf = 0;
    for (const c of cleanups.splice(0)) c();
    if (gl && !lost && !gl.isContextLost()) { bufs.forEach((b) => gl.deleteBuffer(b)); if (vao) gl.deleteVertexArray(vao); if (prog) gl.deleteProgram(prog); }
    canvas.remove(); overlay.remove();
    if (img && keepImg) img.style.visibility = '';
  }

  async function start() {
    const src = el.dataset.src ? new URL(el.dataset.src, document.baseURI) : new URL('brain-data.bin', import.meta.url);
    const res = await fetch(src);
    if (!res.ok) throw new Error('brain-data: ' + res.status);
    data = decode(await res.arrayBuffer());
    if (!alive) return;
    await nextFrame();
    pts = buildPoints(data);
    el.append(canvas, overlay);
    await nextFrame();
    if (!alive) return;
    await initGL();
    if (!alive) return;
    if (showLabels) buildLabels();
    applyTheme();
    if (!resize()) await new Promise((r) => { const ro = new ResizeObserver(() => { if (resize()) { ro.disconnect(); r(); } }); ro.observe(el); cleanups.push(() => { ro.disconnect(); r(); }); });
    if (!alive) return;
    frameDt = 0;
    draw();
    if (gl.isContextLost()) throw new Error('brain: first frame failed');
    ready = true;

    // the first frame is on screen: swap the fallback for the canvas
    await nextFrame();
    if (!alive) return;
    canvas.style.opacity = '1';
    if (img) setTimeout(() => { if (alive && ready && !lost) img.style.visibility = 'hidden'; }, 950);

    if (img && !img.complete) listen(img, 'load', () => resize());
    const ro = new ResizeObserver(() => resize());
    ro.observe(el);
    cleanups.push(() => ro.disconnect());
    const io = new IntersectionObserver((es) => { visible = es[es.length - 1].isIntersecting && !document.hidden; kick(); });
    io.observe(el);
    cleanups.push(() => io.disconnect());
    listen(document, 'visibilitychange', () => { visible = !document.hidden; kick(); });
    const mq = win.matchMedia('(prefers-color-scheme: light)');
    listen(mq, 'change', applyTheme);
    const mo = new MutationObserver(applyTheme);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    cleanups.push(() => mo.disconnect());
    if (!still && win.matchMedia('(hover: hover) and (pointer: fine)').matches) {
      listen(win, 'pointermove', (e) => { tpy = (e.clientX / win.innerWidth - 0.5) * 0.4; tpp = (e.clientY / win.innerHeight - 0.5) * 0.18; }, { passive: true });
    }
    listen(canvas, 'webglcontextlost', (e) => {
      e.preventDefault(); lost = true;
      if (raf) win.cancelAnimationFrame(raf);
      raf = 0;
      if (img) img.style.visibility = '';
      canvas.style.opacity = '0';
    });
    listen(canvas, 'webglcontextrestored', async () => {
      try {
        lost = false; ready = false;
        await initGL();
        ready = true; frameDt = 0; draw();
        canvas.style.opacity = '1';
        if (img) img.style.visibility = 'hidden';
        kick();
      } catch { teardown(true); }
    });
    kick();
  }

  start().catch((err) => { if (alive) { teardown(true); if (opts.debug) console.warn(err); } });

  return {
    destroy() { teardown(true); },
    // for tests: draw n frames synchronously and wait for the gpu; mean ms per frame
    bench(n = 60) {
      if (!ready) return NaN;
      const px = new Uint8Array(4), t0 = performance.now();
      for (let i = 0; i < n; i++) { time += 1 / FPS; yaw += TURN / FPS; frameDt = 1 / FPS; draw(); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px); }
      return (performance.now() - t0) / n;
    },
    get ready() { return ready; },
    get frameMs() { return frameMs; },
  };
}
