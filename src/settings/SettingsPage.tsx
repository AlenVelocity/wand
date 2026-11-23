import { useEffect, useState } from 'react';
import type { RecordingInfo, Snapshot } from '../shared/types';
import { backend } from './api';
import { Setting, Switch } from './ui';

const ANCHORS: [string, string][] = [
  ['pointing', 'Where I point'],
  ['cursor', 'At the cursor'],
  ['center', 'Screen centre'],
];

/** A slider that shows its value live and writes wand.toml when released. */
function Range({ value, min, max, step, unit, digits = 0, onCommit }: { value: number; min: number; max: number; step: number; unit: string; digits?: number; onCommit(v: number): void }) {
  const [local, setLocal] = useState(value);
  useEffect(() => setLocal(value), [value]);
  return (
    <>
      <input
        type="range"
        className="slider"
        min={min}
        max={max}
        step={step}
        value={local}
        onChange={(e) => setLocal(Number(e.target.value))}
        onPointerUp={() => local !== value && onCommit(local)}
        onKeyUp={() => local !== value && onCommit(local)}
      />
      <span className="mono">
        {local.toFixed(digits)}
        {unit}
      </span>
    </>
  );
}

const KIND_WORDS: Record<string, string> = { match: 'Match', no_match: 'No match', too_short: 'Too short', too_long: 'Held too long', captured: 'Sample' };

