//! The pipeline every input source feeds: reports or mouse points in, overlay
//! messages, actions and rumble out.
//!
//! The engine does no I/O. It is handed samples and a callback, so the live
//! controller, dev mode's mouse, the replay tool and the tests all run the
//! same code.
//!
//! One controller, two jobs. In the air it is a wand: hold the trigger and
//! draw. Laid on its side with the rail's optical sensor on a surface it is a
//! mouse, and the same buttons click. What a button press means is decided
//! when it goes down and holds until it comes up, so lifting the controller
//! mid-drag does not drop the drag and setting it down mid-stroke does not
//! turn the stroke into a click.

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
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// What the cursor should do. The engine only says so; the app tells Windows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerEvent {
    /// Pixels before Windows applies its own pointer speed. +x right, +y down.
    Move { dx: f32, dy: f32 },
    Button { button: MouseButton, down: bool },
    /// Wheel notches. Positive is the wheel rolled away from the hand.
    Scroll { notches: f32 },
}

/// The optical sensor as the engine reads it, for the Device page and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Surface {
    /// The buttons are mouse buttons right now.
    pub active: bool,
    /// The cursor is following the sensor right now.
    pub tracking: bool,
    /// The sensor alone says there is a surface, whichever way the rail faces.
    pub seen: bool,
    pub distance: u16,
    pub roughness: u16,
    /// How far the rail is from facing straight down or up, in degrees.
    pub tilt_deg: f32,
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
    Pointer { event: PointerEvent, source: Source },
    /// The controller was set down on a surface, or picked up.
    Surface(Surface),
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
    pub surface: Surface,
}

/// The largest jump between two reports that can be real travel. The position
/// counters are 16 bits wide; a step this big means one of them was reset.
const MOUSE_MAX_STEP: f32 = 8000.0;
/// Stick counts from centre to the end of its travel, roughly. The sticks are
/// 12-bit and uncalibrated.
const STICK_TRAVEL: f32 = 1500.0;
/// A stick that sits within this many counts for `STICK_REST_MS`, near the
/// middle of its range, is at rest, and that reading is its centre.
const STICK_STILL: f32 = 40.0;
const STICK_REST_MS: f32 = 400.0;
const STICK_CENTRE_BAND: f32 = 300.0;

/// Learns where a stick rests. Centres differ from stick to stick by more
/// than a sensible dead zone (this one rests at 1915 of 4095), so the centre
/// is measured, not assumed.
#[derive(Clone, Copy, Debug)]
struct StickRest {
    centre: f32,
    anchor: f32,
    still_ms: f32,
}

impl StickRest {
    const NEW: StickRest = StickRest { centre: 2048.0, anchor: 2048.0, still_ms: 0.0 };

    /// Feed one reading; get the deflection from rest, -1 to 1.
    fn update(&mut self, raw: u16, dt_ms: f32) -> f32 {
        let v = raw as f32;
        if (v - self.anchor).abs() > STICK_STILL {
            self.anchor = v;
            self.still_ms = 0.0;
        } else {
            self.still_ms += dt_ms;
        }
        if self.still_ms >= STICK_REST_MS && (v - 2048.0).abs() < STICK_CENTRE_BAND {
            self.centre = self.anchor;
        }
        ((v - self.centre) / STICK_TRAVEL).clamp(-1.0, 1.0)
    }
}

struct MouseState {
    /// Button masks from the config, made disjoint (left wins, then right).
    left: u32,
    right: u32,
    middle: u32,
    /// Low-passed accelerometer: which way is down, in the controller's axes.
    gravity: Option<Vec3>,
    /// How long the surface test has passed without a break.
    ok_ms: f32,
    /// How long the sensor has seen a surface that the tilt test turned down.
    covered_ms: f32,
    /// Sensor position at the previous report, while tracking.
    last: Option<(u16, u16)>,
    tracking: bool,
    active: bool,
    last_tracking_ms: f64,
    prev_buttons: u32,
    /// Held buttons that went down as mouse buttons.
    owned: u32,
    sticks: [StickRest; 2],
    source: Source,
    view: Surface,
}

