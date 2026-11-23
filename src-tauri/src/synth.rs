//! Synthetic Joy-Con: turns a 2D stroke into the input reports a controller
//! would send while drawing it.
//!
//! This is the stand-in for hardware in tests and in the shipped sample
//! recordings. It models what the pipeline has to cope with: a hand that
//! starts from rest and stops again, wrist roll, gyro bias and noise, gravity
//! on the accelerometer, a hard-iron offset on the magnetometer, int16
//! quantisation, and the trigger going down slightly before the motion and up
//! slightly after. It does not model linear acceleration of the hand or
//! Bluetooth delivery jitter.

use crate::pointing::Basis;
use crate::protocol::{btn, InputReport, ACCEL_G_PER_LSB, GYRO_DPS_PER_LSB, MAG_UT_PER_LSB};
use crate::recognizer::{self, Pt};
use glam::{Mat3, Quat, Vec3};

#[derive(Clone, Debug)]
pub struct SynthOptions {
    pub rate_hz: f32,
    /// Size of the stroke's longest side on screen.
    pub size_px: f32,
    pub px_per_degree: f32,
    pub duration_s: f32,
    /// Heading and pitch of the aim when the stroke starts, degrees.
    pub aim_yaw_deg: f32,
    pub aim_pitch_deg: f32,
    pub wrist_roll_deg: f32,
    /// Rotation of the drawn shape, degrees (sloppiness).
    pub tilt_deg: f32,
    /// Hand wobble as a fraction of size.
    pub wobble: f32,
    pub gyro_bias_dps: [f32; 3],
    pub gyro_noise_dps: f32,
    pub accel_noise_g: f32,
    /// `None` leaves the magnetometer field zero, as with the feature off.
    pub mag_offset_ut: Option<[f32; 3]>,
    pub button: u32,
    pub lead_in_s: f32,
    pub tail_s: f32,
    pub seed: u64,
    pub battery_mv: u16,
}

impl Default for SynthOptions {
    fn default() -> Self {
        SynthOptions {
            rate_hz: 125.0,
            size_px: 700.0,
            // The hand draws for the app as it ships.
            px_per_degree: crate::config::Config::default().pointing.px_per_degree,
            duration_s: 0.7,
            aim_yaw_deg: 25.0,
            aim_pitch_deg: 8.0,
            wrist_roll_deg: 18.0,
            tilt_deg: 0.0,
            wobble: 0.012,
            gyro_bias_dps: [0.6, -0.9, 0.4],
            gyro_noise_dps: 0.12,
            accel_noise_g: 0.004,
            mag_offset_ut: Some([31.0, -14.0, 22.0]),
            button: btn::ZR,
            lead_in_s: 1.2,
            tail_s: 0.25,
            seed: 1,
            battery_mv: 3920,
        }
    }
}

/// Earth's field in the world frame used by the fusion tests: north and down.
pub const WORLD_FIELD_UT: Vec3 = Vec3::new(21.0, 0.0, -42.0);

/// Minimum-jerk progress: starts and ends at rest.
fn min_jerk(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (10.0 - 15.0 * t + 6.0 * t * t)
}

/// Body-to-world rotation with the tip along `forward` and the given roll.
fn attitude(forward: Vec3, roll: f32) -> Quat {
    let b = Basis::new(forward);
    // Body X = right, Y = tip, Z = out of the button face.
    Quat::from_mat3(&Mat3::from_cols(b.right, b.forward, b.up)) * Quat::from_rotation_y(roll)
}

/// The same rotation as the shorter of its two quaternion forms. Building a
/// quaternion from a matrix can return either sign, and the long way round
/// would read as a full turn in one sample.
fn shortest(q: Quat) -> Quat {
    if q.w < 0.0 {
        -q
    } else {
        q
    }
}

fn point_at(path: &[Pt], s: f32) -> Pt {
    let total = recognizer::path_length(path);
    let target = s.clamp(0.0, 1.0) * total;
    let mut acc = 0.0;
    for w in path.windows(2) {
        let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        if acc + d >= target && d > 0.0 {
            let t = (target - acc) / d;
            return [w[0][0] + t * (w[1][0] - w[0][0]), w[0][1] + t * (w[1][1] - w[0][1])];
        }
        acc += d;
    }
    *path.last().unwrap()
}