export function SettingsPage({ snap, toast }: { snap: Snapshot; toast(text: string): void }) {
  const cfg = snap.config;
  const left = snap.status.side === 'left';
  const [recordings, setRecordings] = useState<RecordingInfo[]>([]);

  const set = async (section: string, key: string, value: boolean | number | string) => {
    try {
      await (await backend()).setConfigValue(section, key, value);
    } catch (e) {
      toast(String(e));
    }
  };

  useEffect(() => {
    void backend()
      .then((b) => b.listRecordings())
      .then(setRecordings);
  }, [snap.imu_recording, snap.replaying]);

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Settings</h1>
          <p>
            Everything here is a line in <span className="mono">wand.toml</span>. Edit the file directly and the app picks it up.
          </p>
        </div>
        <button type="button" className="btn" onClick={async () => void (await backend()).openConfig()}>
          Open wand.toml
        </button>
      </header>

      {snap.config_error && (
        <div className="banner">
          <b>Config problem</b>
          <span>{snap.config_error} Defaults are in use until it is fixed.</span>
        </div>
      )}

      <div className="settings">
        <Setting title="Start with Windows" help="Starts hidden, in the tray.">
          <Switch checked={snap.autostart} label="Start with Windows" onChange={async (v) => (await backend()).setAutostart(v).catch((e) => toast(String(e)))} />
        </Setting>
        <Setting title="Pause" help="No strokes and no mouse until you turn this off. Also in the tray menu.">
          <Switch checked={snap.mode === 'paused'} label="Pause" onChange={async (v) => void (await backend()).setMode(v ? 'paused' : 'normal')} />
        </Setting>
        <Setting title="Rumble" help="A tick when a gesture matches, a double pulse when it does not.">
          <Switch checked={cfg.rumble.enabled} label="Rumble" onChange={(v) => void set('rumble', 'enabled', v)} />
        </Setting>
      </div>

      <section className="section">
        <h2>Pointing</h2>
        <div className="settings">
          <Setting title="Where the ember appears" help="Strokes are always drawn relative to where they start; this only places the start.">
            <div className="seg" role="group" aria-label="Where the ember appears">
              {ANCHORS.map(([value, label]) => (
                <button key={value} type="button" aria-pressed={cfg.pointing.anchor === value} onClick={() => void set('pointing', 'anchor', value)}>
                  {label}
                </button>
              ))}
            </div>
          </Setting>
          <Setting title="Sensitivity" help="Screen pixels per degree of wrist rotation. Higher draws bigger with less movement.">
            <Range value={cfg.pointing.px_per_degree} min={15} max={110} step={1} unit=" px/°" onCommit={(v) => void set('pointing', 'px_per_degree', v)} />
          </Setting>
          <Setting title="Mirror left and right" help="Turn on if the trail moves against your hand sideways.">
            <Switch checked={cfg.pointing.invert_x} label="Mirror left and right" onChange={(v) => void set('pointing', 'invert_x', v)} />
          </Setting>
          <Setting title="Mirror up and down">
            <Switch checked={cfg.pointing.invert_y} label="Mirror up and down" onChange={(v) => void set('pointing', 'invert_y', v)} />
          </Setting>
        </div>
      </section>

      <section className="section">
        <h2>Mouse</h2>
        <div className="settings">
          <Setting
            title="Use it as a mouse"
            help={`Lay the Joy-Con on its side, rail down, on a desk, a leg or the sofa. The sensor in the rail moves the cursor, ${left ? 'L' : 'R'} is left click, ${left ? 'ZL' : 'ZR'} is right click and the stick scrolls. Lift it and it is a wand again.`}
          >
            <Switch checked={cfg.mouse.enabled} label="Use it as a mouse" onChange={(v) => void set('mouse', 'enabled', v)} />
          </Setting>
          <Setting title="Cursor speed" help="Windows applies its own pointer speed on top of this.">
            <Range value={cfg.mouse.sensitivity} min={0.3} max={3} step={0.1} unit="×" digits={1} onCommit={(v) => void set('mouse', 'sensitivity', Number(v.toFixed(1)))} />
          </Setting>
          <Setting title="Scroll speed" help="Wheel notches a second with the stick pushed all the way. 0 turns stick scrolling off.">
            <Range value={cfg.mouse.scroll_speed} min={0} max={40} step={1} unit="/s" onCommit={(v) => void set('mouse', 'scroll_speed', v)} />
          </Setting>
          <Setting title="Glide" help="The controller reports about 68 times a second. This spreads each report over the frames in between, so the cursor does not hop on a fast display. It adds up to 15 ms of delay.">
            <Switch checked={cfg.mouse.smoothing_ms > 0} label="Glide" onChange={(v) => void set('mouse', 'smoothing_ms', v ? 15.0 : 0.0)} />
          </Setting>
        </div>
      </section>

      <section className="section">
        <h2>Recognition</h2>
        <div className="settings">
          <Setting title="How strict" help="The score a stroke needs to count as a gesture. Lower forgives sloppier strokes and fires by accident more often.">
            <Range value={cfg.recognizer.min_score} min={0.6} max={0.95} step={0.01} unit="" digits={2} onCommit={(v) => void set('recognizer', 'min_score', Number(v.toFixed(2)))} />
          </Setting>
          <Setting title="Lead between gestures" help="How far the winner must be ahead of the next-best gesture. Raise it if two of yours get confused.">
            <Range value={cfg.recognizer.min_margin} min={0} max={0.2} step={0.01} unit="" digits={2} onCommit={(v) => void set('recognizer', 'min_margin', Number(v.toFixed(2)))} />
          </Setting>
        </div>
      </section>

      <section className="section">
        <h2>Overlay</h2>
        <div className="settings">
          <Setting title="Prediction" help="How far ahead of the newest sample the ember is drawn, to hide Bluetooth latency. Too much and it overshoots on sharp turns.">
            <Range value={cfg.overlay.prediction_ms} min={0} max={30} step={1} unit=" ms" onCommit={(v) => void set('overlay', 'prediction_ms', v)} />
          </Setting>
          <Setting title="Confirmation length" help="The whole match or no-match animation. The action never waits for it.">
            <Range value={cfg.overlay.confirm_ms} min={200} max={900} step={20} unit=" ms" onCommit={(v) => void set('overlay', 'confirm_ms', v)} />
          </Setting>
          <Setting title="Sparks" help="Shed off fast movement.">
            <Switch checked={cfg.overlay.spark_rate > 0} label="Sparks" onChange={(v) => void set('overlay', 'spark_rate', v ? 1.0 : 0.0)} />
          </Setting>
        </div>
      </section>

      <section className="section">
        <h2>Dev mode</h2>
        <div className="settings">
          <Setting title="Draw with the mouse" help={`Hold ${cfg.dev.hold_key} and move the mouse: it goes through the same recogniser and overlay as the wand. Change the key with dev.hold_key in wand.toml.`}>
            <Switch checked={cfg.dev.enabled} label="Dev mode" onChange={(v) => void set('dev', 'enabled', v)} />
          </Setting>
          <Setting title="Dry run" help="Recognise and show the confirmation, but log the action instead of running it.">
            <Switch checked={cfg.dev.dry_run} label="Dry run" onChange={(v) => void set('dev', 'dry_run', v)} />
          </Setting>
          <Setting
            title="Record controller traffic"
            help={snap.imu_recording ? `Writing ${snap.imu_recording}` : 'Saves every report the controller sends to a .wandrec file you can replay later.'}
          >
            <button
              type="button"
              className="btn small"
              onClick={async () => {
                const b = await backend();
                try {
                  if (snap.imu_recording) toast(`Saved ${await b.imuRecordStop()}`);
                  else await b.imuRecordStart();
                } catch (e) {
                  toast(String(e));
                }
              }}
            >
              {snap.imu_recording ? 'Stop' : 'Record'}
            </button>
          </Setting>
        </div>

        {recordings.length > 0 && (
          <ul className="history" style={{ marginTop: 8 }}>
            {recordings.map((r) => (
              <li key={r.path}>
                <span className="mono">{r.source}</span>
                <span>{r.name}</span>
                <span className="mono">
                  {r.seconds.toFixed(1)} s · {r.reports}
                </span>
                <button
                  type="button"
                  className="btn small quiet"
                  title="Feeds the recording through the pipeline at its recorded pace. Actions are not run."
                  onClick={async () => (await backend()).replayFile(r.path, false).catch((e) => toast(String(e)))}
                >
                  Replay
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="row" style={{ marginTop: 14 }}>
          <button type="button" className="btn small quiet" onClick={async () => void (await backend()).openDataDir()}>
            Open data folder
          </button>
          {snap.replaying && (
            <button type="button" className="btn small" onClick={async () => void (await backend()).replayStop()}>
              Stop replay
            </button>
          )}
        </div>
      </section>

      <section className="section">
        <h2>Recent strokes</h2>
        {snap.history.length === 0 ? (
          <p className="empty">Nothing drawn yet. Scores show up here, which helps when tuning how strict recognition is.</p>
        ) : (
          <ul className="history">
            {snap.history.slice(0, 12).map((o, i) => (
              <li key={i}>
                <span className={o.kind === 'match' ? 'coral' : 'muted'}>{KIND_WORDS[o.kind] ?? o.kind}</span>
                <span>{o.gesture ?? (o.closest ? `closest: ${o.closest}` : '')}</span>
                <span className="mono">{o.kind === 'match' || o.kind === 'no_match' ? o.score.toFixed(2) : ''}</span>
                <span className="mono">{o.source}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <p className="note" style={{ marginTop: 36 }}>
        Wand {snap.version} · <span className="mono">{snap.config_path}</span>
      </p>
    </>
  );
}
