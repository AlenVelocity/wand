// The landing page. The trail is the app's own overlay (src/overlay), on a
// canvas fixed over the whole page, fed by the mouse or a finger instead of a
// Joy-Con. Strokes are matched by a browser port of the app's recogniser.

import '@fontsource-variable/dm-sans/wght.css';
import '@fontsource/dm-mono/400.css';
import './styles.css';
import { perform } from '../src/overlay/demo';
import { LABEL_CSS } from '../src/overlay/label';
import { Overlay } from '../src/overlay/overlay';
import { BUILTINS, type Builtin } from '../src/shared/gestures';
import { DEFAULT_OVERLAY_CFG, type Monitor, type OverlayMsg, type Pt, type StrokeResult } from '../src/shared/types';
import { Recognizer, pathLength, resample } from './recognizer';

const $ = <T extends Element>(selector: string) => document.querySelector(selector) as T;
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
const coarse = matchMedia('(pointer: coarse)').matches;

// ------------------------------------------------------------ gesture list

const SVG = 'http://www.w3.org/2000/svg';

function glyph(shape: Pt[]): SVGSVGElement {
  const pts = shape.length > 8 ? shape : resample(shape, 48);
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('viewBox', '-0.62 -0.62 1.24 1.24');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(SVG, 'path');
  path.setAttribute('d', pts.map((p, i) => `${i ? 'L' : 'M'}${p[0].toFixed(3)} ${p[1].toFixed(3)}`).join(''));
  path.setAttribute('pathLength', '1');
  // Where the stroke starts, since for half of these that is the whole point.
  const dot = document.createElementNS(SVG, 'circle');
  dot.setAttribute('cx', shape[0][0].toFixed(3));
  dot.setAttribute('cy', shape[0][1].toFixed(3));
  dot.setAttribute('r', '0.055');
  svg.append(path, dot);
  return svg;
}

function row(g: Builtin, index: number): HTMLLIElement {
  const li = document.createElement('li');
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'shape';
  button.style.setProperty('--delay', `${Math.min(index, 9) * 45}ms`);
  button.dataset.id = g.id;

  const name = document.createElement('span');
  const title = document.createElement('b');
  title.textContent = g.name;
  const how = document.createElement('small');
  how.textContent = g.how;
  name.append(title, how);

  const does = document.createElement('span');
  does.className = 'does';
  does.textContent = g.label;
  if (g.action.type === 'key_combo') {
    const keys = document.createElement('kbd');
    keys.textContent = g.action.keys;
    does.append(keys);
  }

  button.append(glyph(g.variants[0]), name, does);
  li.append(button);
  return li;
}

const list = $<HTMLUListElement>('#shapes');
list.append(...BUILTINS.map(row));
if (reducedMotion || !('IntersectionObserver' in window)) {
  list.dataset.in = '';
} else {
  // Each shape draws itself once, the first time the list comes into view.
  const seen = new IntersectionObserver(
    (entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        list.dataset.in = '';
        seen.disconnect();
      }
    },
    { rootMargin: '0px 0px -12% 0px' },
  );
  seen.observe(list);
}

// ------------------------------------------------------------------ overlay

const canvas = $<HTMLCanvasElement>('#overlay');
const labels = $<HTMLElement>('#labels');
const hero = $<HTMLElement>('#stage');
const pad = $<HTMLElement>('#pad');
const readout = $<HTMLElement>('#readout');
const readoutText = readout.querySelector('span') as HTMLElement;

// Twice the CSS resolution is as sharp as a glow needs to be, and a 3x phone
// has better uses for the other half of its fill rate.
const dpr = Math.min(window.devicePixelRatio || 1, 2);
const measure = (): Monitor => ({ x: 0, y: 0, w: Math.round(innerWidth * dpr), h: Math.round(innerHeight * dpr), scale: dpr, primary: true });

