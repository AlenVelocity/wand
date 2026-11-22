// The overlay for one monitor: receives stroke messages, keeps the live
// strokes, and draws them on every display frame while there is anything to
// draw. When idle nothing runs: no frame loop, no timers, a clear canvas.

import type { Monitor, OverlayCfg, OverlayMsg } from '../shared/types';
import { Geometry } from './geometry';
import { Labels } from './label';
import { Renderer } from './renderer';
import { Stroke, type Env } from './stroke';

export interface FrameStats {
  frames: number;
  seconds: number;
  fps: number;
  /** Frame intervals in milliseconds. */
  median: number;
  p99: number;
  worst: number;
}

export class Overlay {
  private renderer: Renderer;
  private labels: Labels;
  private geo = new Geometry();
  private strokes: Stroke[] = [];
  private env: Env;
  private raf = 0;
  /** Smallest (arrival time - engine time) seen: maps engine time to local time. */
  private clockOffset = Infinity;
  private offsetSamples: number[] = [];
  private intervals: number[] = [];
  private lastFrame = 0;
  /** Overridable so the demo can freeze time for screenshots. */
  now: () => number = () => performance.now();
  onStats: ((stats: FrameStats) => void) | null = null;

  constructor(
    canvas: HTMLCanvasElement,
    labelRoot: HTMLElement,
    private monitor: Monitor,
    cfg: OverlayCfg,
  ) {
    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    this.renderer = new Renderer(canvas);
    this.renderer.resize(monitor.w, monitor.h);
    this.labels = new Labels(labelRoot, reducedMotion);
    this.env = { cfg, scale: monitor.scale, height: monitor.h, reducedMotion };
  }

  setConfig(cfg: OverlayCfg): void {
    this.env.cfg = cfg;
  }

  setMonitor(monitor: Monitor): void {
    this.monitor = monitor;
    this.env.scale = monitor.scale;
    this.env.height = monitor.h;
    this.renderer.resize(monitor.w, monitor.h);
  }

  get labelLayer(): Labels {
    return this.labels;
  }

  get idle(): boolean {
    return this.strokes.length === 0;
  }

  private local(t: number, arrived: number): number {
    // The least-delayed message defines the offset; later ones were held up
    // somewhere and must not drag the clock back.
    const offset = arrived - t;
    this.offsetSamples.push(offset);
    if (this.offsetSamples.length > 240) this.offsetSamples.shift();
    if (offset < this.clockOffset || this.offsetSamples.length % 60 === 0) {
      this.clockOffset = Math.min(...this.offsetSamples);
    }
    return t + this.clockOffset;
  }

  handle(batch: OverlayMsg[]): void {
    const arrived = this.now();
    const { x: ox, y: oy } = this.monitor;
    for (const m of batch) {
      if (m.k === 'down') {
        // More than a few unfinished strokes means something upstream went wrong.
        if (this.strokes.length > 6) this.strokes.shift();
        const t = this.local(m.t, arrived);
        this.strokes.push(new Stroke(m.id, t, m.x - ox, m.y - oy, m.lead_ms));
        this.intervals.length = 0;
        this.lastFrame = 0;
        continue;
      }
      const s = this.strokes.find((s) => s.id === m.id);
      if (!s) continue;
      if (m.k === 'move') {
        for (let i = 0; i + 2 < m.pts.length; i += 3) {
          s.add(Math.min(s.tDown + m.pts[i], arrived), m.pts[i + 1] - ox, m.pts[i + 2] - oy);
        }
      } else if (m.k === 'up') {
        const toLocal = (flat: number[]) => flat.map((v, i) => v - (i % 2 === 0 ? ox : oy));
        s.finish(m.result, arrived, this.env, toLocal);
        if (m.result.kind === 'match') {
          const a = s.labelAnchor(this.env);
          // Only the monitor that holds the shape shows the chip.
          if (a && a.x >= 0 && a.x < this.monitor.w && a.y >= 0 && a.y < this.monitor.h) {
            const dpr = this.monitor.scale;
            this.labels.show(a.x / dpr, a.y / dpr, a.above, m.result.label, m.result.icon, this.env.cfg.confirm_ms);
          }
        }
        this.report();
      } else if (m.k === 'cancel') {
        s.cancel(arrived, this.env);
      }
    }
    this.wake();
  }

  private wake(): void {
    if (!this.raf && this.strokes.length > 0) {
      this.raf = requestAnimationFrame(this.frame);
    }
  }

  private frame = (): void => {
    this.raf = 0;
    const now = this.now();
    if (this.lastFrame) this.intervals.push(now - this.lastFrame);
    this.lastFrame = now;
    this.draw(now);
    if (this.strokes.length > 0 || this.renderer.busy) {
      this.raf = requestAnimationFrame(this.frame);
    } else {
      this.lastFrame = 0;
    }
  };

  /** Draw one frame at time `now`. Public for the demo's frozen frames. */
  draw(now: number): void {
    this.renderer.begin();
    for (let i = this.strokes.length - 1; i >= 0; i--) {
      const s = this.strokes[i];
      // A stroke with no word from the engine for 8 s has lost its release.
      if (s.state === 'drawing' && now - s.lastSampleTime > 8000) s.cancel(now, this.env);
      this.geo.reset();
      const look = s.build(now, this.geo, this.env);
      if (look) this.renderer.draw(this.geo, look);
      if (s.done) this.strokes.splice(i, 1);
    }
    this.renderer.end();
  }

  /** Frame pacing while the last stroke was being drawn. */
  private report(): void {
    if (!this.onStats || this.intervals.length < 20) return;
    const sorted = [...this.intervals].sort((a, b) => a - b);
    const total = sorted.reduce((a, b) => a + b, 0);
    this.onStats({
      frames: sorted.length,
      seconds: total / 1000,
      fps: (sorted.length / total) * 1000,
      median: sorted[sorted.length >> 1],
      p99: sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.99))],
      worst: sorted[sorted.length - 1],
    });
  }
}
