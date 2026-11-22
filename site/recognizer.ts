// The app's stroke recogniser, ported to the browser so the page can match
// what a visitor draws against the real templates.
//
// A line-for-line port of src-tauri/src/recognizer.rs with the default
// thresholds from wand.toml; the reasoning behind each step is written there.

import type { Builtin } from '../src/shared/gestures';
import type { Pt } from '../src/shared/types';

const N = 64;
const MIN_SCORE = 0.8;
const MIN_MARGIN = 0.04;
const ROTATION_TOLERANCE = (12 * Math.PI) / 180;
const SCALE_TOLERANCE = 0.2;
const ELASTIC_WINDOW = 0.1;
const SECOND_CHANCE_EXTRA = 0.03;
const LINE_THINNESS = 0.3;
const MAX_STRETCH = 2.5;
const ZERO_SCORE_DISTANCE = 0.5;
const DENSE = 96;

const dist = (a: Pt, b: Pt) => Math.hypot(a[0] - b[0], a[1] - b[1]);

export function pathLength(pts: Pt[]): number {
  let l = 0;
  for (let i = 1; i < pts.length; i++) l += dist(pts[i - 1], pts[i]);
  return l;
}

export function resample(pts: Pt[], n: number): Pt[] {
  if (pts.length === 0) return Array.from({ length: n }, () => [0, 0] as Pt);
  const total = pathLength(pts);
  if (total <= 1e-9 || pts.length === 1) return Array.from({ length: n }, () => [pts[0][0], pts[0][1]] as Pt);
  const step = total / (n - 1);
  const out: Pt[] = [pts[0]];
  let acc = 0;
  let prev = pts[0];
  let i = 1;
  while (i < pts.length && out.length < n) {
    const cur = pts[i];
    const d = dist(prev, cur);
    if (d > 0 && acc + d >= step) {
      const t = (step - acc) / d;
      const q: Pt = [prev[0] + t * (cur[0] - prev[0]), prev[1] + t * (cur[1] - prev[1])];
      out.push(q);
      prev = q;
      acc = 0;
    } else {
      acc += d;
      prev = cur;
      i++;
    }
  }
  while (out.length < n) out.push(pts[pts.length - 1]);
  return out;
}

function centroid(pts: Pt[]): Pt {
  let x = 0;
  let y = 0;
  for (const p of pts) {
    x += p[0];
    y += p[1];
  }
  return [x / pts.length, y / pts.length];
}

function bounds(pts: Pt[]): [number, number] {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const p of pts) {
    x0 = Math.min(x0, p[0]);
    y0 = Math.min(y0, p[1]);
    x1 = Math.max(x1, p[0]);
    y1 = Math.max(y1, p[1]);
  }
  return [Math.max(x1 - x0, 0), Math.max(y1 - y0, 0)];
}

const extent = (pts: Pt[]) => Math.max(...bounds(pts));

/** 0 for a straight line at any angle, 1 for a circle. */
function thinness(pts: Pt[]): number {
  const c = centroid(pts);
  let sxx = 0;
  let syy = 0;
  let sxy = 0;
  for (const p of pts) {
    const x = p[0] - c[0];
    const y = p[1] - c[1];
    sxx += x * x;
    syy += y * y;
    sxy += x * y;
  }
  const half = (sxx + syy) / 2;
  const disc = Math.sqrt(Math.max(half * half - (sxx * syy - sxy * sxy), 0));
  const major = half + disc;
  return major <= 1e-12 ? 0 : Math.sqrt(Math.max(half - disc, 0) / major);
}

interface Normalized {
  points: Pt[];
  centroid: Pt;
  scale: [number, number];
  line: boolean;
}

function normalized(pts: Pt[], stretch: boolean): Normalized {
  const r = resample(pts, N);
  const c = centroid(r);
  const line = thinness(r) < LINE_THINNESS;
  const [w, h] = bounds(r);
  const big = Math.max(w, h, 1e-6);
  const scale: [number, number] = stretch && !line ? [Math.max(w, big / MAX_STRETCH), Math.max(h, big / MAX_STRETCH)] : [big, big];
  return { points: r.map((p) => [(p[0] - c[0]) / scale[0], (p[1] - c[1]) / scale[1]] as Pt), centroid: c, scale, line };
}

interface Fit {
  score: number;
  rotation: number;
}

