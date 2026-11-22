//! Template matching in the $1 family, adapted to strokes drawn in the air.
//!
//! Like $1: resample the stroke to N points by arc length, move its centroid
//! to the origin, scale it to a unit box, then score by point distance to
//! each template at the best-fitting rotation.
//!
//! Where it departs from $1, and why. The second half of this list comes from
//! looking at strokes drawn with a real Joy-Con, which are nothing like a
//! mouse trace of the template:
//!
//! - Rotation is bounded (`rotation_tolerance_deg`) instead of free, so a
//!   flick up, a flick right and a slash stay three different gestures. A
//!   gesture can opt back into free rotation (the circle does).
//! - Lines are scaled uniformly. $1 stretches every stroke to a square, which
//!   turns a straight flick into noise.
//! - Everything that is not a line *is* stretched to a square, as in $1. A
//!   hand draws ovals and tall narrow Vs; the shape is what counts, not its
//!   proportions.
//! - The best rotation and scale come from the closed-form least-squares fit
//!   (as in Protractor) instead of a golden-section search.
//! - The distance is root-mean-square, and elastic: points may slip a little
//!   along the template (`elastic_window`), so a V whose first arm is short
//!   still lines its corner up with the template's.
//! - A stroke that fails as drawn gets a second chance without the parts a
//!   hand adds around a shape: the wind-up hook before a flick, the recoil
//!   after it, the tail leading into a circle, the extra third of a turn past
//!   where it closed.

use crate::config::RecognizerCfg;

pub type Pt = [f32; 2];

pub fn path_length(pts: &[Pt]) -> f32 {
    pts.windows(2).map(|w| dist(w[0], w[1])).sum()
}

fn dist(a: Pt, b: Pt) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// `n` points evenly spaced along the polyline.
pub fn resample(pts: &[Pt], n: usize) -> Vec<Pt> {
    let n = n.max(2);
    if pts.is_empty() {
        return vec![[0.0, 0.0]; n];
    }
    let total = path_length(pts);
    if total <= f32::EPSILON || pts.len() == 1 {
        return vec![pts[0]; n];
    }
    let step = total / (n - 1) as f32;
    let mut out = Vec::with_capacity(n);
    out.push(pts[0]);
    let mut acc = 0.0f32;
    let mut prev = pts[0];
    let mut i = 1;
    while i < pts.len() && out.len() < n {
        let cur = pts[i];
        let d = dist(prev, cur);
        if d > 0.0 && acc + d >= step {
            let t = (step - acc) / d;
            let q = [prev[0] + t * (cur[0] - prev[0]), prev[1] + t * (cur[1] - prev[1])];
            out.push(q);
            prev = q;
            acc = 0.0;
        } else {
            acc += d;
            prev = cur;
            i += 1;
        }
    }
    while out.len() < n {
        out.push(*pts.last().unwrap());
    }
    out
}

pub fn centroid(pts: &[Pt]) -> Pt {
    let n = pts.len().max(1) as f32;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |(x, y), p| (x + p[0], y + p[1]));
    [sx / n, sy / n]
}

