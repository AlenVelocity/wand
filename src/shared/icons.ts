// One small icon set, drawn for this app: 24 px grid, 1.75 px round strokes.
// Used by the overlay's label (as an SVG string) and the settings window.

type Shape = string;

const speaker = '<path d="M4 9.5h3l4.5-4v13L7 14.5H4z"/>';

export const ICONS: Record<string, Shape> = {
  camera:
    '<path d="M4 8.5A2.5 2.5 0 0 1 6.5 6h1.2l1.1-1.6a1 1 0 0 1 .8-.4h4.8a1 1 0 0 1 .8.4L16.3 6h1.2A2.5 2.5 0 0 1 20 8.5v8a2.5 2.5 0 0 1-2.5 2.5h-11A2.5 2.5 0 0 1 4 16.5z"/><circle cx="12" cy="12.5" r="3.2"/>',
  close: '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="m9.5 9.5 5 5m0-5-5 5"/>',
  undo: '<path d="M8 5 4 9l4 4"/><path d="M4 9h9.5a5.5 5.5 0 0 1 0 11H11"/>',
  paste:
    '<path d="M9 4h6a1 1 0 0 1 1 1v2H8V5a1 1 0 0 1 1-1z"/><path d="M8 6H6.5A1.5 1.5 0 0 0 5 7.5v11A1.5 1.5 0 0 0 6.5 20h11a1.5 1.5 0 0 0 1.5-1.5v-11A1.5 1.5 0 0 0 17.5 6H16"/><path d="M9 12h6m-6 3.5h4"/>',
  copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2.2"/><path d="M15.5 8.5v-2a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/>',
  save: '<path d="M5 6.5A1.5 1.5 0 0 1 6.5 5h9.6a1.5 1.5 0 0 1 1.06.44l1.4 1.4A1.5 1.5 0 0 1 19 7.9v9.6a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 17.5z"/><path d="M8.5 5v3.5h6V5"/><path d="M8 19v-5.5h8V19"/>',
  switch: '<path d="M4.5 8.5H19"/><path d="m15.5 5 3.5 3.5-3.5 3.5"/><path d="M19.5 15.5H5"/><path d="M8.5 12 5 15.5 8.5 19"/>',
  windows: '<rect x="3.5" y="4.5" width="17" height="9.5" rx="2"/><rect x="3.5" y="16.5" width="7" height="3.5" rx="1.2"/><rect x="13.5" y="16.5" width="7" height="3.5" rx="1.2"/>',
  'volume-up': `${speaker}<path d="M15 9.5a3.5 3.5 0 0 1 0 5"/><path d="M17.5 7a7 7 0 0 1 0 10"/>`,
  'volume-down': `${speaker}<path d="M15 9.5a3.5 3.5 0 0 1 0 5"/>`,
  mute: `${speaker}<path d="m15.5 10 4 4m0-4-4 4"/>`,
  prev: '<path d="M6 5v14"/><path d="M19 6.5v11L10.5 12z"/>',
  next: '<path d="M18 5v14"/><path d="M5 6.5v11l8.5-5.5z"/>',
  play: '<path d="M5 5.5v13l8-6.5z"/><path d="M16 6v12m3.5-12v12"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="1.5"/>',
  app: '<path d="M14 5h5v5"/><path d="m19 5-8 8"/><path d="M10 6.5H7a2 2 0 0 0-2 2V17a2 2 0 0 0 2 2h8.5a2 2 0 0 0 2-2v-3"/>',
  terminal: '<rect x="3.5" y="5" width="17" height="14" rx="2.5"/><path d="m7.5 10 3 2.5-3 2.5"/><path d="M12.5 15h4"/>',
  key: '<rect x="4" y="6" width="16" height="12" rx="3"/><path d="M9 10.5h6m-6 3h3"/>',
  spark: '<path d="M12 3.5c.6 4.6 3.9 7.9 8.5 8.5-4.6.6-7.9 3.9-8.5 8.5-.6-4.6-3.9-7.9-8.5-8.5 4.6-.6 7.9-3.9 8.5-8.5z"/>',
};

export const ICON_NAMES = Object.keys(ICONS);

export function iconInner(name: string): string {
  return ICONS[name] ?? ICONS.spark;
}

export function iconSvg(name: string, size = 18): string {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${iconInner(name)}</svg>`;
}

/** The icon that fits an action when the user has not picked one. */
export function suggestIcon(action: { type: string; key?: string }): string {
  switch (action.type) {
    case 'media_key':
      return (
        { volume_up: 'volume-up', volume_down: 'volume-down', mute: 'mute', next_track: 'next', prev_track: 'prev', play_pause: 'play', stop: 'stop' }[
          action.key ?? ''
        ] ?? 'play'
      );
    case 'launch':
      return 'app';
    case 'shell':
      return 'terminal';
    case 'key_combo':
      return 'key';
    default:
      return 'spark';
  }
}
