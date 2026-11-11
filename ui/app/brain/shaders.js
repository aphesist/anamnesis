// GLSL for the stipple brain. Positions, metadata and the per-point state
// bitmask live in textures, fetched by gl_VertexID, so the points draw with
// no vertex attributes and filaments can look up their endpoints the same way.

const COMMON = /* glsl */ `#version 300 es
precision highp float;
precision highp int;
precision highp sampler2D;
precision highp usampler2D;
uniform sampler2D uA;      // anatomy xyz, w = seed
uniform sampler2D uP;      // layout (previous) xyz
uniform sampler2D uN;      // layout (next) xyz
uniform sampler2D uM;      // meta: tNorm, region, kind, speaker
uniform usampler2D uS;     // state bitmask
uniform mat4 uView, uProj;
uniform float uMorph, uLT, uTime, uMotion, uTmin, uTmax, uAnyLit, uNear, uFar;
uniform float uGlow[64];
ivec2 at(int i) { return ivec2(i & 1023, i >> 10); }
vec3 posOf(int i, out float seed) {
  vec4 a = texelFetch(uA, at(i), 0);
  vec3 lay = mix(texelFetch(uP, at(i), 0).xyz, texelFetch(uN, at(i), 0).xyz, uLT);
  seed = a.w;
  vec3 p = mix(a.xyz, lay, uMorph);
  // slow drift so the fog breathes; zero under reduced motion
  p += uMotion * 0.0045 * vec3(sin(uTime * 0.53 + seed * 41.0), cos(uTime * 0.47 + seed * 29.0), sin(uTime * 0.41 + seed * 17.0));
  return p;
}
`;

export const POINT_VS = COMMON + /* glsl */ `
uniform float uSize, uViewH;
uniform vec3 cDot, cDim, cHot, cCite, cFog;
out vec3 vCol;
out float vAlpha, vSeed, vSize, vRing;
void main() {
  int i = gl_VertexID;
  float seed;
  vec3 p = posOf(i, seed);
  vec4 m = texelFetch(uM, at(i), 0);
  uint s = texelFetch(uS, at(i), 0).r;
  vec4 mv = uView * vec4(p, 1.0);
  gl_Position = uProj * mv;
  float depth = -mv.z;
  float fogf = smoothstep(uNear, uFar, depth);
  bool lit = (s & 124u) != 0u;           // selected | cited | search | timewin | focus
  bool hover = (s & 2u) != 0u;
  bool filtered = (s & 1u) != 0u;
  float t = m.x;
  // time window: points outside are not yet written (or brushed out)
  float inTime = step(uTmin - 1e-5, t) * step(t, uTmax + 1e-5);
  // newborn flash while the scrubber replays growth
  float born = uMotion * smoothstep(0.012, 0.0, uTmax - t) * step(uTmax, 0.9999);
  int reg = int(m.y);
  float glow = reg >= 0 && reg < 64 ? uGlow[reg] : 0.0;

  vec3 col = mix(cDot, cDim, fogf * 0.85);
  float alpha = mix(0.9, 0.22, fogf);
  float size = uSize * (0.65 + 0.7 * fract(seed * 7.13));
  if (uAnyLit > 0.5 && !lit) { alpha *= 0.3; size *= 0.85; }
  if (lit) { col = ((s & 8u) != 0u) ? cCite : cHot; alpha = 1.0; size *= 1.55; }
  if (glow > 0.0) { col = mix(col, cHot, glow); alpha = mix(alpha, 1.0, glow); size *= 1.0 + 0.5 * glow; }
  if (born > 0.0) { col = mix(col, cHot, born); alpha = max(alpha, born); size *= 1.0 + born; }
  if (hover) { col = cHot; alpha = 1.0; size *= 2.2; }
  // per-point shimmer
  alpha *= 1.0 - uMotion * 0.28 * (0.5 + 0.5 * sin(uTime * (1.3 + fract(seed * 13.7) * 2.1) + seed * 91.0));
  alpha *= inTime;
  if (filtered) alpha = 0.0;
  float px = size * uProj[1][1] * uViewH * 0.5 / max(depth, 0.05);
  vSize = clamp(px, 1.0, 48.0);
  bool ringed = (s & 72u) != 0u;          // cited | focus get a stipple halo
  if (ringed) vSize = max(vSize * 2.2, 15.0);
  else if (lit) vSize = max(vSize, 3.0);
  // small dots lose ink as they shrink rather than vanishing abruptly
  vAlpha = alpha * min(1.0, px / 1.2);
  gl_PointSize = alpha <= 0.0 ? 0.0 : vSize;
  vCol = col;
  vSeed = seed;
  vRing = (hover || ringed) ? 1.0 : 0.0;
}
`;

