//! Headless companion to the app.
//!
//!   wandtool synth <dir>            write a synthetic recording per default gesture
//!   wandtool replay <file>...       run recordings through the pipeline, print outcomes
//!   wandtool selftest [rounds]      generate many hands per gesture and report scores
//!   wandtool strokes <file.jsonl>   re-recognise strokes the app logged (strokes.jsonl)
//!   wandtool sensor <file>          what the optical sensor read over a recording
//!   wandtool pointer-check          nudge the cursor to see that Windows takes it
//!
//! `replay` uses the same parser, fusion, projection and recogniser as the
//! app; only the overlay and the actions are absent. It reads `wand.toml` and
//! `gestures.json` from `--data-dir <dir>` when given, else the defaults.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use wand_lib::config::{Config, Paths};
use wand_lib::engine::{Engine, EngineEvent, Outcome, Source};
use wand_lib::gestures::{self, Library};
use wand_lib::pointing::Monitor;
use wand_lib::protocol::InputReport;
use wand_lib::recognizer::{self, Pt, Verdict};
use wand_lib::replay::{self, Recording};
use wand_lib::synth::{self, SynthOptions};

fn engine(cfg: Config, lib: Library) -> (Engine, Arc<Mutex<Vec<Outcome>>>) {
    let outcomes: Arc<Mutex<Vec<Outcome>>> = Arc::default();
    let sink = outcomes.clone();
    let mut e = Engine::new(
        cfg,
        lib,
        Box::new(move |ev| {
            if let EngineEvent::Outcome(o) = ev {
                sink.lock().unwrap().push(o);
            }
        }),
    );
    e.set_monitors(vec![Monitor { x: 0.0, y: 0.0, w: 2560.0, h: 1440.0, scale: 1.0, primary: true }]);
    (e, outcomes)
}

fn feed(e: &mut Engine, reports: impl Iterator<Item = (u64, InputReport)>) {
    for (us, r) in reports {
        e.on_report_from(&r, us as f64 * 1e-6, Source::Replay);
    }
}

fn load(data_dir: Option<&Path>) -> (Config, Library) {
    match data_dir {
        Some(dir) => {
            let paths = Paths::new(dir);
            let (cfg, err) = wand_lib::config::load_or_init(&paths.config());
            if let Some(e) = err {
                eprintln!("warning: {e}");
            }
            let (lib, err) = Library::load_or_init(&paths.gestures());
            if let Some(e) = err {
                eprintln!("warning: {e}");
            }
            (cfg, lib)
        }
        None => (Config::default(), gestures::defaults()),
    }
}

fn describe(o: &Outcome) -> String {
    match o.kind {
        "match" => format!(
            "match     {:<12} score {:.3}  runner-up {:.3}  -> {}",
            o.gesture.as_deref().unwrap_or("?"), o.score, o.runner_up, o.action.as_deref().unwrap_or("")
        ),
        "no_match" => format!("no match  closest {:<12} score {:.3}  runner-up {:.3}", o.closest.as_deref().unwrap_or("-"), o.score, o.runner_up),
        other => format!("{other:<9} {:.0} px in {:.0} ms", o.length_px, o.duration_ms),
    }
}

fn cmd_synth(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for g in gestures::defaults().gestures {
        let opts = SynthOptions { seed: g.id.len() as u64 * 31, ..Default::default() };
        let reports = synth::stroke_reports(&g.variants[0].ideal, &opts, 0, 0);
        let path = dir.join(format!("{}.wandrec", g.id));
        replay::write_synthetic(&path, &format!("synthetic {} at 125 Hz", g.name), &reports)?;
        println!("{}  {} reports", path.display(), reports.len());
    }
    // One stroke that is no gesture at all.
    let scribble = vec![[0.0, 0.0], [0.8, -0.2], [0.2, -0.3], [0.9, 0.3], [0.1, 0.45], [0.7, -0.5]];
    let reports = synth::stroke_reports(&scribble, &SynthOptions { duration_s: 1.1, ..Default::default() }, 0, 0);
    replay::write_synthetic(&dir.join("scribble.wandrec"), "synthetic scribble (should not match)", &reports)?;
    println!("{}  {} reports", dir.join("scribble.wandrec").display(), reports.len());
    Ok(())
}