/// Reports for one stroke of `shape` (template space, y down), with times in
/// microseconds from the start of the sequence.
pub fn stroke_reports(shape: &[Pt], o: &SynthOptions, counter0: u32, time0_us: u64) -> Vec<(u64, InputReport)> {
    let mut rng = fastrand::Rng::with_seed(o.seed);
    let mut gauss = move || {
        // Sum of uniforms: close enough to normal for sensor noise.
        (0..6).map(|_| rng.f32()).sum::<f32>() - 3.0
    };
    let dt = 1.0 / o.rate_hz;
    let k = o.px_per_degree.to_degrees(); // pixels per radian

    // Shape in pixels relative to its first point, tilted.
    let (ts, tc) = o.tilt_deg.to_radians().sin_cos();
    let px: Vec<Pt> = shape
        .iter()
        .map(|p| {
            let (x, y) = ((p[0] - shape[0][0]) * o.size_px, (p[1] - shape[0][1]) * o.size_px);
            [x * tc - y * ts, x * ts + y * tc]
        })
        .collect();
    // Dense so corners survive the lookup.
    let px = recognizer::resample(&px, 400);

    let aim = Quat::from_rotation_z(o.aim_yaw_deg.to_radians()) * Quat::from_rotation_x(o.aim_pitch_deg.to_radians());
    let start = Basis::new(aim * Vec3::Y);
    let roll = o.wrist_roll_deg.to_radians();
    let wob = [gauss() * 6.0, gauss() * 6.0, 3.0 + gauss(), 4.0 + gauss()];

    let pose = |t: f32| -> Quat {
        // t in seconds from the start of motion.
        let s = min_jerk(t / o.duration_s);
        let p = point_at(&px, s);
        // Low-frequency wobble that vanishes at the ends.
        let env = (s * (1.0 - s) * 4.0).min(1.0) * o.wobble * o.size_px;
        let wx = env * (wob[0] + t * wob[2] * 6.0).sin();
        let wy = env * (wob[1] + t * wob[3] * 6.0).cos();
        let right = (p[0] + wx) / k;
        let up = -(p[1] + wy) / k;
        attitude(start.unproject(right, up), roll + 0.08 * (t * 5.0).sin() * s * (1.0 - s) * 4.0)
    };

    let bias = Vec3::from(o.gyro_bias_dps).map(f32::to_radians);
    let press_at = o.lead_in_s - 0.04;
    let release_at = o.lead_in_s + o.duration_s + 0.03;
    let total = o.lead_in_s + o.duration_s + o.tail_s;
    let n = (total * o.rate_hz).ceil() as usize;

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 * dt;
        let (q0, q1) = (pose(t - o.lead_in_s), pose(t + dt - o.lead_in_s));
        let w = shortest(q0.inverse() * q1).to_scaled_axis() / dt;
        let gyro = w + bias + Vec3::new(gauss(), gauss(), gauss()) * (o.gyro_noise_dps.to_radians() * 0.5);
        let accel = q1.inverse() * Vec3::Z + Vec3::new(gauss(), gauss(), gauss()) * (o.accel_noise_g * 0.5);
        let mag = o.mag_offset_ut.map(|off| q1.inverse() * WORLD_FIELD_UT + Vec3::from(off) + Vec3::new(gauss(), gauss(), gauss()) * 0.2);
        let q16 = |v: f32| v.round().clamp(-32768.0, 32767.0) as i16;
        let time_us = time0_us + (i as f64 * dt as f64 * 1e6) as u64;
        let report = InputReport {
            counter: counter0.wrapping_add(i as u32),
            buttons: if t >= press_at && t < release_at { o.button } else { 0 },
            left_stick: (2048, 2048),
            right_stick: (2048, 2048),
            mouse: [0; 4],
            mag: mag.map_or([0; 3], |m| (m / MAG_UT_PER_LSB).to_array().map(q16)),
            battery_mv: o.battery_mv,
            charge_state: 0,
            battery_current: 0,
            motion_time_us: time_us as u32,
            temperature: 380,
            accel: (accel / ACCEL_G_PER_LSB).to_array().map(q16),
            gyro: (gyro.map(f32::to_degrees) / GYRO_DPS_PER_LSB).to_array().map(q16),
        };
        out.push((time_us, report));
    }
    out
}

