// Drawing gesture shapes on 2D canvases: the looping demonstration on each
// card, and the overlaid samples in the recording flow.
//
// Every card loops, as asked, so the loop is kept calm: one shared frame
// callback for all cards, each card on its own offset so nothing pulses in
// unison, nothing running while the window is hidden, and under reduced
// motion the finished shape is simply shown.

import { clamp01, easeInOut, easeOut } from '../shared/ease';
import type { Pt } from '../shared/types';

const CORAL = [255, 87, 87];
const HOT = [255, 236, 210];

/** Centre a polyline and scale its longest side to 1. */
export function normalize(points: Pt[]): Pt[] {
  if (points.length === 0) return [];
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const [x, y] of points) {
    minX = Math.min(minX, x);
    maxX = Math.max(maxX, x);
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  }
  const extent = Math.max(maxX - minX, maxY - minY, 1e-6);
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  return points.map(([x, y]) => [(x - cx) / extent, (y - cy) / extent]);
}

function cumulative(points: Pt[]): number[] {
  const out = [0];
  for (let i = 1; i < points.length; i++) {
    out.push(out[i - 1] + Math.hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
  }
  return out;
}

/** `n` points evenly spaced along the polyline. */
export function resample(points: Pt[], n: number): Pt[] {
  if (points.length < 2) return points.slice();
  const cum = cumulative(points);
  const total = cum[cum.length - 1];
  const out: Pt[] = [];
  let j = 0;
  for (let i = 0; i < n; i++) {
    const target = (total * i) / (n - 1);
    while (j < points.length - 2 && cum[j + 1] < target) j++;
    const span = cum[j + 1] - cum[j];
    const t = span > 1e-9 ? clamp01((target - cum[j]) / span) : 0;
    out.push([points[j][0] + (points[j + 1][0] - points[j][0]) * t, points[j][1] + (points[j + 1][1] - points[j][1]) * t]);
  }
  return out;
}

export function fitCanvas(canvas: HTMLCanvasElement): { ctx: CanvasRenderingContext2D; w: number; h: number; dpr: number } | null {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (w === 0 || h === 0) return null;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
  }
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { ctx, w, h, dpr };
}

interface Draw {
  /** Fraction of the path drawn so far, 0..1. */
  progress: number;
  /** Overall opacity. */
  alpha: number;
  /** Show the ember at the leading end. */
  ember: boolean;
  /** 0..1: how lit the stroke is (hovered cards burn brighter). */
  heat: number;
  width: number;
}

/**
 * Draw a normalized shape into a `size` box centred at (cx, cy). The stroke
 * is thin at both ends and at corners is left alone: on a card the shape must
 * read at a glance.
 */
