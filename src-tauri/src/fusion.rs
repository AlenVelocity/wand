//! Sensor fusion: gyro + accelerometer + magnetometer into one orientation.
//!
//! A Mahony-style complementary filter on a quaternion. The gyro drives the
//! estimate, so the output has no lag behind the hand. Gravity pulls pitch and
//! roll back slowly; the magnetic heading pulls only yaw back, around the
//! world vertical, so a distorted field can never tilt the horizon. Gyro bias
//! is re-estimated whenever the wand lies still.
//!
//! World frame: Z up, X along the horizontal magnetic field once the
//! magnetometer is in use (otherwise an arbitrary heading).
//! `orientation()` rotates body vectors into the world.

use crate::config::FusionCfg;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Hard-iron offset, per-axis scale and axis alignment of the magnetometer
/// relative to the IMU. Produced by `MagCalibrator`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct MagCal {
    pub offset: [f32; 3],
    pub scale: [f32; 3],
    /// For each IMU axis: which magnetometer axis feeds it (0..3) and its sign.
    pub axes: [(u8, i8); 3],
    /// Field strength after calibration, microtesla.
    pub field_ut: f32,
    pub valid: bool,
}

impl Default for MagCal {
    fn default() -> Self {
        MagCal { offset: [0.0; 3], scale: [1.0; 3], axes: [(0, 1), (1, 1), (2, 1)], field_ut: 0.0, valid: false }
    }
}

impl MagCal {
    pub fn apply(&self, raw: Vec3) -> Vec3 {
        let c = [
            (raw.x - self.offset[0]) * self.scale[0],
            (raw.y - self.offset[1]) * self.scale[1],
            (raw.z - self.offset[2]) * self.scale[2],
        ];
        Vec3::new(
            c[self.axes[0].0 as usize] * self.axes[0].1 as f32,
            c[self.axes[1].0 as usize] * self.axes[1].1 as f32,
            c[self.axes[2].0 as usize] * self.axes[2].1 as f32,
        )
    }
}

pub struct Fusion {
    cfg: FusionCfg,
    q: Quat,
    bias: Vec3,
    initialised: bool,
    still_ms: f32,
    stationary: bool,
    mag_cal: MagCal,
    /// Heading the magnetic field had when it was first trusted. Yaw is pulled
    /// toward keeping the field at this heading.
    mag_ref: Option<f32>,
    mag_in_use: bool,
    /// Low-passed gyro, what the bias estimate follows while at rest.
    still_gyro: Vec3,
    /// Set while a stroke is being drawn: the bias estimate is frozen.
    hold: bool,
}

impl Fusion {
    pub fn new(cfg: FusionCfg) -> Self {
        Fusion {
            cfg,
            q: Quat::IDENTITY,
            bias: Vec3::ZERO,
            initialised: false,
            still_ms: 0.0,
            stationary: false,
            mag_cal: MagCal::default(),
            mag_ref: None,
            mag_in_use: false,
            still_gyro: Vec3::ZERO,
            hold: false,
        }
    }

    pub fn set_cfg(&mut self, cfg: FusionCfg) {
        self.cfg = cfg;
    }
    pub fn set_mag_cal(&mut self, cal: MagCal) {
        self.mag_cal = cal;
        self.mag_ref = None;
    }
    pub fn mag_cal(&self) -> MagCal {
        self.mag_cal
    }
    pub fn orientation(&self) -> Quat {
        self.q
    }
    pub fn bias(&self) -> Vec3 {
        self.bias
    }
    pub fn set_bias(&mut self, bias: Vec3) {
        self.bias = bias;
    }
    /// Freeze the bias estimate (a stroke is in progress).
    pub fn set_hold(&mut self, hold: bool) {
        self.hold = hold;
        if hold {
            self.still_ms = 0.0;
            self.stationary = false;
        }
    }
    pub fn stationary(&self) -> bool {
        self.stationary
    }
    /// True while the magnetometer is calibrated, enabled and passing its gate.
    pub fn mag_in_use(&self) -> bool {
        self.mag_in_use
    }
    pub fn reset(&mut self) {
        self.initialised = false;
        self.mag_ref = None;
        self.still_ms = 0.0;
    }