impl MouseState {
    fn new() -> MouseState {
        MouseState {
            left: 0,
            right: 0,
            middle: 0,
            gravity: None,
            ok_ms: 0.0,
            covered_ms: 0.0,
            last: None,
            tracking: false,
            active: false,
            last_tracking_ms: 0.0,
            prev_buttons: 0,
            owned: 0,
            sticks: [StickRest::NEW; 2],
            source: Source::Joycon,
            view: Surface::default(),
        }
    }

    fn buttons(&self) -> [(u32, MouseButton); 3] {
        [(self.left, MouseButton::Left), (self.right, MouseButton::Right), (self.middle, MouseButton::Middle)]
    }
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
    /// The direction the rail's flat side faces, in the controller's axes.
    rail_axis: Vec3,
    mouse: MouseState,
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
            rail_axis: Vec3::X,
            mouse: MouseState::new(),
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
        // The rail is on the controller's side: square to both the tip and
        // the button face. Which of the two sides does not matter here.
        self.rail_axis = self.fwd_axis.cross(pointing::axis(&cfg.pointing.up_axis)).normalize_or(Vec3::X);
        self.mouse.left = btn::mask(&cfg.mouse.left_buttons);
        self.mouse.right = btn::mask(&cfg.mouse.right_buttons) & !self.mouse.left;
        self.mouse.middle = btn::mask(&cfg.mouse.middle_buttons) & !(self.mouse.left | self.mouse.right);
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
            self.release_mouse();
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
            surface: self.mouse.view,
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

        // The mouse takes the buttons it is using; the wand sees the rest.
        let buttons = self.mouse_step(r, dt, source);

        // No bias learning while the trigger is down or a calibration is collecting.
        let trigger_now = buttons & self.trigger_mask != 0;
        self.fusion.set_hold(trigger_now || self.mag_cal.is_some());
        self.fusion.update(dt, gyro, accel, mag);

        let pressed = buttons & !self.prev_buttons;
        self.prev_buttons = buttons;
        if pressed & self.recenter_mask != 0 {
            self.recenter();
        }

        let f = pointing::forward(self.fusion.orientation(), self.fwd_axis);
        let trigger = buttons & self.trigger_mask != 0;
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

