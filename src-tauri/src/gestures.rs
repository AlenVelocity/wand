//! The gesture library: what each gesture looks like and what it does.
//!
//! Stored as `gestures.json` next to `wand.toml`. Shapes are polylines in
//! template space (centred, longest side 1, y down as on screen).

use crate::recognizer::{self, Pt, Template};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// "Ctrl+Shift+S", "Alt+F4", "Win+PrintScreen".
    KeyCombo { keys: String },
    /// A media or volume key, pressed `repeat` times.
    MediaKey { key: String, #[serde(default = "one")] repeat: u8 },
    /// Open a program, file, folder or URL.
    Launch { path: String, #[serde(default)] args: String },
    /// Run a command line in the shell named in `wand.toml`.
    Shell { command: String },
    None,
}

fn one() -> u8 {
    1
}

pub const MEDIA_KEYS: &[(&str, &str)] = &[
    ("volume_up", "Volume up"),
    ("volume_down", "Volume down"),
    ("mute", "Mute"),
    ("play_pause", "Play / pause"),
    ("next_track", "Next track"),
    ("prev_track", "Previous track"),
    ("stop", "Stop"),
];

impl Action {
    /// What the binding does, in the words the card and overlay show when no
    /// label was typed.
    pub fn describe(&self) -> String {
        match self {
            Action::KeyCombo { keys } => keys.clone(),
            Action::MediaKey { key, .. } => MEDIA_KEYS
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, l)| l.to_string())
                .unwrap_or_else(|| key.clone()),
            Action::Launch { path, .. } => Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
            Action::Shell { command } => command.clone(),
            Action::None => "Nothing".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    /// Matches only near the drawn angle (`rotation_tolerance_deg`).
    #[default]
    Bounded,
    /// Matches at any rotation and from any starting point (closed shapes).
    Free,
}

/// One way of drawing a gesture. A circle has two (clockwise and counter);
/// a recorded gesture has one, with the three samples it was built from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Variant {
    /// The clean shape: shown on the card and what a matched stroke morphs to.
    pub ideal: Vec<Pt>,
    /// Raw recordings, also matched against.
    #[serde(default)]
    pub samples: Vec<Vec<Pt>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Gesture {
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub rotation: Rotation,
    pub action: Action,
    /// Shown on the overlay at a match. Empty means `action.describe()`.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub icon: String,
    pub variants: Vec<Variant>,
    #[serde(default)]
    pub builtin: bool,
}

fn yes() -> bool {
    true
}

impl Gesture {
    pub fn display_label(&self) -> String {
        if self.label.trim().is_empty() {
            self.action.describe()
        } else {
            self.label.clone()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Library {
    pub version: u32,
    pub gestures: Vec<Gesture>,
}

impl Library {
    /// Templates for every enabled gesture, resampled to `n` points.
    pub fn templates(&self, n: usize) -> Vec<Template> {
        let mut out = Vec::new();
        for (gi, g) in self.gestures.iter().enumerate() {
            if !g.enabled {
                continue;
            }
            for (vi, v) in g.variants.iter().enumerate() {
                let shapes = std::iter::once(&v.ideal).chain(v.samples.iter());
                for s in shapes {
                    if s.len() < 2 {
                        continue;
                    }
                    out.push(Template::new(gi, vi, g.rotation == Rotation::Free, s, n));
                }
            }
        }
        out
    }

    pub fn load_or_init(path: &Path) -> (Library, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Library>(&text) {
                Ok(mut lib) => {
                    if lib.upgrade() {
                        let _ = lib.save(path);
                    }
                    (lib, None)
                }
                Err(e) => (defaults(), Some(format!("gestures.json: {e}. Using the built-in gestures; your file was left untouched."))),
            },
            Err(_) => {
                let lib = defaults();
                let _ = lib.save(path);
                (lib, None)
            }
        }
    }

    /// Bring built-in gestures in a file written by an older version up to
    /// the shapes this version ships. Actions, labels and on/off are the
    /// user's and stay; so does any built-in they drew again themselves.
    /// Returns whether anything changed.
    pub fn upgrade(&mut self) -> bool {
        if self.version >= LIBRARY_VERSION {
            return false;
        }
        let shipped = defaults();
        for g in self.gestures.iter_mut().filter(|g| g.builtin) {
            let redrawn = g.variants.iter().any(|v| !v.samples.is_empty());
            if let (false, Some(new)) = (redrawn, shipped.gestures.iter().find(|d| d.id == g.id)) {
                g.variants = new.variants.clone();
            }
        }
        for new in shipped.gestures {
            let since_then = ADDED.iter().any(|(id, version)| *id == new.id && *version > self.version);
            if since_then && !self.gestures.iter().any(|g| g.id == new.id) {
                self.gestures.push(new);
            }
        }
        self.version = LIBRARY_VERSION;
        true
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write to a sibling file first so a crash mid-write cannot leave a
        // half-written library behind.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn unique_id(&self, name: &str) -> String {
        let base: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let base = if base.is_empty() { "gesture".to_string() } else { base };
        let mut id = base.clone();
        let mut i = 2;
        while self.gestures.iter().any(|g| g.id == id) {
            id = format!("{base}-{i}");
            i += 1;
        }
        id
    }
}

// ------------------------------------------------------------------- defaults

const N: usize = 64;

/// 2: the slash is 45 degrees and may be drawn upward; the V gained a tick.
/// 3: six more gestures (see `ADDED`).
pub const LIBRARY_VERSION: u32 = 3;

/// Built-ins that did not exist before the given library version. A file
/// older than that gains them on upgrade. Any other built-in missing from a
/// file was deleted by its owner and stays deleted.
const ADDED: &[(&str, u32)] = &[("chevron", 3), ("caret", 3), ("infinity", 3), ("star", 3), ("m", 3), ("s", 3)];

fn poly(corners: &[Pt]) -> Vec<Pt> {
    recognizer::normalize_uniform(corners, N).points
}

fn circle(clockwise: bool) -> Vec<Pt> {
    // Starts at the top. On screen y grows downward, so increasing the angle
    // from -90 degrees runs clockwise.
    let dir = if clockwise { 1.0 } else { -1.0 };
    let pts: Vec<Pt> = (0..=96)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + dir * i as f32 / 96.0 * std::f32::consts::TAU;
            [a.cos() * 0.5, a.sin() * 0.5]
        })
        .collect();
    recognizer::normalize_uniform(&pts, N).points
}

/// A triangle with its point up: any corner first, either way round.
fn triangles() -> Vec<Vec<Pt>> {
    let c: [Pt; 3] = [[0.0, -0.433], [0.5, 0.433], [-0.5, 0.433]];
    let mut out = Vec::new();
    for start in 0..3 {
        for step in [1, 2] {
            let at = |k: usize| c[(start + k * step) % 3];
            out.push(poly(&[at(0), at(1), at(2), at(0)]));
        }
    }
    out
}

/// A five-pointed star in one stroke from the top point: every second corner
/// of a pentagon.
fn star(clockwise: bool) -> Vec<Pt> {
    let step = if clockwise { 2 } else { 3 };
    let pts: Vec<Pt> = (0..=5)
        .map(|i| {
            let a = (-90.0 + 72.0 * ((i * step) % 5) as f32).to_radians();
            [a.cos() * 0.5, a.sin() * 0.5]
        })
        .collect();
    poly(&pts)
}

/// A figure eight on its side, from the crossing. `sx` and `sy` pick which
/// way it sets off: right or left, down or up.
fn infinity(sx: f32, sy: f32) -> Vec<Pt> {
    let pts: Vec<Pt> = (0..=96)
        .map(|i| {
            let t = i as f32 / 96.0 * std::f32::consts::TAU;
            [sx * 0.5 * t.sin(), sy * 0.25 * (2.0 * t).sin()]
        })
        .collect();
    poly(&pts)
}

/// An S from its top right end: over the upper bowl, through the middle,
/// round the lower one.
fn s_curve() -> Vec<Pt> {
    let arc = |cy: f32, from: f32, to: f32| {
        (0..=24).map(move |i| {
            let a = (from + (to - from) * i as f32 / 24.0).to_radians();
            [a.cos() * 0.25, cy + a.sin() * 0.25]
        })
    };
    let pts: Vec<Pt> = arc(-0.25, -30.0, -270.0).chain(arc(0.25, -90.0, 210.0)).collect();
    poly(&pts)
}

fn builtin(id: &str, name: &str, rotation: Rotation, action: Action, label: &str, icon: &str, shapes: Vec<Vec<Pt>>) -> Gesture {
    Gesture {
        id: id.into(),
        name: name.into(),
        enabled: true,
        rotation,
        action,
        label: label.into(),
        icon: icon.into(),
        variants: shapes.into_iter().map(|ideal| Variant { ideal, samples: vec![] }).collect(),
        builtin: true,
    }
}

fn key(keys: &str) -> Action {
    Action::KeyCombo { keys: keys.into() }
}
fn media(key: &str, repeat: u8) -> Action {
    Action::MediaKey { key: key.into(), repeat }
}

/// The gestures the app ships with.
pub fn defaults() -> Library {
    use Rotation::*;
    Library {
        version: LIBRARY_VERSION,
        gestures: vec![
            builtin("circle", "Circle", Free, key("Win+PrintScreen"), "Screenshot", "camera",
                vec![circle(true), circle(false)]),
            // A "/" at 45 degrees, drawn either way: down like a falling blade,
            // or up, the way it tends to be drawn in the air.
            builtin("slash", "Slash", Bounded, key("Alt+F4"), "Close window", "close",
                vec![poly(&[[0.5, -0.5], [-0.5, 0.5]]), poly(&[[-0.5, 0.5], [0.5, -0.5]])]),
            builtin("z", "Z", Bounded, key("Ctrl+Z"), "Undo", "undo",
                vec![poly(&[[-0.5, -0.4], [0.5, -0.4], [-0.5, 0.4], [0.5, 0.4]])]),
            // The V as printed, and as it comes out of a wrist: a tick, with a
            // short first arm.
            builtin("v", "V", Bounded, key("Ctrl+V"), "Paste", "paste",
                vec![poly(&[[-0.42, -0.5], [0.0, 0.5], [0.42, -0.5]]), poly(&[[-0.3, -0.05], [-0.05, 0.5], [0.42, -0.5]])]),
            builtin("flick-up", "Flick up", Bounded, media("volume_up", 3), "Volume up", "volume-up",
                vec![poly(&[[0.0, 0.5], [0.0, -0.5]])]),
            builtin("flick-down", "Flick down", Bounded, media("volume_down", 3), "Volume down", "volume-down",
                vec![poly(&[[0.0, -0.5], [0.0, 0.5]])]),
            builtin("flick-left", "Flick left", Bounded, media("prev_track", 1), "Previous track", "prev",
                vec![poly(&[[0.5, 0.0], [-0.5, 0.0]])]),
            builtin("flick-right", "Flick right", Bounded, media("next_track", 1), "Next track", "next",
                vec![poly(&[[-0.5, 0.0], [0.5, 0.0]])]),
            builtin("chevron", "Chevron", Bounded, media("play_pause", 1), "Play / pause", "play",
                vec![poly(&[[-0.3, -0.5], [0.3, 0.0], [-0.3, 0.5]])]),
            // Up and over: the V's opposite, and its opposite number.
            builtin("caret", "Caret", Bounded, key("Ctrl+C"), "Copy", "copy",
                vec![poly(&[[-0.42, 0.5], [0.0, -0.5], [0.42, 0.5]])]),
            builtin("infinity", "Infinity", Bounded, key("Alt+Tab"), "Switch window", "switch",
                vec![infinity(1.0, -1.0), infinity(-1.0, -1.0), infinity(1.0, 1.0), infinity(-1.0, 1.0)]),
            builtin("star", "Star", Free, key("Win+Tab"), "Task view", "windows",
                vec![star(true), star(false)]),
            builtin("m", "M", Bounded, media("mute", 1), "Mute", "mute",
                vec![poly(&[[-0.5, 0.5], [-0.28, -0.5], [0.0, 0.2], [0.28, -0.5], [0.5, 0.5]])]),
            builtin("s", "S", Bounded, key("Ctrl+S"), "Save", "save",
                vec![s_curve()]),
        ],
    }
}

/// Build a gesture from recorded strokes (screen-space polylines).
pub fn from_recordings(lib: &Library, name: &str, strokes: &[Vec<Pt>], rotation: Rotation, action: Action, label: &str, icon: &str) -> Gesture {
    let samples: Vec<Vec<Pt>> = strokes.iter().map(|s| recognizer::normalize_uniform(s, N).points).collect();
    let ideal = recognizer::average(&samples, rotation == Rotation::Free);
    Gesture {
        id: lib.unique_id(name),
        name: name.trim().to_string(),
        enabled: true,
        rotation,
        action,
        label: label.trim().to_string(),
        icon: icon.to_string(),
        variants: vec![Variant { ideal, samples }],
        builtin: false,
    }
}

/// How alike recorded samples are (worst pairwise score), and which existing
/// gesture the new shape would be confused with, if any.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RecordingCheck {
    pub consistency: f32,
    pub pair_scores: Vec<f32>,
    pub conflict: Option<Conflict>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Conflict {
    pub gesture_id: String,
    pub gesture_name: String,
    pub score: f32,
}

pub fn check_recordings(lib: &Library, strokes: &[Vec<Pt>], cfg: &crate::config::RecognizerCfg, ignore_id: Option<&str>) -> RecordingCheck {
    let mut pair_scores = Vec::new();
    for i in 0..strokes.len() {
        for j in i + 1..strokes.len() {
            pair_scores.push(recognizer::similarity(&strokes[i], &strokes[j], cfg, false));
        }
    }
    let consistency = pair_scores.iter().copied().fold(1.0f32, f32::min);

    let mut conflict: Option<Conflict> = None;
    let templates = lib.templates(cfg.resample_points);
    for s in strokes {
        for (gi, score) in recognizer::scores_by_gesture(s, &templates, cfg) {
            let g = &lib.gestures[gi];
            if Some(g.id.as_str()) == ignore_id {
                continue;
            }
            if score >= cfg.min_score - cfg.min_margin && conflict.as_ref().map_or(true, |c| score > c.score) {
                conflict = Some(Conflict { gesture_id: g.id.clone(), gesture_name: g.name.clone(), score });
            }
        }
    }
    RecordingCheck { consistency, pair_scores, conflict }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::recognizer::{recognize, Verdict};

    #[test]
    fn default_templates_do_not_collide() {
        let lib = defaults();
        let cfg = Config::default().recognizer;
        let templates = lib.templates(cfg.resample_points);
        // Every ideal shape must recognise as its own gesture with a clear margin.
        for (gi, g) in lib.gestures.iter().enumerate() {
            for v in &g.variants {
                match recognize(&v.ideal, &templates, &cfg) {
                    Verdict::Match(m) => {
                        assert_eq!(m.gesture, gi, "{} matched as {}", g.name, lib.gestures[m.gesture].name);
                        assert!(m.fit.score > 0.99);
                        assert!(m.fit.score - m.runner_up > 0.15, "{}: runner-up {}", g.name, m.runner_up);
                    }
                    Verdict::NoMatch { best } => panic!("{} did not match itself: {best:?}", g.name),
                }
            }
        }
    }

    #[test]
    fn library_round_trips_through_json() {
        let lib = defaults();
        let text = serde_json::to_string(&lib).unwrap();
        assert_eq!(serde_json::from_str::<Library>(&text).unwrap(), lib);
        assert!(text.contains("\"type\":\"key_combo\""));
        assert!(text.contains("\"type\":\"media_key\""));
    }

    #[test]
    fn disabled_gestures_produce_no_templates() {
        let mut lib = defaults();
        let before = lib.templates(64).len();
        lib.gestures[0].enabled = false; // circle: two variants
        assert_eq!(lib.templates(64).len(), before - 2);
    }

    #[test]
    fn recorded_gesture_is_usable_and_conflicts_are_reported() {
        let lib = defaults();
        let cfg = Config::default().recognizer;
        // An "L": down then right. Three slightly different drawings, in pixels.
        let l = |w: f32, h: f32, ox: f32| -> Vec<Pt> {
            vec![[ox, 0.0], [ox + 4.0, h * 0.5], [ox, h], [ox + w * 0.5, h + 3.0], [ox + w, h]]
        };
        let strokes = vec![l(180.0, 260.0, 500.0), l(200.0, 240.0, 80.0), l(170.0, 280.0, 900.0)];
        let check = check_recordings(&lib, &strokes, &cfg, None);
        assert!(check.consistency > 0.85, "consistency {}", check.consistency);
        assert!(check.conflict.is_none(), "{:?}", check.conflict);

        let g = from_recordings(&lib, "Open Terminal!", &strokes, Rotation::Bounded,
            Action::Launch { path: "wt.exe".into(), args: String::new() }, "", "terminal");
        assert_eq!(g.id, "open-terminal");
        assert_eq!(g.display_label(), "wt");
        assert_eq!(g.variants[0].samples.len(), 3);

        let mut lib2 = lib.clone();
        lib2.gestures.push(g);
        let templates = lib2.templates(cfg.resample_points);
        assert!(matches!(recognize(&l(210.0, 250.0, 300.0), &templates, &cfg), Verdict::Match(m) if lib2.gestures[m.gesture].id == "open-terminal"));

        // Re-recording a flick down must be flagged as a conflict.
        let flick = vec![vec![[0.0, 0.0], [2.0, 300.0]], vec![[0.0, 0.0], [-3.0, 280.0]], vec![[0.0, 0.0], [0.0, 310.0]]];
        let c = check_recordings(&lib, &flick, &cfg, None).conflict.expect("conflict");
        assert_eq!(c.gesture_id, "flick-down");
    }

    #[test]
    fn older_library_gets_new_shapes_but_keeps_bindings() {
        let mut old = defaults();
        old.version = 1;
        // As version 1 shipped it: one steep downward slash, rebound by the user.
        old.gestures[1].variants = vec![Variant { ideal: poly(&[[0.38, -0.5], [-0.38, 0.5]]), samples: vec![] }];
        old.gestures[1].action = Action::KeyCombo { keys: "Ctrl+W".into() };
        // A built-in the user drew again is theirs.
        let mine = Variant { ideal: poly(&[[-0.5, -0.5], [0.0, 0.5], [0.5, -0.4]]), samples: vec![poly(&[[-0.5, -0.5], [0.0, 0.5], [0.5, -0.4]])] };
        old.gestures[3].variants = vec![mine.clone()];
        assert!(old.upgrade());
        assert_eq!(old.version, LIBRARY_VERSION);
        assert_eq!(old.gestures[1].variants.len(), 2);
        assert_eq!(old.gestures[1].action, Action::KeyCombo { keys: "Ctrl+W".into() });
        assert_eq!(old.gestures[3].variants, vec![mine]);
        assert_eq!(old.gestures.len(), defaults().gestures.len(), "gestures added since version 1 arrive");
        assert!(!old.upgrade(), "a current library is left alone");
    }

    #[test]
    fn upgrade_adds_new_gestures_without_restoring_deleted_ones() {
        // A version 2 library: the first eight, with Z deleted by its owner.
        let mut old = defaults();
        old.version = 2;
        old.gestures.retain(|g| !ADDED.iter().any(|(id, _)| *id == g.id) && g.id != "z");
        assert_eq!(old.gestures.len(), 7);
        assert!(old.upgrade());
        let ids: Vec<&str> = old.gestures.iter().map(|g| g.id.as_str()).collect();
        assert!(!ids.contains(&"z"), "a deleted built-in stays deleted");
        for (id, _) in ADDED {
            assert!(ids.contains(id), "{id} was not added");
        }
        assert_eq!(&ids[..7], ["circle", "slash", "v", "flick-up", "flick-down", "flick-left", "flick-right"], "existing order is kept");
    }

    #[test]
    fn unique_ids() {
        let lib = defaults();
        assert_eq!(lib.unique_id("Circle"), "circle-2");
        assert_eq!(lib.unique_id("  "), "gesture");
    }
}