export function drawShape(ctx: CanvasRenderingContext2D, shape: Pt[], cx: number, cy: number, size: number, d: Draw): void {
  const n = shape.length;
  if (n < 2 || d.alpha <= 0.003) return;
  const upto = Math.max(1, Math.min(n - 1, d.progress * (n - 1)));
  const last = Math.floor(upto);
  const frac = upto - last;
  const px = (i: number) => cx + shape[i][0] * size;
  const py = (i: number) => cy + shape[i][1] * size;
  const closed = Math.hypot(shape[0][0] - shape[n - 1][0], shape[0][1] - shape[n - 1][1]) < 0.12;

  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';

  // Soft halo: two wide, faint passes. No canvas shadows (they repaint slowly).
  const halo = (width: number, alpha: number) => {
    ctx.beginPath();
    ctx.moveTo(px(0), py(0));
    for (let i = 1; i <= last; i++) ctx.lineTo(px(i), py(i));
    if (frac > 0 && last < n - 1) ctx.lineTo(px(last) + (px(last + 1) - px(last)) * frac, py(last) + (py(last + 1) - py(last)) * frac);
    ctx.strokeStyle = `rgba(255, 64, 40, ${alpha * d.alpha})`;
    ctx.lineWidth = width;
    ctx.stroke();
  };
  halo(d.width * 5.5, 0.05 + 0.05 * d.heat);
  halo(d.width * 2.6, 0.1 + 0.1 * d.heat);

  // Core: short segments so width and warmth can vary along the path. Butt
  // caps with a hair of overlap: round caps would stack where segments meet
  // and bead the line whenever it is drawn translucent.
  ctx.lineCap = 'butt';
  const head = upto / (n - 1);
  for (let i = 0; i < last + (frac > 0 ? 1 : 0) && i < n - 1; i++) {
    const u = (i + 0.5) / (n - 1);
    const t = i === last ? frac : 1;
    const x0 = px(i);
    const y0 = py(i);
    let x1 = x0 + (px(i + 1) - x0) * t;
    let y1 = y0 + (py(i + 1) - y0) * t;
    const seg = Math.hypot(x1 - x0, y1 - y0) || 1;
    x1 += ((x1 - x0) / seg) * 0.5;
    y1 += ((y1 - y0) / seg) * 0.5;
    const taper = closed ? 1 : 0.42 + 0.58 * Math.sqrt(Math.sin(Math.PI * clamp01(u)));
    // Warm near the ember while drawing, plain coral once the shape is down.
    const near = d.ember ? clamp01(1 - (head - u) * 5) : 0;
    const warm = Math.max(near, d.heat * 0.25);
    const c = CORAL.map((v, k) => Math.round(v + (HOT[k] - v) * warm));
    ctx.beginPath();
    ctx.moveTo(x0, y0);
    ctx.lineTo(x1, y1);
    ctx.strokeStyle = `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${(0.78 + 0.22 * Math.max(near, d.heat)) * d.alpha})`;
    ctx.lineWidth = d.width * taper * (1 + 0.25 * near);
    ctx.stroke();
  }

  if (d.ember && d.progress < 1) {
    const hx = last < n - 1 ? px(last) + (px(last + 1) - px(last)) * frac : px(n - 1);
    const hy = last < n - 1 ? py(last) + (py(last + 1) - py(last)) * frac : py(n - 1);
    const r = d.width * 4.5;
    const g = ctx.createRadialGradient(hx, hy, 0, hx, hy, r);
    g.addColorStop(0, `rgba(255, 244, 228, ${d.alpha})`);
    g.addColorStop(0.22, `rgba(255, 120, 80, ${0.75 * d.alpha})`);
    g.addColorStop(1, 'rgba(255, 64, 40, 0)');
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.arc(hx, hy, r, 0, Math.PI * 2);
    ctx.fill();
  }
}

// ------------------------------------------------------------ the shared loop

type Tick = (now: number) => void;
const ticks = new Set<Tick>();
let raf = 0;
let lastRun = 0;

function run(now: number): void {
  raf = 0;
  // 60 updates a second is plenty for a card; on a 240 Hz display the other
  // three frames out of four are skipped instead of repainting every canvas.
  if (now - lastRun >= 15) {
    lastRun = now;
    for (const tick of ticks) tick(now);
  }
  schedule();
}

function schedule(): void {
  if (!raf && ticks.size > 0 && !document.hidden) raf = requestAnimationFrame(run);
}

document.addEventListener('visibilitychange', schedule);

export function onFrame(tick: Tick): () => void {
  ticks.add(tick);
  schedule();
  return () => {
    ticks.delete(tick);
  };
}

export const reducedMotion = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

const DRAW_MS = 950;
const HOLD_MS = 1500;
const FADE_MS = 360;
const REST_MS = 520;
const CYCLE_MS = DRAW_MS + HOLD_MS + FADE_MS + REST_MS;

/**
 * One frame of a card's loop: the gesture draws itself, holds, fades, rests.
 * `offset` staggers cards so they never move together.
 */
const drawn = new WeakMap<HTMLCanvasElement, string>();