/** RMS distance along the cheapest monotonic pairing within `band` of the diagonal. */
function elasticRms(a: Pt[], b: Pt[], band: number): number {
  const n = a.length;
  const m = b.length;
  const INF = 1e30;
  let prevCost = new Float64Array(m).fill(INF);
  let prevSteps = new Int32Array(m);
  let curCost = new Float64Array(m);
  let curSteps = new Int32Array(m);
  for (let i = 0; i < n; i++) {
    const centre = n > 1 ? Math.floor((i * (m - 1)) / (n - 1)) : 0;
    const lo = Math.max(0, centre - band);
    const hi = Math.min(m - 1, centre + band);
    curCost.fill(INF);
    for (let j = lo; j <= hi; j++) {
      const d = (a[i][0] - b[j][0]) ** 2 + (a[i][1] - b[j][1]) ** 2;
      let best = i === 0 && j === 0 ? 0 : INF;
      let steps = 0;
      if (i > 0 && prevCost[j] < best) {
        best = prevCost[j];
        steps = prevSteps[j];
      }
      if (j > 0 && curCost[j - 1] < best) {
        best = curCost[j - 1];
        steps = curSteps[j - 1];
      }
      if (i > 0 && j > 0 && prevCost[j - 1] < best) {
        best = prevCost[j - 1];
        steps = prevSteps[j - 1];
      }
      if (best < INF) {
        curCost[j] = best + d;
        curSteps[j] = steps + 1;
      }
    }
    [prevCost, curCost] = [curCost, prevCost];
    [prevSteps, curSteps] = [curSteps, prevSteps];
  }
  return Math.sqrt(prevCost[m - 1] / Math.max(prevSteps[m - 1], 1));
}

function fit(stroke: Pt[], template: Pt[], maxRotation: number): Fit {
  const n = Math.min(stroke.length, template.length);
  let dot = 0;
  let cross = 0;
  let ss = 0;
  for (let i = 0; i < n; i++) {
    const c = stroke[i];
    const t = template[i];
    dot += c[0] * t[0] + c[1] * t[1];
    cross += c[0] * t[1] - c[1] * t[0];
    ss += c[0] * c[0] + c[1] * c[1];
  }
  const rotation = Math.min(maxRotation, Math.max(-maxRotation, Math.atan2(cross, dot)));
  const sin = Math.sin(rotation);
  const cos = Math.cos(rotation);
  const scale = ss > 1e-9 ? Math.min(1 + SCALE_TOLERANCE, Math.max(1 - SCALE_TOLERANCE, (dot * cos + cross * sin) / ss)) : 1;
  const aligned = stroke.slice(0, n).map((c) => [(c[0] * cos - c[1] * sin) * scale, (c[0] * sin + c[1] * cos) * scale] as Pt);
  const rms = elasticRms(aligned, template.slice(0, n), Math.round(ELASTIC_WINDOW * n));
  return { score: Math.max(0, 1 - rms / ZERO_SCORE_DISTANCE), rotation };
}

interface Template {
  gesture: number;
  variant: number;
  free: boolean;
  line: boolean;
  aspect: [number, number];
  points: Pt[];
}

export interface Match {
  gesture: Builtin;
  score: number;
  runnerUp: number;
  /** The stroke at 64 points and the clean shape laid over it, both flat [x, y, ...]. */
  from: number[];
  to: number[];
}

export type Verdict = { kind: 'match'; match: Match } | { kind: 'no_match'; closest: Builtin | null; score: number; from: number[] };

interface Candidate {
  template: Template;
  fit: Fit;
  runnerUp: number;
  centroid: Pt;
  size: number;
}

export class Recognizer {
  private templates: Template[] = [];

  constructor(private gestures: Builtin[]) {
    gestures.forEach((g, gi) =>
      g.variants.forEach((shape, vi) => {
        const [w, h] = bounds(shape);
        const big = Math.max(w, h, 1e-6);
        const norm = normalized(shape, true);
        this.templates.push({ gesture: gi, variant: vi, free: g.free, line: norm.line, aspect: [w / big, h / big], points: norm.points });
      }),
    );
  }

