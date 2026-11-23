//! `wand.toml`: the one file that holds every threshold.
//!
//! Reads go through serde; writes go through `toml_edit` so that changing one
//! value from the settings window keeps the user's comments and layout.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_TOML: &str = include_str!("../defaults/wand.toml");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RecognizerCfg {
    pub min_score: f32,
    pub min_margin: f32,
    pub resample_points: usize,
    pub rotation_tolerance_deg: f32,
    pub scale_tolerance: f32,
    pub elastic_window: f32,
    pub second_chance: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StrokeCfg {
    pub min_length_px: f32,
    pub min_duration_ms: u32,
    pub min_length_deg: f32,
    pub min_speed_deg_s: f32,
    pub max_duration_ms: u32,
    pub triggers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PointingCfg {
    pub px_per_degree: f32,
    pub anchor: String,
    pub edge_margin_px: f32,
    pub forward_axis: String,
    pub up_axis: String,
    pub invert_x: bool,
    pub invert_y: bool,
    pub recenter_buttons: Vec<String>,
    pub smoothing_min_cutoff_hz: f32,
    pub smoothing_beta: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MouseCfg {
    pub enabled: bool,
    pub sensitivity: f32,
    pub smoothing_ms: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    pub left_buttons: Vec<String>,
    pub right_buttons: Vec<String>,
    pub middle_buttons: Vec<String>,
    pub scroll_speed: f32,
    pub scroll_deadzone: f32,
    pub invert_scroll: bool,
    pub max_distance: u16,
    pub max_roughness: u16,
    pub max_tilt_deg: f32,
    pub settle_ms: f32,
    pub linger_ms: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FusionCfg {
    pub use_magnetometer: bool,
    pub accel_gain: f32,
    pub accel_gate_g: f32,
    pub mag_gain: f32,
    pub mag_gate: f32,
    pub still_gyro_dps: f32,
    pub still_accel_g: f32,
    pub still_time_ms: f32,
    pub bias_time_constant_s: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OverlayCfg {
    pub prediction_ms: f32,
    pub prediction_mouse_ms: f32,
    pub confirm_ms: f32,
    pub tail_fade_ms: f32,
    pub tail_floor: f32,
    pub width_min_px: f32,
    pub width_max_px: f32,
    pub speed_full_px_s: f32,
    pub spark_rate: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RumbleCfg {
    pub enabled: bool,
    pub match_strength: f32,
    pub match_ms: u32,
    pub no_match_strength: f32,
    pub no_match_ms: u32,
    pub no_match_gap_ms: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DevCfg {
    pub enabled: bool,
    pub hold_key: String,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ActionsCfg {
    pub shell: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub recognizer: RecognizerCfg,
    pub stroke: StrokeCfg,
    pub pointing: PointingCfg,
    pub mouse: MouseCfg,
    pub fusion: FusionCfg,
    pub overlay: OverlayCfg,
    pub rumble: RumbleCfg,
    pub dev: DevCfg,
    pub actions: ActionsCfg,
}

// The Default impls parse the shipped file, so the numbers exist in exactly
// one place: defaults/wand.toml.
fn shipped() -> &'static Config {
    use std::sync::OnceLock;
    static SHIPPED: OnceLock<Config> = OnceLock::new();
    SHIPPED.get_or_init(|| {
        #[derive(Deserialize)]
        struct Strict {
            recognizer: RecognizerRaw,
            stroke: StrokeRaw,
            pointing: PointingRaw,
            mouse: MouseRaw,
            fusion: FusionRaw,
            overlay: OverlayRaw,
            rumble: RumbleRaw,
            dev: DevRaw,
            actions: ActionsRaw,
        }
        let s: Strict = toml::from_str(DEFAULT_TOML).expect("defaults/wand.toml must be complete");
        Config {
            recognizer: s.recognizer.into(),
            stroke: s.stroke.into(),
            pointing: s.pointing.into(),
            mouse: s.mouse.into(),
            fusion: s.fusion.into(),
            overlay: s.overlay.into(),
            rumble: s.rumble.into(),
            dev: s.dev.into(),
            actions: s.actions.into(),
        }
    })
}

/// Mirror structs without `#[serde(default)]`, so a missing key in the shipped
/// file is a build-time-test failure instead of a silent zero.
macro_rules! raw {
    ($raw:ident => $real:ident { $($f:ident : $t:ty),* $(,)? }, $section:ident) => {
        #[derive(Deserialize)]
        struct $raw { $($f: $t),* }
        impl From<$raw> for $real {
            fn from(r: $raw) -> Self { $real { $($f: r.$f),* } }
        }
        impl Default for $real {
            fn default() -> Self { shipped().$section.clone() }
        }
    };
}

raw!(RecognizerRaw => RecognizerCfg {
    min_score: f32, min_margin: f32, resample_points: usize,
    rotation_tolerance_deg: f32, scale_tolerance: f32, elastic_window: f32, second_chance: bool,
}, recognizer);
raw!(StrokeRaw => StrokeCfg {
    min_length_px: f32, min_duration_ms: u32, min_length_deg: f32, min_speed_deg_s: f32,
    max_duration_ms: u32, triggers: Vec<String>,
}, stroke);
raw!(PointingRaw => PointingCfg {
    px_per_degree: f32, anchor: String, edge_margin_px: f32, forward_axis: String,
    up_axis: String, invert_x: bool, invert_y: bool, recenter_buttons: Vec<String>,
    smoothing_min_cutoff_hz: f32, smoothing_beta: f32,
}, pointing);
raw!(MouseRaw => MouseCfg {
    enabled: bool, sensitivity: f32, smoothing_ms: f32, invert_x: bool, invert_y: bool,
    left_buttons: Vec<String>, right_buttons: Vec<String>, middle_buttons: Vec<String>,
    scroll_speed: f32, scroll_deadzone: f32, invert_scroll: bool,
    max_distance: u16, max_roughness: u16, max_tilt_deg: f32, settle_ms: f32, linger_ms: f32,
}, mouse);
raw!(FusionRaw => FusionCfg {
    use_magnetometer: bool, accel_gain: f32, accel_gate_g: f32, mag_gain: f32, mag_gate: f32,
    still_gyro_dps: f32, still_accel_g: f32, still_time_ms: f32, bias_time_constant_s: f32,
}, fusion);
raw!(OverlayRaw => OverlayCfg {
    prediction_ms: f32, prediction_mouse_ms: f32, confirm_ms: f32, tail_fade_ms: f32,
    tail_floor: f32, width_min_px: f32, width_max_px: f32, speed_full_px_s: f32, spark_rate: f32,
}, overlay);
raw!(RumbleRaw => RumbleCfg {
    enabled: bool, match_strength: f32, match_ms: u32, no_match_strength: f32,
    no_match_ms: u32, no_match_gap_ms: u32,
}, rumble);
raw!(DevRaw => DevCfg { enabled: bool, hold_key: String, dry_run: bool }, dev);
raw!(ActionsRaw => ActionsCfg { shell: String }, actions);

impl Config {
    pub fn from_toml(text: &str) -> anyhow::Result<Config> {
        let mut cfg: Config = toml::from_str(text)?;
        cfg.sanitize();
        Ok(cfg)
    }

    /// Clamp values a typo could make dangerous or meaningless.
    pub fn sanitize(&mut self) {
        let r = &mut self.recognizer;
        r.min_score = r.min_score.clamp(0.0, 1.0);
        r.min_margin = r.min_margin.clamp(0.0, 1.0);
        r.resample_points = r.resample_points.clamp(16, 256);
        r.rotation_tolerance_deg = r.rotation_tolerance_deg.clamp(0.0, 180.0);
        r.scale_tolerance = r.scale_tolerance.clamp(0.0, 0.9);
        r.elastic_window = r.elastic_window.clamp(0.0, 0.3);
        let p = &mut self.pointing;
        p.px_per_degree = p.px_per_degree.clamp(4.0, 400.0);
        p.edge_margin_px = p.edge_margin_px.clamp(0.0, 600.0);
        let m = &mut self.mouse;
        m.sensitivity = m.sensitivity.clamp(0.05, 20.0);
        m.smoothing_ms = m.smoothing_ms.clamp(0.0, 60.0);
        m.scroll_speed = m.scroll_speed.clamp(0.0, 200.0);
        m.scroll_deadzone = m.scroll_deadzone.clamp(0.02, 0.9);
        m.max_tilt_deg = m.max_tilt_deg.clamp(5.0, 180.0);
        m.settle_ms = m.settle_ms.clamp(0.0, 2000.0);
        m.linger_ms = m.linger_ms.clamp(0.0, 5000.0);
        let o = &mut self.overlay;
        o.prediction_ms = o.prediction_ms.clamp(0.0, 40.0);
        o.prediction_mouse_ms = o.prediction_mouse_ms.clamp(0.0, 40.0);
        o.confirm_ms = o.confirm_ms.clamp(120.0, 2000.0);
        o.tail_fade_ms = o.tail_fade_ms.clamp(100.0, 5000.0);
        o.tail_floor = o.tail_floor.clamp(0.0, 1.0);
        o.width_min_px = o.width_min_px.clamp(0.5, 20.0);
        o.width_max_px = o.width_max_px.clamp(o.width_min_px, 40.0);
        o.speed_full_px_s = o.speed_full_px_s.max(200.0);
        o.spark_rate = o.spark_rate.clamp(0.0, 4.0);
    }
}

/// Where the config, the gesture library and calibration live on disk.
#[derive(Clone, Debug)]
pub struct Paths {
    pub dir: PathBuf,
}

impl Paths {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    pub fn config(&self) -> PathBuf {
        self.dir.join("wand.toml")
    }
    pub fn gestures(&self) -> PathBuf {
        self.dir.join("gestures.json")
    }
    pub fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }
    pub fn recordings(&self) -> PathBuf {
        self.dir.join("recordings")
    }
    pub fn log(&self) -> PathBuf {
        self.dir.join("wand.log")
    }
}

/// The text of every `[section]` the shipped file has and `text` lacks, with
/// its comments, ready to append. A file written by an older version gets the
/// sections added since, so every threshold stays visible in the one file.
pub fn missing_sections(text: &str) -> String {
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else { return String::new() };
    let mut out = String::new();
    let mut current: Option<&str> = None;
    let mut block = String::new();
    let flush = |name: Option<&str>, block: &mut String, out: &mut String| {
        if let Some(name) = name {
            if doc.get(name).is_none() {
                out.push('\n');
                out.push_str(block.trim_end());
                out.push('\n');
            }
        }
        block.clear();
    };
    for line in DEFAULT_TOML.lines() {
        let header = line.trim().strip_prefix('[').and_then(|l| l.strip_suffix(']'));
        if let Some(name) = header {
            flush(current, &mut block, &mut out);
            current = Some(name);
        }
        if current.is_some() {
            block.push_str(line);
            block.push('\n');
        }
    }
    flush(current, &mut block, &mut out);
    out
}

/// Load `wand.toml`, writing the shipped default first if it does not exist.
/// A file that fails to parse is left alone and the defaults are used, with
/// the parse error returned so the UI can show it.
pub fn load_or_init(path: &Path) -> (Config, Option<String>) {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, DEFAULT_TOML);
    }
    if let Ok(text) = std::fs::read_to_string(path) {
        let added = missing_sections(&text);
        if !added.is_empty() {
            let _ = std::fs::write(path, format!("{}\n{added}", text.trim_end()));
        }
    }
    match std::fs::read_to_string(path) {
        Ok(text) => match Config::from_toml(&text) {
            Ok(cfg) => (cfg, None),
            Err(e) => (Config::default(), Some(format!("wand.toml: {e}"))),
        },
        Err(e) => (Config::default(), Some(format!("wand.toml: {e}"))),
    }
}

/// Set `[section] key = value` in place, preserving everything else.
pub fn set_value(path: &Path, section: &str, key: &str, value: toml_edit::Value) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| DEFAULT_TOML.to_string());
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    if doc.get(section).is_none() {
        doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    // Keep the trailing comment of the line, if any.
    let item = &mut doc[section][key];
    match item.as_value_mut() {
        Some(existing) => {
            let decor = existing.decor().clone();
            let mut v = value;
            *v.decor_mut() = decor;
            *existing = v;
        }
        None => *item = toml_edit::Item::Value(value),
    }
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_defaults_parse_completely() {
        let cfg = Config::default();
        assert!(cfg.recognizer.min_score > 0.5 && cfg.recognizer.min_score < 1.0);
        assert_eq!(cfg.recognizer.resample_points, 64);
        assert_eq!(cfg.stroke.triggers, vec!["ZR", "ZL"]);
        assert_eq!(cfg.dev.hold_key, "RightCtrl");
        assert_eq!(Config::from_toml(DEFAULT_TOML).unwrap(), cfg);
    }

    #[test]
    fn partial_file_falls_back_per_key() {
        let cfg = Config::from_toml("[recognizer]\nmin_score = 0.9\n").unwrap();
        assert_eq!(cfg.recognizer.min_score, 0.9);
        assert_eq!(cfg.recognizer.min_margin, Config::default().recognizer.min_margin);
        assert_eq!(cfg.overlay, Config::default().overlay);
    }

    #[test]
    fn older_files_gain_the_sections_added_since() {
        // A file from before the mouse section existed, with a value changed.
        let mut old = String::new();
        let mut skipping = false;
        for line in DEFAULT_TOML.lines() {
            if line.starts_with('[') {
                skipping = line.trim() == "[mouse]";
            }
            if !skipping {
                old.push_str(line);
                old.push('\n');
            }
        }
        let old = old.replace("min_score = 0.80", "min_score = 0.77");
        assert!(Config::from_toml(&old).unwrap().mouse.enabled, "a missing section reads as the defaults");
        let added = missing_sections(&old);
        assert!(added.contains("[mouse]") && added.contains("# Lay the Joy-Con on its side"));
        assert!(!added.contains("[recognizer]") && !added.contains("[fusion]"));
        let merged = format!("{}\n{added}", old.trim_end());
        let cfg = Config::from_toml(&merged).unwrap();
        assert_eq!(cfg.recognizer.min_score, 0.77);
        assert_eq!(cfg.mouse, Config::default().mouse);
        // Nothing to add the second time, and nothing for a complete file.
        assert!(missing_sections(&merged).is_empty());
        assert!(missing_sections(DEFAULT_TOML).is_empty());
    }

    #[test]
    fn set_value_keeps_comments() {
        let dir = std::env::temp_dir().join(format!("wand-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wand.toml");
        std::fs::write(&path, DEFAULT_TOML).unwrap();
        set_value(&path, "dev", "enabled", true.into()).unwrap();
        set_value(&path, "recognizer", "min_score", 0.75.into()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Every threshold the app uses is in this file"));
        assert!(text.contains("# Dev mode: hold hold_key"));
        let cfg = Config::from_toml(&text).unwrap();
        assert!(cfg.dev.enabled);
        assert_eq!(cfg.recognizer.min_score, 0.75);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