/// A slow tumble through all orientations, as asked for by the magnetometer
/// calibration, with the field mounted through `mount`.
pub fn tumble_reports(seconds: f32, o: &SynthOptions, mount: impl Fn(Vec3) -> Vec3) -> Vec<(u64, InputReport)> {
    let dt = 1.0 / o.rate_hz;
    let pose = |t: f32| Quat::from_rotation_z(t * 2.1) * Quat::from_rotation_x(t * 1.3) * Quat::from_rotation_y(t * 0.7);
    let off = Vec3::from(o.mag_offset_ut.unwrap_or([0.0; 3]));
    let q16 = |v: f32| v.round().clamp(-32768.0, 32767.0) as i16;
    (0..(seconds * o.rate_hz) as usize)
        .map(|i| {
            let t = i as f32 * dt;
            let (q0, q1) = (pose(t), pose(t + dt));
            let w = shortest(q0.inverse() * q1).to_scaled_axis() / dt;
            let time_us = (i as f64 * dt as f64 * 1e6) as u64;
            let report = InputReport {
                counter: i as u32,
                mag: ((mount(q1.inverse() * WORLD_FIELD_UT) + off) / MAG_UT_PER_LSB).to_array().map(q16),
                battery_mv: o.battery_mv,
                motion_time_us: time_us as u32,
                accel: ((q1.inverse() * Vec3::Z) / ACCEL_G_PER_LSB).to_array().map(q16),
                gyro: (w.map(f32::to_degrees) / GYRO_DPS_PER_LSB).to_array().map(q16),
                ..Default::default()
            };
            (time_us, report)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::engine::{Engine, EngineEvent, OverlayMsg, Source, StrokeResult};
    use crate::fusion::MagCal;
    use crate::gestures;
    use crate::pointing::Monitor;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Seen {
        fired: Vec<String>,
        outcomes: Vec<(String, f32, f32)>,
        path: Vec<Pt>,
        down: Option<(f32, f32)>,
        recentered: usize,
        gyro_cal: Option<bool>,
    }

    fn engine(cfg: Config) -> (Engine, Arc<Mutex<Seen>>) {
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let s = seen.clone();
        let mut e = Engine::new(
            cfg,
            gestures::defaults(),
            Box::new(move |ev| {
                let mut s = s.lock().unwrap();
                match ev {
                    EngineEvent::Fire { gesture_id, .. } => s.fired.push(gesture_id),
                    EngineEvent::Outcome(o) => s.outcomes.push((o.kind.to_string(), o.score, o.runner_up)),
                    EngineEvent::Overlay(OverlayMsg::Down { x, y, .. }) => {
                        s.down = Some((x, y));
                        s.path = vec![[x, y]];
                    }
                    EngineEvent::Overlay(OverlayMsg::Move { pts, .. }) => {
                        for c in pts.chunks(3) {
                            s.path.push([c[1], c[2]]);
                        }
                    }
                    EngineEvent::Overlay(OverlayMsg::Up { result: StrokeResult::Match { .. }, .. }) => {}
                    EngineEvent::Recentered => s.recentered += 1,
                    EngineEvent::GyroCalibrated(r) => s.gyro_cal = Some(r.is_ok()),
                    _ => {}
                }
            }),
        );
        e.set_monitors(vec![
            Monitor { x: 0.0, y: 0.0, w: 2560.0, h: 1440.0, scale: 1.0, primary: true },
            Monitor { x: 2560.0, y: 0.0, w: 2560.0, h: 1440.0, scale: 1.0, primary: false },
        ]);
        (e, seen)
    }

    fn feed(e: &mut Engine, reports: &[(u64, InputReport)]) {
        for (t, r) in reports {
            // Through the wire format, as a live controller or a replay would.
            let parsed = InputReport::parse(&r.encode()).unwrap();
            e.on_report_from(&parsed, *t as f64 * 1e-6, Source::Replay);
        }
    }

    fn ideal(id: &str, variant: usize) -> Vec<Pt> {
        gestures::defaults().gestures.into_iter().find(|g| g.id == id).unwrap().variants[variant].ideal.clone()
    }

    #[test]
    fn every_default_gesture_through_the_imu_pipeline() {
        let ids = [
            "circle", "slash", "z", "v", "flick-up", "flick-down", "flick-left", "flick-right", "chevron", "caret", "infinity", "star", "m", "s",
        ];
        // Different hands: size, speed, aim, roll, tilt, sample rate, seed.
        let hands = [
            SynthOptions::default(),
            SynthOptions { size_px: 470.0, duration_s: 0.35, aim_yaw_deg: -70.0, aim_pitch_deg: -12.0, wrist_roll_deg: -35.0, tilt_deg: 7.0, seed: 7, ..Default::default() },
            SynthOptions { size_px: 1000.0, duration_s: 1.3, aim_pitch_deg: 38.0, wrist_roll_deg: 60.0, tilt_deg: -8.0, rate_hz: 66.0, wobble: 0.02, seed: 21, ..Default::default() },
            SynthOptions { rate_hz: 200.0, mag_offset_ut: None, gyro_bias_dps: [-1.4, 1.1, 0.9], seed: 99, ..Default::default() },
        ];
        for id in ids {
            for (h, hand) in hands.iter().enumerate() {
                let (mut e, seen) = engine(Config::default());
                feed(&mut e, &stroke_reports(&ideal(id, 0), hand, 0, 0));
                let s = seen.lock().unwrap();
                assert_eq!(s.fired, vec![id.to_string()], "gesture {id}, hand {h}: outcomes {:?}", s.outcomes);
                let (_, score, runner_up) = s.outcomes[0];
                assert!(score > 0.86, "gesture {id}, hand {h}: score {score}");
                assert!(score - runner_up > 0.08, "gesture {id}, hand {h}: margin {}", score - runner_up);
            }
        }
        // The circle the other way round.
        let (mut e, seen) = engine(Config::default());
        feed(&mut e, &stroke_reports(&ideal("circle", 1), &SynthOptions::default(), 0, 0));
        assert_eq!(seen.lock().unwrap().fired, vec!["circle"]);
    }

    #[test]
    fn projected_path_has_the_drawn_size_and_direction() {
        let (mut e, seen) = engine(Config::default());
        let o = SynthOptions { size_px: 600.0, wobble: 0.0, ..Default::default() };
        feed(&mut e, &stroke_reports(&ideal("flick-right", 0), &o, 0, 0));
        let s = seen.lock().unwrap();
        let (first, last) = (s.path[0], *s.path.last().unwrap());
        // First stroke with no Recentre starts at the middle of the primary monitor.
        assert!((first[0] - 1280.0).abs() < 1.0 && (first[1] - 720.0).abs() < 1.0, "{first:?}");
        assert!((last[0] - first[0] - 600.0).abs() < 25.0, "travelled {} px right", last[0] - first[0]);
        assert!((last[1] - first[1]).abs() < 25.0, "drifted {} px vertically", last[1] - first[1]);
    }

    #[test]
    fn inverted_axes_mirror_the_stroke() {
        let mut cfg = Config::default();
        cfg.pointing.invert_x = true;
        let (mut e, seen) = engine(cfg);
        feed(&mut e, &stroke_reports(&ideal("flick-right", 0), &SynthOptions::default(), 0, 0));
        assert_eq!(seen.lock().unwrap().fired, vec!["flick-left"]);
    }

    #[test]
    fn second_stroke_starts_where_the_wand_points() {
        let (mut e, seen) = engine(Config::default());
        let o = SynthOptions { size_px: 500.0, wobble: 0.0, ..Default::default() };
        let a = stroke_reports(&ideal("flick-right", 0), &o, 0, 0);
        let (t_end, c_end) = (a.last().unwrap().0 + 8000, a.len() as u32);
        feed(&mut e, &a);
        // Second stroke begins 500 px further right, in degrees of yaw.
        let o2 = SynthOptions { aim_yaw_deg: o.aim_yaw_deg - 500.0 / o.px_per_degree, lead_in_s: 0.3, ..o.clone() };
        feed(&mut e, &stroke_reports(&ideal("flick-down", 0), &o2, c_end, t_end));
        let s = seen.lock().unwrap();
        assert_eq!(s.fired, vec!["flick-right", "flick-down"]);
        let (x, y) = s.down.unwrap();
        assert!((x - 1780.0).abs() < 40.0 && (y - 720.0).abs() < 40.0, "second ember at {x},{y}");
    }

    #[test]
    fn aiming_far_off_screen_is_pulled_back_inside() {
        let (mut e, seen) = engine(Config::default());
        let o = SynthOptions { wobble: 0.0, size_px: 300.0, ..Default::default() };
        let a = stroke_reports(&ideal("flick-up", 0), &o, 0, 0);
        let (mut t_end, mut c_end) = (a.last().unwrap().0 + 8000, a.len() as u32);
        feed(&mut e, &a);
        // Swing 110 degrees to the left with the trigger up, in two sweeps:
        // thousands of pixels off the desktop.
        let sweep_px = 55.0 * o.px_per_degree;
        for i in 0..2 {
            let sweep = SynthOptions {
                aim_yaw_deg: o.aim_yaw_deg + 55.0 * i as f32, size_px: sweep_px, button: 0,
                duration_s: 0.5, lead_in_s: 0.1, tail_s: 0.1, ..o.clone()
            };
            let r = stroke_reports(&ideal("flick-left", 0), &sweep, c_end, t_end);
            t_end = r.last().unwrap().0 + 8000;
            c_end += r.len() as u32;
            feed(&mut e, &r);
        }
        let o2 = SynthOptions { aim_yaw_deg: o.aim_yaw_deg + 110.0, lead_in_s: 0.3, ..o.clone() };
        feed(&mut e, &stroke_reports(&ideal("flick-up", 0), &o2, c_end, t_end));
        let (x, y) = seen.lock().unwrap().down.unwrap();
        assert!((0.0..5120.0).contains(&x) && (0.0..1440.0).contains(&y), "ember at {x},{y}");
        assert!((x - 120.0).abs() < 1.0, "clamped to the left margin, got {x}");
    }

    #[test]
    fn recenter_button_and_gyro_calibration() {
        let (mut e, seen) = engine(Config::default());
        let still = SynthOptions { duration_s: 0.01, lead_in_s: 2.6, button: 0, ..Default::default() };
        e.start_gyro_calibration();
        let reports = stroke_reports(&ideal("flick-up", 0), &still, 0, 0);
        feed(&mut e, &reports);
        assert_eq!(seen.lock().unwrap().gyro_cal, Some(true));
        let b = e.pose().gyro_bias_dps;
        assert!((b[0] - 0.6).abs() < 0.1 && (b[1] + 0.9).abs() < 0.1 && (b[2] - 0.4).abs() < 0.1, "bias {b:?}");

        let mut press = reports.last().unwrap().1;
        press.counter += 1;
        press.motion_time_us += 8000;
        press.buttons = btn::R_STICK;
        e.on_report_from(&press, 3.0, Source::Replay);
        assert_eq!(seen.lock().unwrap().recentered, 1);
    }

    #[test]
    fn gyro_calibration_fails_when_moving() {
        let (mut e, seen) = engine(Config::default());
        e.start_gyro_calibration();
        feed(&mut e, &tumble_reports(2.5, &SynthOptions::default(), |v| v));
        assert_eq!(seen.lock().unwrap().gyro_cal, Some(false));
    }

    #[test]
    fn magnetometer_calibration_through_the_engine() {
        let (mut e, _) = engine(Config::default());
        e.start_mag_calibration();
        // The chip mounted rotated a quarter turn about Z relative to the IMU.
        feed(&mut e, &tumble_reports(12.0, &SynthOptions::default(), |b| Vec3::new(-b.y, b.x, b.z)));
        assert!(e.mag_calibration_progress().unwrap().coverage > 0.9);
        let cal: MagCal = e.finish_mag_calibration().expect("solves");
        assert!(cal.valid);
        assert!((cal.field_ut - WORLD_FIELD_UT.length()).abs() < 6.0, "field {}", cal.field_ut);
        assert_ne!(cal.axes, [(0, 1), (1, 1), (2, 1)], "the rotated mounting must be detected");
        assert!(e.finish_mag_calibration().is_err());
    }
}
