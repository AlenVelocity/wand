import '@fontsource-variable/dm-sans/wght.css';
import { DEFAULT_OVERLAY_CFG, type Monitor, type OverlayCfg, type OverlayMsg } from '../shared/types';
import { LABEL_CSS } from './label';
import { Overlay } from './overlay';

const style = document.createElement('style');
style.textContent = LABEL_CSS;
document.head.appendChild(style);

const canvas = document.getElementById('stage') as HTMLCanvasElement;
const labels = document.getElementById('labels') as HTMLElement;
const make = (monitor: Monitor, cfg: OverlayCfg) => new Overlay(canvas, labels, monitor, cfg);

async function startInApp(): Promise<void> {
  const { invoke, Channel } = await import('@tauri-apps/api/core');
  const { listen } = await import('@tauri-apps/api/event');
  const init = await invoke<{ monitor: Monitor; cfg: OverlayCfg }>('overlay_init');
  const overlay = make(init.monitor, init.cfg);
  // A lost GPU context (driver update, GPU process restart) leaves a dead
  // canvas. Starting the page over is the whole recovery.
  canvas.addEventListener('webglcontextlost', () => location.reload());

  // Frame pacing of each stroke goes to the app's log: the only honest way
  // to know the overlay really runs at the display's refresh rate.
  overlay.onStats = (stats) => {
    void invoke('overlay_report', {
      stats: { ...stats, window: [innerWidth, innerHeight, devicePixelRatio], monitor: [init.monitor.w, init.monitor.h] },
    });
  };

  const channel = new Channel<OverlayMsg[]>();
  channel.onmessage = (batch) => overlay.handle(batch);
  await invoke('overlay_subscribe', { channel });
  await listen<OverlayCfg>('wand://overlay-config', (e) => overlay.setConfig(e.payload));
}

if ('__TAURI_INTERNALS__' in window) {
  // The window can finish loading a moment before the app has registered its
  // display; try again instead of staying blank.
  const start = (attempt: number): void => {
    startInApp().catch((e) => {
      console.error('overlay failed to start', e);
      if (attempt < 5) setTimeout(() => start(attempt + 1), 400);
    });
  };
  start(0);
} else {
  void import('./demo').then(({ startDemo }) => startDemo(make, DEFAULT_OVERLAY_CFG));
}
