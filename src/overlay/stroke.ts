// One stroke, from the trigger going down to the last pixel of its
// confirmation. Owns the path, turns it into geometry every frame, and plays
// the ending it is told to play.
//
// Choreography of a match, for confirm_ms = 400 (all times scale with it):
//
//   element            role        starts  lasts   motion
//   trail -> clean     lead        0       150     point-wise morph, ease-out
//   pulse              decoration  0       260     a glint runs along the shape once, in drawing order
//   label chip         support     60      140     fade + 6 px rise (see label.ts)
//   dissolve           exit        270     130     everything together: thin and fade, no movement
//
// The action and the rumble tick have already left by the time any of this
// starts; nothing here is waited on.

import { clamp01, easeOut, lerp, smoothstep } from '../shared/ease';
import type { OverlayCfg, StrokeResult } from '../shared/types';
import { Geometry } from './geometry';
import { arcLengths, catmull, hash, linearResample, noise1, resampleByArc } from './path';
import type { Look } from './renderer';

export interface Env {
  cfg: OverlayCfg;
  /** Device pixel ratio of this monitor. */
  scale: number;
  /** Monitor height in physical pixels (for placing the label). */
  height: number;
  reducedMotion: boolean;
}

type State = 'drawing' | 'match' | 'no_match' | 'captured' | 'cancel';

/** Points in the morph and in a captured or cancelled trail. */
const M = 176;
const EMBER_IN_MS = 110;
const CANCEL_MS = 120;
const MAX_SPARKS = 48;

interface Spark {
  x: number;
  y: number;
  vx: number;
  vy: number;
  born: number;
  life: number;
}

interface Crumb {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  half: number;
  glow: number;
  intensity: number;
  delay: number;
  vx: number;
  vy: number;
  spin: number;
}

function grow(a: Float32Array<ArrayBuffer>, need: number): Float32Array<ArrayBuffer> {
  if (need <= a.length) return a;
  const b = new Float32Array(Math.max(need, a.length * 2));
  b.set(a);
  return b;
}

export class Stroke {
  state: State = 'drawing';
  done = false;
  result: StrokeResult | null = null;
  readonly seed: number;

  // Control samples as they arrive (decimated), times in local ms.
  private cx: number[] = [];
  private cy: number[] = [];
  private ct: number[] = [];

  // Smoothed path built from the control samples. Entries [0, n) are final;
  // the stretch up to the predicted head is rebuilt behind them every frame.
  private px = new Float32Array(1024);
  private py = new Float32Array(1024);
  private pt = new Float32Array(1024);
  private pv = new Float32Array(1024);
  private ps = new Float32Array(1024);
  private n = 0;
  private finalSegments = 0;
  private speed = 0;

  private half = new Float32Array(1024);
  private glow = new Float32Array(1024);
  private inten = new Float32Array(1024);
  private heat = new Float32Array(1024);

  private tEnd = 0;
  private lastBuild = 0;
  private sparkDebt = 0;
  private sparks: Spark[] = [];

  // Ending data, filled in by finish().
  private fx = new Float32Array(0);
  private fy = new Float32Array(0);
  private tx = new Float32Array(0);
  private ty = new Float32Array(0);
  private relHalf = new Float32Array(0);
  private relInt = new Float32Array(0);
  private relHeat = new Float32Array(0);
  private closed = false;
  private crumbs: Crumb[] = [];
  private mx = new Float32Array(M);
  private my = new Float32Array(M);

  constructor(
    readonly id: number,
    readonly tDown: number,
    x: number,
    y: number,
    /** Milliseconds the head is drawn ahead of the newest sample. */
    private lead: number,
  ) {
    this.seed = (id * 37.7) % 1000;
    this.cx.push(x);
    this.cy.push(y);
    this.ct.push(tDown);
    this.px[0] = x;
    this.py[0] = y;
    this.pt[0] = 0;
    this.pv[0] = 0;
    this.ps[0] = 0;
    this.n = 1;
    this.lastBuild = tDown;
  }

  get lastSampleTime(): number {
    return this.ct[this.ct.length - 1];
  }

  /** A new sample, local pixels and local milliseconds. */
  add(t: number, x: number, y: number): void {
    if (this.state !== 'drawing') return;
    const k = this.cx.length - 1;
    const d = Math.hypot(x - this.cx[k], y - this.cy[k]);
    // Drop samples that neither moved nor waited: they only add noise to speed.
    if (d < 1.5 && t - this.ct[k] < 10) return;
    this.cx.push(x);
    this.cy.push(y);
    this.ct.push(Math.max(t, this.ct[k] + 0.25));
    this.finalize();
  }

