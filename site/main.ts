// The landing page. The trail is the app's own overlay (src/overlay), on a
// canvas fixed over the whole page, fed by the mouse or a finger instead of a
// Joy-Con. Strokes are matched by a browser port of the app's recogniser.

// The stylesheet and the fonts are linked from index.html, not imported here:
// a stylesheet that arrives with the script arrives after the first paint.

import { perform } from '../src/overlay/demo';
import { LABEL_CSS } from '../src/overlay/label';
import { Overlay } from '../src/overlay/overlay';
import { BUILTINS, type Builtin } from '../src/shared/gestures';
import { DEFAULT_OVERLAY_CFG, type Monitor, type OverlayMsg, type Pt, type StrokeResult } from '../src/shared/types';
import { Recognizer, pathLength, resample } from './recognizer';

const $ = <T extends Element>(selector: string) => document.querySelector(selector) as T;
const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
const coarse = matchMedia('(pointer: coarse)').matches;

// --------------------------------------------------------------- downloads

// The visitor's own system gets the bright button and goes first. Without
// the script, or anywhere but a Mac, Windows leads. (An iPad also says
// Macintosh, but has a touch screen.)
if (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints < 2) {
  for (const mac of document.querySelectorAll<HTMLAnchorElement>('a[data-os="mac"]')) {
    const win = mac.parentElement?.querySelector<HTMLAnchorElement>('a[data-os="windows"]');
    if (!win) continue;
    win.before(mac);
    mac.classList.remove('quiet');
    win.classList.add('quiet');
  }
}

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

// With a mouse, the overlay is a canvas fixed over the whole page. On a touch
// screen it lives inside the pad instead: a small canvas in the flow of the
// page. A transparent WebGL layer fixed over a scrolling page is the part
// phone browsers get wrong, and a phone's viewport changes size every time
// its address bar slides.
const inPad = coarse;

/** The canvas as it is laid out now, in the pixels the overlay draws in. */
const measure = (): Monitor => {
  const r = canvas.getBoundingClientRect();
  return { x: 0, y: 0, w: Math.max(1, Math.round(r.width * dpr)), h: Math.max(1, Math.round(r.height * dpr)), scale: dpr, primary: true };
};

/** A point on screen, in the canvas's own pixels, wherever the canvas is. */
const local = (clientX: number, clientY: number, r = canvas.getBoundingClientRect()): Pt => [(clientX - r.left) * dpr, (clientY - r.top) * dpr];

function startOverlay(): Overlay | null {
  try {
    const style = document.createElement('style');
    style.textContent = LABEL_CSS;
    document.head.append(style);
    if (inPad) {
      pad.append(canvas, labels);
      document.documentElement.classList.add('in-pad');
    }
    const overlay = new Overlay(canvas, labels, measure(), DEFAULT_OVERLAY_CFG, false);
    // Fires when the canvas's box really changes, not on every scroll.
    new ResizeObserver(() => overlay.setMonitor(measure())).observe(canvas);
    // A phone takes the GPU away from a tab in the background. Carry on
    // without the trail; reloading here could loop on a device short of memory.
    canvas.addEventListener('webglcontextlost', (e) => {
      e.preventDefault();
      document.documentElement.classList.add('no-live');
    });
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

  // The fixed canvas is pinned to the window, so a stroke on it would stay put
  // while the page scrolled away underneath. While anything is drawn, the
  // canvas rides with the page; once it is empty it snaps back over the window.
  let anchor = scrollY;
  const settle = () => {
    if (inPad) return;
    if (overlay.idle) anchor = scrollY;
    const shift = `translate3d(0, ${anchor - scrollY}px, 0)`;
    canvas.style.transform = shift;
    labels.style.transform = shift;
  };
  if (!inPad) addEventListener('scroll', settle, { passive: true });
  settle();

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
      settle();
      pts = [local(e.clientX, e.clientY)];
      send({ k: 'down', id, t: t0, x: pts[0][0], y: pts[0][1], lead_ms: 4 });
      surface.dataset.used = '';
      document.documentElement.dataset.drawn = '';
    });

    surface.addEventListener('pointermove', (e) => {
      if (e.pointerId !== pointer) return;
      // Coalesced events give the full input rate instead of one per frame.
      const events = e.getCoalescedEvents?.() ?? [];
      const flat: number[] = [];
      const box = canvas.getBoundingClientRect();
      for (const ev of events.length ? events : [e]) {
        const p = local(ev.clientX, ev.clientY, box);
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
  if (inPad) bind(pad, () => true);
  else bind(hero, (e) => e.pointerType !== 'touch');

  // --------------------------------------------- the page drawing by itself

  let ghostBusy = false;
  let seed = 3;

  function ghost(g: Builtin, cx: number, cy: number, size: number, done?: () => void): void {
    if (ghostBusy || drawing) {
      done?.();
      return;
    }
    ghostBusy = true;
    settle();
    const [x, y] = local(cx, cy);
    const take = perform(g.variants[0], x, y, size * dpr, 125, seed++);
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
    if (inPad) {
      // The trail is up in the pad and the list is down here, so on a phone
      // the row's own drawing plays again in place.
      const path = button.querySelector('path') as SVGPathElement;
      button.style.setProperty('--delay', '0ms');
      path.style.transition = 'none';
      path.style.strokeDashoffset = '1';
      void path.getBoundingClientRect();
      path.style.transition = '';
      path.style.strokeDashoffset = '';
      return;
    }
    // Large, over the middle of the window.
    ghost(g, innerWidth * 0.5, innerHeight * 0.5, Math.min(innerWidth * 0.5, innerHeight * 0.5, 400));
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
    if (document.documentElement.classList.contains('no-live')) return;
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
  // Someone who has asked for less motion gets the trail for their own
  // strokes, and nothing that moves by itself.
  if (reducedMotion) document.documentElement.classList.add('quiet');
  else setTimeout(tick, 700);
}