fn bounds(pts: &[Pt]) -> (f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    ((x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Larger side of the bounding box.
pub fn extent(pts: &[Pt]) -> f32 {
    let (w, h) = bounds(pts);
    w.max(h)
}

/// How far the points spread across their main direction, relative to along
/// it: 0 for a straight line at any angle, 1 for a circle.
pub fn thinness(pts: &[Pt]) -> f32 {
    let c = centroid(pts);
    let (mut sxx, mut syy, mut sxy) = (0.0f32, 0.0f32, 0.0f32);
    for p in pts {
        let (x, y) = (p[0] - c[0], p[1] - c[1]);
        sxx += x * x;
        syy += y * y;
        sxy += x * y;
    }
    let half_trace = (sxx + syy) * 0.5;
    let disc = (half_trace * half_trace - (sxx * syy - sxy * sxy)).max(0.0).sqrt();
    let (major, minor) = (half_trace + disc, (half_trace - disc).max(0.0));
    if major <= 1e-12 {
        0.0
    } else {
        (minor / major).sqrt()
    }
}

/// Below this a stroke is treated as a line: scaled uniformly and compared
/// only with line templates. Flicks from a real hand, hooks included, sit
/// under 0.15; the narrowest real V sits above 0.4.
pub const LINE_THINNESS: f32 = 0.3;
/// A shape is never stretched by more than this to fill the unit box.
const MAX_STRETCH: f32 = 2.5;

/// A stroke in template space, plus what it took to get there so a template
/// can be mapped back onto the screen.
#[derive(Clone, Debug)]
pub struct Normalized {
    pub points: Vec<Pt>,
    pub centroid: Pt,
    /// What x and y were divided by.
    pub scale: [f32; 2],
    pub line: bool,
}

impl Normalized {
    /// One number for how big the stroke was on screen.
    pub fn size(&self) -> f32 {
        if self.scale[0] == self.scale[1] {
            self.scale[0]
        } else {
            (self.scale[0] + self.scale[1]) * 0.5
        }
    }
}

fn normalized(pts: &[Pt], n: usize, stretch: bool) -> Normalized {
    let r = resample(pts, n);
    let c = centroid(&r);
    let line = thinness(&r) < LINE_THINNESS;
    let (w, h) = bounds(&r);
    let big = w.max(h).max(1e-6);
    let scale = if stretch && !line { [w.max(big / MAX_STRETCH), h.max(big / MAX_STRETCH)] } else { [big, big] };
    let points = r.iter().map(|p| [(p[0] - c[0]) / scale[0], (p[1] - c[1]) / scale[1]]).collect();
    Normalized { points, centroid: c, scale, line }
}

/// Resample, centre and scale so the longer side is 1, keeping proportions.
/// This is the form shapes are stored and displayed in.
pub fn normalize_uniform(pts: &[Pt], n: usize) -> Normalized {
    normalized(pts, n, false)
}

/// The form shapes are matched in: lines as `normalize_uniform`, everything
/// else stretched to fill the unit box.
pub fn normalize(pts: &[Pt], n: usize) -> Normalized {
    normalized(pts, n, true)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub score: f32,
    /// Rotation applied to the stroke to fit the template, radians.
    pub rotation: f32,
    /// Scale applied to the stroke to fit the template.
    pub scale: f32,
}

/// RMS point distance at which the score reaches zero, in units of the
/// stroke's size. With the default `min_score` of 0.80 a stroke may stray
/// from the template by 10% of its own size, root-mean-square.
///
/// $1 averages plain distances. The square is deliberate: wobble spread along
/// the whole stroke costs about the same either way, but one stretch that goes
/// somewhere else entirely (the foot of an "L" against a flick down) is
/// punished about twice as hard, which is what keeps non-gestures out.
const ZERO_SCORE_DISTANCE: f32 = 0.5;

/// RMS distance between two point lists along the cheapest monotonic pairing
/// that never strays more than `band` indices from the diagonal (dynamic time
/// warping with a Sakoe-Chiba band).
fn elastic_rms(a: &[Pt], b: &[Pt], band: usize) -> f32 {
    let (n, m) = (a.len(), b.len());
    const INF: f32 = f32::MAX / 4.0;
    let mut prev = vec![(INF, 0u32); m];
    let mut cur = vec![(INF, 0u32); m];
    for i in 0..n {
        let centre = if n > 1 { i * (m - 1) / (n - 1) } else { 0 };
        let (lo, hi) = (centre.saturating_sub(band), (centre + band).min(m - 1));
        cur.iter_mut().for_each(|c| *c = (INF, 0));
        for j in lo..=hi {
            let d = (a[i][0] - b[j][0]).powi(2) + (a[i][1] - b[j][1]).powi(2);
            let mut best = if i == 0 && j == 0 { (0.0, 0) } else { (INF, 0) };
            if i > 0 && prev[j].0 < best.0 {
                best = prev[j];
            }
            if j > 0 && cur[j - 1].0 < best.0 {
                best = cur[j - 1];
            }
            if i > 0 && j > 0 && prev[j - 1].0 < best.0 {
                best = prev[j - 1];
            }
            if best.0 < INF {
                cur[j] = (best.0 + d, best.1 + 1);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let (cost, steps) = prev[m - 1];
    (cost / steps.max(1) as f32).sqrt()
}

/// Score `stroke` against `template`. Both must be normalized with the same N.
/// `window` is the elastic slack as a fraction of N; 0 pairs points rigidly.
pub fn fit(stroke: &[Pt], template: &[Pt], max_rotation: f32, scale_tolerance: f32, window: f32) -> Fit {
    let n = stroke.len().min(template.len());
    if n == 0 {
        return Fit { score: 0.0, rotation: 0.0, scale: 1.0 };
    }
    let (mut dot, mut cross, mut ss) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..n {
        let (c, t) = (stroke[i], template[i]);
        dot += c[0] * t[0] + c[1] * t[1];
        cross += c[0] * t[1] - c[1] * t[0];
        ss += c[0] * c[0] + c[1] * c[1];
    }
    let rotation = cross.atan2(dot).clamp(-max_rotation, max_rotation);
    let (sin, cos) = rotation.sin_cos();
    // Least-squares scale at that rotation.
    let scale = if ss > 1e-9 {
        ((dot * cos + cross * sin) / ss).clamp(1.0 - scale_tolerance, 1.0 + scale_tolerance)
    } else {
        1.0
    };
    let aligned: Vec<Pt> = stroke[..n]
        .iter()
        .map(|c| [(c[0] * cos - c[1] * sin) * scale, (c[0] * sin + c[1] * cos) * scale])
        .collect();
    let band = (window * n as f32).round() as usize;
    let rms = if band == 0 {
        (aligned.iter().zip(template).map(|(a, t)| (a[0] - t[0]).powi(2) + (a[1] - t[1]).powi(2)).sum::<f32>() / n as f32).sqrt()
    } else {
        elastic_rms(&aligned, &template[..n], band)
    };
    Fit { score: (1.0 - rms / ZERO_SCORE_DISTANCE).max(0.0), rotation, scale }
}

/// One template the recognizer can match: a polyline already in matching
/// form, tagged with the gesture and variant it belongs to.
#[derive(Clone, Debug)]
pub struct Template {
    pub gesture: usize,
    pub variant: usize,
    pub free_rotation: bool,
    pub line: bool,
    /// Width and height of the shape as it is shown, longest side 1.
    pub aspect: [f32; 2],
    pub points: Vec<Pt>,
}

impl Template {
    pub fn new(gesture: usize, variant: usize, free_rotation: bool, shape: &[Pt], n: usize) -> Template {
        let norm = normalize(shape, n);
        let (w, h) = bounds(shape);
        let big = w.max(h).max(1e-6);
        Template { gesture, variant, free_rotation, line: norm.line, aspect: [w / big, h / big], points: norm.points }
    }
}

/// Where a matched stroke sat on screen, for putting the clean shape there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub centroid: Pt,
    pub size: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub gesture: usize,
    pub variant: usize,
    pub fit: Fit,
    /// Best score among the other gestures, for the margin test and the UI.
    pub runner_up: f32,
    pub frame: Frame,
    /// True when the match came from the stroke with its tails or hooks removed.
    pub trimmed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Match(Match),
    /// Something scored best but not well enough, or two gestures tied.
    NoMatch { best: Option<Match> },
}

/// Best fit of the stroke to every template, per gesture: (gesture, score).
pub fn scores_by_gesture(points: &[Pt], templates: &[Template], cfg: &RecognizerCfg) -> Vec<(usize, f32)> {
    let mut out: Vec<(usize, f32)> = Vec::new();
    each_fit(points, templates, cfg, |t, f, _| match out.iter_mut().find(|(g, _)| *g == t.gesture) {
        Some(entry) => entry.1 = entry.1.max(f.score),
        None => out.push((t.gesture, f.score)),
    });
    out
}

fn each_fit(points: &[Pt], templates: &[Template], cfg: &RecognizerCfg, mut visit: impl FnMut(&Template, Fit, &Normalized)) {
    let n = cfg.resample_points;
    let tol = cfg.rotation_tolerance_deg.to_radians();
    let uniform = normalize_uniform(points, n);
    // A line is never stretched: its sag would become a shape of its own.
    let boxed = (!uniform.line).then(|| normalize(points, n));
    for t in templates {
        let stroke = if t.line {
            &uniform
        } else {
            match &boxed {
                Some(b) => b,
                None => continue,
            }
        };
        let max_rot = if t.free_rotation { std::f32::consts::PI } else { tol };
        visit(t, fit(&stroke.points, &t.points, max_rot, cfg.scale_tolerance, cfg.elastic_window), stroke);
    }
}

fn best_match(points: &[Pt], templates: &[Template], cfg: &RecognizerCfg, trimmed: bool) -> Option<Match> {
    let mut best: Option<Match> = None;
    let mut per_gesture: Vec<(usize, f32)> = Vec::new();
    each_fit(points, templates, cfg, |t, f, stroke| {
        match per_gesture.iter_mut().find(|(g, _)| *g == t.gesture) {
            Some(entry) => entry.1 = entry.1.max(f.score),
            None => per_gesture.push((t.gesture, f.score)),
        }
        if best.as_ref().map_or(true, |b| f.score > b.fit.score) {
            // How big the clean shape has to be drawn to cover the stroke. A
            // stretched stroke has a width and a height; the shape has
            // proportions of its own (an infinity sign is half as tall as it
            // is wide), so each side says how big it would make it.
            let size = if t.line {
                stroke.size()
            } else {
                0.5 * (stroke.scale[0] / t.aspect[0].max(0.25) + stroke.scale[1] / t.aspect[1].max(0.25))
            };
            best = Some(Match {
                gesture: t.gesture,
                variant: t.variant,
                fit: f,
                runner_up: 0.0,
                frame: Frame { centroid: stroke.centroid, size },
                trimmed,
            });
        }
    });
    let mut m = best?;
    m.runner_up = per_gesture.iter().filter(|(g, _)| *g != m.gesture).map(|(_, s)| *s).fold(0.0f32, f32::max);
    Some(m)
}

/// A match through trimming has to be this much better than `min_score`:
/// cutting parts off a stroke is a licence that should cost something.
const SECOND_CHANCE_EXTRA: f32 = 0.03;

pub fn recognize(points: &[Pt], templates: &[Template], cfg: &RecognizerCfg) -> Verdict {
    let accept = |m: &Match, extra: f32| m.fit.score >= cfg.min_score + extra && m.fit.score - m.runner_up >= cfg.min_margin;
    let full = best_match(points, templates, cfg, false);
    if let Some(m) = &full {
        if accept(m, 0.0) {
            return Verdict::Match(m.clone());
        }
    }
    if cfg.second_chance {
        let rescued = cleaned(points)
            .iter()
            .filter_map(|v| best_match(v, templates, cfg, true))
            .filter(|m| accept(m, SECOND_CHANCE_EXTRA))
            .max_by(|a, b| a.fit.score.total_cmp(&b.fit.score));
        if let Some(m) = rescued {
            return Verdict::Match(m);
        }
    }
    Verdict::NoMatch { best: full }
}

const DENSE: usize = 96;

/// The stroke with what a hand adds around a shape taken off, in the few ways
/// that can be done: its closed loop alone, or with a hook cut from the
/// start, the end, or both. Empty when the stroke has none of these.
pub fn cleaned(points: &[Pt]) -> Vec<Vec<Pt>> {
    let d = resample(points, DENSE);
    let mut out = Vec::new();
    if let Some((i, j)) = closed_loop(&d) {
        out.push(d[i..=j].to_vec());
    }
    let (start, end) = (hook_at_start(&d), hook_at_end(&d));
    if let Some(s) = start {
        out.push(d[s..].to_vec());
    }
    if let Some(e) = end {
        out.push(d[..=e].to_vec());
    }
    if let (Some(s), Some(e)) = (start, end) {
        out.push(d[s..=e].to_vec());
    }
    out
}

/// The longest stretch of the path that comes back to where it began: a
/// circle without the tail leading into it, or the first full turn of one
/// drawn too far round. `None` if the whole stroke is already that loop, or
/// there is no loop covering at least two fifths of it.
fn closed_loop(d: &[Pt]) -> Option<(usize, usize)> {
    let n = d.len();
    let close = extent(d) * 0.14;
    for span in (n * 2 / 5..n).rev() {
        let mut best: Option<(usize, f32)> = None;
        for i in 0..n - span {
            let gap = dist(d[i], d[i + span]);
            if gap <= close && best.map_or(true, |(_, g)| gap < g) {
                best = Some((i, gap));
            }
        }
        if let Some((i, _)) = best {
            // Trimming three points or fewer changes nothing worth a retry.
            if span >= n - 4 {
                return None;
            }
            return Some((i, i + span));
        }
    }
    None
}

/// Index of the sharpest turn in a range, if it is sharp enough to be a
/// reversal and not just the shape curving.
fn sharpest_turn(d: &[Pt], range: std::ops::RangeInclusive<usize>) -> Option<usize> {
    const ARM: usize = 3;
    let mut best: Option<(usize, f32)> = None;
    for k in range {
        if k < ARM || k + ARM >= d.len() {
            continue;
        }
        let (ax, ay) = (d[k][0] - d[k - ARM][0], d[k][1] - d[k - ARM][1]);
        let (bx, by) = (d[k + ARM][0] - d[k][0], d[k + ARM][1] - d[k][1]);
        let (la, lb) = ((ax * ax + ay * ay).sqrt(), (bx * bx + by * by).sqrt());
        if la < 1e-6 || lb < 1e-6 {
            continue;
        }
        let angle = ((ax * bx + ay * by) / (la * lb)).clamp(-1.0, 1.0).acos();
        if angle > 60f32.to_radians() && best.map_or(true, |(_, a)| angle > a) {
            best = Some((k, angle));
        }
    }
    best.map(|(k, _)| k)
}

/// A hook lives in the first or last fifth of a stroke. The corners of a V or
/// a Z are further in than that and are never mistaken for one.
fn hook_at_start(d: &[Pt]) -> Option<usize> {
    sharpest_turn(d, 3..=d.len() / 5)
}

fn hook_at_end(d: &[Pt]) -> Option<usize> {
    sharpest_turn(d, d.len() - 1 - d.len() / 5..=d.len() - 4)
}

/// How alike two raw strokes are, 0..1, compared the way the recogniser would.
pub fn similarity(a: &[Pt], b: &[Pt], cfg: &RecognizerCfg, free_rotation: bool) -> f32 {
    let n = cfg.resample_points;
    let (ua, ub) = (normalize_uniform(a, n), normalize_uniform(b, n));
    let (pa, pb) = if ua.line || ub.line { (ua.points, ub.points) } else { (normalize(a, n).points, normalize(b, n).points) };
    let max_rot = if free_rotation { std::f32::consts::PI } else { cfg.rotation_tolerance_deg.to_radians() };
    fit(&pa, &pb, max_rot, cfg.scale_tolerance, cfg.elastic_window).score
}

/// Map a shape (as stored: centred, longer side 1) onto the screen where a
/// stroke was drawn, turned by `rotation`.
pub fn place(shape: &[Pt], frame: Frame, rotation: f32) -> Vec<Pt> {
    let (sin, cos) = rotation.sin_cos();
    shape
        .iter()
        .map(|p| {
            [
                frame.centroid[0] + (p[0] * cos - p[1] * sin) * frame.size,
                frame.centroid[1] + (p[0] * sin + p[1] * cos) * frame.size,
            ]
        })
        .collect()
}

/// Point-wise mean of several strokes of the same gesture (each already
/// `normalize_uniform`ed), each rotated and scaled onto the first. Used to
/// turn three recorded samples into one clean shape. Lightly smoothed, ends
/// untouched.
pub fn average(samples: &[Vec<Pt>], free_rotation: bool) -> Vec<Pt> {
    let Some(first) = samples.first() else {
        return Vec::new();
    };
    let n = first.len();
    let mut sum = vec![[0.0f32; 2]; n];
    for s in samples {
        let max_rot = if free_rotation { std::f32::consts::PI } else { 0.5 };
        let f = fit(s, first, max_rot, 0.3, 0.0);
        let (sin, cos) = f.rotation.sin_cos();
        for i in 0..n.min(s.len()) {
            sum[i][0] += (s[i][0] * cos - s[i][1] * sin) * f.scale;
            sum[i][1] += (s[i][0] * sin + s[i][1] * cos) * f.scale;
        }
    }
    let k = samples.len() as f32;
    let mut out: Vec<Pt> = sum.iter().map(|p| [p[0] / k, p[1] / k]).collect();
    for _ in 0..2 {
        let src = out.clone();
        for i in 1..n.saturating_sub(1) {
            out[i] = [
                0.25 * src[i - 1][0] + 0.5 * src[i][0] + 0.25 * src[i + 1][0],
                0.25 * src[i - 1][1] + 0.5 * src[i][1] + 0.25 * src[i + 1][1],
            ];
        }
    }
    normalize_uniform(&out, n).points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn cfg() -> RecognizerCfg {
        Config::default().recognizer
    }

    fn line(from: Pt, to: Pt) -> Vec<Pt> {
        (0..=20).map(|i| {
            let t = i as f32 / 20.0;
            [from[0] + t * (to[0] - from[0]), from[1] + t * (to[1] - from[1])]
        }).collect()
    }

    fn poly(corners: &[Pt]) -> Vec<Pt> {
        corners.windows(2).flat_map(|w| line(w[0], w[1])).collect()
    }

    fn rotated(pts: &[Pt], deg: f32) -> Vec<Pt> {
        let (s, c) = deg.to_radians().sin_cos();
        pts.iter().map(|p| [p[0] * c - p[1] * s, p[0] * s + p[1] * c]).collect()
    }

    fn circle(start_deg: f32, turns: f32, rx: f32, ry: f32) -> Vec<Pt> {
        (0..=64)
            .map(|i| {
                let a = start_deg.to_radians() + turns * i as f32 / 64.0 * std::f32::consts::TAU;
                [a.cos() * rx, a.sin() * ry]
            })
            .collect()
    }

    fn template(gesture: usize, pts: &[Pt], free: bool) -> Template {
        Template::new(gesture, 0, free, pts, 64)
    }

    fn is(v: Verdict, g: usize) -> bool {
        matches!(v, Verdict::Match(m) if m.gesture == g)
    }

    #[test]
    fn resample_is_evenly_spaced_and_keeps_ends() {
        let pts = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        let r = resample(&pts, 21);
        assert_eq!(r.len(), 21);
        assert_eq!(r[0], [0.0, 0.0]);
        assert!(dist(r[20], [10.0, 10.0]) < 1e-3);
        for w in r.windows(2) {
            assert!((dist(w[0], w[1]) - 1.0).abs() < 1e-3);
        }
        assert_eq!(resample(&[[3.0, 4.0]], 8), vec![[3.0, 4.0]; 8]);
        assert_eq!(resample(&[], 4).len(), 4);
    }

    #[test]
    fn thinness_tells_lines_from_shapes_at_any_angle() {
        assert!(thinness(&line([0.0, 0.0], [300.0, 0.0])) < 0.01);
        assert!(thinness(&line([0.0, 0.0], [300.0, 300.0])) < 0.01, "a diagonal fills its box but is still a line");
        assert!(thinness(&circle(0.0, 1.0, 100.0, 100.0)) > 0.9);
        assert!(thinness(&poly(&[[-0.2, -0.5], [0.0, 0.5], [0.2, -0.5]])) > LINE_THINNESS, "a narrow V is a shape");
    }

    #[test]
    fn lines_keep_proportions_and_shapes_fill_the_box() {
        let a = line([0.0, 0.0], [100.0, 40.0]);
        let b: Vec<Pt> = a.iter().map(|p| [p[0] * 7.0 + 900.0, p[1] * 7.0 - 300.0]).collect();
        let (na, nb) = (normalize(&a, 32), normalize(&b, 32));
        assert!(na.line);
        for i in 0..32 {
            assert!(dist(na.points[i], nb.points[i]) < 1e-4);
        }
        assert!((nb.size() / na.size() - 7.0).abs() < 1e-3);
        assert!(na.points.iter().all(|p| (p[1] - p[0] * 0.4).abs() < 1e-4), "a line is not stretched");

        let oval = normalize(&circle(0.0, 1.0, 150.0, 260.0), 64);
        assert!(!oval.line);
        let (w, h) = bounds(&oval.points);
        assert!((w - 1.0).abs() < 0.01 && (h - 1.0).abs() < 0.01, "an oval becomes round");
        assert!((oval.size() - 410.0).abs() < 6.0);
        let kept = normalize_uniform(&circle(0.0, 1.0, 150.0, 260.0), 64);
        let (w, h) = bounds(&kept.points);
        assert!((w - 150.0 / 260.0).abs() < 0.02 && (h - 1.0).abs() < 0.01, "stored shapes keep proportions");
    }

    #[test]
    fn straight_lines_keep_their_direction() {
        let all = vec![
            template(0, &line([0.0, 1.0], [0.0, 0.0]), false),
            template(1, &line([0.0, 0.0], [1.0, 0.0]), false),
            template(2, &line([0.0, 0.0], [0.0, 1.0]), false),
            template(3, &line([0.5, 0.0], [-0.5, 1.0]), false),
        ];
        let c = cfg();
        assert!(is(recognize(&line([5.0, 400.0], [5.0, 20.0]), &all, &c), 0));
        assert!(is(recognize(&line([0.0, 0.0], [300.0, 0.0]), &all, &c), 1));
        assert!(is(recognize(&line([0.0, 0.0], [0.0, 300.0]), &all, &c), 2));
        // A flick up drawn 10 degrees off is still a flick up.
        assert!(is(recognize(&rotated(&line([0.0, 300.0], [0.0, 0.0]), 10.0), &all, &c), 0));
        // The slash is its own gesture, not a sloppy flick down.
        assert!(is(recognize(&line([200.0, 0.0], [0.0, 200.0]), &all, &c), 3));
        // Left has no template here: must not match "right" backwards.
        assert!(matches!(recognize(&line([300.0, 0.0], [0.0, 0.0]), &all, &c), Verdict::NoMatch { .. }));
    }

    #[test]
    fn ambiguous_stroke_is_rejected_by_margin() {
        let down = template(0, &line([0.0, 0.0], [0.0, 1.0]), false);
        let right = template(1, &line([0.0, 0.0], [1.0, 0.0]), false);
        // Exactly between the two.
        assert!(matches!(recognize(&line([0.0, 0.0], [200.0, 200.0]), &[down, right], &cfg()), Verdict::NoMatch { .. }));
    }

    #[test]
    fn free_rotation_matches_any_start_angle_but_not_the_mirror() {
        let cw = template(0, &circle(-90.0, 1.0, 1.0, 1.0), true);
        let c = cfg();
        for start in [0.0, 37.0, 141.0, 250.0] {
            assert!(is(recognize(&circle(start, 1.0, 120.0, 120.0), std::slice::from_ref(&cw), &c), 0), "start {start}");
        }
        assert!(matches!(recognize(&circle(-90.0, -1.0, 120.0, 120.0), &[cw], &c), Verdict::NoMatch { .. }));
    }

    #[test]
    fn what_a_real_hand_does_to_a_circle() {
        let all = vec![template(0, &circle(-90.0, 1.0, 1.0, 1.0), true), template(1, &line([0.0, 0.0], [0.0, 1.0]), false)];
        let c = cfg();
        // An oval, taller than wide.
        assert!(is(recognize(&circle(40.0, 1.0, 150.0, 260.0), &all, &c), 0));
        // A turn and a third: the first full turn is the gesture.
        match recognize(&circle(10.0, 1.33, 200.0, 240.0), &all, &c) {
            Verdict::Match(m) => assert!(m.gesture == 0 && m.trimmed),
            other => panic!("overshoot not rescued: {other:?}"),
        }
        // A tail leading in, like a 6.
        let mut six = line([420.0, -520.0], [215.0, -60.0]);
        six.extend(circle(-20.0, 1.0, 210.0, 200.0));
        match recognize(&six, &all, &c) {
            Verdict::Match(m) => {
                assert!(m.gesture == 0 && m.trimmed);
                // The clean shape belongs on the loop, not on loop plus tail.
                assert!(m.frame.centroid[1].abs() < 40.0, "frame centre {:?}", m.frame.centroid);
            }
            other => panic!("tail not rescued: {other:?}"),
        }
        // Half a circle is not a circle.
        assert!(matches!(recognize(&circle(0.0, 0.5, 200.0, 200.0), &all, &c), Verdict::NoMatch { .. }));
    }

    #[test]
    fn hooks_are_forgiven_but_not_free() {
        let all = vec![template(0, &line([0.0, 1.0], [0.0, 0.0]), false), template(1, &line([0.0, 0.0], [1.0, 0.0]), false)];
        let c = cfg();
        // A flick up that dips first (wind-up) and bounces back at the top.
        let hooked = poly(&[[0.0, 500.0], [8.0, 560.0], [0.0, 0.0], [-10.0, 55.0]]);
        match recognize(&hooked, &all, &c) {
            Verdict::Match(m) => assert_eq!(m.gesture, 0),
            other => panic!("hooked flick missed: {other:?}"),
        }
        // An L is not a flick with a hook: its foot is too long to be one.
        let l = poly(&[[0.0, 0.0], [0.0, 300.0], [200.0, 300.0]]);
        assert!(matches!(recognize(&l, &all, &c), Verdict::NoMatch { .. }));
    }

    #[test]
    fn elastic_matching_lines_up_a_lopsided_corner() {
        let v = template(0, &poly(&[[-0.42, -0.5], [0.0, 0.5], [0.42, -0.5]]), false);
        let c = cfg();
        // A lopsided V: first arm three quarters as long as the second.
        let tick = poly(&[[-110.0, -80.0], [0.0, 200.0], [150.0, -200.0]]);
        let stroke = normalize(&tick, 64);
        let rigid = fit(&stroke.points, &v.points, 0.2, 0.2, 0.0).score;
        let elastic = fit(&stroke.points, &v.points, 0.2, 0.2, c.elastic_window).score;
        assert!(elastic > rigid + 0.02, "rigid {rigid}, elastic {elastic}");
        assert!(is(recognize(&tick, &[v], &c), 0), "elastic {elastic}");
    }

    #[test]
    fn a_sagging_flick_is_not_stretched_into_a_v() {
        let v = template(0, &poly(&[[-0.42, -0.5], [0.0, 0.5], [0.42, -0.5]]), false);
        // Left to right, dipping 12% in the middle.
        let sag: Vec<Pt> = (0..=30).map(|i| {
            let t = i as f32 / 30.0;
            [t * 500.0, (std::f32::consts::PI * t).sin() * 60.0]
        }).collect();
        assert!(matches!(recognize(&sag, &[v], &cfg()), Verdict::NoMatch { best: None }));
    }

    #[test]
    fn place_puts_the_shape_where_the_stroke_was() {
        let stroke_px = line([1000.0, 800.0], [1000.0, 400.0]);
        let up = template(0, &line([0.0, 1.0], [0.0, 0.0]), false);
        let Verdict::Match(m) = recognize(&stroke_px, std::slice::from_ref(&up), &cfg()) else { panic!("no match") };
        let ideal = normalize_uniform(&line([0.0, 1.0], [0.0, 0.0]), 64).points;
        let placed = place(&ideal, m.frame, 0.0);
        assert!(dist(placed[0], [1000.0, 800.0]) < 2.0);
        assert!(dist(placed[63], [1000.0, 400.0]) < 2.0);
    }

    #[test]
    fn average_of_noisy_samples_is_close_to_the_shape() {
        let v = |jx: f32, jy: f32| -> Vec<Pt> {
            normalize_uniform(&poly(&[[-0.45 + jx, -0.5], [0.0, 0.5 + jy], [0.45, -0.5 - jx]]), 64).points
        };
        let ideal = v(0.0, 0.0);
        let avg = average(&[v(0.06, -0.04), v(-0.05, 0.05), v(0.01, 0.02)], false);
        assert!(fit(&avg, &ideal, 0.2, 0.2, 0.0).score > 0.93);
    }
}
