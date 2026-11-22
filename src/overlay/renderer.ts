// WebGL2 renderer for the overlay.
//
// Two passes per stroke, both confined by scissor to the stroke's bounding box:
//
//   1. Field pass. Geometry is drawn into a half-float texture with MAX
//      blending. Each texel ends up holding the strongest core, glow and heat
//      of anything that covers it. MAX, not ADD, is the point: where a stroke
//      crosses itself, folds at a corner or overlaps its own joins, light does
//      not pile up into bright knots.
//   2. Composite pass. The field becomes colour: a hot core, an ember body, a
//      glow that falls to exactly zero. Output is premultiplied with alpha far
//      below the colour for the glow, so over the desktop it adds light
//      instead of painting a grey veil, which is what keeps blacks black on
//      an OLED. A faint dark under-halo keeps the stroke readable on white.

import { Geometry, GLOW_REACH, STRIDE } from './geometry';

export interface Look {
  /** 0..1 overall. */
  opacity: number;
  /** 0 = ember colour, 1 = grey (no match). */
  desat: number;
  /** 0 = ember colour, 1 = ink white (a captured sample). */
  white: number;
  /** Brightness multiplier (the match pulse). */
  gain: number;
}

const FIELD_VS = `#version 300 es
layout(location=0) in vec2 a_pos;
layout(location=1) in vec2 a_local;
layout(location=2) in vec4 a_params; // half, glow, intensity, heat
uniform vec2 u_size;
out vec2 v_local;
out vec4 v_params;
void main() {
  v_local = a_local;
  v_params = a_params;
  gl_Position = vec4(a_pos.x / u_size.x * 2.0 - 1.0, 1.0 - a_pos.y / u_size.y * 2.0, 0.0, 1.0);
}`;

const FIELD_FS = `#version 300 es
precision highp float;
in vec2 v_local;
in vec4 v_params;
out vec4 o;
const float REACH = ${GLOW_REACH.toFixed(2)};
void main() {
  float d = length(v_local);
  float half_w = v_params.x;
  float glow_r = max(v_params.y, 0.001);
  float intensity = v_params.z;
  // Core: an antialiased edge at the half-width.
  float cover = 1.0 - smoothstep(half_w - 0.75, half_w + 0.75, d);
  float core = cover * intensity;
  // Glow: exponential, shifted and rescaled so it is exactly 0 at REACH radii.
  float x = max(d - half_w, 0.0) / glow_r;
  float floor_v = exp(-REACH);
  float glow = max(exp(-x) - floor_v, 0.0) / (1.0 - floor_v) * intensity;
  // Heat: white-hot along the centre line, cooling to the body colour at the
  // rim. Scaled by the temperature the vertex carries.
  float heat = (1.0 - smoothstep(half_w * 0.3, half_w + 0.75, d)) * v_params.w * intensity;
  o = vec4(core, glow, heat, cover);
}`;

const COMP_VS = `#version 300 es
void main() {
  // One triangle that covers the screen; scissor does the cropping.
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const COMP_FS = `#version 300 es
precision highp float;
uniform sampler2D u_field;
uniform vec4 u_look; // opacity, desat, white, gain
out vec4 o;

// Ember, from the outside in. Saturated so it holds up on OLED.
const vec3 GLOW = vec3(1.0, 0.25, 0.14);
const vec3 BODY = vec3(1.0, 0.39, 0.29);
const vec3 HOT  = vec3(1.0, 0.94, 0.82);
const vec3 INK  = vec3(0.957, 0.953, 0.941);

// Interleaved gradient noise (Jimenez): breaks up banding in the glow falloff.
float ign(vec2 p) {
  return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}