    /// Advance by `dt` seconds. `gyro` in rad/s, `accel` in g, `mag` raw in
    /// microtesla (uncalibrated) or `None`.
    pub fn update(&mut self, dt: f32, gyro: Vec3, accel: Vec3, mag: Option<Vec3>) {
        if !(dt > 0.0) || !gyro.is_finite() || !accel.is_finite() {
            return;
        }
        let dt = dt.min(0.1);
        let a_len = accel.length();
        let accel_ok = (a_len - 1.0).abs() < self.cfg.accel_gate_g && a_len > 0.1;

        if !self.initialised {
            if !accel_ok {
                return;
            }
            // Level the estimate: rotate so measured "up" maps to world +Z.
            self.q = Quat::from_rotation_arc(accel / a_len, Vec3::Z);
            self.initialised = true;
        }

        // --- rest detection and gyro bias
        //
        // At rest the gyro reads its bias plus noise: small, and above all
        // steady. A slow deliberate turn is small too, but it is never steady,
        // so the test is how far the reading strays from its own short
        // average. Without that a slow stroke gets absorbed into the bias and
        // everything after it drifts.
        let lp_a = (dt / 0.12).min(1.0);
        self.still_gyro = if self.still_ms == 0.0 && !self.stationary { gyro } else { self.still_gyro.lerp(gyro, lp_a) };
        let steady = (gyro - self.still_gyro).length() < self.cfg.still_gyro_dps.to_radians() * 0.5;
        let slow = (gyro - self.bias).length() < self.cfg.still_gyro_dps.to_radians() * 3.0 && gyro.length() < 12f32.to_radians();
        let level = (a_len - 1.0).abs() < self.cfg.still_accel_g;
        if steady && slow && level && !self.hold {
            self.still_ms += dt * 1000.0;
        } else {
            self.still_ms = 0.0;
        }
        self.stationary = self.still_ms >= self.cfg.still_time_ms;
        if self.stationary {
            let a = (dt / self.cfg.bias_time_constant_s.max(0.05)).min(1.0);
            self.bias = self.bias.lerp(self.still_gyro, a);
        }

        let mut w = gyro - self.bias;

        // --- gravity: pitch and roll
        if accel_ok {
            let up_est = self.q.inverse() * Vec3::Z;
            let err = (accel / a_len).cross(up_est);
            w += err * self.cfg.accel_gain;
        }

        // --- magnetic heading: yaw only
        self.mag_in_use = false;
        if let (true, true, Some(m_raw)) = (self.cfg.use_magnetometer, self.mag_cal.valid, mag) {
            let m = self.mag_cal.apply(m_raw);
            let strength = m.length();
            let expected = self.mag_cal.field_ut.max(1.0);
            if ((strength - expected) / expected).abs() < self.cfg.mag_gate {
                let mw = self.q * m;
                let horizontal = (mw.x * mw.x + mw.y * mw.y).sqrt();
                // Near the magnetic pole of the motion (field almost vertical
                // in the world) heading is undefined; skip.
                if horizontal > 0.2 * strength {
                    let heading = mw.y.atan2(mw.x);
                    let reference = *self.mag_ref.get_or_insert(heading);
                    let err = wrap_pi(heading - reference);
                    // A world-frame rotation about Z, expressed in the body.
                    w += self.q.inverse() * Vec3::new(0.0, 0.0, -err * self.cfg.mag_gain);
                    self.mag_in_use = true;
                }
            }
        }

        self.q = (self.q * Quat::from_scaled_axis(w * dt)).normalize();
    }
}