  /** Turn every control segment that now has both neighbours into path points. */
  private finalize(): void {
    const c = this.cx.length;
    while (this.finalSegments < c - 2) {
      const j = this.finalSegments;
      this.n = this.emitSegment(this.n, j, this.cx[j + 2], this.cy[j + 2], true);
      this.finalSegments++;
    }
  }

  /**
   * Append the Catmull-Rom curve for control segment j -> j+1 at index `at`.
   * (nx, ny) is the point after j+1. Returns the new length.
   */
  private emitSegment(at: number, j: number, nx: number, ny: number, commit: boolean): number {
    const { cx, cy, ct } = this;
    const i0 = Math.max(0, j - 1);
    const len = Math.hypot(cx[j + 1] - cx[j], cy[j + 1] - cy[j]);
    const dt = Math.max(ct[j + 1] - ct[j], 0.25);
    const v = (len / dt) * 1000;
    const a = 1 - Math.exp(-dt / 28);
    const speed = this.speed + (v - this.speed) * a;
    const steps = Math.min(24, Math.max(1, Math.ceil(len / 3)));
    this.ensure(at + steps + 64);
    const startSpeed = this.speed;
    for (let s = 1; s <= steps; s++) {
      const t = s / steps;
      const x = catmull(cx[i0], cx[j], cx[j + 1], nx, t);
      const y = catmull(cy[i0], cy[j], cy[j + 1], ny, t);
      this.px[at] = x;
      this.py[at] = y;
      this.pt[at] = lerp(ct[j], ct[j + 1], t) - this.tDown;
      this.pv[at] = lerp(startSpeed, speed, t);
      this.ps[at] = this.ps[at - 1] + Math.hypot(x - this.px[at - 1], y - this.py[at - 1]);
      at++;
    }
    if (commit) this.speed = speed;
    return at;
  }

  private ensure(need: number): void {
    if (need <= this.px.length) return;
    this.px = grow(this.px, need);
    this.py = grow(this.py, need);
    this.pt = grow(this.pt, need);
    this.pv = grow(this.pv, need);
    this.ps = grow(this.ps, need);
    this.half = grow(this.half, need);
    this.glow = grow(this.glow, need);
    this.inten = grow(this.inten, need);
    this.heat = grow(this.heat, need);
  }

  /**
   * Where the wand is right now: the newest sample carried forward along its
   * velocity and its rate of turn. Following the turn keeps the head on the
   * curve when circling instead of flying off along the tangent.
   */
  private predict(now: number): { x: number; y: number; vx: number; vy: number } {
    const { cx, cy, ct } = this;
    const k = cx.length - 1;
    const since = Math.max(0, now - ct[k]);
    if (k === 0 || this.state !== 'drawing') {
      return { x: cx[k], y: cy[k], vx: 0, vy: 0 };
    }
    // Velocity over at least 8 ms, and the one before it for the turn rate.
    let j = k - 1;
    while (j > 0 && ct[k] - ct[j] < 8) j--;
    const dt1 = Math.max(ct[k] - ct[j], 0.25);
    let vx = (cx[k] - cx[j]) / dt1;
    let vy = (cy[k] - cy[j]) / dt1;
    let turn = 0;
    if (j > 0) {
      let i = j - 1;
      while (i > 0 && ct[j] - ct[i] < 8) i--;
      const dt0 = Math.max(ct[j] - ct[i], 0.25);
      const ux = (cx[j] - cx[i]) / dt0;
      const uy = (cy[j] - cy[i]) / dt0;
      const s0 = Math.hypot(ux, uy);
      const s1 = Math.hypot(vx, vy);
      if (s0 > 0.05 && s1 > 0.05) {
        const ang = Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
        turn = ang / ((dt0 + dt1) / 2); // rad per ms
        // Direction at the newest sample, not at the middle of its interval.
        const half = (turn * dt1) / 2;
        const cs = Math.cos(half);
        const sn2 = Math.sin(half);
        [vx, vy] = [vx * cs - vy * sn2, vx * sn2 + vy * cs];
      }
    }
    // Lead: the age of the sample plus the configured prediction, capped.
    // When samples stop arriving the wand has stopped, so the lead dies away.
    const lead = Math.min(since + this.lead, 28) * Math.exp(-Math.max(since - 8, 0) / 20);
    const maxTurn = 0.7 / Math.max(lead, 1);
    turn = Math.max(-maxTurn, Math.min(maxTurn, turn));
    let x = cx[k];
    let y = cy[k];
    const speed = Math.hypot(vx, vy);
    if (Math.abs(turn) > 1e-5 && speed > 0) {
      const th = Math.atan2(vy, vx);
      x += (speed / turn) * (Math.sin(th + turn * lead) - Math.sin(th));
      y -= (speed / turn) * (Math.cos(th + turn * lead) - Math.cos(th));
      const end = th + turn * lead;
      vx = Math.cos(end) * speed;
      vy = Math.sin(end) * speed;
    } else {
      x += vx * lead;
      y += vy * lead;
    }
    return { x, y, vx: vx * 1000, vy: vy * 1000 };
  }

