// Draws the 1200 x 630 card that link previews show (public/og.png).
//
// Development only: `pnpm site`, open /og.html, and the card is rendered with
// the real overlay and the page's fonts, then saved by the dev server. It is
// not part of the built site.

import '@fontsource-variable/dm-sans/wght.css';
import '@fontsource/dm-mono/400.css';
import { perform } from '../src/overlay/demo';
import { Overlay } from '../src/overlay/overlay';
import { builtin } from '../src/shared/gestures';
import { iconSvg } from '../src/shared/icons';
import { DEFAULT_OVERLAY_CFG } from '../src/shared/types';

const W = 1200;
const H = 630;
const INK = '#f4f3f0';
const CORAL = '#ff5757';
const SANS = '"DM Sans Variable", sans-serif';

async function image(svg: string): Promise<HTMLImageElement> {
  const img = new Image();
  img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  await img.decode();
  return img;
}

async function draw(): Promise<void> {
  await Promise.all([document.fonts.load(`800 96px ${SANS}`), document.fonts.load(`600 24px ${SANS}`), document.fonts.load('400 22px "DM Mono"')]);

  // A circle drawn by the stand-in hand, frozen 110 ms after release: the
  // clean shape with the glint part-way round.
  const glCanvas = document.createElement('canvas');
  const overlay = new Overlay(glCanvas, document.createElement('div'), { x: 0, y: 0, w: W, h: H, scale: 1, primary: true }, DEFAULT_OVERLAY_CFG);
  const circle = builtin('circle')!;
  const take = perform(circle.variants[0], 925, 288, 360, 125, 3);
  let clock = 0;
  overlay.now = () => clock;
  overlay.handle([{ k: 'down', id: 1, t: 0, x: take.samples[0][1], y: take.samples[0][2], lead_ms: DEFAULT_OVERLAY_CFG.prediction_ms }]);
  let next = 1;
  let released = false;
  const stop = take.duration + 110;
  for (clock = 0; clock <= stop; clock += 1000 / 240) {
    const pts: number[] = [];
    while (next < take.samples.length && take.samples[next][0] <= clock) {
      pts.push(...take.samples[next]);
      next++;
    }
    if (pts.length) overlay.handle([{ k: 'move', id: 1, pts }]);
    if (!released && clock >= take.duration) {
      released = true;
      overlay.handle([{ k: 'up', id: 1, t: clock, result: { kind: 'match', gesture: 'circle', label: circle.label, icon: circle.icon, score: 0.93, from: take.from, to: take.to } }]);
    }
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

  c.fillStyle = INK;
  c.textBaseline = 'alphabetic';
  c.font = `800 34px ${SANS}`;
  c.letterSpacing = '-1px';
  c.fillText('Wand', 72, 104);
  const mark = c.measureText('Wand').width;
  c.fillStyle = CORAL;
  c.beginPath();
  c.arc(72 + mark + 10, 98, 5.5, 0, Math.PI * 2);
  c.fill();

  c.fillStyle = INK;
  c.font = `800 92px ${SANS}`;
  c.letterSpacing = '-3.7px';
  c.fillText('Draw in the air', 68, 372);
  c.fillText('with a Joy-Con.', 68, 462);

  c.fillStyle = '#a8a49c';
  c.font = '400 22px "DM Mono"';
  c.letterSpacing = '0px';
  c.fillText('wand.alen.is', 72, 548);

  // The chip that names the action, as the overlay shows it under a match.
  c.font = `600 24px ${SANS}`;
  c.letterSpacing = '-0.2px';
  const label = circle.label;
  const chipW = 20 + 28 + 12 + c.measureText(label).width + 26;
  const chipX = 925 - chipW / 2;
  const chipY = 506;
  c.fillStyle = '#000';
  c.beginPath();
  c.roundRect(chipX, chipY, chipW, 54, 27);
  c.fill();
  c.strokeStyle = 'rgba(244, 243, 240, 0.16)';
  c.lineWidth = 1.5;
  c.stroke();
  c.drawImage(await image(iconSvg(circle.icon, 28).replace('currentColor', CORAL)), chipX + 20, chipY + 13, 28, 28);
  c.fillStyle = INK;
  c.fillText(label, chipX + 60, chipY + 35);

  const blob = await new Promise<Blob | null>((resolve) => card.toBlob(resolve, 'image/png'));
  const res = blob ? await fetch('/__og', { method: 'POST', headers: { 'x-wand-og': '1' }, body: blob }) : null;
  document.title = res?.ok ? 'og.png saved' : 'og.png not saved';
  document.body.dataset.saved = String(!!res?.ok);
}

void draw();
