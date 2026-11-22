// Stand-in for the engine when the overlay page runs in a plain browser.
//
// Used to look at the overlay without the app, and to hold any moment of any
// state still for a screenshot:
//
//   overlay.html                              draw with the mouse; releases alternate match / no match
//   overlay.html?demo=circle&loop=1           plays a hand-drawn circle and its match, on repeat
//   overlay.html?demo=z&state=match&at=150    one frozen frame, 150 ms after release
//   overlay.html?demo=v&state=drawing&at=300  one frozen frame, 300 ms after the trigger went down
//
// Other parameters: bg=black|white|desk, hz=125 (sample rate of the fake
// wand), size=560, x=, y= (centre, CSS px), result=match|nomatch|captured,
// mag=3 (magnify stroke widths to inspect joins).

import { BUILTINS } from '../shared/gestures';
import type { Monitor, OverlayCfg, OverlayMsg, StrokeResult } from '../shared/types';
import { hash, noise1 } from './path';
import type { Overlay } from './overlay';

type P = [number, number];

/** Every built-in gesture by id, drawn the first way it can be drawn. */
export const SHAPES: Record<string, { points: P[]; label: string; icon: string }> = Object.fromEntries(
  BUILTINS.map((b) => [b.id, { points: b.variants[0], label: b.label, icon: b.icon }]),
);

function lengthOf(p: P[]): number {
  let l = 0;
  for (let i = 1; i < p.length; i++) l += Math.hypot(p[i][0] - p[i - 1][0], p[i][1] - p[i - 1][1]);
  return l;
}

function resample(p: P[], n: number): P[] {
  const total = lengthOf(p);
  const out: P[] = [p[0]];
  let acc = 0;
  let j = 1;
  let prev = p[0];
  const step = total / (n - 1);
  while (j < p.length && out.length < n) {
    const d = Math.hypot(p[j][0] - prev[0], p[j][1] - prev[1]);
    if (d > 0 && acc + d >= step) {
      const t = (step - acc) / d;
      prev = [prev[0] + t * (p[j][0] - prev[0]), prev[1] + t * (p[j][1] - prev[1])];
      out.push(prev);
      acc = 0;
    } else {
      acc += d;
      prev = p[j];
      j++;
    }
  }
  while (out.length < n) out.push(p[p.length - 1]);
  return out;
}

const minJerk = (t: number) => t * t * t * (10 - 15 * t + 6 * t * t);

export interface Take {
  /** [ms since press, x, y] in device pixels. */
  samples: [number, number, number][];
  duration: number;
  to: number[];
  from: number[];
}

/** A plausibly human drawing of `shape`: tilted, wobbly, slow at the corners. */
export function perform(shape: P[], cx: number, cy: number, size: number, hz: number, seed: number): Take {
  const tilt = (hash(seed) - 0.5) * 0.2;
  const squash = 0.9 + hash(seed + 1) * 0.2;
  const [cs, sn] = [Math.cos(tilt), Math.sin(tilt)];
  // Time per straight run: longer runs take longer, sub-linearly.
  const runs = shape.length > 8 ? [shape] : shape.slice(1).map((p, i) => [shape[i], p] as P[]);
  const weights = runs.map((r) => Math.pow(lengthOf(r), 0.6));
  const wsum = weights.reduce((a, b) => a + b, 0);
  const duration = 330 + 190 * lengthOf(shape);
  const at = (ms: number): P => {
    let t = Math.min(1, Math.max(0, ms / duration));
    let r = 0;
    while (r < runs.length - 1 && t > weights[r] / wsum) {
      t -= weights[r] / wsum;
      r++;
    }
    const local = Math.min(1, t / (weights[r] / wsum));
    // Mostly minimum-jerk (rest at each corner), partly constant: a hand
    // slows for a corner but does not stop dead.
    const u = 0.72 * minJerk(local) + 0.28 * local;
    const run = runs[r];
    const dense = run.length > 2 ? run : resample(run, 24);
    const f = u * (dense.length - 1);
    const i = Math.min(dense.length - 2, Math.floor(f));
    const k = f - i;
    const x = dense[i][0] + (dense[i + 1][0] - dense[i][0]) * k;
    const y = dense[i][1] + (dense[i + 1][1] - dense[i][1]) * k;
    const prog = ms / duration;
    const wob = 0.034 * Math.min(1, prog * 5) * Math.min(1, (1 - prog) * 5 + 0.3);
    const wx = (noise1(prog * 4.3 + seed) - 0.5) * 2 * wob;
    const wy = (noise1(prog * 3.7 + seed + 40) - 0.5) * 2 * wob;
    const px = x * squash + wx;
    const py = y + wy;
    return [cx + (px * cs - py * sn) * size, cy + (px * sn + py * cs) * size];
  };
  const samples: [number, number, number][] = [];
  const step = 1000 / hz;
  for (let ms = 0; ms <= duration + 0.001; ms += step) {
    const [x, y] = at(ms);
    samples.push([ms, x, y]);
  }
  // What the engine would send back: the stroke at 64 points, and the clean
  // shape placed over it.
  const drawn = resample(samples.map(([, x, y]) => [x, y] as P), 64);
  const mx = drawn.reduce((a, p) => a + p[0], 0) / 64;
  const my = drawn.reduce((a, p) => a + p[1], 0) / 64;
  const ext = (pts: P[]) => {
    const xs = pts.map((p) => p[0]);
    const ys = pts.map((p) => p[1]);
    return Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys));
  };
  const clean = resample(shape.length > 8 ? shape : resample(shape, 200), 64);
  const ccx = clean.reduce((a, p) => a + p[0], 0) / 64;
  const ccy = clean.reduce((a, p) => a + p[1], 0) / 64;
  const scale = ext(drawn) / ext(clean);
  return {
    samples,
    duration,
    from: drawn.flat(),
    to: clean.flatMap((p) => [mx + (p[0] - ccx) * scale, my + (p[1] - ccy) * scale]),
  };
}