function startOverlay(): Overlay | null {
  // Without motion the trail would be a still picture of itself.
  if (reducedMotion) return null;
  try {
    const style = document.createElement('style');
    style.textContent = LABEL_CSS;
    document.head.append(style);
    const fit = () => {
      canvas.style.width = `${innerWidth}px`;
      canvas.style.height = `${innerHeight}px`;
    };
    fit();
    const overlay = new Overlay(canvas, labels, measure(), DEFAULT_OVERLAY_CFG);
    addEventListener('resize', () => {
      fit();
      overlay.setMonitor(measure());
    });
    canvas.addEventListener('webglcontextlost', () => location.reload());
    return overlay;
  } catch {
    return null;
  }
}

const overlay = startOverlay();
if (!overlay) {
  document.documentElement.classList.add('no-live');
} else {
  live(overlay);
}

function live(overlay: Overlay): void {
  const recognizer = new Recognizer(BUILTINS);
  const send = (m: OverlayMsg) => overlay.handle([m]);
  let nextId = 1;
  let lastTouched = -Infinity;
  let drawing = false;

  const say = (text: string, kind: 'hint' | 'match' | 'miss') => {
    readoutText.textContent = text;
    readout.dataset.kind = kind;
  };
  if (coarse) say('Draw in the box with a finger. Start with a circle.', 'hint');

  // ------------------------------------------------- a visitor's own stroke

  function bind(surface: HTMLElement, accepts: (e: PointerEvent) => boolean): void {
    let id = 0;
    let pointer = -1;
    let t0 = 0;
    let pts: Pt[] = [];

    surface.addEventListener('pointerdown', (e) => {
      if (pointer !== -1 || !e.isPrimary || e.button !== 0 || !accepts(e)) return;
      if ((e.target as Element).closest('a, button')) return;
      // No text selection and no image drag while a stroke is being drawn.
      e.preventDefault();
      try {
        // Keeps the stroke alive when it leaves the surface or the window.
        surface.setPointerCapture(e.pointerId);
      } catch {
        // A pointer that cannot be captured still draws while it stays inside.
      }
      pointer = e.pointerId;
      drawing = true;
      lastTouched = performance.now();
      id = nextId++;
      t0 = performance.now();
      pts = [[e.clientX * dpr, e.clientY * dpr]];
      send({ k: 'down', id, t: t0, x: pts[0][0], y: pts[0][1], lead_ms: 4 });
      surface.dataset.used = '';
    });

    surface.addEventListener('pointermove', (e) => {
      if (e.pointerId !== pointer) return;
      // Coalesced events give the full input rate instead of one per frame.
      const events = e.getCoalescedEvents?.() ?? [];
      const flat: number[] = [];
      for (const ev of events.length ? events : [e]) {
        const p: Pt = [ev.clientX * dpr, ev.clientY * dpr];
        pts.push(p);
        flat.push(performance.now() - t0, p[0], p[1]);
      }
      send({ k: 'move', id, pts: flat });
    });

    const end = (e: PointerEvent) => {
      if (e.pointerId !== pointer) return;
      pointer = -1;
      drawing = false;
      lastTouched = performance.now();
      // The same floor the app applies to a mouse stroke.
      if (e.type === 'pointercancel' || pathLength(pts) < 90 * dpr || performance.now() - t0 < 60) {
        send({ k: 'cancel', id });
        return;
      }
      const verdict = recognizer.recognize(pts);
      let result: StrokeResult;
      if (verdict.kind === 'match') {
        const { gesture, score, from, to } = verdict.match;
        result = { kind: 'match', gesture: gesture.id, label: gesture.label, icon: gesture.icon, score, from, to };
        const keys = gesture.action.type === 'key_combo' ? `, ${gesture.action.keys}` : '';
        say(`${gesture.name}, ${score.toFixed(2)}. In the app: ${gesture.label}${keys}.`, 'match');
      } else {
        result = { kind: 'no_match', from: verdict.from };
        say(
          verdict.closest && verdict.score >= 0.5
            ? `No match. Closest was ${verdict.closest.name} at ${verdict.score.toFixed(2)}; it takes 0.80.`
            : 'No match. The shapes it knows are listed below.',
          'miss',
        );
      }
      send({ k: 'up', id, t: performance.now(), result });
    };
    surface.addEventListener('pointerup', end);
    surface.addEventListener('pointercancel', end);
  }

  // A mouse or pen can use the whole hero. A finger gets the pad, so the rest
  // of the page still scrolls under it.
  bind(hero, (e) => e.pointerType !== 'touch');
  bind(pad, () => true);

  // --------------------------------------------- the page drawing by itself

  let ghostBusy = false;
  let seed = 3;

  function ghost(g: Builtin, cx: number, cy: number, size: number, done?: () => void): void {
    if (ghostBusy || drawing) {
      done?.();
      return;
    }
    ghostBusy = true;
    const take = perform(g.variants[0], cx * dpr, cy * dpr, size * dpr, 125, seed++);
    const id = nextId++;
    const t0 = performance.now();
    let next = 1;
    send({ k: 'down', id, t: t0, x: take.samples[0][1], y: take.samples[0][2], lead_ms: DEFAULT_OVERLAY_CFG.prediction_ms });
    const pump = () => {
      const elapsed = performance.now() - t0;
      const flat: number[] = [];
      while (next < take.samples.length && take.samples[next][0] <= elapsed) {
        flat.push(...take.samples[next]);
        next++;
      }
      if (flat.length) send({ k: 'move', id, pts: flat });
      if (next < take.samples.length) {
        requestAnimationFrame(pump);
      } else {
        send({ k: 'up', id, t: t0 + elapsed, result: { kind: 'match', gesture: g.id, label: g.label, icon: g.icon, score: 0.93, from: take.from, to: take.to } });
        ghostBusy = false;
        done?.();
      }
    };
    requestAnimationFrame(pump);
  }

  /** Where there is room to draw: the pad on a phone, the open side of the hero otherwise. */
  function room(): { cx: number; cy: number; size: number } {
    const r = pad.getBoundingClientRect();
    if (r.width > 0) return { cx: r.left + r.width / 2, cy: r.top + r.height / 2, size: Math.min(r.width, r.height) * 0.62 };
    const wide = innerWidth >= 920;
    return {
      cx: innerWidth * (wide ? 0.79 : 0.5),
      cy: innerHeight * (wide ? 0.38 : 0.3),
      size: Math.min(innerWidth * (wide ? 0.26 : 0.5), innerHeight * 0.48, 400),
    };
  }

  list.addEventListener('click', (e) => {
    const button = (e.target as Element).closest<HTMLButtonElement>('.shape');
    const g = button && BUILTINS.find((b) => b.id === button.dataset.id);
    if (!g) return;
    lastTouched = performance.now();
    // Next to the list on a wide screen, over it on a narrow one.
    const wide = innerWidth >= 920;
    ghost(g, innerWidth * (wide ? 0.5 : 0.5), innerHeight * 0.5, Math.min(innerWidth * 0.5, innerHeight * 0.5, 400));
  });

  // The one thing on the page that moves unasked: every few seconds the hero
  // draws a gesture, until the visitor draws one or scrolls on.
  const tour = ['circle', 'flick-right', 'v', 'star', 'z', 'infinity', 'chevron', 's'];
  let stop = 0;
  const tick = () => {
    let started = false;
    const idle = performance.now() - lastTouched > 9000;
    const r = pad.getBoundingClientRect();
    // On a phone: most of the pad on screen. Otherwise: still at the top of the page.
    const shown = Math.min(r.bottom, innerHeight) - Math.max(r.top, 0);
    const inView = r.width > 0 ? shown >= r.height * 0.7 : scrollY < innerHeight * 0.3;
    if (idle && inView && !document.hidden) {
      const g = BUILTINS.find((b) => b.id === tour[stop % tour.length]);
      stop++;
      const { cx, cy, size } = room();
      if (g) {
        started = true;
        // The next one sets off once this one has matched and cleared.
        ghost(g, cx, cy, size, () => setTimeout(tick, DEFAULT_OVERLAY_CFG.confirm_ms + 1100));
      }
    }
    if (!started) setTimeout(tick, 1000);
  };
  setTimeout(tick, 700);
}
