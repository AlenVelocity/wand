import { useCallback, useEffect, useRef, useState } from 'react';
import type { DeviceStatus, Outcome, Snapshot } from '../shared/types';
import { backend } from './api';
import { Device } from './Device';
import { Gestures } from './Gestures';
import { SettingsPage } from './SettingsPage';

type Page = 'gestures' | 'device' | 'settings';
const PAGES: [Page, string][] = [
  ['gestures', 'Gestures'],
  ['device', 'Device'],
  ['settings', 'Settings'],
];

const LINK_WORDS: Record<string, string> = {
  bluetooth_off: 'Bluetooth off',
  searching: 'No controller',
  connecting: 'Connecting',
  pairing: 'Pairing',
  connected: 'Connected',
  reconnecting: 'Reconnecting',
};

export function App() {
  const initial = new URLSearchParams(location.search).get('page') as Page | null;
  const [page, setPage] = useState<Page>(initial ?? 'gestures');
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [toastText, setToastText] = useState('');
  const [toastOn, setToastOn] = useState(false);
  const toastTimer = useRef(0);

  const toast = useCallback((text: string) => {
    setToastText(text);
    setToastOn(true);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToastOn(false), 3600);
  }, []);

  useEffect(() => {
    let offs: (() => void)[] = [];
    let alive = true;
    void (async () => {
      const b = await backend();
      const refresh = async () => {
        const s = await b.snapshot();
        if (alive) setSnap(s);
      };
      await refresh();
      const subs = await Promise.all([
        b.on('wand://changed', () => void refresh()),
        b.on<DeviceStatus>('wand://status', (status) => setSnap((s) => (s ? { ...s, status } : s))),
        b.on<Outcome>('wand://outcome', (o) => setSnap((s) => (s ? { ...s, history: [o, ...s.history].slice(0, 30) } : s))),
        b.on<boolean>('wand://replay', (replaying) => setSnap((s) => (s ? { ...s, replaying } : s))),
        b.on<string>('wand://action-error', (message) => toast(message)),
      ]);
      if (alive) offs = subs;
      else subs.forEach((off) => off());
    })();
    return () => {
      alive = false;
      offs.forEach((off) => off());
    };
  }, [toast]);

  if (!snap) return null;
  const s = snap.status;
  const connected = s.state === 'connected';

  return (
    <div className="shell">
      <aside className="rail">
        <div className="wordmark">
          Wand
          <i aria-hidden="true" />
        </div>
        <nav className="nav" aria-label="Pages">
          {PAGES.map(([id, label]) => (
            <button key={id} type="button" aria-current={page === id ? 'page' : undefined} onClick={() => setPage(id)}>
              {label}
              <svg viewBox="0 0 100 7" preserveAspectRatio="none" aria-hidden="true">
                <path d="M1 4.6 C 22 1.2, 46 6.4, 68 3.2 S 92 2.2, 99 4" fill="none" stroke="var(--coral)" strokeWidth="2.2" strokeLinecap="round" vectorEffect="non-scaling-stroke" />
              </svg>
            </button>
          ))}
        </nav>

        <div className="rail-foot">
          <div className="row wrap" style={{ gap: 6 }}>
            {snap.mode === 'paused' && <span className="tag" data-tone="coral">Paused</span>}
            {snap.config.dev.enabled && <span className="tag">Dev mode</span>}
            {snap.dry_run && <span className="tag">Dry run</span>}
            {snap.replaying && <span className="tag">Replay</span>}
          </div>
          <div className="state">
            <span className="dot" data-on={connected} />
            {connected ? s.name : LINK_WORDS[s.state]}
          </div>
          {connected && s.battery_percent !== null && (
            <span className="mono muted">
              {s.battery_percent}%{s.charging ? ' charging' : ''}
            </span>
          )}
        </div>
      </aside>

      <main className="page" key={page}>
        {page === 'gestures' && <Gestures snap={snap} />}
        {page === 'device' && <Device snap={snap} toast={toast} />}
        {page === 'settings' && <SettingsPage snap={snap} toast={toast} />}
      </main>

      <div className="toast" role="status" data-show={toastOn}>
        {toastText}
      </div>
    </div>
  );
}