fn cmd_replay(files: &[PathBuf], data_dir: Option<&Path>) -> anyhow::Result<bool> {
    let (cfg, lib) = load(data_dir);
    let mut all_as_named = true;
    for file in files {
        let rec = Recording::load(file)?;
        let (mut e, outcomes) = engine(cfg.clone(), lib.clone());
        feed(&mut e, rec.parsed());
        let outcomes = outcomes.lock().unwrap();
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        println!("{}  [{} / {} reports / {:.2} s]", file.display(), rec.header.source, rec.reports.len(), rec.duration_s());
        if outcomes.is_empty() {
            println!("  no stroke: the trigger was never held");
        }
        for o in outcomes.iter() {
            println!("  {}", describe(o));
        }
        // A file named after a gesture id is expected to produce that gesture.
        if lib.gestures.iter().any(|g| g.id == stem) {
            all_as_named &= outcomes.len() == 1 && outcomes[0].gesture_id.as_deref() == Some(stem.as_str());
        } else if stem == "scribble" {
            all_as_named &= outcomes.iter().all(|o| o.kind != "match");
        }
    }
    Ok(all_as_named)
}

/// Run strokes the app logged back through the recogniser as it is now.
/// Options: --window <f>, --min-score <f>, --px-per-degree <f>.
fn cmd_strokes(file: &Path, data_dir: Option<&Path>, opts: &[String]) -> anyhow::Result<()> {
    let (mut cfg, lib) = load(data_dir);
    let opt = |name: &str| opts.iter().position(|a| a == name).and_then(|i| opts.get(i + 1)).and_then(|v| v.parse::<f32>().ok());
    if let Some(w) = opt("--window") {
        cfg.recognizer.elastic_window = w;
    }
    if let Some(m) = opt("--min-score") {
        cfg.recognizer.min_score = m;
    }
    let k = opt("--px-per-degree").unwrap_or(cfg.pointing.px_per_degree);
    let templates = lib.templates(cfg.recognizer.resample_points);
    let text = std::fs::read_to_string(file)?;
    let (mut matched, mut total) = (0, 0);
    println!("{:>3}  {:<26} {:<34}", "#", "logged", "now");
    for (i, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let v: serde_json::Value = serde_json::from_str(line)?;
        let path: Vec<[f32; 3]> = serde_json::from_value(v["path"].clone())?;
        let pts: Vec<Pt> = path.iter().map(|p| [p[0], p[1]]).collect();
        let length = recognizer::path_length(&pts);
        let ms = path.last().map_or(0.0, |p| p[2]).max(1.0);
        let old = format!(
            "{} {} {:.2}",
            v["kind"].as_str().unwrap_or("?"),
            v["gesture"].as_str().or(v["closest"].as_str()).unwrap_or("-"),
            v["score"].as_f64().unwrap_or(0.0)
        );
        let wand = v["source"].as_str() != Some("mouse");
        let small = wand && (length < cfg.stroke.min_length_deg * k || length / (ms / 1000.0) < cfg.stroke.min_speed_deg_s * k);
        let now = if small || length < cfg.stroke.min_length_px {
            format!("dropped ({:.1} deg at {:.0} deg/s)", length / k, length / k / (ms / 1000.0))
        } else {
            total += 1;
            match recognizer::recognize(&pts, &templates, &cfg.recognizer) {
                Verdict::Match(m) => {
                    matched += 1;
                    format!("MATCH {} {:.2} (next {:.2}){}", lib.gestures[m.gesture].id, m.fit.score, m.runner_up, if m.trimmed { " trimmed" } else { "" })
                }
                Verdict::NoMatch { best: Some(m) } => format!("no    ~{} {:.2} (next {:.2})", lib.gestures[m.gesture].id, m.fit.score, m.runner_up),
                Verdict::NoMatch { best: None } => "no    nothing comparable".to_string(),
            }
        };
        println!("{i:>3}  {old:<26} {now}");
    }
    println!("{matched} of {total} kept strokes match");
    Ok(())
}

