// Vertex builder for the light field.
//
// Every vertex carries where it is on screen and where it is relative to the
// thing it belongs to (`local`, in pixels). The fragment shader turns
// length(local) into a core and a glow, so one shader draws ribbons (local =
// distance across the stroke), round sprites (local = offset from the centre)
// and capsules (both).

export const STRIDE = 8; // x, y, lx, ly, half, glow, intensity, heat
/** The glow is cut off this many glow-radii out; the shader fades to exactly zero there. */
export const GLOW_REACH = 2.6;

export class Geometry {
  data: Float32Array;
  floats = 0;
  minX = Infinity;
  minY = Infinity;
  maxX = -Infinity;
  maxY = -Infinity;

  constructor(vertices = 40_000) {
    this.data = new Float32Array(vertices * STRIDE);
  }

  reset(): void {
    this.floats = 0;
    this.minX = this.minY = Infinity;
    this.maxX = this.maxY = -Infinity;
  }

  get vertices(): number {
    return this.floats / STRIDE;
  }

  private room(vertices: number): void {
    const need = this.floats + vertices * STRIDE;
    if (need <= this.data.length) return;
    const bigger = new Float32Array(Math.max(need, this.data.length * 2));
    bigger.set(this.data.subarray(0, this.floats));
    this.data = bigger;
  }

  private v(x: number, y: number, lx: number, ly: number, half: number, glow: number, intensity: number, heat: number): void {
    const d = this.data;
    let i = this.floats;
    d[i++] = x;
    d[i++] = y;
    d[i++] = lx;
    d[i++] = ly;
    d[i++] = half;
    d[i++] = glow;
    d[i++] = intensity;
    d[i++] = heat;
    this.floats = i;
    if (x < this.minX) this.minX = x;
    if (x > this.maxX) this.maxX = x;
    if (y < this.minY) this.minY = y;
    if (y > this.maxY) this.maxY = y;
  }

  /** A round point of light: the ember, a stroke's end, a sharp corner. */
  sprite(x: number, y: number, half: number, glow: number, intensity: number, heat: number): void {
    if (intensity <= 0.002) return;
    this.room(6);
    const e = half + glow * GLOW_REACH + 1.5;
    this.v(x - e, y - e, -e, -e, half, glow, intensity, heat);
    this.v(x + e, y - e, e, -e, half, glow, intensity, heat);
    this.v(x - e, y + e, -e, e, half, glow, intensity, heat);
    this.v(x + e, y - e, e, -e, half, glow, intensity, heat);
    this.v(x + e, y + e, e, e, half, glow, intensity, heat);
    this.v(x - e, y + e, -e, e, half, glow, intensity, heat);
  }

  /** A straight segment with round ends: sparks and crumbs. */
  capsule(x0: number, y0: number, x1: number, y1: number, half: number, glow: number, intensity: number, heat: number): void {
    if (intensity <= 0.002) return;
    let tx = x1 - x0;
    let ty = y1 - y0;
    const len = Math.hypot(tx, ty);
    if (len < 0.01) {
      this.sprite(x0, y0, half, glow, intensity, heat);
      return;
    }
    tx /= len;
    ty /= len;
    const nx = -ty;
    const ny = tx;
    const e = half + glow * GLOW_REACH + 1.5;
    this.room(18);
    // Along the segment local.y is 0; past each end it grows, which rounds the cap.
    const strip = (ax: number, ay: number, al: number, bx: number, by: number, bl: number) => {
      this.v(ax + nx * e, ay + ny * e, e, al, half, glow, intensity, heat);
      this.v(ax - nx * e, ay - ny * e, -e, al, half, glow, intensity, heat);
      this.v(bx + nx * e, by + ny * e, e, bl, half, glow, intensity, heat);
      this.v(ax - nx * e, ay - ny * e, -e, al, half, glow, intensity, heat);
      this.v(bx - nx * e, by - ny * e, -e, bl, half, glow, intensity, heat);
      this.v(bx + nx * e, by + ny * e, e, bl, half, glow, intensity, heat);
    };
    strip(x0 - tx * e, y0 - ty * e, e, x0, y0, 0);
    strip(x0, y0, 0, x1, y1, 0);
    strip(x1, y1, 0, x1 + tx * e, y1 + ty * e, e);
  }

