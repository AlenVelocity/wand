// The chip that names the action at a match: icon and label on true black.
//
// Support role in the choreography: it starts 60 ms after release, while the
// trail is still morphing, fades and rises 6 px in 140 ms (ease-out), and
// leaves by opacity alone, faster than it came. Transitions, not keyframes,
// so a second match that arrives mid-flight retargets cleanly.

import { iconSvg } from '../shared/icons';

const ENTER_MS = 140;
const EXIT_MS = 100;

export class Labels {
  constructor(
    private root: HTMLElement,
    private reducedMotion: boolean,
  ) {}

  /**
   * Show a chip centred on (x, y) in CSS pixels. `confirmMs` is the length of
   * the whole confirmation; the chip is gone when it ends.
   */
  show(x: number, y: number, above: boolean, label: string, icon: string, confirmMs: number): void {
    const el = document.createElement('div');
    el.className = 'chip';
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.dataset.above = String(above);
    el.innerHTML = `<span class="chip-icon">${iconSvg(icon, 16)}</span><span class="chip-text"></span>`;
    (el.lastElementChild as HTMLElement).textContent = label;
    this.root.appendChild(el);

    const scale = confirmMs / 400;
    const enterAt = 60 * scale;
    const exitAt = Math.max(enterAt + ENTER_MS, confirmMs - EXIT_MS);
    window.setTimeout(() => {
      // Read layout once so the transition has a starting point.
      void el.offsetWidth;
      el.dataset.state = 'in';
    }, enterAt);
    window.setTimeout(() => {
      el.dataset.state = 'out';
    }, exitAt);
    window.setTimeout(() => el.remove(), exitAt + EXIT_MS + 40);
    if (this.reducedMotion) el.dataset.still = 'true';
  }

  /** Demo and screenshots: put a chip in a given state with no timers. */
  freeze(x: number, y: number, above: boolean, label: string, icon: string, state: 'in' | 'out' | 'hidden'): void {
    this.root.replaceChildren();
    if (state === 'hidden') return;
    const el = document.createElement('div');
    el.className = 'chip';
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.style.transition = 'none';
    el.dataset.above = String(above);
    el.dataset.state = state;
    el.innerHTML = `<span class="chip-icon">${iconSvg(icon, 16)}</span><span class="chip-text"></span>`;
    (el.lastElementChild as HTMLElement).textContent = label;
    this.root.appendChild(el);
  }

  clear(): void {
    this.root.replaceChildren();
  }
}

export const LABEL_CSS = `
.chip {
  position: fixed;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  height: 34px;
  padding: 0 14px 0 11px;
  border-radius: 999px;
  background: #000;
  box-shadow: inset 0 0 0 1px rgba(244, 243, 240, 0.16);
  color: #f4f3f0;
  font: 600 14px/1 'DM Sans Variable', 'DM Sans', system-ui, sans-serif;
  letter-spacing: -0.01em;
  white-space: nowrap;
  opacity: 0;
  transform: translate(-50%, -50%) translateY(6px) scale(0.97);
  transition: opacity ${ENTER_MS}ms cubic-bezier(0.23, 1, 0.32, 1), transform ${ENTER_MS}ms cubic-bezier(0.23, 1, 0.32, 1);
  will-change: transform, opacity;
}
.chip[data-above='true'] { transform: translate(-50%, -50%) translateY(-6px) scale(0.97); }
.chip[data-state='in'] { opacity: 1; transform: translate(-50%, -50%); }
.chip[data-state='out'] {
  opacity: 0;
  transform: translate(-50%, -50%);
  transition: opacity ${EXIT_MS}ms cubic-bezier(0.23, 1, 0.32, 1);
}
.chip[data-still='true'] { transform: translate(-50%, -50%); }
.chip-icon { display: inline-flex; color: #ff5757; }
`;
