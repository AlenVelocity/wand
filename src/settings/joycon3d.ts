// Live 3D preview of the controller's fused orientation.
//
// A small hand-written WebGL renderer: one mesh built here, one shader, no
// library. The model's axes are the sensor's: +X right, +Y out of the tip,
// +Z out of the button face. The camera sits where you sit, behind the wand,
// looking at the screen.
//
// The model is a Joy-Con 2 as measured, not as remembered. The outline and
// the place of every control were read off a flat, top-down photograph of a
// real pair (Wikimedia Commons, "Nintendo Switch 2 Joy-Con 2 Blue and
// Orange", Crisco 1492, CC BY-SA 4.0) by scaling its long side to the 116 mm
// in Nintendo's specifications; the width then comes out at 41.4 mm, which is
// the figure Nintendo gives. Thickness is Nintendo's too: 14.4 mm for the
// body, 30.7 mm from the top of the stick to the back of the trigger.

type V3 = [number, number, number];
/** Colour and how shiny: r, g, b, gloss (0 matt plastic, 1 polished). */
type Paint = [number, number, number, number];

interface MeshData {
  pos: number[];
  nor: number[];
  col: number[];
}

function hexToRgb(hex: string, fallback: V3): V3 {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return fallback;
  const n = parseInt(m[1], 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

function tri(m: MeshData, p: V3[], n: V3[], paint: Paint): void {
  for (let k = 0; k < 3; k++) {
    m.pos.push(...p[k]);
    m.nor.push(...n[k]);
    m.col.push(...paint);
  }
}

function quad(m: MeshData, p: V3[], n: V3[], paint: Paint): void {
  tri(m, [p[0], p[1], p[2]], [n[0], n[1], n[2]], paint);
  tri(m, [p[0], p[2], p[3]], [n[0], n[2], n[3]], paint);
}

/**
 * A box with rounded edges: a subdivided cube pulled onto a rounded surface.
 * Each face is cut where its rounding begins, so the flat part is one flat
 * panel and only the edges curve.
 */
function roundedBox(m: MeshData, center: V3, half: V3, radius: number, paint: Paint): void {
  const inner: V3 = [half[0] - radius, half[1] - radius, half[2] - radius];
  const faces: [V3, V3, V3][] = [
    [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
    [[0, 1, 0], [0, 0, 1], [1, 0, 0]],
    [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
    [[0, 0, 1], [1, 0, 0], [0, 1, 0]],
    [[0, 0, -1], [0, 1, 0], [1, 0, 0]],
  ];
  const point = (n: V3, u: V3, v: V3, a: number, b: number): { p: V3; nrm: V3 } => {
    const raw: V3 = [0, 0, 0];
    for (let k = 0; k < 3; k++) raw[k] = (n[k] + u[k] * a + v[k] * b) * half[k];
    const c: V3 = [0, 0, 0];
    for (let k = 0; k < 3; k++) c[k] = Math.max(-inner[k], Math.min(inner[k], raw[k]));
    let d: V3 = [raw[0] - c[0], raw[1] - c[1], raw[2] - c[2]];
    const len = Math.hypot(d[0], d[1], d[2]) || 1;
    d = [d[0] / len, d[1] / len, d[2] / len];
    return { p: [center[0] + c[0] + d[0] * radius, center[1] + c[1] + d[1] * radius, center[2] + c[2] + d[2] * radius], nrm: d };
  };
  // Where to cut a face along one of its axes, as fractions of the half-length.
  const cuts = (axis: V3): number[] => {
    const h = Math.abs(axis[0]) * half[0] + Math.abs(axis[1]) * half[1] + Math.abs(axis[2]) * half[2];
    const f = Math.min(1, radius / h);
    if (f > 0.9) return [-1, -0.66, -0.33, 0, 0.33, 0.66, 1];
    return [-1, -(1 - f * 0.35), -(1 - f * 0.7), -(1 - f), 1 - f, 1 - f * 0.7, 1 - f * 0.35, 1];
  };
  for (const [n, u, v] of faces) {
    const [as, bs] = [cuts(u), cuts(v)];
    for (let i = 0; i + 1 < as.length; i++) {
      for (let j = 0; j + 1 < bs.length; j++) {
        const q = [point(n, u, v, as[i], bs[j]), point(n, u, v, as[i + 1], bs[j]), point(n, u, v, as[i + 1], bs[j + 1]), point(n, u, v, as[i], bs[j + 1])];
        quad(m, q.map((c) => c.p), q.map((c) => c.nrm), paint);
      }
    }
  }
}

/**
 * A shape turned on a lathe about +Z at (cx, cy). `profile` is its outline as
 * [radius, z] pairs, from the base up the outside to the top. Neighbouring
 * bands share a normal, so curves are smooth; repeat a point for a hard edge.
 */
function lathe(m: MeshData, cx: number, cy: number, z: number, profile: [number, number][], paint: Paint, seg = 36): void {
  const bands: { a: [number, number]; b: [number, number]; n: [number, number] }[] = [];
  for (let i = 0; i + 1 < profile.length; i++) {
    const [a, b] = [profile[i], profile[i + 1]];
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
    bands.push({ a, b, n: len < 1e-6 ? [0, 0] : [(b[1] - a[1]) / len, (a[0] - b[0]) / len] });
  }
  // The normal where band `self` meets band `other`: the two averaged, unless
  // there is no neighbour or it is a repeated point (a hard edge).
  const meet = (self: number, other: number): [number, number] => {
    const [p, q] = [bands[self].n, bands[other]?.n];
    if (!q || (q[0] === 0 && q[1] === 0)) return p;
    const len = Math.hypot(p[0] + q[0], p[1] + q[1]) || 1;
    return [(p[0] + q[0]) / len, (p[1] + q[1]) / len];
  };
  bands.forEach((band, i) => {
    if (band.n[0] === 0 && band.n[1] === 0) return;
    const n0 = meet(i, i - 1);
    const n1 = meet(i, i + 1);
    for (let k = 0; k < seg; k++) {
      const [t0, t1] = [(k / seg) * Math.PI * 2, ((k + 1) / seg) * Math.PI * 2];
      const [c0, s0, c1, s1] = [Math.cos(t0), Math.sin(t0), Math.cos(t1), Math.sin(t1)];
      const at = (r: number, h: number, c: number, sn: number): V3 => [cx + r * c, cy + r * sn, z + h];
      const nr = (n: [number, number], c: number, sn: number): V3 => [n[0] * c, n[0] * sn, n[1]];
      quad(
        m,
        [at(band.a[0], band.a[1], c0, s0), at(band.a[0], band.a[1], c1, s1), at(band.b[0], band.b[1], c1, s1), at(band.b[0], band.b[1], c0, s0)],
        [nr(n0, c0, s0), nr(n0, c1, s1), nr(n1, c1, s1), nr(n1, c0, s0)],
        paint,
      );
    }
  });
}

/** An ellipsoid: a sphere stretched to the three half-lengths in `r`. */
function ellipsoid(m: MeshData, center: V3, r: V3, paint: Paint, seg = 20): void {
  const at = (i: number, j: number): { p: V3; n: V3 } => {
    const [th, ph] = [(i / seg) * Math.PI, (j / (seg * 2)) * Math.PI * 2];
    const d: V3 = [Math.sin(th) * Math.cos(ph), Math.sin(th) * Math.sin(ph), Math.cos(th)];
    const n: V3 = [d[0] / r[0], d[1] / r[1], d[2] / r[2]];
    const len = Math.hypot(n[0], n[1], n[2]) || 1;
    return { p: [center[0] + d[0] * r[0], center[1] + d[1] * r[1], center[2] + d[2] * r[2]], n: [n[0] / len, n[1] / len, n[2] / len] };
  };
  for (let i = 0; i < seg; i++) {
    for (let j = 0; j < seg * 2; j++) {
      const q = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
      quad(m, q.map((c) => c.p), q.map((c) => c.n), paint);
    }
  }
}

// ---------------------------------------------------------------- the model
//
// Everything below is in millimetres. `u` runs down the controller from its
// top end (0) to its bottom end (116). `c` runs across it from the curved
// outer edge (0) to the flat edge the rail is on.

const LENGTH = 116;
/** Width of the black body. The rail adds 6 mm, for 41.4 in all. */
const BODY = 35.4;
const WIDTH = 41.4;
const THICK = 14.4;

/**
 * The curved corner, as measured: how far from the end the outline is at each
 * millimetre across, starting 1 mm in from the outer edge. It is close to an
 * ellipse but holds its curve longer, which is most of what makes the shape
 * read as a Joy-Con and not as a rounded rectangle.
 */
const CORNER = [
  23.2, 18.9, 16.1, 13.8, 12.1, 10.7, 9.3, 8.3, 7.4, 6.5, 5.65, 5.0, 4.4, 3.8, 3.3, 2.9, 2.5, 2.1, 1.75, 1.45, 1.2, 1.0, 0.8, 0.65, 0.5, 0.4,
  0.25, 0.2, 0.1, 0,
];
/** The last of the curve, where the outer edge is nearly straight: [from the end, across]. */
const CORNER_TAIL: [number, number][] = [[58, 0], [45, 0.02], [40, 0.1], [35, 0.2], [31, 0.3], [27, 0.45], [25, 0.6]];
/** Radius of the two nearly square corners on the rail side. */
const INNER_RADIUS = 2.4;

/** The body's outline as [u, c] points, once round. */
function outline(): [number, number][] {
  const half: [number, number][] = [...CORNER_TAIL, ...CORNER.map((e, i): [number, number] => [e, i + 1])];
  const flat = BODY - INNER_RADIUS;
  for (let k = 0; k <= 6; k++) {
    const t = (k / 6) * (Math.PI / 2);
    half.push([INNER_RADIUS - INNER_RADIUS * Math.cos(t), flat + INNER_RADIUS * Math.sin(t)]);
  }
  // Down the straight edge in steps, so the long faces are not single slivers.
  for (let e = 10; e < LENGTH / 2; e += 8) half.push([e, BODY]);
  half.push([LENGTH / 2, BODY]);
  const other = half
    .slice(1, -1)
    .reverse()
    .map(([e, c]): [number, number] => [LENGTH - e, c]);
  return [...half, ...other];
}

interface Ring {
  /** Position on the XY plane and the outward normal there. */
  p: [number, number][];
  n: [number, number][];
}

/**
 * A slab with the given convex outline, its top and bottom edges rounded
 * over. Returns the outline with its outward normals, for parts that follow
 * the edge.
 */
function slab(m: MeshData, pts: [number, number][], z0: number, z1: number, rTop: number, rBottom: number, paint: Paint): Ring {
  const count = pts.length;
  // Outward normal at each point, from its two neighbours. The outline is
  // convex, so "outward" is away from the centroid.
  const mid: [number, number] = [0, 0];
  for (const p of pts) {
    mid[0] += p[0] / count;
    mid[1] += p[1] / count;
  }
  const normals = pts.map((p, i): [number, number] => {
    const [a, b] = [pts[(i + count - 1) % count], pts[(i + 1) % count]];
    let n: [number, number] = [b[1] - a[1], a[0] - b[0]];
    const len = Math.hypot(n[0], n[1]) || 1;
    n = [n[0] / len, n[1] / len];
    return n[0] * (p[0] - mid[0]) + n[1] * (p[1] - mid[1]) < 0 ? [-n[0], -n[1]] : n;
  });
  // Rings from the bottom face's edge, round the bottom fillet, up the wall,
  // round the top fillet, to the top face's edge.
  const rings: { inset: number; z: number; up: number }[] = [];
  const steps = 5;
  for (let k = 0; k <= steps; k++) {
    const t = (k / steps) * (Math.PI / 2);
    rings.push({ inset: rBottom * (1 - Math.sin(t)), z: z0 + rBottom * (1 - Math.cos(t)), up: -Math.cos(t) });
  }
  for (let k = 0; k <= steps; k++) {
    const t = (k / steps) * (Math.PI / 2);
    rings.push({ inset: rTop * (1 - Math.cos(t)), z: z1 - rTop * (1 - Math.sin(t)), up: Math.sin(t) });
  }
  const at = (ring: (typeof rings)[number], i: number): { p: V3; n: V3 } => {
    const side = Math.sqrt(Math.max(0, 1 - ring.up * ring.up));
    return {
      p: [pts[i][0] - normals[i][0] * ring.inset, pts[i][1] - normals[i][1] * ring.inset, ring.z],
      n: [normals[i][0] * side, normals[i][1] * side, ring.up],
    };
  };
  for (let r = 0; r + 1 < rings.length; r++) {
    for (let i = 0; i < count; i++) {
      const j = (i + 1) % count;
      const q = [at(rings[r], i), at(rings[r], j), at(rings[r + 1], j), at(rings[r + 1], i)];
      quad(m, q.map((c) => c.p), q.map((c) => c.n), paint);
    }
  }
  // The two faces, as fans.
  for (const [ring, up] of [[rings[rings.length - 1], 1], [rings[0], -1]] as const) {
    const n: V3 = [0, 0, up];
    for (let i = 0; i < count; i++) {
      const j = (i + 1) % count;
      tri(m, [[mid[0], mid[1], ring.z], at(ring, i).p, at(ring, j).p], [n, n, n], paint);
    }
  }
  return { p: pts, n: normals };
}

function buildJoyCon(body: V3, accent: V3, right: boolean): MeshData {
  const m: MeshData = { pos: [], nor: [], col: [] };
  const shell: Paint = [...body, 0.12];
  const key: Paint = [0.075, 0.078, 0.085, 0.55];
  const colour: Paint = [...accent, 0.2];
  const colourDim: Paint = [accent[0] * 0.84, accent[1] * 0.84, accent[2] * 0.84, 0.2];

  // The rail is on the left of a right Joy-Con and the right of a left one.
  const side = right ? -1 : 1;
  const x = (c: number) => side * (c - BODY / 2);
  const y = (u: number) => LENGTH / 2 - u;
  const face = THICK / 2;

  // Body.
  const edge = slab(m, outline().map(([u, c]): [number, number] => [x(c), y(u)]), -face, face, 1.6, 2.0, shell);

  // Rail: the coloured strip the console holds on to, thinner than the body
  // and a little shorter, with the SL and SR buttons standing out of it.
  const railMid = (BODY - 1.2 + WIDTH) / 2;
  roundedBox(m, [x(railMid), 0, 0], [(WIDTH - BODY + 1.2) / 2, LENGTH / 2 - 6.4, 4.3], 1.1, colour);
  for (const u of [29, 87]) roundedBox(m, [x(WIDTH - 0.1), y(u), 0], [0.9, 7.5, 2.1], 0.8, colourDim);

  // Stick: a coloured collar, a stem, and a cap dished in the middle.
  const stick = (u: number, c: number) => {
    lathe(m, x(c), y(u), face, [[10.2, -0.4], [10.2, 0.5], [9.5, 1.25], [6.4, 2.5]], colour);
    lathe(
      m,
      x(c),
      y(u),
      face,
      [[5.7, 2.0], [5.2, 5.0], [7.5, 5.6], [8.1, 6.2], [8.1, 8.0], [7.6, 8.6], [6.7, 8.6], [5.9, 8.15], [3.2, 7.8], [0, 7.7]],
      key,
    );
  };
  const button = (u: number, c: number, r: number, h: number) =>
    lathe(m, x(c), y(u), face, [[r, -0.5], [r, h * 0.6], [r - 0.4, h * 0.9], [r * 0.55, h], [0, h + 0.05]], key, 24);
  const diamond = (u: number, c: number) => {
    for (const [du, dc] of [[-8.2, 0], [8.2, 0], [0, -8.3], [0, 8.3]]) button(u + du, c + dc, 4.3, 1.5);
  };
  const bar = (u: number, c: number, along: number, across: number) =>
    roundedBox(m, [x(c), y(u), face + 0.25], [across, along, 0.8], 0.6, key);

  if (right) {
    bar(13.1, 26.3, 2.8, 0.95); // plus
    bar(13.1, 26.3, 0.95, 2.8);
    diamond(29.3, 16.9); // X, B, A, Y
    stick(60, 19);
    button(79.9, 23.1, 3.7, 0.95); // home
    roundedBox(m, [x(22.5), y(91.8), face + 0.1], [3.5, 3.5, 0.8], 0.75, key); // C
  } else {
    bar(14.1, 26.8, 0.95, 2.8); // minus
    stick(29.5, 19.6);
    diamond(59.3, 17.5); // up, down, left, right
    roundedBox(m, [x(24.5), y(80.6), face + 0.1], [3.5, 3.5, 0.8], 0.75, key); // capture
  }

  // On the back, at the top: the trigger (ZR or ZL). It is what makes the
  // controller 30.7 mm deep.
  ellipsoid(m, [x(20), y(12), -face], [12, 8, 7], key);
  // The shoulder button (R or L): a thin strip round the top corner, set into
  // the back half of the edge.
  const count = edge.p.length;
  for (let i = 0; i < count; i++) {
    const j = (i + 1) % count;
    const inRun = (k: number) => edge.p[k][1] > y(11) && edge.n[k][1] > 0.25 && side * edge.p[k][0] + BODY / 2 < 31;
    if (!inRun(i) || !inRun(j)) continue;
    const out = 0.55;
    const at = (k: number, z: number, d: number): V3 => [edge.p[k][0] + edge.n[k][0] * d, edge.p[k][1] + edge.n[k][1] * d, z];
    const flat = (k: number): V3 => [edge.n[k][0], edge.n[k][1], 0];
    quad(m, [at(i, -5.4, out), at(j, -5.4, out), at(j, -0.4, out), at(i, -0.4, out)], [flat(i), flat(j), flat(j), flat(i)], key);
    const up: V3 = [0, 0, 1];
    quad(m, [at(i, -0.4, 0), at(j, -0.4, 0), at(j, -0.4, out), at(i, -0.4, out)], [up, up, up, up], key);
  }

  // Millimetres to scene units.
  m.pos = m.pos.map((v) => v * 0.01);
  return m;
}

// ------------------------------------------------------------------- maths

type M4 = Float32Array;

function perspective(fovY: number, aspect: number, near: number, far: number): M4 {
  const f = 1 / Math.tan(fovY / 2);
  const m = new Float32Array(16);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = (far + near) / (near - far);
  m[11] = -1;
  m[14] = (2 * far * near) / (near - far);
  return m;
}

function lookAt(eye: V3, target: V3, up: V3): M4 {
  const z: V3 = [eye[0] - target[0], eye[1] - target[1], eye[2] - target[2]];
  const zl = Math.hypot(...z);
  z[0] /= zl; z[1] /= zl; z[2] /= zl;
  const x: V3 = [up[1] * z[2] - up[2] * z[1], up[2] * z[0] - up[0] * z[2], up[0] * z[1] - up[1] * z[0]];
  const xl = Math.hypot(...x);
  x[0] /= xl; x[1] /= xl; x[2] /= xl;
  const y: V3 = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
  const m = new Float32Array(16);
  m.set([x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, 0, 0, 0, 1]);
  m[12] = -(x[0] * eye[0] + x[1] * eye[1] + x[2] * eye[2]);
  m[13] = -(y[0] * eye[0] + y[1] * eye[1] + y[2] * eye[2]);
  m[14] = -(z[0] * eye[0] + z[1] * eye[1] + z[2] * eye[2]);
  return m;
}

function mul(a: M4, b: M4): M4 {
  const o = new Float32Array(16);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      o[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
    }
  }
  return o;
}

/** Rotation matrix (column-major 4x4) of a unit quaternion x, y, z, w. */
function fromQuat([x, y, z, w]: [number, number, number, number]): M4 {
  const m = new Float32Array(16);
  m.set([
    1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w), 0,
    2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w), 0,
    2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y), 0,
    0, 0, 0, 1,
  ]);
  return m;
}

