import { useEffect, useMemo, useRef, useState } from 'react';
import type { Gesture, Pt, Snapshot } from '../shared/types';
import { ActionEditor, Keys } from './ActionEditor';
import { backend, describeAction } from './api';
import { Recorder } from './Recorder';
import { drawCardFrame, onFrame, resample } from './strokes';
import { Dialog, Icon, Setting, Switch } from './ui';

function Card({ gesture, index, mediaKeys, onOpen }: { gesture: Gesture; index: number; mediaKeys: [string, string][]; onOpen(): void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const hovered = useRef(false);
  const shape = useMemo<Pt[]>(() => resample(gesture.variants[0]?.ideal ?? [], 64), [gesture]);

  useEffect(() => {
    // Offsets from a golden-ratio sequence: no two cards share a phase.
    const offset = ((index * 0.618034) % 1) * 3330;
    return onFrame((now) => {
      if (canvas.current) drawCardFrame(canvas.current, shape, now, offset, hovered.current);
    });
  }, [shape, index]);

  const a = gesture.action;
  const label = gesture.label || describeAction(a, mediaKeys);
  // The second, quieter part: the keys themselves, a repeat count, or the
  // target, whichever the label does not already say.
  const detail =
    a.type === 'key_combo' ? a.keys : a.type === 'media_key' ? (a.repeat > 1 ? `×${a.repeat}` : '') : gesture.label ? describeAction(a, mediaKeys) : '';
  return (
    <button
      type="button"
      className="card"
      data-off={!gesture.enabled}
      onClick={onOpen}
      onPointerEnter={() => (hovered.current = true)}
      onPointerLeave={() => (hovered.current = false)}
    >
      <canvas ref={canvas} aria-hidden="true" />
      {!gesture.enabled && <span className="tag">Off</span>}
      <span className="card-body">
        <span className="card-name">{gesture.name}</span>
        <span className="card-action">
          <Icon name={gesture.icon || 'spark'} size={15} />
          <span>{label}</span>
          {detail && detail !== label && <span className="mono">{detail}</span>}
        </span>
      </span>
    </button>
  );
}

function Editor({ gesture, snap, onDone, onRerecord }: { gesture: Gesture; snap: Snapshot; onDone(): void; onRerecord(): void }) {
  const [draft, setDraft] = useState<Gesture>(gesture);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const connected = snap.status.state === 'connected';

  const save = async () => {
    setBusy(true);
    setError('');
    try {
      await (await backend()).saveGesture(draft);
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div className="dialog-head">
        <div>
          <h2>{gesture.name}</h2>
          <p className="note" style={{ marginTop: 4 }}>
            {gesture.builtin ? 'Built in.' : 'Recorded by you.'} {draft.action.type === 'key_combo' && draft.action.keys && <Keys combo={draft.action.keys} />}
          </p>
        </div>
        <button type="button" className="btn quiet small" onClick={onDone}>
          Close
        </button>
      </div>

      <div className="stack">
        <label className="field">
          <span>Name</span>
          <input className="input" value={draft.name} maxLength={32} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
        </label>

        <ActionEditor
          action={draft.action}
          label={draft.label}
          icon={draft.icon}
          mediaKeys={snap.media_keys}
          onChange={(next) => setDraft({ ...draft, ...next })}
        />

        <div className="settings">
          <Setting title="On" help="Off keeps the gesture in the library without matching it.">
            <Switch checked={draft.enabled} onChange={(enabled) => setDraft({ ...draft, enabled })} label="Gesture on" />
          </Setting>
          <Setting title="Match at any angle" help="For closed shapes like a circle: any starting point, any rotation. Leave off for shapes whose direction matters.">
            <Switch checked={draft.rotation === 'free'} onChange={(free) => setDraft({ ...draft, rotation: free ? 'free' : 'bounded' })} label="Match at any angle" />
          </Setting>
        </div>
        {error && <p className="error">{error}</p>}
      </div>

      <div className="dialog-foot">
        <div className="row">
          <button
            type="button"
            className="btn small"
            title="Plays this gesture through the motion pipeline with generated sensor data. The action is not run."
            onClick={async () => {
              onDone();
              await (await backend()).replayGesture(gesture.id);
            }}
          >
            Play on screen
          </button>
          <button type="button" className="btn small" onClick={onRerecord} title={connected || snap.config.dev.enabled ? '' : 'Needs a controller, or dev mode'}>
            Record again
          </button>
          {!gesture.builtin && (
            <button
              type="button"
              className="btn small quiet danger"
              onClick={async () => {
                await (await backend()).deleteGesture(gesture.id);
                onDone();
              }}
            >
              Delete
            </button>
          )}
        </div>
        <button type="submit" className="btn primary" disabled={busy}>
          Save
        </button>
      </div>
    </form>
  );
}

export function Gestures({ snap }: { snap: Snapshot }) {
  const [editing, setEditing] = useState<string | null>(null);
  const [recording, setRecording] = useState<{ replace: Gesture | null } | null>(null);
  const gestures = snap.library.gestures;
  const current = gestures.find((g) => g.id === editing) ?? null;
  const canDraw = snap.status.state === 'connected' || snap.config.dev.enabled;

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Gestures</h1>
          <p>Hold {snap.config.stroke.triggers[0] ?? 'ZR'}, draw, let go. {gestures.filter((g) => g.enabled).length} of {gestures.length} are on.</p>
        </div>
        <button type="button" className="btn primary" onClick={() => setRecording({ replace: null })}>
          Record a gesture
        </button>
      </header>

      {snap.library_error && (
        <div className="banner">
          <b>Library problem</b>
          <span>{snap.library_error}</span>
        </div>
      )}
      {!canDraw && (
        <div className="banner">
          <b>Nothing to draw with</b>
          <span>No controller is connected and dev mode is off. Connect a Joy-Con on the Device page, or turn on dev mode in Settings to draw with the mouse.</span>
        </div>
      )}

      <div className="cards">
        {gestures.map((g, i) => (
          <Card key={g.id} gesture={g} index={i} mediaKeys={snap.media_keys} onOpen={() => setEditing(g.id)} />
        ))}
      </div>

      <div className="row" style={{ marginTop: 26 }}>
        <button
          type="button"
          className="btn small quiet"
          title="Puts the eight built-in gestures back to their shipped shapes and actions. Gestures you recorded are kept."
          onClick={async () => void (await backend()).resetGestures()}
        >
          Restore built-in gestures
        </button>
      </div>

      <Dialog open={current !== null} onClose={() => setEditing(null)}>
        {current && (
          <Editor
            key={current.id}
            gesture={current}
            snap={snap}
            onDone={() => setEditing(null)}
            onRerecord={() => {
              setEditing(null);
              setRecording({ replace: current });
            }}
          />
        )}
      </Dialog>

      <Dialog open={recording !== null} onClose={() => setRecording(null)} wide>
        {recording && <Recorder snap={snap} replace={recording.replace} onDone={() => setRecording(null)} />}
      </Dialog>
    </>
  );
}
