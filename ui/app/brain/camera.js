// Orbit camera with inertia, auto-rotation ("turning") and minimum-jerk fly-to.
import { perspective, lookAt, mul, mj, lerp, clamp } from './math.js';

export class OrbitCamera {
  constructor() {
    this.yaw = 1.25; this.pitch = 0.12; this.dist = 4.5;
    this.target = [0, 0, 0];
    this.vYaw = 0; this.vPitch = 0; this.vZoom = 0;
    this.turning = true; this.speed = 0.35; this.motion = 1;
    this.fov = 0.72;
    this.view = new Float32Array(16); this.proj = new Float32Array(16); this.viewProj = new Float32Array(16);
    this.eye = [0, 0, 0];
    this.flight = null;
    this.dragging = false;
    this.sway = false; this.swayCentre = 0;
    this.version = 0;
    this.home = { yaw: 1.25, pitch: 0.12, dist: 4.5, target: [0, 0, 0] };
    this.shift = [0, 0]; // lens shift in ndc, to keep the brain clear of overlays
  }
  update(dt, aspect) {
    // the lens shift eases toward its target (a floating panel opening beside the brain)
    let shifting = false;
    if (this.shiftTarget) for (let k = 0; k < 2; k++) {
      const d = this.shiftTarget[k] - this.shift[k];
      if (Math.abs(d) > 1e-3) { this.shift[k] += d * Math.min(1, dt * (this.motion > 0 ? 5 : 60)); shifting = true; } else this.shift[k] = this.shiftTarget[k];
    }
    const before = this.yaw + this.pitch * 7 + this.dist * 13 + this.target[0] * 17 + this.target[1] * 19 + this.target[2] * 23;
    if (this.flight) {
      const f = this.flight;
      f.t += dt / f.dur;
      const k = mj(Math.min(1, f.t));
      this.yaw = lerp(f.from.yaw, f.to.yaw, k);
      this.pitch = lerp(f.from.pitch, f.to.pitch, k);
      this.dist = lerp(f.from.dist, f.to.dist, k);
      for (let i = 0; i < 3; i++) this.target[i] = lerp(f.from.target[i], f.to.target[i], k);
      if (f.t >= 1) this.flight = null;
    } else {
      if (!this.dragging) {
        const decay = Math.pow(0.04, dt); // inertia
        this.vYaw *= decay; this.vPitch *= decay; this.vZoom *= decay;
        if (Math.abs(this.vYaw) < 1e-4) this.vYaw = 0;
        if (Math.abs(this.vPitch) < 1e-4) this.vPitch = 0;
        if (Math.abs(this.vZoom) < 1e-4) this.vZoom = 0;
      }
      this.yaw += this.vYaw * dt;
      this.pitch = clamp(this.pitch + this.vPitch * dt, -1.35, 1.35);
      this.dist = clamp(this.dist * Math.exp(this.vZoom * dt), 0.6, 12);
      if (this.turning && this.motion > 0 && !this.dragging) {
        if (this.sway) {
          // flat layouts: sway gently around the front instead of turning away
          this.swayT = (this.swayT || 0) + dt * this.speed * 0.6;
          this.yaw += (this.swayCentre + 0.32 * Math.sin(this.swayT) - this.yaw) * Math.min(1, dt * 2);
        } else this.yaw += this.speed * 0.25 * dt;
      }
    }
    const cp = Math.cos(this.pitch);
    // short stages (a bottom dock open) back the camera off so the brain still fits
    const fit = Math.min(1.8, Math.pow(Math.max(1, aspect / 1.1), 0.6));
    const d = this.dist * fit;
    this.effDist = d;
    this.eye[0] = this.target[0] + d * cp * Math.sin(this.yaw);
    this.eye[1] = this.target[1] + d * Math.sin(this.pitch);
    this.eye[2] = this.target[2] + d * cp * Math.cos(this.yaw);
    lookAt(this.view, this.eye, this.target, [0, 1, 0]);
    perspective(this.proj, this.fov, aspect, 0.05, 50);
    this.proj[8] = -this.shift[0]; this.proj[9] = -this.shift[1];
    mul(this.viewProj, this.proj, this.view);
    const after = this.yaw + this.pitch * 7 + this.dist * 13 + this.target[0] * 17 + this.target[1] * 19 + this.target[2] * 23;
    if (after !== before || this._aspect !== aspect) { this.version++; this._aspect = aspect; }
    return shifting || this.flight != null || this.vYaw !== 0 || this.vPitch !== 0 || this.vZoom !== 0 || (this.turning && this.motion > 0);
  }
  drag(dx, dy, dt) {
    this.flight = null;
    const k = 0.006;
    this.yaw -= dx * k; this.pitch = clamp(this.pitch + dy * k, -1.35, 1.35);
    if (dt > 0) { this.vYaw = (-dx * k) / dt; this.vPitch = (dy * k) / dt; }
  }
  zoom(delta) { this.flight = null; this.vZoom += delta * 0.0025; }
  stop() { this.vYaw = this.vPitch = this.vZoom = 0; }
  flyTo(target, dist, opts = {}) {
    const to = { yaw: opts.yaw ?? this.yaw, pitch: opts.pitch ?? this.pitch, dist: dist ?? this.dist, target: [...target] };
    // shortest way round in yaw
    while (to.yaw - this.yaw > Math.PI) to.yaw -= 2 * Math.PI;
    while (to.yaw - this.yaw < -Math.PI) to.yaw += 2 * Math.PI;
    this.stop();
    if (this.motion === 0 || opts.instant) {
      this.flight = null; this.yaw = to.yaw; this.pitch = to.pitch; this.dist = to.dist; this.target = to.target;
      return;
    }
    this.flight = { t: 0, dur: (opts.dur ?? 900) / 1000, from: { yaw: this.yaw, pitch: this.pitch, dist: this.dist, target: [...this.target] }, to };
  }
  reset(instant) { const h = this.home; this.flyTo(h.target, h.dist, { yaw: h.yaw, pitch: h.pitch, instant }); }
}