/// What the optical sensor said over a recording, and what the engine made of
/// it. For setting `mouse.max_distance`, `max_roughness` and `max_tilt_deg`
/// from a real controller: record it on a desk, on a leg and in the hand.
fn cmd_sensor(file: &Path, data_dir: Option<&Path>) -> anyhow::Result<()> {
    use wand_lib::engine::{MouseButton, PointerEvent};
    let (cfg, lib) = load(data_dir);
    let rec = Recording::load(file)?;
    #[derive(Default)]
    struct Seen {
        travel: (f32, f32),
        clicks: [u32; 3],
        notches: f32,
        set_down: u32,
    }
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let sink = seen.clone();
    let mut e = Engine::new(
        cfg.clone(),
        lib,
        Box::new(move |ev| {
            let mut s = sink.lock().unwrap();
            match ev {
                EngineEvent::Pointer { event: PointerEvent::Move { dx, dy }, .. } => s.travel = (s.travel.0 + dx.abs(), s.travel.1 + dy.abs()),
                EngineEvent::Pointer { event: PointerEvent::Button { button, down: true }, .. } => {
                    s.clicks[match button {
                        MouseButton::Left => 0,
                        MouseButton::Right => 1,
                        MouseButton::Middle => 2,
                    }] += 1
                }
                EngineEvent::Pointer { event: PointerEvent::Scroll { notches }, .. } => s.notches += notches.abs(),
                EngineEvent::Surface(surface) if surface.active => s.set_down += 1,
                _ => {}
            }
        }),
    );
    e.set_monitors(vec![Monitor { x: 0.0, y: 0.0, w: 2560.0, h: 1440.0, scale: 1.0, primary: true }]);

    // Readings while the engine was tracking, and while it was not.
    let mut rows: [Vec<[f32; 3]>; 2] = [vec![], vec![]];
    let mut zero_distance = 0usize;
    for (us, r) in rec.parsed() {
        e.on_report_from(&r, us as f64 * 1e-6, Source::Replay);
        let s = e.pose().surface;
        if s.distance == 0 {
            zero_distance += 1;
        }
        rows[usize::from(!s.tracking)].push([s.distance as f32, s.roughness as f32, s.tilt_deg]);
    }
    let total = rows[0].len() + rows[1].len();
    println!("{}  [{} reports / {:.1} s]", file.display(), total, rec.duration_s());
    if zero_distance == total {
        println!("  the sensor sent nothing: the mouse feature was off when this was recorded");
        return Ok(());
    }
    println!("  limits in use: distance 1..{}, roughness below {}, rail within {:.0} degrees of vertical", cfg.mouse.max_distance, cfg.mouse.max_roughness, cfg.mouse.max_tilt_deg);
    println!("  {:<13} {:>8}   {:<22} {:<22} {:<18}", "", "reports", "distance", "roughness", "tilt (degrees)");
    for (name, set) in [("tracking", &rows[0]), ("not tracking", &rows[1])] {
        let spread = |col: usize| {
            let mut v: Vec<f32> = set.iter().map(|r| r[col]).collect();
            v.sort_by(|a, b| a.total_cmp(b));
            match v.len() {
                0 => "-".to_string(),
                n => format!("{:.0} / {:.0} / {:.0}", v[n / 20], v[n / 2], v[n - 1 - n / 20]),
            }
        };
        println!("  {:<13} {:>8}   {:<22} {:<22} {:<18}", name, set.len(), spread(0), spread(1), spread(2));
    }
    println!("  (each cell: 5th percentile / median / 95th percentile)");
    let s = seen.lock().unwrap();
    println!(
        "  set down {} times; cursor travel {:.0} x {:.0} px; clicks left {} right {} middle {}; scrolled {:.1} notches",
        s.set_down, s.travel.0, s.travel.1, s.clicks[0], s.clicks[1], s.clicks[2], s.notches
    );
    Ok(())
}

/// Move the real cursor two pixels right and back, and say whether Windows
/// took it. The one part of the mouse that tests cannot reach.
fn cmd_pointer_check() -> bool {
    let Some((a, b)) = wand_lib::pointer::nudge(2, 0) else {
        println!("could not read the cursor position");
        return false;
    };
    let back = wand_lib::pointer::nudge(-2, 0);
    println!("cursor {a:?} -> {b:?} -> {:?}", back.map(|(_, c)| c));
    let moved = b.0 > a.0 && b.1 == a.1;
    println!("{}", if moved { "Windows accepts injected mouse movement" } else { "the cursor did not move: a window running as administrator may be in front" });
    moved
}

