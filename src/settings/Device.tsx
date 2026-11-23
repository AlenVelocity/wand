import { useEffect, useRef, useState } from 'react';
import type { Pose, Snapshot, Surface } from '../shared/types';
import { backend } from './api';
import { isMac } from './platform';
import { JoyConView } from './joycon3d';

const STATE_WORDS: Record<string, string> = {
  bluetooth_off: 'Bluetooth is off',
  searching: 'Looking for a controller',
  connecting: 'Connecting',
  pairing: 'Pairing',
  connected: 'Connected',
  reconnecting: 'Reconnecting',
};

function Preview({ snap, live, onSurface }: { snap: Snapshot; live: boolean; onSurface(s: Surface): void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const view = useRef<JoyConView | null>(null);
  const [pose, setPose] = useState<Pose | null>(null);

  useEffect(() => {
    if (!canvas.current) return;
    const v = new JoyConView(canvas.current);
    view.current = v;
    let stop = () => {};
    let offRecenter = () => {};
    let alive = true;
    let lastUi = 0;
    void (async () => {
      const b = await backend();
      const un = await b.subscribePose((p) => {
        v.setPose(p.q);
        // The readout below the model does not need 60 updates a second.
        const now = performance.now();
        if (now - lastUi > 250) {
          lastUi = now;
          setPose(p);
          onSurface(p.surface);
        }
      });
      const un2 = await b.on('wand://recentered', () => v.recenter());
      if (alive) {
        stop = un;
        offRecenter = un2;
      } else {
        un();
        un2();
      }
    })();
    return () => {
      alive = false;
      stop();
      offRecenter();
      v.dispose();
      view.current = null;
    };
  }, []);

  useEffect(() => {
    view.current?.setLook({ body: snap.status.body_color, accent: snap.status.accent_color, right: snap.status.side !== 'left' });
  }, [snap.status.body_color, snap.status.accent_color, snap.status.side]);

  return (
    <figure className="device-view" style={{ margin: 0 }}>
      <canvas ref={canvas} aria-label="Live orientation of the controller" />
      <figcaption>
        {live ? (
          <>
            Hold it like a remote, buttons up, tip toward the screen: the model should do the same. If it is mirrored or upside down, see <span className="mono">forward_axis</span> in wand.toml.
            {pose && (
              <span className="mono" style={{ display: 'block', marginTop: 6 }}>
                {pose.stationary ? 'at rest' : 'moving'} · yaw held by {pose.mag_in_use ? 'magnetometer' : 'gyro only'}
              </span>
            )}
            {pose && snap.status.has_mouse && (
              <>
                <span className="mono" style={{ display: 'block' }}>
                  sensor: distance {pose.surface.distance} · roughness {pose.surface.roughness}
                </span>
                <span className="mono" style={{ display: 'block' }}>rail {pose.surface.tilt_deg.toFixed(0)}° off vertical</span>
              </>
            )}
          </>
        ) : (
          'The model follows the controller once one is connected.'
        )}
      </figcaption>
    </figure>
  );
}

export function Device({ snap, toast }: { snap: Snapshot; toast(text: string): void }) {
  const s = snap.status;
  const connected = s.state === 'connected';
  const [gyroBusy, setGyroBusy] = useState(false);
  const [surface, setSurface] = useState<Surface | null>(null);
  const mouseWord = !snap.config.mouse.enabled || !s.has_mouse ? 'off' : snap.mode === 'paused' ? 'paused' : !surface ? '--' : surface.active ? 'on a surface' : surface.seen ? 'covered' : 'in the air';
  const [mag, setMag] = useState<{ coverage: number } | null>(null);

  const act = async (action: string, done?: string) => {
    try {
      await (await backend()).deviceAction(action);
      if (done) toast(done);
    } catch (e) {
      toast(String(e));
    }
  };

  useEffect(() => {
    let off = () => {};
    void backend()
      .then((b) =>
        b.on<null | { Err?: string }>('wand://gyro-calibrated', (result) => {
          setGyroBusy(false);
          const failed = result && typeof result === 'object' && 'Err' in result ? result.Err : null;
          toast(failed ?? 'Gyro calibrated. The offset is saved for this controller.');
        }),
      )
      .then((un) => (off = un));
    return () => off();
  }, [toast]);

  // Poll coverage while the magnetometer calibration is collecting.
  useEffect(() => {
    if (!mag) return;
    const id = window.setInterval(async () => {
      const p = await (await backend()).magProgress();
      if (p) setMag({ coverage: p.coverage });
    }, 200);
    return () => window.clearInterval(id);
  }, [mag !== null]);

  const finishMag = async () => {
    try {
      await (await backend()).deviceAction('mag_finish');
      toast('Magnetometer calibrated. Yaw is now held against drift.');
    } catch (e) {
      toast(String(e));
    }
    setMag(null);
  };

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Device</h1>
          <p>{connected ? 'Calibrate once. After that the controller reconnects whenever you press a button on it.' : s.detail || STATE_WORDS[s.state]}</p>
        </div>
      </header>

      <div className="device">
        <div>
          <h3>Status</h3>
          <h2 style={{ marginTop: 4 }}>
            {connected ? (
              <>
                {s.accent_color && <i className="swatch" style={{ background: s.accent_color }} />}
                {s.name}
              </>
            ) : (
              STATE_WORDS[s.state]
            )}
          </h2>

          {connected ? (
            <>
              <div style={{ marginTop: 26 }}>
                <h3>Battery</h3>
                <div className="battery">
                  <strong>
                    {s.battery_percent ?? '--'}
                    <sup>%</sup>
                  </strong>
                  <span className="muted mono" style={{ fontSize: 13 }}>
                    {(s.battery_mv / 1000).toFixed(2)} V{s.charging ? ' · charging' : ''}
                  </span>
                </div>
                <div className="meter" data-low={(s.battery_percent ?? 100) <= 15}>
                  <i style={{ transform: `scaleX(${(s.battery_percent ?? 0) / 100})` }} />
                </div>
                <p className="note" style={{ marginTop: 8 }}>
                  Estimated from cell voltage, which is all the controller reports.
                </p>
              </div>

              <div className="facts">
                <div className="fact">
                  <span>Reports</span>
                  <b>{s.report_hz ? `${s.report_hz} Hz` : '--'}</b>
                </div>
                <div className="fact">
                  <span>Link interval</span>
                  <b>{s.interval_ms ? `${s.interval_ms} ms` : '--'}</b>
                </div>
                <div className="fact">
                  <span>Magnetometer</span>
                  <b>{!s.has_magnetometer ? 'no data' : snap.calibration.mag_valid ? 'calibrated' : 'not calibrated'}</b>
                </div>
                <div className="fact">
                  <span>Mouse</span>
                  <b>{mouseWord}</b>
                </div>
              </div>

              <div className="section" style={{ marginTop: 34 }}>
                <h2>Calibration</h2>
                <div className="settings">
                  <div className="setting">
                    <div>
                      <b>Recentre</b>
                      <small>Point at the middle of the main screen, then press this or click the stick. The ember appears relative to that aim.</small>
                    </div>
                    <div className="control">
                      <button type="button" className="btn small" onClick={() => void act('recenter', 'Recentred on the main screen.')}>
                        Recentre
                      </button>
                    </div>
                  </div>
                  <div className="setting">
                    <div>
                      <b>Gyro</b>
                      <small>
                        Lay the controller on a table and leave it for two seconds. Wand also re-learns the offset whenever it is at rest.{' '}
                        <span className="mono">{snap.calibration.gyro_bias_dps.map((v) => v.toFixed(2)).join(' / ')} °/s</span>
                      </small>
                    </div>
                    <div className="control">
                      <button
                        type="button"
                        className="btn small"
                        disabled={gyroBusy}
                        onClick={() => {
                          setGyroBusy(true);
                          void act('calibrate_gyro');
                        }}
                      >
                        {gyroBusy ? 'Hold still…' : 'Calibrate'}
                      </button>
                    </div>
                  </div>
                  <div className="setting">
                    <div>
                      <b>Magnetometer</b>
                      <small>
                        {mag
                          ? 'Turn the controller slowly through every direction: roll it, tip it end over end, spin it flat. Stop when the bar is full.'
                          : 'Needed once per controller, away from speakers and the PC case. Without it the gyro alone holds yaw, which drifts slowly.'}
                      </small>
                      {mag && (
                        <div className="meter" style={{ maxWidth: 360 }}>
                          <i style={{ transform: `scaleX(${mag.coverage})` }} />
                        </div>
                      )}
                    </div>
                    <div className="control">
                      {mag ? (
                        <>
                          <button
                            type="button"
                            className="btn small quiet"
                            onClick={() => {
                              void act('mag_cancel');
                              setMag(null);
                            }}
                          >
                            Cancel
                          </button>
                          <button type="button" className="btn small primary" disabled={mag.coverage < 0.75} onClick={() => void finishMag()}>
                            Done
                          </button>
                        </>
                      ) : (
                        <>
                          {snap.calibration.mag_valid && (
                            <button type="button" className="btn small quiet" onClick={() => void act('mag_clear', 'Magnetometer calibration cleared.')}>
                              Clear
                            </button>
                          )}
                          <button
                            type="button"
                            className="btn small"
                            disabled={!s.has_magnetometer}
                            onClick={async () => {
                              try {
                                await (await backend()).deviceAction('mag_start');
                                setMag({ coverage: 0 });
                              } catch (e) {
                                toast(String(e));
                              }
                            }}
                          >
                            Calibrate
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                  <div className="setting">
                    <div>
                      <b>Rumble</b>
                      <small>One crisp tick for a match, two soft pulses for a miss.</small>
                    </div>
                    <div className="control">
                      <button type="button" className="btn small" onClick={() => void act('rumble_match')}>
                        Match
                      </button>
                      <button type="button" className="btn small" onClick={() => void act('rumble_no_match')}>
                        Miss
                      </button>
                    </div>
                  </div>
                  <div className="setting">
                    <div>
                      <b>Disconnect</b>
                      <small>Drops the link. Press any button on the controller to bring it back.</small>
                    </div>
                    <div className="control">
                      <button type="button" className="btn small quiet danger" onClick={() => void act('disconnect')}>
                        Disconnect
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </>
          ) : (
            <ol className="howto">
              <li>
                {isMac ? (
                  <span>
                    <b>Do not pair it in System Settings.</b> Wand pairs it itself. If it is listed under Bluetooth there, choose Forget This Device first.
                  </span>
                ) : (
                  <span>
                    <b>Do not pair it in Windows settings.</b> If it is listed under Bluetooth devices there, remove it first: Windows pairing makes the controller hang up.
                  </span>
                )}
              </li>
              <li>
                <span>
                  <b>Hold the sync button</b> (the small round one on the rail side) until the lights start chasing.
                </span>
              </li>
              <li>
                <span>
                  <b>Wait a few seconds.</b> Wand finds it, pairs, and the player light goes steady. After that, pressing any button reconnects it.
                </span>
              </li>
            </ol>
          )}
        </div>

        <Preview snap={snap} live={connected || snap.replaying} onSurface={setSurface} />
      </div>
    </>
  );
}
