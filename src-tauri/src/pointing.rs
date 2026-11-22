//! From orientation to screen position.
//!
//! The wand's tip direction is projected onto a plane tangent to where it
//! pointed at a reference moment: the start of the stroke for the path, the
//! last Recentre for where the ember appears. Wrist roll does not move the
//! point, and the projection is undistorted around the reference whatever the
//! pitch (plain yaw/pitch angles would stretch strokes drawn while aiming up).

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// "+x", "-z", ... to a unit vector. Unknown strings fall back to +Y.
pub fn axis(name: &str) -> Vec3 {
    let n = name.trim().to_ascii_lowercase();
    let sign = if n.starts_with('-') { -1.0 } else { 1.0 };
    let v = match n.trim_start_matches(['+', '-']) {
        "x" => Vec3::X,
        "z" => Vec3::Z,
        _ => Vec3::Y,
    };
    v * sign
}

/// Right-handed frame around a pointing direction, built against world up.
#[derive(Clone, Copy, Debug)]
pub struct Basis {
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

impl Basis {
    pub fn new(forward: Vec3) -> Basis {
        let f = forward.normalize_or(Vec3::Y);
        let mut right = f.cross(Vec3::Z);
        if right.length_squared() < 1e-4 {
            // Pointing straight up or down: any horizontal axis will do.
            right = Vec3::X;
        }
        let right = right.normalize();
        Basis { forward: f, right, up: right.cross(f).normalize() }
    }

    /// Angles (radians) of `dir` to the right of and above the reference.
    pub fn project(&self, dir: Vec3) -> (f32, f32) {
        let f = dir.dot(self.forward).max(1e-3); // behind the plane clamps to the edge
        (dir.dot(self.right).atan2(f), dir.dot(self.up).atan2(f))
    }