  /** The trigger was released and the engine has decided what this was. */
  finish(result: StrokeResult, now: number, env: Env, toLocal: (flat: number[]) => number[]): void {
    if (this.state !== 'drawing') return;
    // Freeze the path as it is on screen, including the predicted tip.
    const head = this.predict(now);
    const total = this.buildLive(now, env, head.x, head.y);
    this.tEnd = now;
    this.result = result;

    const cum = new Float32Array(total);
    arcLengths(this.px, this.py, total, cum);
    this.fx = new Float32Array(M);
    this.fy = new Float32Array(M);
    this.relHalf = new Float32Array(M);
    this.relInt = new Float32Array(M);
    this.relHeat = new Float32Array(M);
    if (total >= 2 && cum[total - 1] > 1) {
      resampleByArc(this.px, cum, total, M, this.fx);
      resampleByArc(this.py, cum, total, M, this.fy);
      resampleByArc(this.half, cum, total, M, this.relHalf);
      resampleByArc(this.inten, cum, total, M, this.relInt);
      resampleByArc(this.heat, cum, total, M, this.relHeat);
    } else {
      this.fx.fill(this.px[0]);
      this.fy.fill(this.py[0]);
    }

    if (result.kind === 'match') {
      // Straight resampling: the clean shape keeps its corners.
      const to = linearResample(toLocal(result.to), M);
      this.tx = to.xs;
      this.ty = to.ys;
      let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
      for (let i = 0; i < M; i++) {
        minX = Math.min(minX, this.tx[i]);
        maxX = Math.max(maxX, this.tx[i]);
        minY = Math.min(minY, this.ty[i]);
        maxY = Math.max(maxY, this.ty[i]);
      }
      const extent = Math.max(maxX - minX, maxY - minY, 1);
      this.closed = Math.hypot(this.tx[0] - this.tx[M - 1], this.ty[0] - this.ty[M - 1]) < extent * 0.12;
      this.state = 'match';
    } else if (result.kind === 'no_match') {
      this.makeCrumbs(env, cum[total - 1] || 0);
      this.state = 'no_match';
    } else {
      this.state = 'captured';
    }
  }

  cancel(now: number, env: Env): void {
    if (this.state !== 'drawing') return;
    const head = this.predict(now);
    const total = this.buildLive(now, env, head.x, head.y);
    const cum = new Float32Array(Math.max(total, 2));
    arcLengths(this.px, this.py, total, cum);
    this.fx = new Float32Array(M);
    this.fy = new Float32Array(M);
    this.relHalf = new Float32Array(M);
    this.relInt = new Float32Array(M);
    this.relHeat = new Float32Array(M);
    if (total >= 2 && cum[total - 1] > 1) {
      resampleByArc(this.px, cum, total, M, this.fx);
      resampleByArc(this.py, cum, total, M, this.fy);
      resampleByArc(this.half, cum, total, M, this.relHalf);
      resampleByArc(this.inten, cum, total, M, this.relInt);
      resampleByArc(this.heat, cum, total, M, this.relHeat);
    } else {
      this.fx.fill(this.px[0]);
      this.fy.fill(this.py[0]);
      this.relInt.fill(1);
      this.relHalf.fill(2 * env.scale);
    }
    this.tEnd = now;
    this.state = 'cancel';
  }

