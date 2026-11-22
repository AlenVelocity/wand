//! Strokes drawn with a real Joy-Con 2 (R) on 2026-11-22, at 24 px per
//! degree, as the engine handed them to the recogniser. The synthetic hands in
//! `synth.rs` draw the template; a person does not. These are what a person
//! does: ovals, tails into a circle, more than one turn, a V that is a tick,
//! a slash drawn upward, a dip before a flick.
//!
//! Each stroke is labelled with what it was meant to be, by eye. The first
//! recogniser matched 20 of these; changes must not go back below what is
//! asserted here.

use wand_lib::config::Config;
use wand_lib::engine::{too_small, Source};
use wand_lib::gestures;
use wand_lib::recognizer::{self, Pt, Verdict};

const PX_PER_DEGREE: f32 = 24.0;

/// Index in the fixture -> the gesture it was meant as.
const MEANT: &[(&str, &[usize])] = &[
    ("circle", &[1, 2, 6, 7, 8, 9, 10, 11, 12, 30]),
    ("flick-left", &[5, 14, 37, 39, 41]),
    ("flick-right", &[13, 36, 38, 40, 43]),
    ("flick-up", &[32, 33]),
    ("flick-down", &[34, 35]),
    ("v", &[17, 18, 19]),
    ("z", &[31]),
    ("slash", &[42]),
];
/// Shapes that are no gesture in the default library: half a circle and a
/// triangle. Stroke 15 is a second triangle and is left out on purpose: with
/// its closing side cut off as a hook it reads as a caret (0.84), which is a
/// known misreading, not something to pin either way.
const NOT_GESTURES: &[usize] = &[4, 16];
/// Twitches and drifts with the trigger held. 21 scored 0.96 as a slash and 22
/// scored 0.83 as a flick left before the size and speed guard existed.
const ACCIDENTS: &[usize] = &[21, 22, 23, 24, 25, 26, 27, 28, 29];

struct Stroke {
    points: Vec<Pt>,
    length_px: f32,
    ms: f32,
}

fn fixture() -> Vec<Stroke> {
    include_str!("data/real-strokes.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            let path: Vec<[f32; 3]> = serde_json::from_value(v["path"].clone()).unwrap();
            Stroke {
                points: path.iter().map(|p| [p[0], p[1]]).collect(),
                length_px: v["length_px"].as_f64().unwrap() as f32,
                ms: v["ms"].as_f64().unwrap() as f32,
            }
        })
        .collect()
}

fn config() -> Config {
    let mut cfg = Config::default();
    cfg.pointing.px_per_degree = PX_PER_DEGREE;
    cfg
}

#[test]
fn deliberate_strokes_match_what_they_were_meant_as() {
    let (cfg, lib, strokes) = (config(), gestures::defaults(), fixture());
    let templates = lib.templates(cfg.recognizer.resample_points);
    let mut misses = Vec::new();
    for (gesture, indices) in MEANT {
        for &i in *indices {
            let s = &strokes[i];
            assert!(!too_small(s.length_px, s.ms, Source::Joycon, &cfg), "stroke {i} ({gesture}) was dropped as too small");
            match recognizer::recognize(&s.points, &templates, &cfg.recognizer) {
                Verdict::Match(m) if lib.gestures[m.gesture].id == *gesture => {}
                Verdict::Match(m) => misses.push(format!("{i}: meant {gesture}, matched {}", lib.gestures[m.gesture].id)),
                Verdict::NoMatch { best } => misses.push(format!("{i}: meant {gesture}, no match (best {:?})", best.map(|b| b.fit.score))),
            }
        }
    }
    assert!(misses.is_empty(), "{misses:#?}");
}

#[test]
fn other_shapes_do_not_match() {
    let (cfg, lib, strokes) = (config(), gestures::defaults(), fixture());
    let templates = lib.templates(cfg.recognizer.resample_points);
    for &i in NOT_GESTURES {
        let verdict = recognizer::recognize(&strokes[i].points, &templates, &cfg.recognizer);
        assert!(matches!(verdict, Verdict::NoMatch { .. }), "stroke {i} matched: {verdict:?}");
    }
}

#[test]
fn twitches_and_drifts_are_dropped_before_recognition() {
    let (cfg, strokes) = (config(), fixture());
    for &i in ACCIDENTS {
        let s = &strokes[i];
        assert!(too_small(s.length_px, s.ms, Source::Joycon, &cfg), "stroke {i}: {} px in {} ms was kept", s.length_px, s.ms);
    }
}