pub fn wrap_pi(a: f32) -> f32 {
    let mut a = a % std::f32::consts::TAU;
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    } else if a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// Picks the time step for each report.
///
/// Bluetooth delivers reports in bursts, so arrival times are useless for
/// integration. The sensor's own microsecond timestamp is used when it
/// advances plausibly; otherwise the report counter times a running estimate
/// of the report period.
///
/// On a real Joy-Con 2 (R) the "timestamp" turned out not to be microseconds
/// (it advances about 44,800 a second), so in practice the fallback is what
/// runs: one report is one period, and the period comes from the host clock
/// averaged over many reports.
pub struct SampleClock {
    last_ts: Option<u32>,
    last_counter: Option<u32>,
    last_host: Option<f64>,
    nominal: f32,
    seen_host: f32,
    seen_steps: u32,
}

impl Default for SampleClock {
    fn default() -> Self {
        // 15 ms is what Windows gives a Joy-Con 2 with a throughput-optimised link.
        SampleClock { last_ts: None, last_counter: None, last_host: None, nominal: 0.015, seen_host: 0.0, seen_steps: 0 }
    }
}

impl SampleClock {
    pub fn nominal_hz(&self) -> f32 {
        1.0 / self.nominal
    }

    /// `host_s` is a monotonic arrival time in seconds.
    pub fn step(&mut self, ts_us: u32, counter: u32, host_s: f64) -> f32 {
        let steps = match self.last_counter {
            Some(c) => counter.wrapping_sub(c),
            None => 1,
        };
        let ts_delta = self.last_ts.map(|t| ts_us.wrapping_sub(t));
        let host_delta = self.last_host.map(|h| (host_s - h) as f32);
        self.last_ts = Some(ts_us);
        self.last_counter = Some(counter);
        self.last_host = Some(host_s);

        // Long-run period from the host clock: immune to bursts on average.
        if let Some(hd) = host_delta {
            if (1..=20).contains(&steps) && hd > 0.0 && hd < 0.5 {
                // A plain average at first, so the estimate is right within a
                // fraction of a second of connecting; a slow moving average after.
                if self.seen_steps < 400 {
                    self.seen_host += hd;
                    self.seen_steps += steps;
                    if self.seen_steps >= 16 {
                        self.nominal = (self.seen_host / self.seen_steps as f32).clamp(0.001, 0.05);
                    }
                } else {
                    let per = (hd / steps as f32).clamp(0.001, 0.05);
                    self.nominal += (per - self.nominal) * 0.01;
                }
            }
        }

        let steps_f = if (1..=50).contains(&steps) { steps as f32 } else { 1.0 };
        if let Some(d) = ts_delta {
            let s = d as f32 * 1e-6;
            // Plausible if within 4x of what the counter implies.
            if d > 0 && s < 0.2 && s > steps_f * self.nominal * 0.25 && s < steps_f * self.nominal * 4.0 {
                return s;
            }
        }
        (steps_f * self.nominal).min(0.1)
    }
}

/// Collects magnetometer and gyro samples while the user turns the wand
/// through all orientations, then solves the hard-iron offset, per-axis scale
/// and (because nobody has documented how the AK09919 is mounted relative to
/// the IMU) which signed axis permutation makes the field rotate consistently
/// with the gyro.
#[derive(Default)]
pub struct MagCalibrator {
    samples: Vec<(Vec3, Vec3, f32)>, // (mag raw µT, gyro rad/s bias-corrected, dt)
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct MagCalProgress {
    /// 0..1: how much of each axis' range has been covered.
    pub coverage: f32,
    pub samples: usize,
}

impl MagCalibrator {
    pub fn new() -> Self {
        MagCalibrator { samples: Vec::new(), min: [f32::MAX; 3], max: [f32::MIN; 3] }
    }

    pub fn push(&mut self, mag: Vec3, gyro: Vec3, dt: f32) {
        if self.samples.len() >= 20_000 {
            return;
        }
        for (i, v) in mag.to_array().into_iter().enumerate() {
            self.min[i] = self.min[i].min(v);
            self.max[i] = self.max[i].max(v);
        }
        self.samples.push((mag, gyro, dt));
    }

    pub fn progress(&self) -> MagCalProgress {
        // Earth's field is 25..65 µT, so a fully turned axis spans >= 50 µT.
        let cov = (0..3)
            .map(|i| if self.samples.is_empty() { 0.0 } else { ((self.max[i] - self.min[i]) / 60.0).clamp(0.0, 1.0) })
            .fold(1.0f32, f32::min);
        MagCalProgress { coverage: cov, samples: self.samples.len() }
    }