function rotZ(a: number): M4 {
  const m = new Float32Array(16);
  const [c, s] = [Math.cos(a), Math.sin(a)];
  m.set([c, s, 0, 0, -s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
  return m;
}

const VS = `#version 300 es
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_nor;
layout(location=2) in vec4 a_col;
uniform mat4 u_vp;
uniform mat4 u_model;
out vec3 v_nor;
out vec4 v_col;
out vec3 v_world;
void main() {
  vec4 world = u_model * vec4(a_pos, 1.0);
  v_world = world.xyz;
  v_nor = mat3(u_model) * a_nor;
  v_col = a_col;
  gl_Position = u_vp * world;
}`;

const FS = `#version 300 es
precision highp float;
in vec3 v_nor;
in vec4 v_col;
in vec3 v_world;
uniform vec3 u_eye;
out vec4 o;
void main() {
  vec3 n = normalize(v_nor);
  vec3 view = normalize(u_eye - v_world);
  // One key light from above and behind the viewer, a little fill, and a rim
  // so a near-black controller still has an outline on a black page.
  vec3 light = normalize(vec3(-0.35, -0.55, 0.76));
  float key = max(dot(n, light), 0.0);
  float fill = max(dot(n, normalize(vec3(0.6, -0.2, 0.3))), 0.0) * 0.25;
  float rim = pow(1.0 - max(dot(n, view), 0.0), 3.0);
  // Matt plastic gives a wide, faint highlight; the buttons a tight one.
  float gloss = v_col.a;
  float shine = pow(max(dot(n, normalize(light + view)), 0.0), mix(10.0, 70.0, gloss)) * mix(0.07, 0.5, gloss);
  vec3 ink = vec3(0.957, 0.953, 0.941);
  vec3 c = v_col.rgb * (0.16 + 0.9 * key + fill) + ink * (rim * 0.3 + shine);
  o = vec4(c, 1.0);
}`;

const RAY_VS = `#version 300 es
layout(location=0) in vec3 a_pos;
layout(location=1) in float a_t;
uniform mat4 u_vp;
uniform mat4 u_model;
out float v_t;
void main() {
  v_t = a_t;
  gl_Position = u_vp * u_model * vec4(a_pos, 1.0);
}`;

const RAY_FS = `#version 300 es
precision highp float;
in float v_t;
out vec4 o;
void main() {
  // Where the wand is pointing: coral, fading with distance.
  float a = (1.0 - v_t) * (1.0 - v_t);
  o = vec4(vec3(1.0, 0.34, 0.34) * a, a);
}`;

function program(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const p = gl.createProgram()!;
  for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]] as const) {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? 'shader');
    gl.attachShader(p, s);
  }
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? 'link');
  return p;
}