    /// The mouse half of a report. Returns the buttons left for the wand.
    fn mouse_step(&mut self, r: &InputReport, dt: f32, source: Source) -> u32 {
        let m = r.mouse_sample();
        let c = &self.cfg.mouse;
        let on = c.enabled && self.mode != Mode::Paused;
        let (sens_x, sens_y) = (
            c.sensitivity * if c.invert_x { -1.0 } else { 1.0 },
            c.sensitivity * if c.invert_y { -1.0 } else { 1.0 },
        );
        let (max_distance, max_roughness, max_tilt, settle_ms, linger_ms) =
            (c.max_distance, c.max_roughness, c.max_tilt_deg, c.settle_ms, c.linger_ms as f64);
        let (scroll_speed, deadzone) = (c.scroll_speed * if c.invert_scroll { -1.0 } else { 1.0 }, c.scroll_deadzone);
        let t = self.t_ms;
        let dt_ms = dt * 1000.0;
        self.mouse.source = source;

        // Which way the rail faces. Flat on a desk it faces straight down; in
        // a hand it mostly faces sideways, with fingers over the sensor.
        let accel = Vec3::from(r.accel_g());
        let gravity = match self.mouse.gravity {
            Some(g) => g + (accel - g) * (dt / 0.08).clamp(0.0, 1.0),
            None => accel,
        };
        self.mouse.gravity = Some(gravity);
        let tilt_deg = if gravity.length() > 0.3 {
            (gravity.dot(self.rail_axis).abs() / gravity.length()).clamp(0.0, 1.0).acos().to_degrees()
        } else {
            90.0
        };

        let seen = on && m.distance != 0 && m.distance < max_distance && m.roughness < max_roughness;
        let surface = seen && tilt_deg <= max_tilt;
        self.mouse.ok_ms = if surface { self.mouse.ok_ms + dt_ms } else { 0.0 };
        let was_tracking = self.mouse.tracking;
        // Becoming a mouse takes a steady look at the surface. Once it is one,
        // a lift to reposition picks up again on the first report back down.
        // A stroke in progress stays a stroke: the cursor waits until it ends.
        let settled = was_tracking || self.mouse.active || self.mouse.ok_ms >= settle_ms;
        let tracking = surface && settled && !self.imu_trigger;
        if tracking {
            // The first report after a lift only sets the reference, so the
            // cursor does not jump by however far the controller was carried.
            if let (true, Some((px, py))) = (was_tracking, self.mouse.last) {
                let dx = m.x.wrapping_sub(px) as i16 as f32;
                let dy = m.y.wrapping_sub(py) as i16 as f32;
                if (dx != 0.0 || dy != 0.0) && dx.abs() < MOUSE_MAX_STEP && dy.abs() < MOUSE_MAX_STEP {
                    let event = PointerEvent::Move { dx: dx * sens_x, dy: dy * sens_y };
                    (self.emit)(EngineEvent::Pointer { event, source });
                }
            }
            self.mouse.last = Some((m.x, m.y));
            self.mouse.last_tracking_ms = t;
        } else {
            self.mouse.last = None;
        }
        self.mouse.tracking = tracking;
        let active = on && (tracking || (self.mouse.active && t - self.mouse.last_tracking_ms < linger_ms));

        // A press is a click if the controller is a mouse when it goes down,
        // and stays one until it comes up.
        let pressed = r.buttons & !self.mouse.prev_buttons;
        self.mouse.prev_buttons = r.buttons;
        let before = self.mouse.owned;
        let mut owned = if on { before & r.buttons } else { 0 };
        if active {
            owned |= pressed & (self.mouse.left | self.mouse.right | self.mouse.middle);
        }
        self.mouse.owned = owned;
        for (mask, button) in self.mouse.buttons() {
            let (was, now) = (before & mask != 0, owned & mask != 0);
            if was != now {
                (self.emit)(EngineEvent::Pointer { event: PointerEvent::Button { button, down: now }, source });
            }
        }

        // The stick scrolls. A single Joy-Con reports the stick it lacks as
        // centred, so adding the two is the same as reading the real one.
        let push = self.mouse.sticks[0].update(r.left_stick.1, dt_ms) + self.mouse.sticks[1].update(r.right_stick.1, dt_ms);
        if active && scroll_speed != 0.0 && push.abs() > deadzone {
            // Squared, so a small push is a slow, fine scroll.
            let v = ((push.abs() - deadzone) / (1.0 - deadzone)).min(1.0);
            let notches = v * v * push.signum() * scroll_speed * dt;
            (self.emit)(EngineEvent::Pointer { event: PointerEvent::Scroll { notches }, source });
        }

        let view = Surface { active, tracking, seen, distance: m.distance, roughness: m.roughness, tilt_deg };
        // Something over the sensor for a full second that the tilt test will
        // not accept is worth one line in the log: either a hand, or a limit
        // that is wrong for this controller.
        let covered_before = self.mouse.covered_ms;
        self.mouse.covered_ms = if seen && !surface && !active && !self.imu_trigger { covered_before + dt_ms } else { 0.0 };
        let covered = covered_before < 1000.0 && self.mouse.covered_ms >= 1000.0;
        if active != self.mouse.active || covered {
            (self.emit)(EngineEvent::Surface(view));
        }
        self.mouse.active = active;
        self.mouse.view = view;
        r.buttons & !owned
    }