function resultFor(kind: string, take: Take, shape: { label: string; icon: string }, id: string): StrokeResult {
  if (kind === 'nomatch') return { kind: 'no_match', from: take.from };
  if (kind === 'captured') return { kind: 'captured', from: take.from };
  return { kind: 'match', gesture: id, label: shape.label, icon: shape.icon, score: 0.93, from: take.from, to: take.to };
}

export function startDemo(make: (monitor: Monitor, cfg: OverlayCfg) => Overlay, cfg: OverlayCfg): void {
  // A tab opened in the background can report a zero-sized viewport at first.
  if (innerWidth === 0 || innerHeight === 0) {
    setTimeout(() => startDemo(make, cfg), 50);
    return;
  }
  const q = new URLSearchParams(location.search);
  const dpr = window.devicePixelRatio || 1;
  // mag=3 draws every width three times as large on the same canvas: a
  // magnifying glass for joins, caps and edges.
  const mag = Number(q.get('mag') ?? 1);
  const monitor: Monitor = { x: 0, y: 0, w: Math.round(innerWidth * dpr), h: Math.round(innerHeight * dpr), scale: dpr * mag, primary: true };
  const bg = q.get('bg') ?? 'black';
  document.documentElement.dataset.bg = bg;
  const overlay = make(monitor, { ...cfg, confirm_ms: Number(q.get('confirm') ?? cfg.confirm_ms) });
  (window as unknown as { __overlay: Overlay }).__overlay = overlay;

  const id = q.get('demo');
  const shape = id ? SHAPES[id] : undefined;
  const resultKind = q.get('result') ?? (q.get('state') === 'nomatch' ? 'nomatch' : q.get('state') === 'captured' ? 'captured' : 'match');

  if (!id || !shape) {
    interactive(overlay, dpr, q.get('result'));
    return;
  }

  const size = Number(q.get('size') ?? 520) * dpr;
  const cx = Number(q.get('x') ?? innerWidth / 2) * dpr;
  const cy = Number(q.get('y') ?? innerHeight / 2) * dpr;
  const hz = Number(q.get('hz') ?? 125);
  const take = perform(shape.points, cx, cy, size, hz, Number(q.get('seed') ?? 3));
  const lead = Number(q.get('lead') ?? cfg.prediction_ms);

  const state = q.get('state');
  if (state) {
    // One frozen frame. Simulate from the press at 240 fps so sparks, tail
    // fade and prediction are exactly what a live run would show.
    const at = Number(q.get('at') ?? 0);
    const stop = state === 'drawing' ? Math.min(at, take.duration) : take.duration + at;
    let clock = 0;
    overlay.now = () => clock;
    overlay.handle([{ k: 'down', id: 1, t: 0, x: take.samples[0][1], y: take.samples[0][2], lead_ms: lead }]);
    let next = 1;
    let released = false;
    const tick = 1000 / 240;
    for (clock = 0; clock <= stop + 0.001; clock += tick) {
      const pts: number[] = [];
      while (next < take.samples.length && take.samples[next][0] <= clock) {
        pts.push(...take.samples[next]);
        next++;
      }
      if (pts.length) overlay.handle([{ k: 'move', id: 1, pts }]);
      if (!released && state !== 'drawing' && clock >= take.duration) {
        released = true;
        overlay.handle([{ k: 'up', id: 1, t: clock, result: resultFor(resultKind, take, shape, id) }]);
      }
      overlay.draw(clock);
    }
    overlay.labelLayer.clear();
    if (state === 'match') {
      const C = Number(q.get('confirm') ?? cfg.confirm_ms);
      const xs = take.to.filter((_, i) => i % 2 === 0);
      const ys = take.to.filter((_, i) => i % 2 === 1);
      const lx = (Math.min(...xs) + Math.max(...xs)) / 2 / dpr;
      const ly = (Math.max(...ys) + 30 * dpr) / dpr;
      // The chip's transition is 140 ms from 60 ms; show its settled states.
      const s = at < 60 * (C / 400) + 70 ? 'hidden' : at < C - 100 ? 'in' : 'out';
      overlay.labelLayer.freeze(lx, ly, false, shape.label, shape.icon, s);
    }
    // Keep presenting the frozen frame so a capture never lands on an empty buffer.
    const frozen = clock - tick;
    const hold = () => {
      overlay.draw(frozen);
      requestAnimationFrame(hold);
    };
    requestAnimationFrame(hold);
    document.documentElement.dataset.ready = 'true';
    return;
  }

  // Real-time playback, once or on a loop.
  const play = () => {
    const t0 = performance.now();
    let next = 1;
    const strokeId = Math.floor(t0) % 100000;
    overlay.handle([{ k: 'down', id: strokeId, t: t0, x: take.samples[0][1], y: take.samples[0][2], lead_ms: lead }]);
    const pump = () => {
      const el = performance.now() - t0;
      const pts: number[] = [];
      while (next < take.samples.length && take.samples[next][0] <= el) {
        pts.push(...take.samples[next]);
        next++;
      }
      if (pts.length) overlay.handle([{ k: 'move', id: strokeId, pts }]);
      if (next < take.samples.length) {
        setTimeout(pump, 1000 / hz);
      } else {
        overlay.handle([{ k: 'up', id: strokeId, t: t0 + el, result: resultFor(resultKind, take, shape, id) }]);
        if (q.get('loop')) setTimeout(play, 1400);
      }
    };
    pump();
  };
  setTimeout(play, 300);
}