void main() {
  vec4 f = texelFetch(u_field, ivec2(gl_FragCoord.xy), 0);
  float cover = clamp(f.a, 0.0, 1.0);
  // Brightness of the core where there is core; 0 elsewhere.
  float lit = cover > 0.001 ? clamp(f.r / cover, 0.0, 1.0) : 0.0;
  float glow = f.g;
  float heat = clamp(f.b, 0.0, 1.0);
  float gain = u_look.w;

  vec3 glow_c = mix(GLOW, INK * 0.55, u_look.z);
  vec3 body_c = mix(BODY, INK, u_look.z);
  vec3 hot_c  = mix(HOT, vec3(1.0), u_look.z);

  // Two glows from one field: a tight bloom hugging the core and a wide,
  // faint halo. A single bright band would read as a neon tube.
  float bloom = pow(glow, 4.0);
  float halo = pow(glow, 1.5);
  float g2 = 0.55 * bloom + 0.5 * halo;
  vec3 light = glow_c * g2 * gain;
  vec3 solid = mix(body_c, hot_c, heat) * lit * min(gain, 1.6);
  light = mix(light, solid, cover);

  // No match: the colour drains to a cool grey and dims.
  float lum = dot(light, vec3(0.2126, 0.7152, 0.0722));
  light = mix(light, vec3(lum * 1.35 + cover * lit * 0.18) * vec3(0.92, 0.95, 1.0), u_look.y);

  // Alpha: the core is opaque however dim it is, so the faded tail is a dark
  // coral line on a white window instead of vanishing into it; it lets go
  // only when a piece is fading out altogether. Under the glow, a faint dark
  // halo that only light backgrounds reveal.
  float alpha = max(cover * 0.94 * smoothstep(0.0, 0.22, lit), halo * 0.3);

  float a = u_look.x;
  light *= a;
  alpha *= a;
  if (alpha <= 0.0004 && max(light.r, max(light.g, light.b)) <= 0.0004) discard;
  // Dither only where there is signal to dither, so the far edge of the halo
  // settles to true black instead of a sprinkle of single-step pixels.
  float peak = max(light.r, max(light.g, light.b));
  light += (ign(gl_FragCoord.xy) - 0.5) / 255.0 * smoothstep(1.0 / 255.0, 4.0 / 255.0, peak);
  o = vec4(max(light, 0.0), alpha);
}`;

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
  const s = gl.createShader(type)!;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    throw new Error(`shader: ${gl.getShaderInfoLog(s)}`);
  }
  return s;
}

function link(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const p = gl.createProgram()!;
  gl.attachShader(p, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw new Error(`program: ${gl.getProgramInfoLog(p)}`);
  }
  return p;
}

export class Renderer {
  readonly gl: WebGL2RenderingContext;
  private field: WebGLProgram;
  private comp: WebGLProgram;
  private uSize: WebGLUniformLocation;
  private uLook: WebGLUniformLocation;
  private vao: WebGLVertexArrayObject;
  private emptyVao: WebGLVertexArrayObject;
  private vbo: WebGLBuffer;
  private vboBytes = 0;
  private fbo: WebGLFramebuffer;
  private tex: WebGLTexture;
  private halfFloat: boolean;
  private w = 0;
  private h = 0;
  /** True when the last presented frame had pixels in it. */
  private dirty = false;
  private drewThisFrame = false;

  constructor(readonly canvas: HTMLCanvasElement) {
    const gl = canvas.getContext('webgl2', {
      alpha: true,
      premultipliedAlpha: true,
      antialias: false,
      depth: false,
      stencil: false,
      desynchronized: true,
      powerPreference: 'high-performance',
      preserveDrawingBuffer: false,
    });
    if (!gl) throw new Error('WebGL2 is not available');
    this.gl = gl;
    // Rendering to half-float needs this extension; 8 bits is the fallback
    // (the glow then bands slightly before dithering).
    this.halfFloat = !!gl.getExtension('EXT_color_buffer_float') || !!gl.getExtension('EXT_color_buffer_half_float');

    this.field = link(gl, FIELD_VS, FIELD_FS);
    this.comp = link(gl, COMP_VS, COMP_FS);
    this.uSize = gl.getUniformLocation(this.field, 'u_size')!;
    this.uLook = gl.getUniformLocation(this.comp, 'u_look')!;
    gl.useProgram(this.comp);
    gl.uniform1i(gl.getUniformLocation(this.comp, 'u_field'), 0);

    this.vao = gl.createVertexArray()!;
    this.emptyVao = gl.createVertexArray()!;
    this.vbo = gl.createBuffer()!;
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo);
    const bytes = STRIDE * 4;
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, bytes, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 2, gl.FLOAT, false, bytes, 8);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 4, gl.FLOAT, false, bytes, 16);
    gl.bindVertexArray(null);

    this.fbo = gl.createFramebuffer()!;
    this.tex = gl.createTexture()!;
    gl.disable(gl.DEPTH_TEST);
    gl.enable(gl.BLEND);
    gl.clearColor(0, 0, 0, 0);
  }

  resize(w: number, h: number): void {
    w = Math.max(1, Math.round(w));
    h = Math.max(1, Math.round(h));
    if (w === this.w && h === this.h) return;
    this.w = w;
    this.h = h;
    this.canvas.width = w;
    this.canvas.height = h;
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    if (this.halfFloat) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
    } else {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    }
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.tex, 0);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE && this.halfFloat) {
      this.halfFloat = false;
      this.w = 0;
      this.resize(w, h);
      return;
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, w, h);
  }

  /**
   * Start a frame. Whatever the last frame left is cleared here, explicitly:
   * relying on the browser to hand back an empty buffer breaks as soon as two
   * frames are drawn without a present in between, and the glow then stacks.
   */
  begin(): void {
    this.drewThisFrame = false;
    if (this.dirty) {
      const gl = this.gl;
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.disable(gl.SCISSOR_TEST);
      gl.clear(gl.COLOR_BUFFER_BIT);
    }
  }

  /** Draw one stroke's geometry with one look. */
  draw(geo: Geometry, look: Look): void {
    if (geo.vertices === 0 || look.opacity <= 0.001) return;
    // Bounding box, clipped to this monitor. Nothing to do if the stroke is elsewhere.
    const x0 = Math.max(0, Math.floor(geo.minX));
    const y0 = Math.max(0, Math.floor(geo.minY));
    const x1 = Math.min(this.w, Math.ceil(geo.maxX));
    const y1 = Math.min(this.h, Math.ceil(geo.maxY));
    if (x1 <= x0 || y1 <= y0) return;

    const gl = this.gl;
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x0, this.h - y1, x1 - x0, y1 - y0); // GL's origin is bottom-left

    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(this.field);
    gl.uniform2f(this.uSize, this.w, this.h);
    gl.blendEquation(gl.MAX);
    gl.blendFunc(gl.ONE, gl.ONE);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo);
    const view = geo.data.subarray(0, geo.floats);
    if (view.byteLength > this.vboBytes) {
      this.vboBytes = geo.data.byteLength;
      gl.bufferData(gl.ARRAY_BUFFER, this.vboBytes, gl.DYNAMIC_DRAW);
    }
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, view);
    gl.drawArrays(gl.TRIANGLES, 0, geo.vertices);

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.useProgram(this.comp);
    gl.uniform4f(this.uLook, look.opacity, look.desat, look.white, look.gain);
    gl.blendEquation(gl.FUNC_ADD);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.bindVertexArray(this.emptyVao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.disable(gl.SCISSOR_TEST);
    this.drewThisFrame = true;
  }

  /** Finish the frame.  stays true for one empty frame so the last image gets cleared. */
  end(): void {
    this.dirty = this.drewThisFrame;
  }

  get busy(): boolean {
    return this.dirty;
  }
}