    /// Inverse of `project`, for the synthetic generator.
    pub fn unproject(&self, right: f32, up: f32) -> Vec3 {
        (self.forward + self.right * right.tan() + self.up * up.tan()).normalize()
    }
}

pub fn forward(q: Quat, forward_axis: Vec3) -> Vec3 {
    q * forward_axis
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Monitor {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub scale: f32,
    pub primary: bool,
}

impl Monitor {
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

pub fn primary(monitors: &[Monitor]) -> Monitor {
    monitors
        .iter()
        .find(|m| m.primary)
        .or(monitors.first())
        .copied()
        .unwrap_or(Monitor { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, scale: 1.0, primary: true })
}

/// Nearest point that is at least `margin` inside one of the monitors.
pub fn clamp_to_monitors(p: (f32, f32), monitors: &[Monitor], margin: f32) -> (f32, f32) {
    let mut best: Option<((f32, f32), f32)> = None;
    for m in monitors {
        let mx = margin.min(m.w * 0.4);
        let my = margin.min(m.h * 0.4);
        let c = (p.0.clamp(m.x + mx, m.x + m.w - mx), p.1.clamp(m.y + my, m.y + m.h - my));
        let d = (c.0 - p.0).powi(2) + (c.1 - p.1).powi(2);
        if best.map_or(true, |(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    best.map(|(c, _)| c).unwrap_or(p)
}

/// One-euro filter (Casiez et al.): a low-pass whose cutoff rises with speed,
/// so the point is steady when the hand is still and tight when it moves.
#[derive(Clone, Copy, Debug)]
pub struct OneEuro {
    min_cutoff: f32,
    beta: f32,
    x: Option<f32>,
    dx: f32,
}

impl OneEuro {
    pub fn new(min_cutoff: f32, beta: f32) -> Self {
        OneEuro { min_cutoff, beta, x: None, dx: 0.0 }
    }
    pub fn reset(&mut self) {
        self.x = None;
        self.dx = 0.0;
    }
    fn alpha(cutoff: f32, dt: f32) -> f32 {
        let tau = 1.0 / (std::f32::consts::TAU * cutoff);
        1.0 / (1.0 + tau / dt)
    }
    pub fn filter(&mut self, value: f32, dt: f32) -> f32 {
        if self.min_cutoff <= 0.0 || !(dt > 0.0) {
            self.x = Some(value);
            return value;
        }
        let Some(prev) = self.x else {
            self.x = Some(value);
            return value;
        };
        let raw_dx = (value - prev) / dt;
        self.dx += Self::alpha(1.0, dt) * (raw_dx - self.dx);
        let cutoff = self.min_cutoff + self.beta * self.dx.abs();
        let x = prev + Self::alpha(cutoff, dt) * (value - prev);
        self.x = Some(x);
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_names() {
        assert_eq!(axis("+y"), Vec3::Y);
        assert_eq!(axis("-Z"), -Vec3::Z);
        assert_eq!(axis("x"), Vec3::X);
        assert_eq!(axis("nonsense"), Vec3::Y);
    }

    #[test]
    fn projection_signs_match_the_screen() {
        // Wand aimed north (+Y), level.
        let b = Basis::new(Vec3::Y);
        assert!((b.right - Vec3::X).length() < 1e-5);
        assert!((b.up - Vec3::Z).length() < 1e-5);
        // Turning right (clockwise seen from above) moves the point right.
        let (x, y) = b.project(Quat::from_rotation_z(-0.2) * Vec3::Y);
        assert!((x - 0.2).abs() < 1e-4 && y.abs() < 1e-4);
        // Tilting the tip up moves the point up.
        let (x, y) = b.project(Quat::from_rotation_x(0.15) * Vec3::Y);
        assert!(x.abs() < 1e-4 && (y - 0.15).abs() < 1e-4);
    }

    #[test]
    fn roll_does_not_move_the_point() {
        let fwd = axis("+y");
        let aim = Quat::from_rotation_z(0.3) * Quat::from_rotation_x(0.2);
        let b = Basis::new(aim * fwd);
        for roll in [-1.0f32, 0.4, 2.0] {
            let q = aim * Quat::from_rotation_y(roll);
            let (x, y) = b.project(forward(q, fwd));
            assert!(x.abs() < 1e-4 && y.abs() < 1e-4);
        }
    }

    #[test]
    fn unproject_inverts_project_even_when_aiming_high() {
        let b = Basis::new(Quat::from_rotation_x(1.0) * Vec3::Y); // 57 degrees up
        for (r, u) in [(0.2f32, 0.1f32), (-0.3, 0.25), (0.0, -0.4)] {
            let (x, y) = b.project(b.unproject(r, u));
            assert!((x - r).abs() < 1e-4 && (y - u).abs() < 1e-4);
        }
        // Straight up does not produce NaNs.
        let up = Basis::new(Vec3::Z);
        assert!(up.right.is_finite() && up.up.is_finite());
    }

    #[test]
    fn clamp_picks_the_nearest_monitor() {
        let mons = [
            Monitor { x: 0.0, y: 0.0, w: 2560.0, h: 1440.0, scale: 1.0, primary: true },
            Monitor { x: 2560.0, y: 0.0, w: 2560.0, h: 1440.0, scale: 1.0, primary: false },
        ];
        assert_eq!(clamp_to_monitors((1000.0, 700.0), &mons, 100.0), (1000.0, 700.0));
        assert_eq!(clamp_to_monitors((-500.0, 700.0), &mons, 100.0), (100.0, 700.0));
        assert_eq!(clamp_to_monitors((6000.0, 3000.0), &mons, 100.0), (5020.0, 1340.0));
        assert_eq!(primary(&mons).center(), (1280.0, 720.0));
        assert_eq!(clamp_to_monitors((5.0, 5.0), &[], 100.0), (5.0, 5.0));
    }

    #[test]
    fn one_euro_is_steady_at_rest_and_tight_in_motion() {
        let dt = 1.0 / 200.0;
        // At rest with +-1 px jitter: output jitter shrinks.
        let mut f = OneEuro::new(9.0, 0.035);
        let mut out = Vec::new();
        for i in 0..400 {
            out.push(f.filter(500.0 + if i % 2 == 0 { 1.0 } else { -1.0 }, dt));
        }
        let jitter = out[200..].iter().map(|v| (v - 500.0).abs()).fold(0.0f32, f32::max);
        assert!(jitter < 0.5, "rest jitter {jitter}");
        // At 4000 px/s: lag stays under 12 px (3 ms of travel).
        let mut f = OneEuro::new(9.0, 0.035);
        let mut lag = 0.0f32;
        for i in 0..200 {
            let x = i as f32 * 4000.0 * dt;
            let y = f.filter(x, dt);
            if i > 50 {
                lag = lag.max(x - y);
            }
        }
        assert!(lag < 12.0, "lag at speed {lag}px");
    }
}