  /** Break the frozen trail into pieces that will let go and fall. */
  private makeCrumbs(env: Env, length: number): void {
    const count = Math.max(8, Math.min(M - 1, Math.round(length / (7 * env.scale))));
    const step = (M - 1) / count;
    const C = env.cfg.confirm_ms;
    this.crumbs = [];
    for (let c = 0; c < count; c++) {
      const a = Math.round(c * step);
      const b = Math.min(M - 1, Math.round((c + 1) * step));
      if (b <= a) continue;
      // Neighbours let go together (low-frequency noise), so it crumbles in
      // clumps instead of dissolving evenly.
      const clump = noise1(c * 0.16 + this.seed);
      const jitter = hash(c * 7.3 + this.seed);
      this.crumbs.push({
        x0: this.fx[a],
        y0: this.fy[a],
        x1: this.fx[b],
        y1: this.fy[b],
        half: Math.max(this.relHalf[a], 0.9 * env.scale),
        glow: 5 * env.scale,
        intensity: Math.max(this.relInt[a], 0.72),
        // The line holds for a beat while the colour drains, then lets go.
        delay: C * (0.12 + 0.3 * clump + 0.06 * jitter),
        vx: (jitter - 0.5) * 90 * env.scale,
        vy: (0.2 + 0.8 * hash(c * 3.1 + this.seed)) * 70 * env.scale,
        spin: (hash(c * 11.9 + this.seed) - 0.5) * 9,
      });
    }
  }

  /**
   * Fill the attribute arrays for the live trail up to the head at (hx, hy)
   * and return the number of points.
   */
  private buildLive(now: number, env: Env, hx: number, hy: number): number {
    const { cfg, scale } = env;
    let total = this.n;
    const c = this.cx.length;
    // Provisional stretch: the newest control segment, then on to the head.
    if (c >= 2) {
      total = this.emitSegment(total, c - 2, hx, hy, false);
    }
    this.ensure(total + 16);
    const lx = this.px[total - 1];
    const ly = this.py[total - 1];
    const gap = Math.hypot(hx - lx, hy - ly);
    if (gap > 0.75) {
      const steps = Math.min(8, Math.max(1, Math.ceil(gap / 4)));
      const i0 = Math.max(0, total - 2);
      for (let s = 1; s <= steps; s++) {
        const t = s / steps;
        const x = catmull(this.px[i0], lx, hx, hx, t);
        const y = catmull(this.py[i0], ly, hy, hy, t);
        this.px[total] = x;
        this.py[total] = y;
        this.pt[total] = now - this.tDown;
        this.pv[total] = this.pv[total - 1];
        this.ps[total] = this.ps[total - 1] + Math.hypot(x - this.px[total - 1], y - this.py[total - 1]);
        total++;
      }
    }

    const tau = cfg.tail_fade_ms / 2.6;
    const floor = cfg.tail_floor;
    const wSpan = cfg.width_max_px - cfg.width_min_px;
    const age0 = now - this.tDown;
    const fibreScale = 0.045 / scale;
    for (let i = 0; i < total; i++) {
      const sn = clamp01(this.pv[i] / cfg.speed_full_px_s);
      const fresh = Math.exp(-Math.max(0, age0 - this.pt[i]) / tau);
      // Thin where the hand was slow, wide where it was fast: corners pinch
      // and both ends taper on their own.
      const start = 0.15 + 0.85 * smoothstep(0, 46 * scale, this.ps[i]);
      this.half[i] = (cfg.width_min_px + wSpan * Math.pow(sn, 0.6)) * scale * (0.4 + 0.6 * fresh) * start;
      // A fine grain along the stroke, fixed to the path, so the ribbon is
      // not one flat tube of light.
      const fibre = 0.84 + 0.16 * noise1(this.ps[i] * fibreScale + this.seed);
      this.inten[i] = (floor + (1 - floor) * fresh * (0.78 + 0.22 * sn)) * fibre;
      // Temperature: white-hot where the ember just was, cooling to plain
      // coral within a third of the fade.
      this.heat[i] = (0.45 + 0.55 * Math.pow(sn, 0.8)) * Math.exp(-Math.max(0, age0 - this.pt[i]) / (tau * 0.55));
      this.glow[i] = scale * (16 + 16 * sn) * (0.55 + 0.45 * fresh);
    }
    return total;
  }