  private best(points: Pt[]): Candidate | null {
    const uniform = normalized(points, false);
    // A line is never stretched: its sag would become a shape of its own.
    const boxed = uniform.line ? null : normalized(points, true);
    let best: Candidate | null = null;
    const perGesture = new Map<number, number>();
    for (const t of this.templates) {
      const stroke = t.line ? uniform : boxed;
      if (!stroke) continue;
      const f = fit(stroke.points, t.points, t.free ? Math.PI : ROTATION_TOLERANCE);
      perGesture.set(t.gesture, Math.max(perGesture.get(t.gesture) ?? 0, f.score));
      if (!best || f.score > best.fit.score) {
        const size = t.line ? stroke.scale[0] : 0.5 * (stroke.scale[0] / Math.max(t.aspect[0], 0.25) + stroke.scale[1] / Math.max(t.aspect[1], 0.25));
        best = { template: t, fit: f, runnerUp: 0, centroid: stroke.centroid, size };
      }
    }
    if (!best) return null;
    for (const [g, s] of perGesture) if (g !== best.template.gesture) best.runnerUp = Math.max(best.runnerUp, s);
    return best;
  }

  recognize(points: Pt[]): Verdict {
    const from = resample(points, N).flat();
    const accept = (c: Candidate, extra: number) => c.fit.score >= MIN_SCORE + extra && c.fit.score - c.runnerUp >= MIN_MARGIN;
    const full = this.best(points);
    let winner = full && accept(full, 0) ? full : null;
    if (!winner) {
      // A second chance without what a hand adds around a shape.
      for (const variant of cleaned(points)) {
        const c = this.best(variant);
        if (c && accept(c, SECOND_CHANCE_EXTRA) && (!winner || c.fit.score > winner.fit.score)) winner = c;
      }
    }
    if (!winner) {
      return { kind: 'no_match', closest: full ? this.gestures[full.template.gesture] : null, score: full?.fit.score ?? 0, from };
    }
    const gesture = this.gestures[winner.template.gesture];
    // The clean shape, where the stroke was. A free-rotation shape is turned
    // to meet the stroke so the morph is a short trip.
    const ideal = normalized(gesture.variants[winner.template.variant], false).points;
    const turn = gesture.free ? -winner.fit.rotation : 0;
    const sin = Math.sin(turn);
    const cos = Math.cos(turn);
    const to = ideal.flatMap((p) => [winner.centroid[0] + (p[0] * cos - p[1] * sin) * winner.size, winner.centroid[1] + (p[0] * sin + p[1] * cos) * winner.size]);
    return { kind: 'match', match: { gesture, score: winner.fit.score, runnerUp: winner.runnerUp, from, to } };
  }
}

/** The stroke's closed loop alone, or with a hook cut from either end. */
function cleaned(points: Pt[]): Pt[][] {
  const d = resample(points, DENSE);
  const out: Pt[][] = [];
  const loop = closedLoop(d);
  if (loop) out.push(d.slice(loop[0], loop[1] + 1));
  const start = sharpestTurn(d, 3, Math.floor(d.length / 5));
  const end = sharpestTurn(d, d.length - 1 - Math.floor(d.length / 5), d.length - 4);
  if (start !== null) out.push(d.slice(start));
  if (end !== null) out.push(d.slice(0, end + 1));
  if (start !== null && end !== null) out.push(d.slice(start, end + 1));
  return out;
}

function closedLoop(d: Pt[]): [number, number] | null {
  const n = d.length;
  const close = extent(d) * 0.14;
  for (let span = n - 1; span >= Math.floor((n * 2) / 5); span--) {
    let best = -1;
    let bestGap = Infinity;
    for (let i = 0; i < n - span; i++) {
      const gap = dist(d[i], d[i + span]);
      if (gap <= close && gap < bestGap) {
        best = i;
        bestGap = gap;
      }
    }
    if (best >= 0) return span >= n - 4 ? null : [best, best + span];
  }
  return null;
}

function sharpestTurn(d: Pt[], from: number, to: number): number | null {
  const ARM = 3;
  let best: number | null = null;
  let bestAngle = (60 * Math.PI) / 180;
  for (let k = from; k <= to; k++) {
    if (k < ARM || k + ARM >= d.length) continue;
    const ax = d[k][0] - d[k - ARM][0];
    const ay = d[k][1] - d[k - ARM][1];
    const bx = d[k + ARM][0] - d[k][0];
    const by = d[k + ARM][1] - d[k][1];
    const la = Math.hypot(ax, ay);
    const lb = Math.hypot(bx, by);
    if (la < 1e-6 || lb < 1e-6) continue;
    const angle = Math.acos(Math.min(1, Math.max(-1, (ax * bx + ay * by) / (la * lb))));
    if (angle > bestAngle) {
      best = k;
      bestAngle = angle;
    }
  }
  return best;
}