    /// Let go of every mouse button the controller is holding and stop being
    /// a mouse: the controller went away, or everything was paused.
    fn release_mouse(&mut self) {
        let owned = std::mem::take(&mut self.mouse.owned);
        let source = self.mouse.source;
        for (mask, button) in self.mouse.buttons() {
            if owned & mask != 0 {
                (self.emit)(EngineEvent::Pointer { event: PointerEvent::Button { button, down: false }, source });
            }
        }
        self.mouse.tracking = false;
        self.mouse.last = None;
        self.mouse.ok_ms = 0.0;
        self.mouse.covered_ms = 0.0;
        self.mouse.view.tracking = false;
        self.mouse.view.seen = false;
        if self.mouse.active {
            self.mouse.active = false;
            self.mouse.view.active = false;
            (self.emit)(EngineEvent::Surface(self.mouse.view));
        }
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
            self.release_mouse();
            self.mouse.prev_buttons = 0;
            self.mouse.gravity = None;
            self.mouse.view = Surface::default();
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
                EngineEvent::Pointer { event, .. } => l.lock().unwrap().push(match event {
                    PointerEvent::Move { dx, dy } => format!("move:{dx:.0},{dy:.0}"),
                    PointerEvent::Button { button, down } => format!("{button:?}:{}", if down { "down" } else { "up" }),
                    PointerEvent::Scroll { notches } => format!("scroll:{notches}"),
                }),
                EngineEvent::Surface(s) => {
                    l.lock().unwrap().push(format!("surface:{}", if s.active { "on" } else if s.seen { "covered" } else { "off" }))
                }
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

    // ------------------------------------------------------------- the mouse

    /// A right Joy-Con and a desk. Reports arrive every 14.7 ms, as they do
    /// over Bluetooth.
    struct Desk {
        e: Engine,
        log: Log,
        overlay: Arc<Mutex<Vec<OverlayMsg>>>,
        i: u32,
        sensor: (u16, u16),
        /// Where this controller's stick rests (the real one rests off-centre).
        stick: u16,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Place {
        /// Rail down on the desk: the sensor sees a surface.
        Desk,
        /// Lifted a little to reposition: rail still down, no surface.
        Hover,
        /// Held like a wand, face up: no surface.
        Hand,
    }
    const DOWN: Place = Place::Desk;
    const HOVER: Place = Place::Hover;
    const UP: Place = Place::Hand;

    impl Desk {
        fn new() -> Desk {
            Desk::with(Config::default())
        }

        fn with(cfg: Config) -> Desk {
            let (mut e, log, overlay) = engine();
            e.set_config(cfg);
            Desk { e, log, overlay, i: 0, sensor: (100, 100), stick: 1915 }
        }

        /// `n` reports from `place`. The sensor's counters advance by `step`
        /// each report wherever the controller is.
        fn feed(&mut self, n: usize, place: Place, buttons: u32, step: (i16, i16)) {
            for _ in 0..n {
                self.i += 1;
                self.sensor = (self.sensor.0.wrapping_add(step.0 as u16), self.sensor.1.wrapping_add(step.1 as u16));
                let r = InputReport {
                    counter: self.i,
                    buttons,
                    left_stick: (2047, 2047),
                    right_stick: (2050, self.stick),
                    mouse: if place == DOWN { [self.sensor.0, self.sensor.1, 900, 300] } else { [self.sensor.0, self.sensor.1, 6000, 2500] },
                    accel: if place == UP { [0, 0, 4096] } else { [4096, 0, 0] },
                    ..Default::default()
                };
                self.e.on_report(&r, self.i as f64 * 0.0147);
            }
        }

        /// What happened since the last call.
        fn take(&mut self) -> Vec<String> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }

        fn strokes_started(&self) -> usize {
            self.overlay.lock().unwrap().iter().filter(|m| matches!(m, OverlayMsg::Down { .. })).count()
        }
    }

    #[test]
    fn set_down_on_a_desk_it_moves_the_cursor() {
        let mut d = Desk::new();
        // Still moving while it settles: none of that reaches the cursor.
        d.feed(4, DOWN, 0, (50, 50));
        d.feed(10, DOWN, 0, (0, 0));
        assert_eq!(d.take(), ["surface:on"]);
        d.feed(3, DOWN, 0, (10, -4));
        assert_eq!(d.take(), ["move:10,-4", "move:10,-4", "move:10,-4"]);
        assert!(d.e.pose().surface.tracking && d.e.pose().surface.active);
        assert_eq!(d.strokes_started(), 0);
    }

    #[test]
    fn sensor_counters_wrap_without_a_jump() {
        let mut d = Desk::new();
        d.sensor = (65_530, 6);
        d.feed(10, DOWN, 0, (0, 0));
        d.take();
        d.feed(2, DOWN, 0, (10, -10));
        assert_eq!(d.take(), ["move:10,-10", "move:10,-10"]);
        assert_eq!(d.sensor, (14, 65_522));
    }

    #[test]
    fn sensitivity_and_mirroring_apply_to_the_cursor() {
        let mut cfg = Config::default();
        cfg.mouse.sensitivity = 2.0;
        cfg.mouse.invert_y = true;
        let mut d = Desk::with(cfg);
        d.feed(10, DOWN, 0, (0, 0));
        d.take();
        d.feed(1, DOWN, 0, (10, -4));
        assert_eq!(d.take(), ["move:20,8"]);
    }

    #[test]
    fn lifting_to_reposition_does_not_drag_the_cursor_along() {
        let mut d = Desk::new();
        d.feed(10, DOWN, 0, (0, 0));
        d.feed(2, DOWN, 0, (5, 0));
        d.take();
        // Carried through the air: whatever the counters do, nothing moves.
        d.feed(6, HOVER, 0, (300, -200));
        assert!(d.take().is_empty(), "a short lift is still mouse mode, and silent");
        // Back down somewhere else. The first report is only the new
        // reference; the one after it already moves the cursor.
        d.feed(1, DOWN, 0, (40, 40));
        assert!(d.take().is_empty());
        d.feed(1, DOWN, 0, (3, 3));
        assert_eq!(d.take(), ["move:3,3"]);
    }

    #[test]
    fn on_a_desk_r_and_zr_click_and_nothing_is_drawn() {
        let mut d = Desk::new();
        d.feed(10, DOWN, 0, (0, 0));
        d.take();
        d.feed(3, DOWN, btn::R, (0, 0));
        d.feed(2, DOWN, 0, (0, 0));
        d.feed(3, DOWN, btn::ZR, (0, 0));
        d.feed(2, DOWN, 0, (0, 0));
        d.feed(2, DOWN, btn::R_STICK, (0, 0));
        d.feed(2, DOWN, 0, (0, 0));
        assert_eq!(d.take(), ["Left:down", "Left:up", "Right:down", "Right:up", "Middle:down", "Middle:up"]);
        assert_eq!(d.strokes_started(), 0, "ZR on a desk is a right click, not a stroke");
    }

    #[test]
    fn in_the_air_the_same_buttons_are_the_wand() {
        let mut d = Desk::new();
        d.feed(10, UP, 0, (0, 0));
        d.feed(4, UP, btn::R, (0, 0));
        d.feed(4, UP, btn::ZR, (7, 7));
        d.feed(2, UP, 0, (0, 0));
        // R does nothing in the air; ZR began a stroke that went nowhere.
        assert_eq!(d.take(), ["outcome:too_short"]);
        assert_eq!(d.strokes_started(), 1);
    }

    #[test]
    fn a_hand_around_the_rail_is_not_a_desk() {
        let mut d = Desk::new();
        // The sensor reports a surface (fingers), but the rail faces sideways.
        for _ in 0..40 {
            d.i += 1;
            d.sensor.0 = d.sensor.0.wrapping_add(9);
            let r = InputReport {
                counter: d.i,
                buttons: if d.i > 20 { btn::ZR } else { 0 },
                left_stick: (2047, 2047),
                right_stick: (2050, 1915),
                mouse: [d.sensor.0, d.sensor.1, 900, 300],
                accel: [0, 0, 4096],
                ..Default::default()
            };
            d.e.on_report(&r, d.i as f64 * 0.0147);
        }
        assert!(d.take().is_empty(), "no cursor movement and no click");
        assert_eq!(d.strokes_started(), 1, "ZR still draws");
        let s = d.e.pose().surface;
        assert!(!s.active && s.seen && s.distance == 300 && s.tilt_deg > 80.0);
        // Left like that with the trigger up, it is reported once, for the log.
        for _ in 0..200 {
            d.i += 1;
            let r = InputReport { counter: d.i, left_stick: (2047, 2047), right_stick: (2050, 1915), mouse: [0, 0, 900, 300], accel: [0, 0, 4096], ..Default::default() };
            d.e.on_report(&r, d.i as f64 * 0.0147);
        }
        let log = d.take();
        assert_eq!(log.iter().filter(|l| *l == "surface:covered").count(), 1, "{log:?}");
    }

    #[test]
    fn a_drag_survives_a_lift() {
        let mut d = Desk::new();
        d.feed(10, DOWN, 0, (0, 0));
        d.feed(2, DOWN, btn::R, (4, 0));
        // Within a report the cursor arrives first, then the button goes down.
        assert_eq!(d.take(), ["surface:on", "move:4,0", "Left:down", "move:4,0"]);
        // Up in the air for a second with R still held.
        d.feed(68, UP, btn::R, (20, 20));
        assert_eq!(d.take(), ["surface:off"]);
        // Down again, still holding: the drag carries on.
        d.feed(14, DOWN, btn::R, (0, 0));
        d.feed(1, DOWN, btn::R, (6, 1));
        d.feed(1, DOWN, 0, (0, 0));
        assert_eq!(d.take(), ["surface:on", "move:6,1", "Left:up"]);
    }

    #[test]
    fn buttons_stay_mouse_buttons_for_a_moment_after_a_lift() {
        let mut d = Desk::new();
        d.feed(10, DOWN, 0, (0, 0));
        d.take();
        // 150 ms after lifting: still a click.
        d.feed(10, UP, 0, (0, 0));
        d.feed(2, UP, btn::ZR, (0, 0));
        d.feed(1, UP, 0, (0, 0));
        assert_eq!(d.take(), ["Right:down", "Right:up"]);
        // A second later it is a wand again.
        d.feed(68, UP, 0, (0, 0));
        d.feed(4, UP, btn::ZR, (0, 0));
        d.feed(1, UP, 0, (0, 0));
        assert_eq!(d.take(), ["surface:off", "outcome:too_short"]);
        assert_eq!(d.strokes_started(), 1);
    }

    #[test]
    fn a_stroke_in_progress_is_not_turned_into_a_mouse() {
        let mut d = Desk::new();
        d.feed(10, UP, 0, (0, 0));
        d.feed(3, UP, btn::ZR, (0, 0));
        // The controller comes to rest on something with the trigger held.
        d.feed(20, DOWN, btn::ZR, (9, 9));
        assert!(d.take().is_empty(), "no cursor movement and no right click during the stroke");
        d.feed(3, DOWN, 0, (0, 0));
        assert_eq!(d.take(), ["outcome:too_short", "surface:on"]);
    }

    #[test]
    fn the_stick_scrolls_on_a_desk_only() {
        let notches = |log: &[String]| -> f32 {
            log.iter().filter_map(|l| l.strip_prefix("scroll:")).map(|n| n.parse::<f32>().unwrap()).sum()
        };
        let mut d = Desk::new();
        // At rest, off-centre as the real stick is: learned, so no creep.
        d.feed(40, DOWN, 0, (0, 0));
        assert_eq!(d.take(), ["surface:on"]);
        // Pushed all the way forward for a second.
        d.stick = 3400;
        d.feed(68, DOWN, 0, (0, 0));
        let up = notches(&d.take());
        assert!((14.0..=18.5).contains(&up), "about scroll_speed notches a second, got {up}");
        // Pulled a little: slower, and the other way.
        d.stick = 1915 - 700;
        d.feed(68, DOWN, 0, (0, 0));
        let down = notches(&d.take());
        assert!(down < -0.5 && down > -6.0, "{down}");
        // In the air the stick is not a wheel.
        d.stick = 1915;
        d.feed(68, UP, 0, (0, 0));
        d.take();
        d.stick = 3400;
        d.feed(30, UP, 0, (0, 0));
        assert_eq!(notches(&d.take()), 0.0);
    }

    #[test]
    fn losing_the_controller_lets_go_of_the_button() {
        let mut d = Desk::new();
        d.feed(10, DOWN, 0, (0, 0));
        d.feed(2, DOWN, btn::R | btn::ZR, (0, 0));
        d.take();
        d.e.source_lost(Source::Joycon);
        assert_eq!(d.take(), ["Left:up", "Right:up", "surface:off"]);
    }

    #[test]
    fn pausing_stops_the_mouse_too() {
        let mut d = Desk::new();
        d.feed(10, DOWN, 0, (0, 0));
        d.feed(2, DOWN, btn::R, (0, 0));
        d.take();
        d.e.set_mode(Mode::Paused);
        assert_eq!(d.take(), ["Left:up", "surface:off"]);
        d.feed(20, DOWN, btn::R, (8, 8));
        d.feed(5, DOWN, btn::ZR, (8, 8));
        assert!(d.take().is_empty());
        assert_eq!(d.strokes_started(), 0);
    }

    #[test]
    fn turned_off_the_sensor_is_ignored_and_zr_draws_on_a_desk() {
        let mut cfg = Config::default();
        cfg.mouse.enabled = false;
        let mut d = Desk::with(cfg);
        d.feed(20, DOWN, 0, (10, 10));
        d.feed(4, DOWN, btn::R, (10, 10));
        d.feed(4, DOWN, btn::ZR, (10, 10));
        d.feed(2, DOWN, 0, (0, 0));
        assert_eq!(d.take(), ["outcome:too_short"]);
        assert_eq!(d.strokes_started(), 1);
    }
}