  private stepSparks(now: number, env: Env, head: { x: number; y: number; vx: number; vy: number }, sn: number): void {
    const dt = Math.min(0.05, Math.max(0, (now - this.lastBuild) / 1000));
    const { scale, cfg } = env;
    if (this.state === 'drawing' && !env.reducedMotion && cfg.spark_rate > 0 && sn > 0.32) {
      this.sparkDebt += cfg.spark_rate * 70 * Math.pow(sn - 0.32, 1.3) * dt;
      const speed = Math.hypot(head.vx, head.vy) || 1;
      const tx = head.vx / speed;
      const ty = head.vy / speed;
      while (this.sparkDebt >= 1 && this.sparks.length < MAX_SPARKS) {
        this.sparkDebt -= 1;
        const r = Math.random();
        const side = Math.random() < 0.5 ? -1 : 1;
        const across = side * (90 + 290 * Math.random()) * scale;
        const along = (-0.16 + 0.2 * r) * speed;
        this.sparks.push({
          x: head.x - tx * 6 * scale,
          y: head.y - ty * 6 * scale,
          vx: tx * along - ty * across,
          vy: ty * along + tx * across,
          born: now,
          life: 160 + 180 * Math.random(),
        });
      }
      this.sparkDebt = Math.min(this.sparkDebt, 2);
    }
    const drag = Math.exp(-3.2 * dt);
    for (let i = this.sparks.length - 1; i >= 0; i--) {
      const s = this.sparks[i];
      if (now - s.born >= s.life) {
        this.sparks.splice(i, 1);
        continue;
      }
      s.vx *= drag;
      s.vy = s.vy * drag + 700 * scale * dt;
      s.x += s.vx * dt;
      s.y += s.vy * dt;
    }
  }

  private drawSparks(now: number, geo: Geometry, scale: number): void {
    for (const s of this.sparks) {
      const k = (now - s.born) / s.life;
      const fade = Math.pow(1 - k, 1.5);
      geo.capsule(s.x - s.vx * 0.014, s.y - s.vy * 0.014, s.x, s.y, 0.75 * scale, 3.2 * scale, fade, 1 - k);
    }
  }

