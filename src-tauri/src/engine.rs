//! The pipeline every input source feeds: reports or mouse points in, overlay
//! messages, actions and rumble out.
//!
//! The engine does no I/O. It is handed samples and a callback, so the live
//! controller, dev mode's mouse, the replay tool and the tests all run the
//! same code.

use crate::config::Config;
use crate::fusion::{Fusion, MagCal, MagCalProgress, MagCalibrator, SampleClock};
use crate::gestures::{Action, Library, Rotation};
use crate::pointing::{self, Basis, Monitor, OneEuro};
use crate::protocol::{btn, InputReport};
use crate::recognizer::{self, Pt, Template, Verdict};
use glam::Vec3;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Strokes are recognised and their actions run.
    Normal,
    /// Strokes are captured for the recording flow; nothing runs.
    Recording,
    /// Triggers are ignored.
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Joycon,
    Mouse,
    Replay,
}

/// What the overlay windows receive. Coordinates are physical pixels of the
/// virtual desktop; times are milliseconds on the engine's clock.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum OverlayMsg {
    Down { id: u32, t: f64, x: f32, y: f32, lead_ms: f32 },
    /// Flat triples: milliseconds since `Down`, x, y.
    Move { id: u32, pts: Vec<f32> },
    Up { id: u32, t: f64, result: StrokeResult },
    Cancel { id: u32 },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StrokeResult {
    /// `from` is the stroke resampled to N points, `to` the gesture's clean
    /// shape with the same N, placed where the stroke was. Flat x,y pairs.
    Match { gesture: String, label: String, icon: String, score: f32, from: Vec<f32>, to: Vec<f32> },
    NoMatch { from: Vec<f32> },
    Captured { from: Vec<f32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rumble {
    Match,
    NoMatch,
    Captured,
    Recenter,
}

/// One line of history per finished stroke, for the log and the UI.
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub kind: &'static str,
    pub gesture_id: Option<String>,
    pub gesture: Option<String>,
    pub score: f32,
    pub runner_up: f32,
    /// On a miss: the gesture that came closest.
    pub closest: Option<String>,
    pub action: Option<String>,
    pub source: Source,
    pub length_px: f32,
    pub duration_ms: f32,
    pub points: usize,
    /// What the recogniser was given: x, y in screen pixels and milliseconds
    /// since the trigger went down. Kept out of UI events; the app writes it
    /// to strokes.jsonl so a missed gesture can be looked at afterwards.
    #[serde(skip)]
    pub path: Vec<[f32; 3]>,
}

pub enum EngineEvent {
    Overlay(OverlayMsg),
    Fire { gesture_id: String, action: Action, label: String },
    Rumble(Rumble),
    Recorded { points: Vec<Pt> },
    Outcome(Outcome),
    Recentered,
    GyroCalibrated(Result<[f32; 3], String>),
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Pose {
    /// Body-to-world rotation, x y z w.
    pub q: [f32; 4],
    pub stationary: bool,
    pub mag_in_use: bool,
    pub report_hz: f32,
    pub gyro_bias_dps: [f32; 3],
}

struct Active {
    id: u32,
    source: Source,
    pts: Vec<Pt>,
    times: Vec<f32>,
    t0: f64,
    t_last: f64,
    length: f32,
}

fn path_of(s: &Active) -> Vec<[f32; 3]> {
    s.pts.iter().zip(&s.times).map(|(p, t)| [p[0], p[1], *t]).collect()
}

struct GyroCal {
    n: u32,
    sum: Vec3,
    sum_sq: Vec3,
    elapsed: f32,
}

const GYRO_CAL_SECONDS: f32 = 2.0;

pub struct Engine {
    cfg: Config,
    lib: Library,
    templates: Vec<Template>,
    emit: Box<dyn FnMut(EngineEvent) + Send>,
    mode: Mode,
    monitors: Vec<Monitor>,
    cursor: Option<Box<dyn Fn() -> Option<(f32, f32)> + Send>>,

    fusion: Fusion,
    clock: SampleClock,
    t_ms: f64,
    have_report: bool,
    prev_buttons: u32,
    imu_trigger: bool,
    trigger_mask: u32,
    recenter_mask: u32,
    fwd_axis: Vec3,
    center: Option<Basis>,
    drift: (f32, f32),
    stroke_basis: Option<Basis>,
    anchor: (f32, f32),
    fx: OneEuro,
    fy: OneEuro,
    gyro_cal: Option<GyroCal>,
    mag_cal: Option<MagCalibrator>,

    stroke: Option<Active>,
    next_id: u32,
    mouse_down: bool,
}

impl Engine {
    pub fn new(cfg: Config, lib: Library, emit: Box<dyn FnMut(EngineEvent) + Send>) -> Engine {
        let mut e = Engine {
            fusion: Fusion::new(cfg.fusion.clone()),
            fx: OneEuro::new(0.0, 0.0),
            fy: OneEuro::new(0.0, 0.0),
            cfg: Config::default(),
            lib: Library { version: 1, gestures: vec![] },
            templates: vec![],
            emit,
            mode: Mode::Normal,
            monitors: vec![],
            cursor: None,
            clock: SampleClock::default(),
            t_ms: 0.0,
            have_report: false,
            prev_buttons: 0,
            imu_trigger: false,
            trigger_mask: btn::ZR | btn::ZL,
            recenter_mask: 0,
            fwd_axis: Vec3::Y,
            center: None,
            drift: (0.0, 0.0),
            stroke_basis: None,
            anchor: (0.0, 0.0),
            gyro_cal: None,
            mag_cal: None,
            stroke: None,
            next_id: 1,
            mouse_down: false,
        };
        e.lib = lib;
        e.set_config(cfg);
        e
    }

    pub fn set_config(&mut self, cfg: Config) {
        self.trigger_mask = btn::mask(&cfg.stroke.triggers);
        if self.trigger_mask == 0 {
            self.trigger_mask = btn::ZR | btn::ZL;
        }
        // A button cannot both draw and recentre.
        self.recenter_mask = btn::mask(&cfg.pointing.recenter_buttons) & !self.trigger_mask;
        self.fwd_axis = pointing::axis(&cfg.pointing.forward_axis);
        self.fx = OneEuro::new(cfg.pointing.smoothing_min_cutoff_hz, cfg.pointing.smoothing_beta);
        self.fy = self.fx;
        self.fusion.set_cfg(cfg.fusion.clone());
        self.cfg = cfg;
        self.rebuild_templates();
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn set_library(&mut self, lib: Library) {
        self.lib = lib;
        self.rebuild_templates();
    }

    pub fn library(&self) -> &Library {
        &self.lib
    }

    fn rebuild_templates(&mut self) {
        self.templates = self.lib.templates(self.cfg.recognizer.resample_points);
    }

    pub fn set_monitors(&mut self, monitors: Vec<Monitor>) {
        self.monitors = monitors;
    }

    /// Where the mouse cursor is, for `anchor = "cursor"`.
    pub fn set_cursor_provider(&mut self, f: Box<dyn Fn() -> Option<(f32, f32)> + Send>) {
        self.cursor = Some(f);
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Paused {
            self.cancel_stroke();
        }
        self.mode = mode;
    }

    pub fn pose(&self) -> Pose {
        let q = self.fusion.orientation();
        let b = self.fusion.bias();
        Pose {
            q: [q.x, q.y, q.z, q.w],
            stationary: self.fusion.stationary(),
            mag_in_use: self.fusion.mag_in_use(),
            report_hz: self.clock.nominal_hz(),
            gyro_bias_dps: [b.x.to_degrees(), b.y.to_degrees(), b.z.to_degrees()],
        }
    }

    pub fn gyro_bias(&self) -> [f32; 3] {
        self.fusion.bias().to_array()
    }
    pub fn set_gyro_bias(&mut self, bias: [f32; 3]) {
        self.fusion.set_bias(Vec3::from(bias));
    }
    pub fn mag_cal(&self) -> MagCal {
        self.fusion.mag_cal()
    }
    pub fn set_mag_cal(&mut self, cal: MagCal) {
        self.fusion.set_mag_cal(cal);
    }

    // ------------------------------------------------------------ calibration

    /// Average the gyro for two seconds. The wand must lie still.
    pub fn start_gyro_calibration(&mut self) {
        self.gyro_cal = Some(GyroCal { n: 0, sum: Vec3::ZERO, sum_sq: Vec3::ZERO, elapsed: 0.0 });
    }

    pub fn start_mag_calibration(&mut self) {
        self.mag_cal = Some(MagCalibrator::new());
    }

    pub fn mag_calibration_progress(&self) -> Option<MagCalProgress> {
        self.mag_cal.as_ref().map(|c| c.progress())
    }

    pub fn finish_mag_calibration(&mut self) -> Result<MagCal, String> {
        let cal = self.mag_cal.take().ok_or("Calibration is not running.")?;
        if cal.progress().samples == 0 {
            return Err("The controller sent no magnetometer data.".into());
        }
        let solved = cal.solve()?;
        self.fusion.set_mag_cal(solved);
        Ok(solved)
    }

    pub fn cancel_mag_calibration(&mut self) {
        self.mag_cal = None;
    }

    /// Make the current aim the middle of the primary monitor.
    pub fn recenter(&mut self) {
        if !self.have_report {
            return;
        }
        let f = pointing::forward(self.fusion.orientation(), self.fwd_axis);
        self.center = Some(Basis::new(f));
        self.drift = (0.0, 0.0);
        (self.emit)(EngineEvent::Recentered);
        (self.emit)(EngineEvent::Rumble(Rumble::Recenter));
    }

    // ----------------------------------------------------------------- inputs

    pub fn on_report(&mut self, r: &InputReport, host_s: f64) {
        self.on_report_from(r, host_s, Source::Joycon);
    }

    pub fn on_report_from(&mut self, r: &InputReport, host_s: f64, source: Source) {
        let dt = self.clock.step(r.motion_time_us, r.counter, host_s);

        // Sample time: advances by the sensor's own step so velocity is clean,
        // but never runs ahead of, or more than 40 ms behind, arrival time.
        let host_ms = host_s * 1000.0;
        self.t_ms += dt as f64 * 1000.0;
        if !self.have_report || self.t_ms > host_ms {
            self.t_ms = host_ms;
        } else if host_ms - self.t_ms > 40.0 {
            self.t_ms = host_ms - 40.0;
        }
        self.have_report = true;

        let gyro = Vec3::from(r.gyro_rad_s());
        let accel = Vec3::from(r.accel_g());
        let mag = r.mag_ut().map(Vec3::from);

        if let Some(cal) = &mut self.gyro_cal {
            cal.n += 1;
            cal.sum += gyro;
            cal.sum_sq += gyro * gyro;
            cal.elapsed += dt;
            if cal.elapsed >= GYRO_CAL_SECONDS {
                let cal = self.gyro_cal.take().unwrap();
                let n = cal.n.max(1) as f32;
                let mean = cal.sum / n;
                let var = (cal.sum_sq / n - mean * mean).max(Vec3::ZERO);
                let noise_dps = var.max_element().sqrt().to_degrees();
                let result = if noise_dps > 1.5 || mean.length().to_degrees() > 12.0 {
                    Err("The wand moved. Put it on a table and try again.".to_string())
                } else {
                    self.fusion.set_bias(mean);
                    Ok(mean.to_array())
                };
                (self.emit)(EngineEvent::GyroCalibrated(result));
            }
        }
        if let (Some(cal), Some(m)) = (&mut self.mag_cal, mag) {
            cal.push(m, gyro - self.fusion.bias(), dt);
        }

        // No bias learning while the trigger is down or a calibration is collecting.
        let trigger_now = r.buttons & self.trigger_mask != 0;
        self.fusion.set_hold(trigger_now || self.mag_cal.is_some());
        self.fusion.update(dt, gyro, accel, mag);

        let pressed = r.buttons & !self.prev_buttons;
        self.prev_buttons = r.buttons;
        if pressed & self.recenter_mask != 0 {
            self.recenter();
        }

        let f = pointing::forward(self.fusion.orientation(), self.fwd_axis);
        let trigger = r.buttons & self.trigger_mask != 0;
        let t = self.t_ms;
        let k = self.cfg.pointing.px_per_degree.to_degrees(); // pixels per radian
        let sx = if self.cfg.pointing.invert_x { -1.0 } else { 1.0 };
        let sy = if self.cfg.pointing.invert_y { -1.0 } else { 1.0 };

        if trigger && !self.imu_trigger {
            if self.mode != Mode::Paused && self.stroke.is_none() {
                let anchor = self.anchor_for(f, k, sx, sy);
                self.anchor = anchor;
                self.stroke_basis = Some(Basis::new(f));
                self.fx.reset();
                self.fy.reset();
                self.begin_stroke(source, t, anchor, self.cfg.overlay.prediction_ms);
            }
        } else if trigger {
            if let (Some(b), true) = (self.stroke_basis, self.stroke.as_ref().is_some_and(|s| s.source == source)) {
                let (ax, ay) = b.project(f);
                let x = self.fx.filter(self.anchor.0 + ax * k * sx, dt);
                let y = self.fy.filter(self.anchor.1 - ay * k * sy, dt);
                self.add_point(t, x, y);
            }
        } else if self.imu_trigger && self.stroke.as_ref().is_some_and(|s| s.source == source) {
            self.end_stroke(t);
        }
        self.imu_trigger = trigger;
    }

    /// Dev mode: the mouse stands in for the wand. `host_s` monotonic seconds.
    pub fn on_pointer(&mut self, host_s: f64, x: f32, y: f32, down: bool) {
        let t = host_s * 1000.0;
        if down && !self.mouse_down {
            if self.mode != Mode::Paused && self.stroke.is_none() {
                self.begin_stroke(Source::Mouse, t, (x, y), self.cfg.overlay.prediction_mouse_ms);
            }
        } else if down {
            if self.stroke.as_ref().is_some_and(|s| s.source == Source::Mouse) {
                self.add_point(t, x, y);
            }
        } else if self.mouse_down && self.stroke.as_ref().is_some_and(|s| s.source == Source::Mouse) {
            self.end_stroke(t);
        }
        self.mouse_down = down;
    }

    /// The controller (or replay) went away.
    pub fn source_lost(&mut self, source: Source) {
        if self.stroke.as_ref().is_some_and(|s| s.source == source) {
            self.cancel_stroke();
        }
        if source != Source::Mouse {
            self.fusion.reset();
            self.clock = SampleClock::default();
            self.have_report = false;
            self.imu_trigger = false;
            self.prev_buttons = 0;
            self.gyro_cal = None;
            self.mag_cal = None;
        } else {
            self.mouse_down = false;
        }
    }

    // ---------------------------------------------------------------- strokes

    fn anchor_for(&mut self, f: Vec3, k: f32, sx: f32, sy: f32) -> (f32, f32) {
        let centre = pointing::primary(&self.monitors).center();
        match self.cfg.pointing.anchor.as_str() {
            "cursor" => self.cursor.as_ref().and_then(|c| c()).unwrap_or(centre),
            "center" => centre,
            _ => {
                // First press with no Recentre yet: wherever the wand aims is
                // the middle of the screen.
                let basis = *self.center.get_or_insert_with(|| Basis::new(f));
                let (ax, ay) = basis.project(f);
                let raw = (centre.0 + ax * k * sx + self.drift.0, centre.1 - ay * k * sy + self.drift.1);
                let inside = pointing::clamp_to_monitors(raw, &self.monitors, self.cfg.pointing.edge_margin_px);
                // Aiming past the edge drags the centre along, like lifting a
                // mouse: yaw drift corrects itself the next time you overshoot.
                self.drift.0 += inside.0 - raw.0;
                self.drift.1 += inside.1 - raw.1;
                inside
            }
        }
    }

    fn begin_stroke(&mut self, source: Source, t: f64, at: (f32, f32), lead_ms: f32) {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.stroke = Some(Active { id, source, pts: vec![[at.0, at.1]], times: vec![0.0], t0: t, t_last: t, length: 0.0 });
        (self.emit)(EngineEvent::Overlay(OverlayMsg::Down { id, t, x: at.0, y: at.1, lead_ms }));
    }

    fn add_point(&mut self, t: f64, x: f32, y: f32) {
        let max_ms = self.cfg.stroke.max_duration_ms as f64;
        let Some(s) = self.stroke.as_mut() else { return };
        if t - s.t0 > max_ms {
            let (source, length, dur, n, path) = (s.source, s.length, (t - s.t0) as f32, s.pts.len(), path_of(s));
            self.cancel_stroke();
            (self.emit)(EngineEvent::Outcome(Outcome {
                kind: "too_long", gesture_id: None, gesture: None, score: 0.0, runner_up: 0.0, closest: None,
                action: None, source, length_px: length, duration_ms: dur, points: n, path,
            }));
            return;
        }
        s.t_last = t;
        let last = *s.pts.last().unwrap();
        let d = ((x - last[0]).powi(2) + (y - last[1]).powi(2)).sqrt();
        if d < 0.3 {
            return; // no visible movement
        }
        s.length += d;
        s.pts.push([x, y]);
        s.times.push((t - s.t0) as f32);
        let (id, dt) = (s.id, (t - s.t0) as f32);
        (self.emit)(EngineEvent::Overlay(OverlayMsg::Move { id, pts: vec![dt, x, y] }));
    }

    fn cancel_stroke(&mut self) {
        if let Some(s) = self.stroke.take() {
            (self.emit)(EngineEvent::Overlay(OverlayMsg::Cancel { id: s.id }));
        }
        self.stroke_basis = None;
    }

    fn end_stroke(&mut self, t: f64) {
        let Some(s) = self.stroke.take() else { return };
        self.stroke_basis = None;
        let duration_ms = (t.max(s.t_last) - s.t0) as f32;
        let mut outcome = Outcome {
            kind: "too_short", gesture_id: None, gesture: None, score: 0.0, runner_up: 0.0, closest: None,
            action: None, source: s.source, length_px: s.length, duration_ms, points: s.pts.len(), path: path_of(&s),
        };
        if too_small(s.length, duration_ms, s.source, &self.cfg) {
            (self.emit)(EngineEvent::Overlay(OverlayMsg::Cancel { id: s.id }));
            (self.emit)(EngineEvent::Outcome(outcome));
            return;
        }

        let n = self.cfg.recognizer.resample_points;
        let flat = |pts: &[Pt]| pts.iter().flat_map(|p| [p[0], p[1]]).collect::<Vec<f32>>();
        let from = flat(&recognizer::resample(&s.pts, n));

        if self.mode == Mode::Recording {
            outcome.kind = "captured";
            (self.emit)(EngineEvent::Recorded { points: s.pts });
            (self.emit)(EngineEvent::Rumble(Rumble::Captured));
            (self.emit)(EngineEvent::Overlay(OverlayMsg::Up { id: s.id, t, result: StrokeResult::Captured { from } }));
            (self.emit)(EngineEvent::Outcome(outcome));
            return;
        }

        match recognizer::recognize(&s.pts, &self.templates, &self.cfg.recognizer) {
            Verdict::Match(m) => {
                let g = &self.lib.gestures[m.gesture];
                let label = g.display_label();
                // The action goes out before anything visual is prepared.
                (self.emit)(EngineEvent::Fire { gesture_id: g.id.clone(), action: g.action.clone(), label: label.clone() });
                (self.emit)(EngineEvent::Rumble(Rumble::Match));
                // The clean shape, in its own proportions, where the stroke
                // (or the part of it that matched) was. A free-rotation shape
                // is turned to meet the stroke so the morph is a short trip.
                let ideal = recognizer::normalize_uniform(&g.variants[m.variant].ideal, n).points;
                let turn = if g.rotation == Rotation::Free { -m.fit.rotation } else { 0.0 };
                let to = recognizer::place(&ideal, m.frame, turn);
                outcome.kind = "match";
                outcome.gesture_id = Some(g.id.clone());
                outcome.gesture = Some(g.name.clone());
                outcome.score = m.fit.score;
                outcome.runner_up = m.runner_up;
                outcome.action = Some(g.action.describe());
                let result = StrokeResult::Match {
                    gesture: g.id.clone(), label, icon: g.icon.clone(), score: m.fit.score, from, to: flat(&to),
                };
                (self.emit)(EngineEvent::Overlay(OverlayMsg::Up { id: s.id, t, result }));
            }
            Verdict::NoMatch { best } => {
                (self.emit)(EngineEvent::Rumble(Rumble::NoMatch));
                outcome.kind = "no_match";
                if let Some(m) = best {
                    outcome.score = m.fit.score;
                    outcome.runner_up = m.runner_up;
                    outcome.closest = Some(self.lib.gestures[m.gesture].name.clone());
                }
                (self.emit)(EngineEvent::Overlay(OverlayMsg::Up { id: s.id, t, result: StrokeResult::NoMatch { from } }));
            }
        }
        (self.emit)(EngineEvent::Outcome(outcome));
    }
}

/// Whether a finished stroke is too slight to be a gesture at all.
///
/// The wand has to mean it: enough travel in degrees, fast enough. A real
/// Joy-Con showed twitches of 4 degrees while the trigger was pressed and slow
/// drifts with it held, and one of each scored as a gesture (the twitch as a
/// slash, which closes a window). The mouse keeps its plain pixel limits.
pub fn too_small(length_px: f32, duration_ms: f32, source: Source, cfg: &Config) -> bool {
    let stroke = &cfg.stroke;
    let (min_length, min_speed) = if source == Source::Mouse {
        (stroke.min_length_px, 0.0)
    } else {
        let k = cfg.pointing.px_per_degree;
        (stroke.min_length_px.max(stroke.min_length_deg * k), stroke.min_speed_deg_s * k)
    };
    let speed = length_px / (duration_ms / 1000.0).max(1e-3);
    length_px < min_length || duration_ms < stroke.min_duration_ms as f32 || speed < min_speed
}

/// Merge runs of `Move` messages for the same stroke. The forwarder drains its
/// queue and calls this, so the overlay gets one message per wake-up however
/// fast samples arrive.
pub fn coalesce(msgs: Vec<OverlayMsg>) -> Vec<OverlayMsg> {
    let mut out: Vec<OverlayMsg> = Vec::with_capacity(msgs.len());
    for m in msgs {
        if let (OverlayMsg::Move { id, pts }, Some(OverlayMsg::Move { id: last_id, pts: last_pts })) = (&m, out.last_mut()) {
            if id == last_id {
                last_pts.extend_from_slice(pts);
                continue;
            }
        }
        out.push(m);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gestures;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<String>>>;

    fn engine() -> (Engine, Log, Arc<Mutex<Vec<OverlayMsg>>>) {
        let log: Log = Arc::default();
        let overlay: Arc<Mutex<Vec<OverlayMsg>>> = Arc::default();
        let (l, o) = (log.clone(), overlay.clone());
        let mut e = Engine::new(
            Config::default(),
            gestures::defaults(),
            Box::new(move |ev| match ev {
                EngineEvent::Overlay(m) => o.lock().unwrap().push(m),
                EngineEvent::Fire { gesture_id, .. } => l.lock().unwrap().push(format!("fire:{gesture_id}")),
                EngineEvent::Rumble(r) => l.lock().unwrap().push(format!("rumble:{r:?}")),
                EngineEvent::Recorded { points } => l.lock().unwrap().push(format!("recorded:{}", points.len())),
                EngineEvent::Outcome(o) => l.lock().unwrap().push(format!("outcome:{}", o.kind)),
                EngineEvent::Recentered => l.lock().unwrap().push("recentered".into()),
                EngineEvent::GyroCalibrated(r) => l.lock().unwrap().push(format!("gyrocal:{}", r.is_ok())),
            }),
        );
        e.set_monitors(vec![Monitor { x: 0.0, y: 0.0, w: 2560.0, h: 1440.0, scale: 1.0, primary: true }]);
        (e, log, overlay)
    }

    fn draw(e: &mut Engine, pts: &[(f32, f32)], t0: f64) -> f64 {
        let mut t = t0;
        e.on_pointer(t, pts[0].0, pts[0].1, true);
        for w in pts.windows(2) {
            for i in 1..=20 {
                t += 0.005;
                let k = i as f32 / 20.0;
                e.on_pointer(t, w[0].0 + k * (w[1].0 - w[0].0), w[0].1 + k * (w[1].1 - w[0].1), true);
            }
        }
        t += 0.002;
        let last = pts[pts.len() - 1];
        e.on_pointer(t, last.0, last.1, false);
        t + 0.1
    }

    #[test]
    fn mouse_stroke_fires_before_feedback() {
        let (mut e, log, overlay) = engine();
        draw(&mut e, &[(900.0, 700.0), (900.0, 300.0)], 1.0);
        let log = log.lock().unwrap().clone();
        assert_eq!(log, vec!["fire:flick-up", "rumble:Match", "outcome:match"]);
        let ov = overlay.lock().unwrap();
        assert!(matches!(ov.first(), Some(OverlayMsg::Down { x, y, .. }) if *x == 900.0 && *y == 700.0));
        match ov.last() {
            Some(OverlayMsg::Up { result: StrokeResult::Match { gesture, label, from, to, .. }, .. }) => {
                assert_eq!(gesture, "flick-up");
                assert_eq!(label, "Volume up");
                assert_eq!(from.len(), 128);
                assert_eq!(to.len(), 128);
                // The clean shape lands where the stroke was drawn.
                assert!((to[0] - 900.0).abs() < 8.0 && (to[1] - 700.0).abs() < 8.0);
                assert!((to[126] - 900.0).abs() < 8.0 && (to[127] - 300.0).abs() < 8.0);
            }
            other => panic!("expected a match, got {other:?}"),
        }
    }

    #[test]
    fn every_default_gesture_by_mouse() {
        let cases: Vec<(&str, Vec<(f32, f32)>)> = vec![
            ("flick-down", vec![(500.0, 200.0), (510.0, 620.0)]),
            ("flick-left", vec![(1400.0, 600.0), (800.0, 590.0)]),
            ("flick-right", vec![(800.0, 600.0), (1400.0, 615.0)]),
            ("slash", vec![(1300.0, 300.0), (1000.0, 700.0)]),
            ("z", vec![(700.0, 400.0), (1100.0, 395.0), (710.0, 720.0), (1120.0, 730.0)]),
            ("v", vec![(700.0, 300.0), (860.0, 700.0), (1010.0, 310.0)]),
        ];
        for (id, pts) in cases {
            let (mut e, log, _) = engine();
            draw(&mut e, &pts, 1.0);
            assert_eq!(log.lock().unwrap()[0], format!("fire:{id}"), "drawing {id}");
        }
        // Circle, both ways round, starting somewhere other than the top.
        for dir in [1.0f32, -1.0] {
            let (mut e, log, _) = engine();
            let pts: Vec<(f32, f32)> = (0..=40)
                .map(|i| {
                    let a = 0.7 + dir * i as f32 / 40.0 * std::f32::consts::TAU;
                    (1200.0 + a.cos() * 220.0, 700.0 + a.sin() * 200.0)
                })
                .collect();
            draw(&mut e, &pts, 1.0);
            assert_eq!(log.lock().unwrap()[0], "fire:circle");
        }
    }

    #[test]
    fn scribble_does_not_fire() {
        let (mut e, log, overlay) = engine();
        let pts = [(500.0, 500.0), (900.0, 420.0), (600.0, 380.0), (950.0, 640.0), (520.0, 700.0), (880.0, 300.0)];
        draw(&mut e, &pts, 1.0);
        assert_eq!(*log.lock().unwrap(), vec!["rumble:NoMatch", "outcome:no_match"]);
        assert!(matches!(overlay.lock().unwrap().last(), Some(OverlayMsg::Up { result: StrokeResult::NoMatch { .. }, .. })));
    }

    #[test]
    fn taps_are_dropped_silently() {
        let (mut e, log, overlay) = engine();
        draw(&mut e, &[(500.0, 500.0), (520.0, 505.0)], 1.0);
        assert_eq!(*log.lock().unwrap(), vec!["outcome:too_short"]);
        assert!(matches!(overlay.lock().unwrap().last(), Some(OverlayMsg::Cancel { .. })));
    }

    #[test]
    fn recording_mode_captures_instead_of_firing() {
        let (mut e, log, overlay) = engine();
        e.set_mode(Mode::Recording);
        draw(&mut e, &[(900.0, 700.0), (900.0, 300.0)], 1.0);
        let log = log.lock().unwrap().clone();
        assert!(log[0].starts_with("recorded:"));
        assert!(!log.iter().any(|l| l.starts_with("fire")));
        assert!(matches!(overlay.lock().unwrap().last(), Some(OverlayMsg::Up { result: StrokeResult::Captured { .. }, .. })));
    }

    #[test]
    fn paused_ignores_the_trigger_and_cancels_a_live_stroke() {
        let (mut e, log, overlay) = engine();
        e.on_pointer(1.0, 100.0, 100.0, true);
        e.on_pointer(1.01, 300.0, 100.0, true);
        e.set_mode(Mode::Paused);
        assert!(matches!(overlay.lock().unwrap().last(), Some(OverlayMsg::Cancel { .. })));
        e.on_pointer(1.02, 600.0, 100.0, false);
        draw(&mut e, &[(900.0, 700.0), (900.0, 300.0)], 2.0);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn held_too_long_is_abandoned() {
        let (mut e, log, _) = engine();
        e.on_pointer(1.0, 100.0, 100.0, true);
        e.on_pointer(1.5, 400.0, 100.0, true);
        e.on_pointer(8.0, 800.0, 100.0, true);
        e.on_pointer(8.1, 900.0, 100.0, false);
        assert_eq!(*log.lock().unwrap(), vec!["outcome:too_long"]);
    }

    #[test]
    fn coalesce_merges_moves_per_stroke() {
        let msgs = vec![
            OverlayMsg::Down { id: 1, t: 0.0, x: 0.0, y: 0.0, lead_ms: 0.0 },
            OverlayMsg::Move { id: 1, pts: vec![1.0, 1.0, 1.0] },
            OverlayMsg::Move { id: 1, pts: vec![2.0, 2.0, 2.0] },
            OverlayMsg::Cancel { id: 1 },
            OverlayMsg::Move { id: 2, pts: vec![3.0, 3.0, 3.0] },
        ];
        let out = coalesce(msgs);
        assert_eq!(out.len(), 4);
        assert!(matches!(&out[1], OverlayMsg::Move { pts, .. } if pts.len() == 6));
    }
}
