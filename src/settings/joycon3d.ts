// Live 3D preview of the controller's fused orientation.
//
// A small hand-written WebGL renderer: one mesh built here from rounded boxes
// and cylinders, one shader, no library. The model's axes are the sensor's:
// +X right, +Y out of the tip, +Z out of the button face. The camera sits
// where you sit, behind the wand, looking at the screen.

type V3 = [number, number, number];

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

/** A box with rounded edges: a subdivided cube pulled onto a rounded surface. */
function roundedBox(m: MeshData, center: V3, half: V3, radius: number, color: V3, seg = 8): void {
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
  for (const [n, u, v] of faces) {
    for (let i = 0; i < seg; i++) {
      for (let j = 0; j < seg; j++) {
        const a0 = (i / seg) * 2 - 1;
        const a1 = ((i + 1) / seg) * 2 - 1;
        const b0 = (j / seg) * 2 - 1;
        const b1 = ((j + 1) / seg) * 2 - 1;
        const q = [point(n, u, v, a0, b0), point(n, u, v, a1, b0), point(n, u, v, a1, b1), point(n, u, v, a0, b1)];
        for (const k of [0, 1, 2, 0, 2, 3]) {
          m.pos.push(...q[k].p);
          m.nor.push(...q[k].nrm);
          m.col.push(...color);
        }
      }
    }
  }
}

/** A cylinder along +Z with a flat top, from z0 to z1. */
function cylinder(m: MeshData, cx: number, cy: number, z0: number, z1: number, r: number, color: V3, seg = 28): void {
  for (let i = 0; i < seg; i++) {
    const a0 = (i / seg) * Math.PI * 2;
    const a1 = ((i + 1) / seg) * Math.PI * 2;
    const [c0, s0, c1, s1] = [Math.cos(a0), Math.sin(a0), Math.cos(a1), Math.sin(a1)];
    const side: [V3, V3][] = [
      [[cx + c0 * r, cy + s0 * r, z0], [c0, s0, 0]],
      [[cx + c1 * r, cy + s1 * r, z0], [c1, s1, 0]],
      [[cx + c1 * r, cy + s1 * r, z1], [c1, s1, 0]],
      [[cx + c0 * r, cy + s0 * r, z1], [c0, s0, 0]],
    ];
    for (const k of [0, 1, 2, 0, 2, 3]) {
      m.pos.push(...side[k][0]);
      m.nor.push(...side[k][1]);
      m.col.push(...color);
    }
    const top: V3[] = [[cx, cy, z1], [cx + c0 * r, cy + s0 * r, z1], [cx + c1 * r, cy + s1 * r, z1]];
    for (const p of top) {
      m.pos.push(...p);
      m.nor.push(0, 0, 1);
      m.col.push(...color);
    }
  }
}

function buildJoyCon(body: V3, accent: V3, right: boolean): MeshData {
  const m: MeshData = { pos: [], nor: [], col: [] };
  const dark: V3 = [0.09, 0.09, 0.085];
  const key: V3 = [0.2, 0.2, 0.19];
  // Body: 36 x 116 x 15 mm, in decimetres-ish units.
  roundedBox(m, [0, 0, 0], [0.18, 0.58, 0.075], 0.07, body, 10);
  // The rail edge (the flat side that meets the console) carries the accent.
  const side = right ? -1 : 1;
  roundedBox(m, [side * 0.176, 0, 0], [0.012, 0.5, 0.05], 0.01, accent, 2);
  // Stick, with its accent ring.
  const stickY = right ? -0.04 : 0.24;
  cylinder(m, 0, stickY, 0.07, 0.085, 0.105, accent);
  cylinder(m, 0, stickY, 0.07, 0.15, 0.078, dark);
  cylinder(m, 0, stickY, 0.15, 0.165, 0.086, key);
  // Four face buttons in a diamond.
  const padY = right ? 0.27 : -0.07;
  for (const [dx, dy] of [[0, 0.085], [0, -0.085], [0.085, 0], [-0.085, 0]]) {
    cylinder(m, dx, padY + dy, 0.07, 0.092, 0.038, key, 18);
  }
  // Plus / minus and home / capture.
  cylinder(m, side * -0.1, 0.47, 0.07, 0.086, 0.022, key, 14);
  cylinder(m, side * -0.02, right ? -0.27 : -0.3, 0.07, 0.086, 0.03, key, 16);
  // Shoulder and trigger at the tip, on the back.
  roundedBox(m, [side * -0.04, 0.55, -0.03], [0.12, 0.05, 0.07], 0.035, dark, 4);
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
layout(location=2) in vec3 a_col;
uniform mat4 u_vp;
uniform mat4 u_model;
out vec3 v_nor;
out vec3 v_col;
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
in vec3 v_col;
in vec3 v_world;
uniform vec3 u_eye;
out vec4 o;
void main() {
  vec3 n = normalize(v_nor);
  vec3 view = normalize(u_eye - v_world);
  // One key light from above and behind the viewer, a little fill, and a rim
  // so a black controller still has an outline on a black page.
  float key = max(dot(n, normalize(vec3(-0.35, -0.55, 0.76))), 0.0);
  float fill = max(dot(n, normalize(vec3(0.6, -0.2, 0.3))), 0.0) * 0.25;
  float rim = pow(1.0 - max(dot(n, view), 0.0), 3.0);
  vec3 c = v_col * (0.16 + 0.9 * key + fill) + vec3(0.957, 0.953, 0.941) * rim * 0.34;
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
    // Joy-Con 2 bodies are near black; lift very dark colours enough to shade.
    let body = hexToRgb(look.body, [0.13, 0.13, 0.125]);
    if (Math.max(...body) < 0.12) body = [0.13, 0.13, 0.125];
    const accent = hexToRgb(look.accent, [1, 0.34, 0.34]);
    const mesh = buildJoyCon(body, Math.max(...accent) < 0.2 ? [1, 0.34, 0.34] : accent, look.right);
    this.count = mesh.pos.length / 3;
    gl.bindVertexArray(this.vao);
    [mesh.pos, mesh.nor, mesh.col].forEach((data, i) => {
      const b = (this.buffers[i] ??= gl.createBuffer()!);
      gl.bindBuffer(gl.ARRAY_BUFFER, b);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(data), gl.STATIC_DRAW);
      gl.enableVertexAttribArray(i);
      gl.vertexAttribPointer(i, 3, gl.FLOAT, false, 0, 0);
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
