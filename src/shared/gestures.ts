// The built-in gestures as the web side knows them: the browser stand-ins for
// the app (overlay demo, settings mock) and the landing page, which matches
// strokes against them for real.
//
// A mirror of `defaults()` in src-tauri/src/gestures.rs. The app itself never
// reads this file: it gets its library from Rust. If a shape changes there,
// change it here.

import type { Action, Pt } from './types';

export interface Builtin {
  id: string;
  name: string;
  /** Matches at any rotation and from any starting point. */
  free: boolean;
  action: Action;
  label: string;
  icon: string;
  /** How to draw it, in a few words. */
  how: string;
  /** Each way of drawing it: corners or a dense curve, y down, about one unit across. */
  variants: Pt[][];
}

const TAU = Math.PI * 2;

const circle = (dir: 1 | -1): Pt[] =>
  Array.from({ length: 97 }, (_, i) => {
    const a = -Math.PI / 2 + (dir * i * TAU) / 96;
    return [Math.cos(a) * 0.5, Math.sin(a) * 0.5];
  });

/** A five-pointed star in one stroke from the top point. */
const star = (clockwise: boolean): Pt[] =>
  Array.from({ length: 6 }, (_, i) => {
    const a = ((-90 + 72 * ((i * (clockwise ? 2 : 3)) % 5)) * Math.PI) / 180;
    return [Math.cos(a) * 0.5, Math.sin(a) * 0.5];
  });

/** A figure eight on its side, from the crossing. */
const infinity = (sx: number, sy: number): Pt[] =>
  Array.from({ length: 97 }, (_, i) => {
    const t = (i / 96) * TAU;
    return [sx * 0.5 * Math.sin(t), sy * 0.25 * Math.sin(2 * t)];
  });

/** An S from its top right end. */
const sCurve = (): Pt[] => {
  const arc = (cy: number, from: number, to: number): Pt[] =>
    Array.from({ length: 25 }, (_, i) => {
      const a = ((from + ((to - from) * i) / 24) * Math.PI) / 180;
      return [Math.cos(a) * 0.25, cy + Math.sin(a) * 0.25];
    });
  return [...arc(-0.25, -30, -270), ...arc(0.25, -90, 210)];
};

const keys = (k: string): Action => ({ type: 'key_combo', keys: k });
const media = (key: string, repeat: number): Action => ({ type: 'media_key', key, repeat });

export const BUILTINS: Builtin[] = [
  { id: 'circle', name: 'Circle', free: true, action: keys('Win+PrintScreen'), label: 'Screenshot', icon: 'camera', how: 'either way round', variants: [circle(1), circle(-1)] },
  {
    id: 'slash',
    name: 'Slash',
    free: false,
    action: keys('Alt+F4'),
    label: 'Close window',
    icon: 'close',
    how: 'a diagonal, down or up',
    variants: [
      [[0.5, -0.5], [-0.5, 0.5]],
      [[-0.5, 0.5], [0.5, -0.5]],
    ],
  },
  { id: 'z', name: 'Z', free: false, action: keys('Ctrl+Z'), label: 'Undo', icon: 'undo', how: 'as written', variants: [[[-0.5, -0.4], [0.5, -0.4], [-0.5, 0.4], [0.5, 0.4]]] },
  {
    id: 'v',
    name: 'V',
    free: false,
    action: keys('Ctrl+V'),
    label: 'Paste',
    icon: 'paste',
    how: 'as written, or a tick',
    variants: [
      [[-0.42, -0.5], [0, 0.5], [0.42, -0.5]],
      [[-0.3, -0.05], [-0.05, 0.5], [0.42, -0.5]],
    ],
  },
  { id: 'flick-up', name: 'Flick up', free: false, action: media('volume_up', 3), label: 'Volume up', icon: 'volume-up', how: 'a straight line', variants: [[[0, 0.5], [0, -0.5]]] },
  { id: 'flick-down', name: 'Flick down', free: false, action: media('volume_down', 3), label: 'Volume down', icon: 'volume-down', how: 'a straight line', variants: [[[0, -0.5], [0, 0.5]]] },
  { id: 'flick-left', name: 'Flick left', free: false, action: media('prev_track', 1), label: 'Previous track', icon: 'prev', how: 'a straight line', variants: [[[0.5, 0], [-0.5, 0]]] },
  { id: 'flick-right', name: 'Flick right', free: false, action: media('next_track', 1), label: 'Next track', icon: 'next', how: 'a straight line', variants: [[[-0.5, 0], [0.5, 0]]] },
  { id: 'chevron', name: 'Chevron', free: false, action: media('play_pause', 1), label: 'Play / pause', icon: 'play', how: 'top to bottom', variants: [[[-0.3, -0.5], [0.3, 0], [-0.3, 0.5]]] },
  { id: 'caret', name: 'Caret', free: false, action: keys('Ctrl+C'), label: 'Copy', icon: 'copy', how: 'left to right', variants: [[[-0.42, 0.5], [0, -0.5], [0.42, 0.5]]] },
  {
    id: 'infinity',
    name: 'Infinity',
    free: false,
    action: keys('Alt+Tab'),
    label: 'Switch window',
    icon: 'switch',
    how: 'from the crossing',
    variants: [infinity(1, -1), infinity(-1, -1), infinity(1, 1), infinity(-1, 1)],
  },
  { id: 'star', name: 'Star', free: true, action: keys('Win+Tab'), label: 'Task view', icon: 'windows', how: 'five points, one stroke', variants: [star(true), star(false)] },
  { id: 'm', name: 'M', free: false, action: media('mute', 1), label: 'Mute', icon: 'mute', how: 'as written', variants: [[[-0.5, 0.5], [-0.28, -0.5], [0, 0.2], [0.28, -0.5], [0.5, 0.5]]] },
  { id: 's', name: 'S', free: false, action: keys('Ctrl+S'), label: 'Save', icon: 'save', how: 'from the top right', variants: [sCurve()] },
];

export const builtin = (id: string): Builtin | undefined => BUILTINS.find((b) => b.id === id);