    pub fn solve(&self) -> Result<MagCal, String> {
        if self.samples.len() < 200 {
            return Err("Not enough samples. Keep turning the wand for a few seconds.".into());
        }
        let mut offset = [0.0f32; 3];
        let mut half = [0.0f32; 3];
        for i in 0..3 {
            offset[i] = (self.max[i] + self.min[i]) * 0.5;
            half[i] = (self.max[i] - self.min[i]) * 0.5;
            if half[i] < 12.0 {
                return Err("The wand was not turned far enough on every axis. Roll, tilt and spin it.".into());
            }
        }
        let mean = (half[0] + half[1] + half[2]) / 3.0;
        let scale = [mean / half[0], mean / half[1], mean / half[2]];
        let mut cal = MagCal { offset, scale, axes: [(0, 1), (1, 1), (2, 1)], field_ut: mean, valid: true };

        // For a constant world field, the body-frame field obeys dm/dt = -w x m.
        // Try all 48 signed axis permutations and keep the one that obeys it best.
        let calibrated: Vec<(Vec3, Vec3, f32)> =
            self.samples.iter().map(|&(m, w, dt)| (cal.apply(m), w, dt)).collect();
        let perms = [[0u8, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        let mut best: Option<([(u8, i8); 3], f32)> = None;
        let mut identity_res = f32::MAX;
        let mut signal = 0.0f32;
        for p in perms {
            for signs in 0..8u8 {
                let axes = [
                    (p[0], if signs & 1 == 0 { 1i8 } else { -1 }),
                    (p[1], if signs & 2 == 0 { 1 } else { -1 }),
                    (p[2], if signs & 4 == 0 { 1 } else { -1 }),
                ];
                let map = |m: Vec3| {
                    let a = m.to_array();
                    Vec3::new(
                        a[axes[0].0 as usize] * axes[0].1 as f32,
                        a[axes[1].0 as usize] * axes[1].1 as f32,
                        a[axes[2].0 as usize] * axes[2].1 as f32,
                    )
                };
                let mut res = 0.0f32;
                let mut sig = 0.0f32;
                for pair in calibrated.windows(2) {
                    let (m0, w, _) = pair[0];
                    let (m1, _, dt) = pair[1];
                    if !(dt > 0.0) || w.length() < 0.3 {
                        continue;
                    }
                    let (a, b) = (map(m0), map(m1));
                    let predicted = -w.cross(a) * dt;
                    res += ((b - a) - predicted).length_squared();
                    sig += predicted.length_squared();
                }
                if p == [0, 1, 2] && signs == 0 {
                    identity_res = res;
                }
                signal = signal.max(sig);
                if best.map_or(true, |(_, r)| res < r) {
                    best = Some((axes, res));
                }
            }
        }
        let (axes, res) = best.unwrap();
        if signal <= 0.0 {
            return Err("The wand was not rotating while samples were taken.".into());
        }
        // Prefer the identity mapping unless another is clearly better; noise
        // alone should not flip axes.
        cal.axes = if identity_res <= res * 1.15 { [(0, 1), (1, 1), (2, 1)] } else { axes };
        let quality = res / signal;
        if quality > 0.6 {
            return Err(format!(
                "The magnetic field did not rotate with the wand (residual {quality:.2}). Move away from speakers, the PC case and metal desks, then try again."
            ));
        }
        Ok(cal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn cfg() -> FusionCfg {
        Config::default().fusion
    }

    const FIELD: Vec3 = Vec3::new(21.0, 0.0, -42.0); // north and down, µT

    /// Simulate a rigid body following `truth(t)`, returning the final
    /// estimate and the final truth.
    fn run(
        fusion: &mut Fusion,
        seconds: f32,
        hz: f32,
        truth: impl Fn(f32) -> Quat,
        gyro_bias: Vec3,
        mag: Option<(Vec3, Vec3)>, // (hard iron offset, world field)
    ) -> (Quat, Quat) {
        let dt = 1.0 / hz;
        let n = (seconds * hz) as usize;
        let mut q_true = truth(0.0);
        for i in 0..n {
            let t = i as f32 * dt;
            let q0 = truth(t);
            let q1 = truth(t + dt);
            let w = (q0.inverse() * q1).to_scaled_axis() / dt;
            let accel = q1.inverse() * Vec3::Z;
            let m = mag.map(|(off, field)| q1.inverse() * field + off);
            fusion.update(dt, w + gyro_bias, accel, m);
            q_true = q1;
        }
        (fusion.orientation(), q_true)
    }

    fn tilt_error_deg(est: Quat, truth: Quat) -> f32 {
        // Angle between the two estimates of world-up in the body frame.
        (est.inverse() * Vec3::Z).angle_between(truth.inverse() * Vec3::Z).to_degrees()
    }

    #[test]
    fn levels_from_gravity_and_tracks_rotation() {
        let mut f = Fusion::new(cfg());
        let truth = |t: f32| {
            Quat::from_rotation_z(0.9 * (t * 1.3).sin()) * Quat::from_rotation_x(0.5 * (t * 2.1).sin())
                * Quat::from_rotation_y(0.3)
        };
        let (est, tru) = run(&mut f, 6.0, 200.0, truth, Vec3::ZERO, None);
        assert!(tilt_error_deg(est, tru) < 1.0, "tilt error {}", tilt_error_deg(est, tru));
        // Relative rotation over the run is right even though absolute yaw is arbitrary.
        let rel_est = est * f32_identity();
        assert!(rel_est.is_normalized());
    }

    fn f32_identity() -> Quat {
        Quat::IDENTITY
    }

    #[test]
    fn learns_gyro_bias_at_rest() {
        let mut f = Fusion::new(cfg());
        let bias = Vec3::new(0.012, -0.02, 0.008); // about 1 deg/s
        let still = |_t: f32| Quat::from_rotation_x(0.4);
        run(&mut f, 4.0, 200.0, still, bias, None);
        assert!(f.stationary());
        assert!((f.bias() - bias).length() < 0.002, "bias {:?}", f.bias());
    }

    #[test]
    fn slow_turn_is_not_mistaken_for_rest() {
        let mut f = Fusion::new(cfg());
        // 20 deg/s yaw: far above the rest threshold, must not be absorbed.
        let turn = |t: f32| Quat::from_rotation_z(20f32.to_radians() * t);
        run(&mut f, 3.0, 200.0, turn, Vec3::ZERO, None);
        assert!(!f.stationary());
        assert!(f.bias().length() < 0.002);
    }

    #[test]
    fn yaw_drifts_without_magnetometer_and_holds_with_it() {
        // An uncorrected gyro bias while the wand moves the whole time, so
        // rest detection never gets to remove it.
        let bias = Vec3::new(0.0, 0.0, 0.01);
        let wobble = |t: f32| {
            Quat::from_rotation_z(0.6 * (t * 1.9).sin()) * Quat::from_rotation_x(0.5 * (t * 2.7).sin())
        };
        let heading_err = |est: Quat, tru: Quat| {
            let f_est = est * Vec3::Y;
            let f_tru = tru * Vec3::Y;
            wrap_pi(f_est.y.atan2(f_est.x) - f_tru.y.atan2(f_tru.x)).to_degrees()
        };

        let mut without = Fusion::new(cfg());
        let (e0, t0) = run(&mut without, 0.02, 200.0, wobble, bias, None);
        let start = heading_err(e0, t0);
        let (e1, t1) = run(&mut without, 60.0, 200.0, wobble, bias, None);
        let drift_without = wrap_pi((heading_err(e1, t1) - start).to_radians()).to_degrees().abs();
        assert!(drift_without > 15.0, "expected visible drift, got {drift_without}");

        let offset = Vec3::new(35.0, -18.0, 9.0);
        let mut with = Fusion::new(cfg());
        with.set_mag_cal(MagCal { offset: offset.to_array(), field_ut: FIELD.length(), valid: true, ..Default::default() });
        let (e0, t0) = run(&mut with, 0.02, 200.0, wobble, bias, Some((offset, FIELD)));
        let start = heading_err(e0, t0);
        let (e1, t1) = run(&mut with, 60.0, 200.0, wobble, bias, Some((offset, FIELD)));
        let drift_with = wrap_pi((heading_err(e1, t1) - start).to_radians()).to_degrees().abs();
        assert!(with.mag_in_use());
        assert!(drift_with < 3.0, "magnetometer should hold yaw, drift {drift_with}");
    }

    #[test]
    fn distorted_field_is_gated_out() {
        let mut f = Fusion::new(cfg());
        f.set_mag_cal(MagCal { field_ut: FIELD.length(), valid: true, ..Default::default() });
        let still = |_t: f32| Quat::IDENTITY;
        // A magnet nearby: three times the calibrated field strength.
        run(&mut f, 1.0, 200.0, still, Vec3::ZERO, Some((Vec3::ZERO, FIELD * 3.0)));
        assert!(!f.mag_in_use());
    }

    #[test]
    fn mag_calibration_recovers_offset_and_axis_mapping() {
        // Hidden mounting: mag X = -body Y, mag Y = body X, mag Z = body Z.
        let mount = |b: Vec3| Vec3::new(-b.y, b.x, b.z);
        let offset = Vec3::new(40.0, -25.0, 12.0);
        let tumble = |t: f32| {
            Quat::from_rotation_z(t * 2.1) * Quat::from_rotation_x(t * 1.3) * Quat::from_rotation_y(t * 0.7)
        };
        let mut cal = MagCalibrator::new();
        let dt = 1.0 / 200.0;
        for i in 0..2400 {
            let t = i as f32 * dt;
            let (q0, q1) = (tumble(t), tumble(t + dt));
            let w = (q0.inverse() * q1).to_scaled_axis() / dt;
            cal.push(mount(q0.inverse() * FIELD) + offset, w, dt);
        }
        assert!(cal.progress().coverage > 0.9);
        let solved = cal.solve().expect("calibration should solve");
        for i in 0..3 {
            assert!((solved.offset[i] - offset.to_array()[i]).abs() < 2.5, "offset {:?}", solved.offset);
        }
        // The solved mapping must undo the mounting for any orientation.
        for t in [0.3f32, 1.1, 2.9] {
            let q = tumble(t);
            let body_field = q.inverse() * FIELD;
            let recovered = solved.apply(mount(body_field) + offset);
            assert!((recovered - body_field).length() < 4.0, "{recovered:?} vs {body_field:?}");
        }
    }

    #[test]
    fn mag_calibration_rejects_too_little_motion() {
        let mut cal = MagCalibrator::new();
        for _ in 0..500 {
            cal.push(Vec3::new(20.0, 1.0, -40.0), Vec3::ZERO, 0.005);
        }
        assert!(cal.solve().is_err());
    }

    #[test]
    fn sample_clock_prefers_sensor_time_and_survives_bursts() {
        let mut c = SampleClock::default();
        let mut host = 0.0f64;
        let mut total = 0.0f32;
        // 200 Hz sensor delivered in bursts of three every 15 ms.
        for i in 0..600u32 {
            if i % 3 == 0 {
                host += 0.015;
            }
            total += c.step(i * 5000, i, host);
        }
        assert!((total - 3.0).abs() < 0.05, "integrated {total}s over 3s of samples");

        // Timestamp stuck at zero: fall back to counter x estimated period.
        let mut c = SampleClock::default();
        let mut host = 0.0f64;
        let mut total = 0.0f32;
        for i in 0..1000u32 {
            host += 0.008;
            total += c.step(0, i, host);
        }
        assert!((total - 8.0).abs() < 0.4, "fallback integrated {total}s over 8s");
        // A dropped report advances time by two periods.
        let dt = c.step(0, 1001, host + 0.016);
        assert!((dt - 0.016).abs() < 0.004, "dropped report dt {dt}");
    }
}