  /**
   * A ribbon through `n` points with per-point half-width, glow radius,
   * intensity and heat.
   *
   * Where the path bends gently, neighbouring segments share a cross-section
   * (the bisector), so the ribbon is continuous. Where it turns sharply, each
   * segment ends square to its own direction and a round sprite fills the
   * join. A shared bisector there would shear the core to a sliver and leave a
   * hook at every corner of a Z or a V.
   */
  ribbon(
    n: number,
    xs: Float32Array,
    ys: Float32Array,
    halfs: Float32Array,
    glows: Float32Array,
    ints: Float32Array,
    heats: Float32Array,
  ): void {
    if (n < 2) return;
    const SHARP = 0.92; // cosine of about 23 degrees
    let pdx = 0;
    let pdy = 0;
    let havePrev = false;
    for (let i = 1; i < n; i++) {
      const j = i - 1;
      let dx = xs[i] - xs[j];
      let dy = ys[i] - ys[j];
      const len = Math.hypot(dx, dy);
      if (len < 1e-4) continue;
      dx /= len;
      dy /= len;

      // Cross-section direction at the start of this segment.
      let sx = dx;
      let sy = dy;
      if (havePrev && pdx * dx + pdy * dy >= SHARP) {
        sx = pdx + dx;
        sy = pdy + dy;
        const l = Math.hypot(sx, sy);
        sx /= l;
        sy /= l;
      }
      // And at its end, looking at the next segment.
      let ex = dx;
      let ey = dy;
      if (i < n - 1) {
        let ndx = xs[i + 1] - xs[i];
        let ndy = ys[i + 1] - ys[i];
        const nl = Math.hypot(ndx, ndy);
        if (nl >= 1e-4) {
          ndx /= nl;
          ndy /= nl;
          if (dx * ndx + dy * ndy >= SHARP) {
            ex = dx + ndx;
            ey = dy + ndy;
            const l = Math.hypot(ex, ey);
            ex /= l;
            ey /= l;
          } else {
            this.sprite(xs[i], ys[i], halfs[i], glows[i], ints[i], heats[i]);
          }
        }
      }

      const e0 = halfs[j] + glows[j] * GLOW_REACH + 1.5;
      const e1 = halfs[i] + glows[i] * GLOW_REACH + 1.5;
      const lx0 = xs[j] - sy * e0;
      const ly0 = ys[j] + sx * e0;
      const rx0 = xs[j] + sy * e0;
      const ry0 = ys[j] - sx * e0;
      const lx1 = xs[i] - ey * e1;
      const ly1 = ys[i] + ex * e1;
      const rx1 = xs[i] + ey * e1;
      const ry1 = ys[i] - ex * e1;
      // Two quads per segment, split along the centre line, so the zero line
      // of the distance field is the path itself whatever the cross-sections do.
      this.room(12);
      this.v(lx0, ly0, e0, 0, halfs[j], glows[j], ints[j], heats[j]);
      this.v(xs[j], ys[j], 0, 0, halfs[j], glows[j], ints[j], heats[j]);
      this.v(lx1, ly1, e1, 0, halfs[i], glows[i], ints[i], heats[i]);
      this.v(xs[j], ys[j], 0, 0, halfs[j], glows[j], ints[j], heats[j]);
      this.v(xs[i], ys[i], 0, 0, halfs[i], glows[i], ints[i], heats[i]);
      this.v(lx1, ly1, e1, 0, halfs[i], glows[i], ints[i], heats[i]);
      this.v(xs[j], ys[j], 0, 0, halfs[j], glows[j], ints[j], heats[j]);
      this.v(rx0, ry0, -e0, 0, halfs[j], glows[j], ints[j], heats[j]);
      this.v(xs[i], ys[i], 0, 0, halfs[i], glows[i], ints[i], heats[i]);
      this.v(rx0, ry0, -e0, 0, halfs[j], glows[j], ints[j], heats[j]);
      this.v(rx1, ry1, -e1, 0, halfs[i], glows[i], ints[i], heats[i]);
      this.v(xs[i], ys[i], 0, 0, halfs[i], glows[i], ints[i], heats[i]);

      pdx = dx;
      pdy = dy;
      havePrev = true;
    }
  }
}
