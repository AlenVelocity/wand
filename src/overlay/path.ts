// Polyline helpers: Catmull-Rom smoothing and arc-length resampling.

export function catmull(p0: number, p1: number, p2: number, p3: number, t: number): number {
  const t2 = t * t;
  const t3 = t2 * t;
  return 0.5 * (2 * p1 + (p2 - p0) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 + (3 * p1 - p0 - 3 * p2 + p3) * t3);
}

/** Cumulative arc length of the first `n` points; returns the total. */
export function arcLengths(xs: ArrayLike<number>, ys: ArrayLike<number>, n: number, out: Float32Array): number {
  let acc = 0;
  out[0] = 0;
  for (let i = 1; i < n; i++) {
    acc += Math.hypot(xs[i] - xs[i - 1], ys[i] - ys[i - 1]);
    out[i] = acc;
  }
  return acc;
}

/**
 * `m` values of `values` at evenly spaced arc-length fractions along a path
 * whose cumulative lengths are `cum`. Linear between neighbours.
 */
export function resampleByArc(values: ArrayLike<number>, cum: Float32Array, n: number, m: number, out: Float32Array): void {
  const total = cum[n - 1];
  let j = 0;
  for (let i = 0; i < m; i++) {
    const target = total * (i / (m - 1));
    while (j < n - 2 && cum[j + 1] < target) j++;
    const span = cum[j + 1] - cum[j];
    const t = span > 1e-6 ? Math.min(1, Math.max(0, (target - cum[j]) / span)) : 0;
    out[i] = values[j] + (values[j + 1] - values[j]) * t;
  }
}

/**
 * Smooth a flat [x0, y0, x1, y1, ...] polyline with Catmull-Rom and return
 * `m` points evenly spaced along it.
 */
export function smoothResample(flat: ArrayLike<number>, m: number): { xs: Float32Array<ArrayBuffer>; ys: Float32Array<ArrayBuffer> } {
  const n = flat.length >> 1;
  const xs = new Float32Array(m);
  const ys = new Float32Array(m);
  if (n === 0) return { xs, ys };
  if (n === 1) {
    xs.fill(flat[0]);
    ys.fill(flat[1]);
    return { xs, ys };
  }
  const SUB = 4;
  const dn = (n - 1) * SUB + 1;
  const dx = new Float32Array(dn);
  const dy = new Float32Array(dn);
  const at = (i: number, c: number) => flat[Math.min(n - 1, Math.max(0, i)) * 2 + c];
  let k = 0;
  for (let i = 0; i < n - 1; i++) {
    for (let s = 0; s < SUB; s++) {
      const t = s / SUB;
      dx[k] = catmull(at(i - 1, 0), at(i, 0), at(i + 1, 0), at(i + 2, 0), t);
      dy[k] = catmull(at(i - 1, 1), at(i, 1), at(i + 1, 1), at(i + 2, 1), t);
      k++;
    }
  }
  dx[k] = at(n - 1, 0);
  dy[k] = at(n - 1, 1);
  const cum = new Float32Array(dn);
  arcLengths(dx, dy, dn, cum);
  resampleByArc(dx, cum, dn, m, xs);
  resampleByArc(dy, cum, dn, m, ys);
  return { xs, ys };
}

/** Smooth value noise in one dimension, 0..1. Deterministic. */
export function noise1(x: number): number {
  const i = Math.floor(x);
  const f = x - i;
  const u = f * f * (3 - 2 * f);
  return hash(i) * (1 - u) + hash(i + 1) * u;
}

export function hash(n: number): number {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453123;
  return s - Math.floor(s);
}

/** `m` points evenly spaced along a flat [x0, y0, ...] polyline, corners kept sharp. */
export function linearResample(flat: ArrayLike<number>, m: number): { xs: Float32Array<ArrayBuffer>; ys: Float32Array<ArrayBuffer> } {
  const n = flat.length >> 1;
  const xs = new Float32Array(m);
  const ys = new Float32Array(m);
  if (n < 2) {
    xs.fill(n ? flat[0] : 0);
    ys.fill(n ? flat[1] : 0);
    return { xs, ys };
  }
  const sx = new Float32Array(n);
  const sy = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    sx[i] = flat[i * 2];
    sy[i] = flat[i * 2 + 1];
  }
  const cum = new Float32Array(n);
  arcLengths(sx, sy, n, cum);
  resampleByArc(sx, cum, n, m, xs);
  resampleByArc(sy, cum, n, m, ys);
  return { xs, ys };
}