fn cmd_selftest(rounds: usize) -> bool {
    let lib = gestures::defaults();
    let mut ok = true;
    println!("{:<12} {:>5} {:>7} {:>7} {:>9}", "gesture", "hits", "min", "mean", "min margin");
    for g in &lib.gestures {
        for (vi, v) in g.variants.iter().enumerate() {
            let (mut hits, mut min, mut sum, mut min_margin) = (0usize, 1.0f32, 0.0f32, 1.0f32);
            for round in 0..rounds {
                let mut rng = fastrand::Rng::with_seed(round as u64 * 977 + vi as u64);
                let mut range = |lo: f32, hi: f32| lo + rng.f32() * (hi - lo);
                let opts = SynthOptions {
                    rate_hz: [66.0, 100.0, 125.0, 200.0][round % 4],
                    size_px: range(450.0, 1100.0),
                    duration_s: range(0.3, 1.2),
                    aim_yaw_deg: range(-180.0, 180.0),
                    aim_pitch_deg: range(-25.0, 40.0),
                    wrist_roll_deg: range(-60.0, 60.0),
                    tilt_deg: range(-9.0, 9.0),
                    wobble: range(0.0, 0.03),
                    gyro_bias_dps: [range(-1.5, 1.5), range(-1.5, 1.5), range(-1.5, 1.5)],
                    mag_offset_ut: if round % 3 == 0 { None } else { Some([range(-40.0, 40.0), range(-40.0, 40.0), range(-40.0, 40.0)]) },
                    seed: round as u64 + 1,
                    ..Default::default()
                };
                let (mut e, outcomes) = engine(Config::default(), lib.clone());
                feed(&mut e, synth::stroke_reports(&v.ideal, &opts, 0, 0).into_iter());
                let outcomes = outcomes.lock().unwrap();
                if let Some(o) = outcomes.first() {
                    if o.gesture_id.as_deref() == Some(g.id.as_str()) {
                        hits += 1;
                    } else if std::env::var("WAND_VERBOSE").is_ok() {
                        println!("  miss {} round {round}: {} | {:.0}Hz size {:.0} dur {:.2} yaw {:.0} pitch {:.0} roll {:.0} tilt {:.1} wobble {:.3}", g.id, describe(o), opts.rate_hz, opts.size_px, opts.duration_s, opts.aim_yaw_deg, opts.aim_pitch_deg, opts.wrist_roll_deg, opts.tilt_deg, opts.wobble);
                    }
                    min = min.min(o.score);
                    sum += o.score;
                    min_margin = min_margin.min(o.score - o.runner_up);
                } else {
                    min = 0.0;
                }
            }
            let name = if g.variants.len() > 1 { format!("{} #{}", g.id, vi + 1) } else { g.id.clone() };
            println!("{:<12} {:>5} {:>7.3} {:>7.3} {:>9.3}", name, format!("{hits}/{rounds}"), min, sum / rounds as f32, min_margin);
            ok &= hits == rounds;
        }
    }
    ok
}

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = args.iter().position(|a| a == "--data-dir").map(|i| {
        let dir = PathBuf::from(args.get(i + 1).cloned().unwrap_or_default());
        args.drain(i..(i + 2).min(args.len()));
        dir
    });
    let ok = match args.first().map(String::as_str) {
        Some("synth") => {
            cmd_synth(Path::new(args.get(1).map(String::as_str).unwrap_or("recordings")))?;
            true
        }
        Some("replay") if args.len() > 1 => {
            let files: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            cmd_replay(&files, data_dir.as_deref())?
        }
        Some("selftest") => cmd_selftest(args.get(1).and_then(|n| n.parse().ok()).unwrap_or(40)),
        Some("strokes") if args.len() > 1 => {
            cmd_strokes(Path::new(&args[1]), data_dir.as_deref(), &args[2..])?;
            true
        }
        Some("sensor") if args.len() > 1 => {
            cmd_sensor(Path::new(&args[1]), data_dir.as_deref())?;
            true
        }
        Some("pointer-check") => cmd_pointer_check(),
        _ => {
            eprintln!("usage:\n  wandtool synth <dir>\n  wandtool replay [--data-dir <dir>] <file.wandrec>...\n  wandtool selftest [rounds]
  wandtool strokes [--data-dir <dir>] <strokes.jsonl> [--window f] [--min-score f] [--px-per-degree f]
  wandtool sensor [--data-dir <dir>] <file.wandrec>
  wandtool pointer-check");
            false
        }
    };
    std::process::exit(if ok { 0 } else { 1 });
}