export interface Look {
  body: string;
  accent: string;
  right: boolean;
}

export class JoyConView {
  private gl: WebGL2RenderingContext;
  private prog: WebGLProgram;
  private ray: WebGLProgram;
  private vao: WebGLVertexArrayObject;
  private rayVao: WebGLVertexArrayObject;
  private buffers: WebGLBuffer[] = [];
  private count = 0;
  private q: [number, number, number, number] = [0, 0, 0, 1];
  /** Heading that counts as "at the screen"; set on the first pose and on Recentre. */
  private heading: number | null = null;
  private raf = 0;
  private look = '';

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
    if (!gl) throw new Error('WebGL2 is not available');
    this.gl = gl;
    this.prog = program(gl, VS, FS);
    this.ray = program(gl, RAY_VS, RAY_FS);
    this.vao = gl.createVertexArray()!;
    this.rayVao = gl.createVertexArray()!;
    gl.bindVertexArray(this.rayVao);
    const rb = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, rb);
    // A thin quad from the tip forward.
    const w = 0.006;
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-w, 0.6, 0, 0, w, 0.6, 0, 0, w, 2.6, 0, 1, -w, 0.6, 0, 0, w, 2.6, 0, 1, -w, 2.6, 0, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 16, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 1, gl.FLOAT, false, 16, 12);
    gl.bindVertexArray(null);
    this.setLook({ body: '', accent: '', right: true });
  }

  setLook(look: Look): void {
    const key = JSON.stringify(look);
    if (key === this.look) return;
    this.look = key;
    const gl = this.gl;
    // A Joy-Con 2 is charcoal, and reports itself as nearly black. Anything
    // that dark is drawn as the charcoal it looks like in daylight, so the
    // model still has shading on a black page.
    const charcoal: V3 = [0.168, 0.176, 0.19];
    let body = hexToRgb(look.body, charcoal);
    if (Math.max(...body) < 0.22) body = charcoal;
    const coral: V3 = [1, 0.34, 0.34];
    const accent = hexToRgb(look.accent, coral);
    const flat = Math.max(...accent) - Math.min(...accent) < 0.2;
    const mesh = buildJoyCon(body, flat ? coral : accent, look.right);
    this.count = mesh.pos.length / 3;
    gl.bindVertexArray(this.vao);
    [mesh.pos, mesh.nor, mesh.col].forEach((data, i) => {
      const b = (this.buffers[i] ??= gl.createBuffer()!);
      gl.bindBuffer(gl.ARRAY_BUFFER, b);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(data), gl.STATIC_DRAW);
      gl.enableVertexAttribArray(i);
      gl.vertexAttribPointer(i, i === 2 ? 4 : 3, gl.FLOAT, false, 0, 0);
    });
    gl.bindVertexArray(null);
    this.request();
  }

  setPose(q: [number, number, number, number]): void {
    this.q = q;
    if (this.heading === null) this.recenter();
    this.request();
  }

  /** Treat the current heading as facing the screen. */
  recenter(): void {
    const m = fromQuat(this.q);
    // Body +Y in world coordinates is the matrix's second column.
    this.heading = Math.atan2(m[4], m[5]);
    this.request();
  }

  private request(): void {
    if (!this.raf) this.raf = requestAnimationFrame(() => this.draw());
  }

  private draw(): void {
    this.raf = 0;
    const gl = this.gl;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(this.canvas.clientWidth * dpr);
    const h = Math.round(this.canvas.clientHeight * dpr);
    if (w === 0 || h === 0) return;
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
    }
    gl.viewport(0, 0, w, h);
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.enable(gl.DEPTH_TEST);

    const eye: V3 = [0.55, -2.75, 1.25];
    const vp = mul(perspective((30 * Math.PI) / 180, w / h, 0.1, 20), lookAt(eye, [0, 0.25, 0], [0, 0, 1]));
    const model = mul(rotZ(this.heading ?? 0), fromQuat(this.q));

    gl.useProgram(this.prog);
    gl.uniformMatrix4fv(gl.getUniformLocation(this.prog, 'u_vp'), false, vp);
    gl.uniformMatrix4fv(gl.getUniformLocation(this.prog, 'u_model'), false, model);
    gl.uniform3fv(gl.getUniformLocation(this.prog, 'u_eye'), eye);
    gl.bindVertexArray(this.vao);
    gl.drawArrays(gl.TRIANGLES, 0, this.count);

    gl.useProgram(this.ray);
    gl.uniformMatrix4fv(gl.getUniformLocation(this.ray, 'u_vp'), false, vp);
    gl.uniformMatrix4fv(gl.getUniformLocation(this.ray, 'u_model'), false, model);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE);
    gl.depthMask(false);
    gl.bindVertexArray(this.rayVao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.depthMask(true);
    gl.disable(gl.BLEND);
    gl.bindVertexArray(null);
  }

  dispose(): void {
    cancelAnimationFrame(this.raf);
    this.gl.getExtension('WEBGL_lose_context')?.loseContext();
  }
}