  /** Emit this frame's geometry. Returns how to composite it, or null if there is nothing to draw. */
  build(now: number, geo: Geometry, env: Env): Look | null {
    const { cfg, scale } = env;
    const look: Look = { opacity: 1, desat: 0, white: 0, gain: 1 };
    const C = cfg.confirm_ms;

    if (this.state === 'drawing') {
      const head = this.predict(now);
      const sn = clamp01(this.speed / cfg.speed_full_px_s) * Math.exp(-Math.max(now - this.lastSampleTime - 8, 0) / 60);
      const total = this.buildLive(now, env, head.x, head.y);
      this.stepSparks(now, env, head, sn);
      this.lastBuild = now;
      geo.ribbon(total, this.px, this.py, this.half, this.glow, this.inten, this.heat);
      geo.sprite(this.px[0], this.py[0], this.half[0], this.glow[0], this.inten[0], 0);
      // The ember: scales and fades in from 60%, never from nothing, and
      // flickers a little so it is alive while the hand is still.
      const k = easeOut(clamp01((now - this.tDown) / EMBER_IN_MS));
      const flicker = 0.93 + 0.07 * noise1(now * 0.011 + this.seed);
      geo.sprite(head.x, head.y, scale * (2.6 + 1.6 * sn) * (0.6 + 0.4 * k), scale * (22 + 10 * sn) * (0.7 + 0.3 * k), k * flicker, 1);
      this.drawSparks(now, geo, scale);
      return look;
    }

    const tau = now - this.tEnd;
    this.stepSparks(now, env, { x: 0, y: 0, vx: 0, vy: 0 }, 0);
    this.lastBuild = now;

    if (this.state === 'match') {
      const Tm = 0.375 * C;
      const Td = 0.275 * C;
      const still = env.reducedMotion;
      const e = still ? 1 : easeOut(clamp01(tau / Tm));
      const rise = easeOut(clamp01(tau / Tm));
      const out = easeOut(clamp01((tau - 0.675 * C) / (0.325 * C)));
      // The pulse is a light with a position: it runs once along the shape in
      // the order it was drawn, arriving at the far end as the morph settles.
      // A uniform flash would make every gesture's ending look the same.
      // Constant speed (linear): it is a light crossing the shape, not an
      // element arriving somewhere.
      const sweep = clamp01(tau / (Tm + Td)) * 1.3 - 0.12;
      const lift = tau < Tm ? rise : 1 - easeOut(clamp01((tau - Tm) / Td));
      const cleanW = (cfg.width_min_px + (cfg.width_max_px - cfg.width_min_px) * 0.35) * scale;
      this.ensure(M);
      for (let i = 0; i < M; i++) {
        const u = i / (M - 1);
        this.mx[i] = lerp(this.fx[i], this.tx[i], e);
        this.my[i] = lerp(this.fy[i], this.ty[i], e);
        const taper = this.closed ? 1 : 0.4 + 0.6 * Math.sqrt(Math.sin(Math.PI * u));
        // On a closed shape the glint wraps, so the two ends that meet are
        // lit alike and no seam shows in the glow where they join.
        const du = this.closed ? ((((u - sweep + 0.5) % 1) + 1) % 1) - 0.5 : u - sweep;
        const k = du / 0.2;
        const glint = still ? 0.5 * lift : Math.exp(-k * k);
        this.half[i] = lerp(this.relHalf[i], cleanW * taper, rise) * (1 + 0.4 * glint) * (1 - 0.5 * out);
        this.inten[i] = lerp(this.relInt[i], 0.9, rise) + 0.7 * glint;
        this.heat[i] = clamp01(lerp(this.relHeat[i], 0.15, rise) + 0.85 * glint);
        this.glow[i] = scale * (14 + 4 * rise + 14 * glint);
      }
      geo.ribbon(M, this.mx, this.my, this.half, this.glow, this.inten, this.heat);
      // Round both ends; let the ember die down where it stopped.
      geo.sprite(this.mx[0], this.my[0], this.half[0], this.glow[0], this.inten[0], this.heat[0]);
      const ember = 1 - clamp01(tau / 90);
      geo.sprite(this.mx[M - 1], this.my[M - 1], Math.max(this.half[M - 1], scale * 2.6 * ember), this.glow[M - 1], Math.max(this.inten[M - 1], ember), this.heat[M - 1]);
      this.drawSparks(now, geo, scale);
      look.opacity = 1 - out;
      look.gain = 1 + 0.12 * lift;
      if (tau >= C) this.done = true;
      return look;
    }

    if (this.state === 'no_match') {
      const G = 2600 * scale;
      const fadeAll = 1 - smoothstep(0.6, 1, tau / C);
      for (const c of this.crumbs) {
        const tt = env.reducedMotion ? 0 : Math.max(0, tau - c.delay) / 1000;
        const dx = c.vx * tt;
        const dy = c.vy * tt + 0.5 * G * tt * tt;
        const cxm = (c.x0 + c.x1) / 2;
        const cym = (c.y0 + c.y1) / 2;
        const cs = Math.cos(c.spin * tt);
        const sn = Math.sin(c.spin * tt);
        const hx = ((c.x1 - c.x0) / 2) * cs - ((c.y1 - c.y0) / 2) * sn;
        const hy = ((c.x1 - c.x0) / 2) * sn + ((c.y1 - c.y0) / 2) * cs;
        const falling = 1 - 0.35 * clamp01((tt * 1000) / 260);
        geo.capsule(cxm - hx + dx, cym - hy + dy, cxm + hx + dx, cym + hy + dy, c.half, c.glow, c.intensity * fadeAll * falling, 0);
      }
      this.drawSparks(now, geo, scale);
      look.desat = clamp01(tau / 90);
      look.gain = 1 - 0.2 * look.desat;
      if (tau >= C) this.done = true;
      return look;
    }

    // Captured sample or cancelled tap: the frozen trail, then gone.
    this.ensure(M);
    for (let i = 0; i < M; i++) {
      this.half[i] = this.relHalf[i];
      this.inten[i] = this.relInt[i];
      this.heat[i] = this.relHeat[i];
      this.glow[i] = scale * 8;
    }
    geo.ribbon(M, this.fx, this.fy, this.half, this.glow, this.inten, this.heat);
    this.drawSparks(now, geo, scale);
    if (this.state === 'captured') {
      // Turns to ink (it has been taken down), holds, fades.
      look.white = easeOut(clamp01(tau / 90));
      look.opacity = 1 - easeOut(clamp01((tau - 0.5 * C) / (0.5 * C)));
      if (tau >= C) this.done = true;
    } else {
      look.opacity = 1 - easeOut(clamp01(tau / CANCEL_MS));
      if (tau >= CANCEL_MS) this.done = true;
    }
    return look;
  }

  /** Where the label goes for a match: under the clean shape, or above it near the bottom edge. Local px. */
  labelAnchor(env: Env): { x: number; y: number; above: boolean } | null {
    if (this.state !== 'match') return null;
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    for (let i = 0; i < M; i++) {
      minX = Math.min(minX, this.tx[i]);
      maxX = Math.max(maxX, this.tx[i]);
      minY = Math.min(minY, this.ty[i]);
      maxY = Math.max(maxY, this.ty[i]);
    }
    const gap = 30 * env.scale;
    const above = maxY + gap + 48 * env.scale > env.height;
    return { x: (minX + maxX) / 2, y: above ? minY - gap : maxY + gap, above };
  }
}