export const POINT_FS = /* glsl */ `#version 300 es
precision highp float;
in vec3 vCol;
in float vAlpha, vSeed, vSize, vRing;
uniform float uLight;
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
      vec2 o = vec2(h(vSeed * 3.1 + fj * 1.7), h(vSeed * 5.7 + fj * 2.3)) * 2.0 - 1.0;
      o *= 0.55;
      float rr = 0.16 + 0.12 * h(vSeed + fj * 4.1);
      a += smoothstep(rr, rr * 0.35, length(pc - o));
    }
    a = min(1.0, a) * smoothstep(1.0, 0.75, r);
    // dither against noise: ink, not paint
    a *= step(h2(gl_FragCoord.xy + vSeed * 97.0) * 0.6, a);
  }
  if (vRing > 0.5) {
    // a ring of stipple: dashed by angle so it reads as dots, not a line
    float ang = atan(pc.y, pc.x);
    float dash = step(0.5, fract(ang * 2.546 + vSeed * 3.0));
    a = max(min(1.0, a * 1.6) * step(r, 0.5), smoothstep(0.09, 0.0, abs(r - 0.84)) * dash);
  }
  a *= vAlpha;
  if (a < 0.004) discard;
  frag = vec4(enc(vCol) * a, a);
}
`;

export const LINK_VS = COMMON + /* glsl */ `
layout(location = 0) in float aT;
layout(location = 1) in ivec3 aLink;   // from, to, kind (0 written, 1 manual, 2 accepted)
uniform vec3 cThread, cHot;
uniform float uLinkAlpha;
out float vT, vAlpha, vSeed, vKind;
out vec3 vCol;
void main() {
  float s0, s1;
  vec3 a = posOf(aLink.x, s0);
  vec3 b = posOf(aLink.y, s1);
  uint sa = texelFetch(uS, at(aLink.x), 0).r, sb = texelFetch(uS, at(aLink.y), 0).r;
  float ta = texelFetch(uM, at(aLink.x), 0).x, tb = texelFetch(uM, at(aLink.y), 0).x;
  vec3 mid = 0.5 * (a + b);
  // bend toward the centre: cheap pseudo-bundling
  vec3 c = mix(mid, mid * 0.35, 0.55 + 0.25 * fract(s0 * 9.1));
  float t = aT;
  vec3 p = (1.0 - t) * (1.0 - t) * a + 2.0 * (1.0 - t) * t * c + t * t * b;
  vec4 mv = uView * vec4(p, 1.0);
  gl_Position = uProj * mv;
  float fogf = smoothstep(uNear, uFar, -mv.z);
  bool lit = ((sa | sb) & 126u) != 0u;
  bool off = ((sa | sb) & 1u) != 0u;
  float inTime = step(ta, uTmax + 1e-5) * step(tb, uTmax + 1e-5) * step(uTmin - 1e-5, max(ta, tb));
  float alpha = uLinkAlpha * (aLink.z == 0 ? 0.7 : 1.15) * mix(1.0, 0.35, fogf);
  vCol = cThread;
  if (uAnyLit > 0.5) alpha *= lit ? 6.0 : 0.35;
  if (lit) vCol = mix(cThread, cHot, 0.7);
  if (off) alpha = 0.0;
  vAlpha = alpha * inTime;
  vT = t; vSeed = s0 + s1; vKind = float(aLink.z);
}
`;

export const LINK_FS = /* glsl */ `#version 300 es
precision highp float;
in float vT, vAlpha, vSeed, vKind;
in vec3 vCol;
out vec4 frag;
vec3 enc(vec3 c) { return mix(c * 12.92, 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055, step(vec3(0.0031308), c)); }
void main() {
  // written links: dotted stipple; hand links: longer brush dashes
  float k = vKind < 0.5 ? 46.0 : 18.0;
  float duty = vKind < 0.5 ? 0.4 : 0.72;
  float d = step(fract(vT * k + vSeed * 3.0), duty);
  float ends = smoothstep(0.0, 0.08, vT) * smoothstep(1.0, 0.92, vT);
  float a = vAlpha * d * ends;
  if (a < 0.002) discard;
  frag = vec4(enc(vCol) * a, a);
}
`;

// Ornamental outline: faint, tiny, not pickable; fades out as the form morphs.
export const SHELL_VS = /* glsl */ `#version 300 es
precision highp float;
layout(location = 0) in vec3 aPos;
uniform mat4 uView, uProj;
uniform float uViewH, uAlpha, uTime, uMotion, uNear, uFar;
out float vA;
void main() {
  vec4 mv = uView * vec4(aPos, 1.0);
  gl_Position = uProj * mv;
  float fogf = smoothstep(uNear, uFar, -mv.z);
  float seed = fract(sin(float(gl_VertexID) * 12.9898) * 43758.5453);
  vA = uAlpha * mix(1.0, 0.25, fogf) * (0.55 + 0.45 * seed) * (1.0 - uMotion * 0.3 * (0.5 + 0.5 * sin(uTime * 0.9 + seed * 60.0)));
  gl_PointSize = max(1.0, 0.006 * uProj[1][1] * uViewH * 0.5 / max(-mv.z, 0.05));
}
`;
export const SHELL_FS = /* glsl */ `#version 300 es
precision highp float;
in float vA;
uniform vec3 cDim;
out vec4 frag;
vec3 enc(vec3 c) { return mix(c * 12.92, 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055, step(vec3(0.0031308), c)); }
void main() {
  float r = length(gl_PointCoord * 2.0 - 1.0);
  float a = vA * smoothstep(1.0, 0.2, r);
  if (a < 0.003) discard;
  frag = vec4(enc(cDim) * a, a);
}
`;