export function drawCardFrame(canvas: HTMLCanvasElement, shape: Pt[], now: number, offset: number, hovered: boolean): void {
  // Loops run while the window has focus or the card is under the pointer.
  // In the background every card simply shows its finished shape.
  const live = !reducedMotion() && (document.hasFocus() || hovered);
  const t = (now + offset) % CYCLE_MS;
  // Most of a cycle is holding or resting. Skip the repaint when this frame
  // would look exactly like the last one.
  const phase = !live ? 'still' : t < DRAW_MS ? `draw${Math.round(t)}` : t < DRAW_MS + HOLD_MS ? 'hold' : t < DRAW_MS + HOLD_MS + FADE_MS ? `fade${Math.round(t)}` : 'rest';
  const key = `${phase}|${hovered}|${canvas.clientWidth}x${canvas.clientHeight}|${shape.length}`;
  if (drawn.get(canvas) === key) return;
  drawn.set(canvas, key);
  const fit = fitCanvas(canvas);
  if (!fit) return;
  const { ctx, w, h } = fit;
  ctx.clearRect(0, 0, w, h);
  const size = Math.min(w, h) * 0.6;
  const width = Math.max(2, Math.min(w, h) * 0.02);
  const heat = hovered ? 1 : 0;
  // The shape itself never leaves the card: a quiet trace is always there to
  // read, and the lit stroke draws over it, holds, and fades back to it.
  trace(ctx, shape, w / 2, h / 2, size, width);
  if (!live) {
    drawShape(ctx, shape, w / 2, h / 2, size, { progress: 1, alpha: 1, ember: false, heat, width });
    return;
  }
  if (t < DRAW_MS) {
    // On-screen movement that demonstrates a path: ease-in-out.
    drawShape(ctx, shape, w / 2, h / 2, size, { progress: easeInOut(t / DRAW_MS), alpha: 1, ember: true, heat, width });
  } else if (t < DRAW_MS + HOLD_MS) {
    drawShape(ctx, shape, w / 2, h / 2, size, { progress: 1, alpha: 1, ember: false, heat, width });
  } else if (t < DRAW_MS + HOLD_MS + FADE_MS) {
    const out = easeOut((t - DRAW_MS - HOLD_MS) / FADE_MS);
    drawShape(ctx, shape, w / 2, h / 2, size, { progress: 1, alpha: 1 - out, ember: false, heat, width });
  }
}

function trace(ctx: CanvasRenderingContext2D, shape: Pt[], cx: number, cy: number, size: number, width: number): void {
  if (shape.length < 2) return;
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  ctx.beginPath();
  shape.forEach(([x, y], i) => (i ? ctx.lineTo(cx + x * size, cy + y * size) : ctx.moveTo(cx + x * size, cy + y * size)));
  ctx.strokeStyle = 'rgba(244, 243, 240, 0.16)';
  ctx.lineWidth = width * 0.7;
  ctx.stroke();
}

/** Recorded samples laid over each other, the newest brightest. */
export function drawSamples(canvas: HTMLCanvasElement, samples: Pt[][], ghost: Pt[] | null): void {
  const fit = fitCanvas(canvas);
  if (!fit) return;
  const { ctx, w, h } = fit;
  ctx.clearRect(0, 0, w, h);
  const size = Math.min(w, h) * 0.62;
  const width = Math.max(2, Math.min(w, h) * 0.012);
  if (ghost) {
    // The shape being re-recorded, as a faint guide.
    ctx.setLineDash([3, 7]);
    const g = resample(normalize(ghost), 96);
    ctx.beginPath();
    g.forEach(([x, y], i) => (i ? ctx.lineTo(w / 2 + x * size, h / 2 + y * size) : ctx.moveTo(w / 2 + x * size, h / 2 + y * size)));
    ctx.strokeStyle = 'rgba(244, 243, 240, 0.22)';
    ctx.lineWidth = 1.5;
    ctx.stroke();
    ctx.setLineDash([]);
  }
  samples.forEach((s, i) => {
    const shape = resample(normalize(s), 96);
    const newest = i === samples.length - 1;
    drawShape(ctx, shape, w / 2, h / 2, size, { progress: 1, alpha: newest ? 1 : 0.5, ember: false, heat: newest ? 0.6 : 0, width });
  });
}

export function drawThumb(canvas: HTMLCanvasElement, sample: Pt[]): void {
  const fit = fitCanvas(canvas);
  if (!fit) return;
  const { ctx, w, h } = fit;
  ctx.clearRect(0, 0, w, h);
  drawShape(ctx, resample(normalize(sample), 64), w / 2, h / 2, Math.min(w, h) * 0.66, { progress: 1, alpha: 1, ember: false, heat: 0, width: 1.8 });
}
