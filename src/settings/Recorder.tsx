// Recording a gesture: draw it three times, look at the three laid over each
// other, then say what it does.
//
// While this is open the engine is in recording mode: strokes are captured
// and handed here, and nothing runs.

import { useEffect, useRef, useState } from 'react';
import { suggestIcon } from '../shared/icons';
import type { Action, Gesture, Pt, RecordingCheck, Snapshot } from '../shared/types';
import { ActionEditor } from './ActionEditor';
import { backend } from './api';
import { drawSamples, drawThumb } from './strokes';
import { Setting, Switch } from './ui';

const NEEDED = 3;

function Thumb({ sample }: { sample: Pt[] }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    if (ref.current) drawThumb(ref.current, sample);
  }, [sample]);
  return <canvas ref={ref} aria-hidden="true" />;
}

export function Recorder({ snap, replace, onDone }: { snap: Snapshot; replace: Gesture | null; onDone(): void }) {
  const [samples, setSamples] = useState<Pt[][]>([]);
  const [check, setCheck] = useState<RecordingCheck | null>(null);
  const [step, setStep] = useState<'draw' | 'bind'>('draw');
  const [name, setName] = useState(replace?.name ?? '');
  const [binding, setBinding] = useState<{ action: Action; label: string; icon: string }>(
    replace ? { action: replace.action, label: replace.label, icon: replace.icon } : { action: { type: 'key_combo', keys: '' }, label: '', icon: suggestIcon({ type: 'key_combo' }) },
  );
  const [free, setFree] = useState(replace?.rotation === 'free');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const stage = useRef<HTMLCanvasElement>(null);
  const samplesRef = useRef(samples);
  samplesRef.current = samples;
  const stepRef = useRef(step);
  stepRef.current = step;

  // Recording mode for as long as the flow is open.
  useEffect(() => {
    let off = () => {};
    let alive = true;
    void (async () => {
      const b = await backend();
      await b.setMode('recording');
      const un = await b.on<Pt[]>('wand://recorded', (points) => {
        if (stepRef.current !== 'draw' || samplesRef.current.length >= NEEDED) return;
        setSamples((s) => (s.length < NEEDED ? [...s, points] : s));
      });
      if (alive) off = un;
      else un();
    })();
    return () => {
      alive = false;
      off();
      void backend().then((b) => b.setMode('normal'));
    };
  }, []);

  useEffect(() => {
    if (stage.current) drawSamples(stage.current, samples, replace && samples.length === 0 ? replace.variants[0]?.ideal ?? null : null);
    if (samples.length < 2) {
      setCheck(null);
      return;
    }
    let live = true;
    void backend()
      .then((b) => b.checkRecordings(samples, replace?.id))
      .then((c) => live && setCheck(c));
    return () => {
      live = false;
    };
    // `step` too: each step has its own canvas element.
  }, [samples, replace, step]);

  const full = samples.length >= NEEDED;
  const uneven = check !== null && check.consistency < 0.72;
  // With three samples and three pair scores, the odd one out is the sample
  // missing from the best pair: pairs are (0,1), (0,2), (1,2).
  const odd = full && uneven && check ? [2, 1, 0][check.pair_scores.indexOf(Math.max(...check.pair_scores))] : -1;
  const trigger = snap.status.state === 'connected' ? `Hold ${snap.config.stroke.triggers[0] ?? 'ZR'} and draw in the air.` : snap.config.dev.enabled ? `Hold ${snap.config.dev.hold_key} and draw with the mouse.` : 'Connect a controller, or turn on dev mode in Settings.';

  const save = async () => {
    setBusy(true);
    setError('');
    try {
      const b = await backend();
      // With `replace`, the same gesture gets a new shape and keeps its place.
      await b.createGesture({ name, strokes: samples, freeRotation: free, ...binding, replaceId: replace?.id });
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const verdict = (
    <>
      {check && samples.length >= 2 && (
        <p className={uneven ? 'note warn' : 'note'}>
          {uneven ? 'These do not look alike yet. Redo the odd one: the closer the three are, the more reliably it matches.' : 'The samples agree.'}{' '}
          <span className="mono">{Math.round(check.consistency * 100)}%</span>
        </p>
      )}
      {check?.conflict && (
        <p className="note warn">
          Close to <b>{check.conflict.gesture_name}</b> (<span className="mono">{Math.round(check.conflict.score * 100)}%</span>). The two may be mistaken for each other.
        </p>
      )}
    </>
  );

  return (
    <div>
      <div className="dialog-head">
        <div>
          <div className="steps">
            <b>{step === 'draw' ? '1 Draw' : '2 Bind'}</b>
            <span>/</span>
            <span>{step === 'draw' ? '2 Bind' : '1 Draw'}</span>
          </div>
          <h2 style={{ marginTop: 8 }}>{step === 'draw' ? (replace ? `Redraw ${replace.name}` : 'Draw it three times') : 'What should it do?'}</h2>
        </div>
        <button type="button" className="btn quiet small" onClick={onDone}>
          Cancel
        </button>
      </div>

      {step === 'draw' ? (
        <div className="rec">
          <div className="rec-stage">
            <canvas ref={stage} aria-label="The recorded samples, overlaid" />
            {samples.length === 0 && (
              <div className="rec-hint">
                <div>
                  <strong>Ready when you are</strong>
                  {trigger}
                </div>
              </div>
            )}
          </div>

          <div className="stack" style={{ gap: 14 }}>
            <div className="samples">
              {Array.from({ length: NEEDED }, (_, i) => {
                const s = samples[i];
                return (
                  <div className="sample" key={s ? `s${i}-${s.length}` : `e${i}`} data-filled={!!s} data-odd={i === odd}>
                    {s ? <Thumb sample={s} /> : <span className="slot">{i + 1}</span>}
                    <span>{s ? (i === odd ? 'Unlike the others' : `Sample ${i + 1}`) : i === samples.length ? 'Draw now' : 'Waiting'}</span>
                    {s && (
                      <button type="button" className="btn quiet small" aria-label={`Discard sample ${i + 1}`} onClick={() => setSamples((all) => all.filter((_, k) => k !== i))}>
                        Redo
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
            {verdict}
          </div>
        </div>
      ) : (
        // Binding: the overlaid samples stay in view, small, beside the form.
        <div className="rec bind">
          <div>
            <div className="rec-stage">
              <canvas ref={stage} aria-label="The recorded samples, overlaid" />
            </div>
            <div style={{ marginTop: 12 }}>{verdict}</div>
          </div>
          <div className="stack">
            <label className="field">
              <span>Name</span>
              <input className="input" value={name} maxLength={32} placeholder="Triangle" autoFocus onChange={(e) => setName(e.target.value)} />
            </label>
            <ActionEditor {...binding} mediaKeys={snap.media_keys} onChange={setBinding} />
            <div className="settings">
              <Setting title="Match at any angle" help="For closed shapes like a circle. Leave off when direction matters.">
                <Switch checked={free} onChange={setFree} label="Match at any angle" />
              </Setting>
            </div>
            {error && <p className="error">{error}</p>}
          </div>
        </div>
      )}

      <div className="dialog-foot">
        <span className="note">{step === 'draw' ? `${samples.length} of ${NEEDED} drawn` : 'Nothing runs while you record.'}</span>
        {step === 'draw' ? (
          <button type="button" className="btn primary" disabled={!full} onClick={() => setStep('bind')}>
            Next
          </button>
        ) : (
          <div className="row">
            <button type="button" className="btn" onClick={() => setStep('draw')}>
              Back
            </button>
            <button type="button" className="btn primary" disabled={busy || !name.trim()} onClick={() => void save()}>
              Save gesture
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