/** Draw with the mouse. Releases alternate match and no match unless `result` pins one. */
function interactive(overlay: Overlay, dpr: number, pinned: string | null): void {
  let id = 0;
  let down = false;
  let t0 = 0;
  let pts: P[] = [];
  let flip = false;
  const send = (m: OverlayMsg) => overlay.handle([m]);
  window.addEventListener('pointerdown', (e) => {
    down = true;
    id++;
    t0 = performance.now();
    pts = [[e.clientX * dpr, e.clientY * dpr]];
    send({ k: 'down', id, t: t0, x: pts[0][0], y: pts[0][1], lead_ms: 4 });
  });
  window.addEventListener('pointermove', (e) => {
    if (!down) return;
    // Coalesced events give the full input rate instead of one per frame.
    const events = e.getCoalescedEvents?.() ?? [e];
    const flat: number[] = [];
    for (const ev of events.length ? events : [e]) {
      const p: P = [ev.clientX * dpr, ev.clientY * dpr];
      pts.push(p);
      flat.push(performance.now() - t0, p[0], p[1]);
    }
    send({ k: 'move', id, pts: flat });
  });
  window.addEventListener('pointerup', () => {
    if (!down) return;
    down = false;
    if (lengthOf(pts) < 60 * dpr) {
      send({ k: 'cancel', id });
      return;
    }
    flip = !flip;
    const kind = pinned ?? (flip ? 'match' : 'nomatch');
    const from = resample(pts, 64);
    // With no recogniser here, "the clean shape" is the stroke's own chord
    // smoothed: enough to judge the morph by eye.
    const to = from.map((_, i) => {
      const a = Math.max(0, i - 6);
      const b = Math.min(63, i + 6);
      let x = 0;
      let y = 0;
      for (let j = a; j <= b; j++) {
        x += from[j][0];
        y += from[j][1];
      }
      return [x / (b - a + 1), y / (b - a + 1)];
    });
    const result: StrokeResult =
      kind === 'nomatch'
        ? { kind: 'no_match', from: from.flat() }
        : kind === 'captured'
          ? { kind: 'captured', from: from.flat() }
          : { kind: 'match', gesture: 'demo', label: 'Screenshot', icon: 'camera', score: 0.9, from: from.flat(), to: to.flat() };
    send({ k: 'up', id, t: performance.now(), result });
  });
}
