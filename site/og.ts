// Draws the 1200 x 630 card that link previews show (public/og.png).
//
// Development only: `pnpm site`, open /og.html, and the card is rendered with
// the real overlay and the page's fonts, then saved by the dev server. It is
// not part of the built site.
//
// Three things on black and nothing else: the name, one line saying what it
// is, and a stroke caught while it is still being drawn. The preview's own
// title and address sit next to the image, so the card repeats neither.

import '@fontsource-variable/dm-sans/wght.css';
import { perform } from '../src/overlay/demo';
import { Overlay } from '../src/overlay/overlay';
import { builtin } from '../src/shared/gestures';
import { DEFAULT_OVERLAY_CFG } from '../src/shared/types';

const W = 1200;
const H = 630;
const INK = '#f4f3f0';
const MUTED = '#a8a49c';
const CORAL = '#ff5757';
const SANS = '"DM Sans Variable", sans-serif';

/** How far through the stroke the hand is when the card is taken. */
const CAUGHT_AT = 0.74;

/**
 * The card is looked at small, a third of this size in most previews, so the
 * stroke is drawn as it would be on a display of this pixel density. Sparks
 * are left out: in a still they read as specks.
 */
const DENSITY = 1.8;
const CFG = { ...DEFAULT_OVERLAY_CFG, spark_rate: 0 };

async function draw(): Promise<void> {
  await Promise.all([document.fonts.load(`800 128px ${SANS}`), document.fonts.load(`500 33px ${SANS}`)]);

  // A circle drawn by the stand-in hand and stopped before it closes: the
  // tail thinning out behind, the ember at the head.
  const glCanvas = document.createElement('canvas');
  const overlay = new Overlay(glCanvas, document.createElement('div'), { x: 0, y: 0, w: W, h: H, scale: DENSITY, primary: true }, CFG);
  const circle = builtin('circle')!;
  const take = perform(circle.variants[0], 896, 315, 396, 125, 3);
  let clock = 0;
  overlay.now = () => clock;
  overlay.handle([{ k: 'down', id: 1, t: 0, x: take.samples[0][1], y: take.samples[0][2], lead_ms: CFG.prediction_ms }]);
  let next = 1;
  const stop = take.duration * CAUGHT_AT;
  for (clock = 0; clock <= stop; clock += 1000 / 240) {
    const pts: number[] = [];
    while (next < take.samples.length && take.samples[next][0] <= clock) {
      pts.push(...take.samples[next]);
      next++;
    }
    if (pts.length) overlay.handle([{ k: 'move', id: 1, pts }]);
    overlay.draw(clock);
  }

  const card = document.querySelector('canvas') as HTMLCanvasElement;
  card.width = W;
  card.height = H;
  const c = card.getContext('2d')!;
  c.fillStyle = '#000';
  c.fillRect(0, 0, W, H);
  // Straight after the last frame, while the GL canvas still holds it.
  c.drawImage(glCanvas, 0, 0);

  c.textBaseline = 'alphabetic';
  c.fillStyle = INK;
  c.font = `800 128px ${SANS}`;
  c.letterSpacing = '-5px';
  c.fillText('Wand', 84, 338);
  const mark = c.measureText('Wand').width;
  c.fillStyle = CORAL;
  c.beginPath();
  c.arc(84 + mark + 20, 322, 14, 0, Math.PI * 2);
  c.fill();

  c.fillStyle = MUTED;
  c.font = `500 33px ${SANS}`;
  c.letterSpacing = '-0.5px';
  c.fillText('Draw in the air with a Joy-Con.', 91, 400);

  const blob = await new Promise<Blob | null>((resolve) => card.toBlob(resolve, 'image/png'));
  const res = blob ? await fetch('/__og', { method: 'POST', headers: { 'x-wand-og': '1' }, body: blob }) : null;
  document.title = res?.ok ? 'og.png saved' : 'og.png not saved';
  document.body.dataset.saved = String(!!res?.ok);
}

void draw();
