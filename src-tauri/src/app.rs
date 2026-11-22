//! The desktop app: windows, tray, state and the commands the two web views
//! call. Everything interesting happens in `engine.rs`; this file moves its
//! events to where they need to go.

use crate::actions::{self, RunOptions};
use crate::config::{self, Config, Paths};
use crate::devmouse::{self, DevMouse};
use crate::engine::{self, Engine, EngineEvent, Mode, Outcome, OverlayMsg, Pose, Rumble, Source};
use crate::fusion::{MagCal, MagCalProgress};
use crate::gestures::{self, Action, Gesture, Library, RecordingCheck, Rotation};
use crate::joycon::{self, Control, DeviceStatus, Hooks, LinkState};
use crate::pointing::Monitor;
use crate::protocol::InputReport;
use crate::recognizer::Pt;
use crate::replay::{Header, Recorder, Recording};
use crate::synth::{self, SynthOptions};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::mpsc::UnboundedSender;

/// Chromium flags shared by every web view (they share one browser process,
/// so the flags must match). The overlay is never focused and often sits
/// under other topmost windows; without these Chromium would throttle its
/// frames. The first three are Tauri's own defaults, kept.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion \
     --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DeviceCal {
    #[serde(default)]
    gyro_bias: [f32; 3],
    #[serde(default)]
    mag: Option<MagCal>,
}

/// `state.json`: what the app learns about a controller and should not make
/// the user redo. Keyed by controller serial number.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    devices: HashMap<String, DeviceCal>,
}

struct TrayItems {
    pause: CheckMenuItem<tauri::Wry>,
    dev: CheckMenuItem<tauri::Wry>,
}

pub struct AppState {
    handle: AppHandle,
    paths: Paths,
    t0: Instant,
    engine: Mutex<Engine>,
    config: RwLock<Config>,
    config_error: Mutex<Option<String>>,
    library_error: Mutex<Option<String>>,
    status: Mutex<DeviceStatus>,
    persisted: Mutex<Persisted>,
    serial: Mutex<Option<String>>,
    joycon: UnboundedSender<Control>,
    dev: Arc<DevMouse>,
    recorder: Mutex<Option<(Recorder, PathBuf)>>,
    replaying: AtomicBool,
    replay_gen: AtomicU64,
    /// Demo replays from the settings window never run real actions.
    demo: AtomicBool,
    overlays: Mutex<Vec<(String, Channel<Vec<OverlayMsg>>)>>,
    pose_gen: AtomicU64,
    overlay_gen: AtomicU64,
    history: Mutex<VecDeque<Outcome>>,
    monitors: Mutex<Vec<Monitor>>,
    tray: Mutex<Option<TrayItems>>,
    force_dry_run: bool,
    actions: mpsc::Sender<(Action, String)>,
}

type State<'a> = tauri::State<'a, Arc<AppState>>;

#[derive(Clone, Debug, Default)]
pub struct Cli {
    pub hidden: bool,
    pub dry_run: bool,
    pub dev: bool,
    pub replay: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

impl Cli {
    pub fn parse(args: impl Iterator<Item = String>) -> Cli {
        let mut cli = Cli::default();
        let mut args = args.skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--hidden" => cli.hidden = true,
                "--dry-run" => cli.dry_run = true,
                "--dev" => cli.dev = true,
                "--replay" => cli.replay = args.next().map(PathBuf::from),
                "--data-dir" => cli.data_dir = args.next().map(PathBuf::from),
                _ => {}
            }
        }
        cli
    }
}

// --------------------------------------------------------------------- start

pub fn run(cli: Cli) {
    let cli_for_setup = cli.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            snapshot,
            overlay_init,
            overlay_subscribe,
            overlay_report,
            pose_subscribe,
            pose_unsubscribe,
            set_mode,
            save_gesture,
            create_gesture,
            delete_gesture,
            reset_gestures,
            check_recordings,
            set_config_value,
            set_autostart,
            open_config,
            open_data_dir,
            pick_program,
            device_action,
            mag_progress,
            imu_record_start,
            imu_record_stop,
            list_recordings,
            replay_file,
            replay_gesture,
            replay_stop,
        ])
        .setup(move |app| {
            setup(app.handle().clone(), cli_for_setup.clone())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the settings window really closes it: its web view is
            // freed and the app carries on in the tray with only the overlays.
            // Whatever the window had going must not outlive it.
            if let (WindowEvent::Destroyed, "settings") = (event, window.label()) {
                if let Some(state) = window.app_handle().try_state::<Arc<AppState>>() {
                    state.pose_gen.fetch_add(1, Ordering::SeqCst);
                    if state.engine.lock().mode() == Mode::Recording {
                        state.apply_mode(Mode::Normal);
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to start Wand");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Some(state) = handle.try_state::<Arc<AppState>>() {
                let _ = state.joycon.send(Control::Shutdown);
                state.dev.stop();
                if let Some((rec, _)) = state.recorder.lock().take() {
                    let _ = rec.finish();
                }
            }
        }
    });
}

fn setup(handle: AppHandle, cli: Cli) -> anyhow::Result<()> {
    let dir = match &cli.data_dir {
        Some(d) => d.clone(),
        None => handle.path().app_config_dir()?,
    };
    std::fs::create_dir_all(&dir)?;
    let paths = Paths::new(dir);
    init_logging(&paths);
    log::info!("Wand {} starting, data in {}", env!("CARGO_PKG_VERSION"), paths.dir.display());

    let (mut cfg, config_error) = config::load_or_init(&paths.config());
    if cli.dev {
        cfg.dev.enabled = true;
    }
    let (library, library_error) = Library::load_or_init(&paths.gestures());
    let persisted: Persisted = std::fs::read_to_string(paths.state())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();

    let (ev_tx, ev_rx) = mpsc::channel::<EngineEvent>();
    let mut engine = Engine::new(cfg.clone(), library, Box::new(move |ev| drop(ev_tx.send(ev))));
    engine.set_cursor_provider(Box::new(devmouse::cursor));

    let t0 = Instant::now();
    let dev = DevMouse::new();
    if let Err(e) = dev.configure(cfg.dev.enabled, &cfg.dev.hold_key) {
        log::warn!("{e}");
    }

    // The Joy-Con driver reaches the state through a slot filled in below.
    let slot: Arc<std::sync::OnceLock<Arc<AppState>>> = Arc::default();
    let hooks = {
        let (a, b, c, d, e) = (slot.clone(), slot.clone(), slot.clone(), slot.clone(), slot.clone());
        Hooks {
            report: Box::new(move |raw, report, t| {
                if let Some(s) = a.get() {
                    s.on_report(raw, report, t);
                }
            }),
            status: Box::new(move |status| {
                if let Some(s) = b.get() {
                    s.set_status(status.clone());
                }
            }),
            connected: Box::new(move |info| {
                if let Some(s) = c.get() {
                    s.on_connected(&info.serial);
                }
            }),
            lost: Box::new(move || {
                if let Some(s) = d.get() {
                    s.engine.lock().source_lost(Source::Joycon);
                    *s.serial.lock() = None;
                }
            }),
            rumble_cfg: Box::new(move || e.get().map(|s| s.config.read().rumble.clone()).unwrap_or_default()),
        }
    };
    let joycon = joycon::spawn(hooks, t0);

    let (act_tx, act_rx) = mpsc::channel::<(Action, String)>();
    let state = Arc::new(AppState {
        handle: handle.clone(),
        paths,
        t0,
        engine: Mutex::new(engine),
        config: RwLock::new(cfg),
        config_error: Mutex::new(config_error),
        library_error: Mutex::new(library_error),
        status: Mutex::new(DeviceStatus::default()),
        persisted: Mutex::new(persisted),
        serial: Mutex::new(None),
        joycon,
        dev: dev.clone(),
        recorder: Mutex::new(None),
        replaying: AtomicBool::new(false),
        replay_gen: AtomicU64::new(0),
        demo: AtomicBool::new(false),
        overlays: Mutex::new(Vec::new()),
        pose_gen: AtomicU64::new(0),
        overlay_gen: AtomicU64::new(0),
        history: Mutex::new(VecDeque::new()),
        monitors: Mutex::new(Vec::new()),
        tray: Mutex::new(None),
        force_dry_run: cli.dry_run,
        actions: act_tx,
    });
    let _ = slot.set(state.clone());
    handle.manage(state.clone());

    // Engine events: actions and rumble leave first, then the overlay batch.
    {
        let s = state.clone();
        std::thread::Builder::new().name("wand-dispatch".into()).spawn(move || dispatch(s, ev_rx))?;
    }
    // Actions run on their own thread so a slow program launch cannot hold
    // up the overlay or the next stroke.
    {
        let s = state.clone();
        std::thread::Builder::new().name("wand-actions".into()).spawn(move || {
            for (action, label) in act_rx {
                let opts = {
                    let cfg = s.config.read();
                    RunOptions {
                        dry_run: s.force_dry_run || cfg.dev.dry_run || s.demo.load(Ordering::Relaxed),
                        shell: cfg.actions.shell.clone(),
                    }
                };
                match actions::run(&action, &opts) {
                    Ok(what) => log::info!("action [{label}]: {what}"),
                    Err(e) => {
                        log::warn!("action [{label}] failed: {e}");
                        let _ = s.handle.emit_to("settings", "wand://action-error", format!("{label}: {e}"));
                    }
                }
            }
        })?;
    }
    // Dev mode's mouse.
    {
        let s = state.clone();
        std::thread::Builder::new()
            .name("wand-devmouse".into())
            .spawn(move || dev.run(t0, |t, x, y, down| s.engine.lock().on_pointer(t, x, y, down)))?;
    }
    // Config hot reload and display changes.
    {
        let s = state.clone();
        std::thread::Builder::new().name("wand-watch".into()).spawn(move || watch(s))?;
    }

    build_settings_window(&handle, !cli.hidden)?;
    sync_overlays(&state);
    build_tray(&state)?;

    if let Some(file) = cli.replay {
        let s = state.clone();
        std::thread::spawn(move || {
            // Give the overlay windows time to load and subscribe.
            std::thread::sleep(Duration::from_millis(2500));
            match Recording::load(&file) {
                Ok(rec) => s.play(rec.parsed().collect(), false),
                Err(e) => log::error!("--replay {}: {e}", file.display()),
            }
        });
    }
    Ok(())
}

fn init_logging(paths: &Paths) {
    use simplelog::*;
    let config = ConfigBuilder::new()
        .set_time_format_rfc3339()
        .add_filter_ignore_str("btleplug")
        .build();
    let mut loggers: Vec<Box<dyn SharedLogger>> =
        vec![TermLogger::new(LevelFilter::Info, config.clone(), TerminalMode::Stderr, ColorChoice::Never)];
    let log_path = paths.log();
    // Start a fresh log once the old one passes 2 MB.
    let big = std::fs::metadata(&log_path).map(|m| m.len() > 2_000_000).unwrap_or(false);
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(!big).write(true).truncate(big).open(&log_path) {
        loggers.push(WriteLogger::new(LevelFilter::Info, config, file));
    }
    let _ = CombinedLogger::init(loggers);
}

// ------------------------------------------------------------------- windows

fn build_settings_window(handle: &AppHandle, visible: bool) -> anyhow::Result<()> {
    let window = WebviewWindowBuilder::new(handle, "settings", WebviewUrl::App("index.html".into()))
        .title("Wand")
        .inner_size(1080.0, 740.0)
        .min_inner_size(900.0, 620.0)
        .theme(Some(tauri::Theme::Dark))
        .background_color(tauri::window::Color(0, 0, 0, 255))
        .additional_browser_args(BROWSER_ARGS)
        .visible(false)
        .center()
        .build()?;
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        native::black_caption(hwnd.0 as _);
    }
    if visible {
        window.show()?;
        let _ = window.set_focus();
    }
    Ok(())
}

fn show_settings(handle: &AppHandle) {
    if let Some(w) = handle.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    // Windows cannot be created from inside an event handler on the main
    // thread, which is where tray clicks arrive.
    let handle = handle.clone();
    std::thread::spawn(move || {
        if let Err(e) = build_settings_window(&handle, true) {
            log::error!("could not open the settings window: {e}");
        }
    });
}

fn read_monitors(handle: &AppHandle) -> Vec<Monitor> {
    let primary = handle.primary_monitor().ok().flatten().map(|m| (m.position().x, m.position().y));
    handle
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Monitor {
            x: m.position().x as f32,
            y: m.position().y as f32,
            w: m.size().width as f32,
            h: m.size().height as f32,
            scale: m.scale_factor() as f32,
            primary: primary == Some((m.position().x, m.position().y)),
        })
        .collect()
}

/// One transparent, click-through, topmost window per monitor. Separate
/// windows (not one spanning the desktop) so each renders at its own
/// monitor's refresh rate.
fn sync_overlays(state: &Arc<AppState>) {
    let monitors = read_monitors(&state.handle);
    if monitors.is_empty() || *state.monitors.lock() == monitors {
        return;
    }
    log::info!("displays: {monitors:?}");
    for (label, w) in state.handle.webview_windows() {
        if label.starts_with("overlay-") {
            let _ = w.destroy();
        }
    }
    state.overlays.lock().clear();
    // Known before the windows load, so their first question has an answer.
    state.engine.lock().set_monitors(monitors.clone());
    *state.monitors.lock() = monitors.clone();
    // A fresh label each time displays change: a window still being torn down
    // must not collide with its replacement.
    let generation = state.overlay_gen.fetch_add(1, Ordering::SeqCst);
    for (i, m) in monitors.iter().enumerate() {
        let built = WebviewWindowBuilder::new(&state.handle, format!("overlay-{i}-{generation}"), WebviewUrl::App("overlay.html".into()))
            .title("Wand overlay")
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .visible(false)
            .additional_browser_args(BROWSER_ARGS)
            .build();
        match built {
            Ok(w) => {
                // Physical placement after creation: the builder works in
                // logical units of whichever monitor the window opens on.
                let pos = tauri::PhysicalPosition::new(m.x as i32, m.y as i32);
                let size = tauri::PhysicalSize::new(m.w as u32, m.h as u32);
                let _ = w.set_position(pos);
                let _ = w.set_size(size);
                let _ = w.set_position(pos);
                let _ = w.set_ignore_cursor_events(true);
                #[cfg(windows)]
                if let Ok(hwnd) = w.hwnd() {
                    native::overlay_styles(hwnd.0 as _);
                }
                let _ = w.show();
                let _ = w.set_size(size);
            }
            Err(e) => log::error!("could not create the overlay for display {i}: {e}"),
        }
    }
}

#[cfg(windows)]
mod native {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT,
    };

    /// Never takes focus, never appears in Alt+Tab, passes every click through.
    pub fn overlay_styles(hwnd: *mut core::ffi::c_void) {
        let hwnd = HWND(hwnd);
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let add = (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED).0 as isize;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | add);
        }
    }

    /// True black title bar with off-white text (Windows 11; ignored on 10).
    pub fn black_caption(hwnd: *mut core::ffi::c_void) {
        let hwnd = HWND(hwnd);
        let set = |attr, color: u32| unsafe {
            let _ = DwmSetWindowAttribute(hwnd, attr, &color as *const u32 as *const _, 4);
        };
        set(DWMWA_CAPTION_COLOR, 0x0000_0000);
        set(DWMWA_BORDER_COLOR, 0x001A_1C1C);
        set(DWMWA_TEXT_COLOR, 0x00F0_F3F4); // COLORREF is 0x00BBGGRR
    }
}

fn build_tray(state: &Arc<AppState>) -> anyhow::Result<()> {
    let h = &state.handle;
    let open = MenuItem::with_id(h, "open", "Open Wand", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(h, "pause", "Pause gestures", true, false, None::<&str>)?;
    let dev = CheckMenuItem::with_id(h, "dev", "Dev mode (hold key + mouse)", true, state.config.read().dev.enabled, None::<&str>)?;
    let quit = MenuItem::with_id(h, "quit", "Quit Wand", true, None::<&str>)?;
    let menu = Menu::with_items(h, &[&open, &PredefinedMenuItem::separator(h)?, &pause, &dev, &PredefinedMenuItem::separator(h)?, &quit])?;
    let mut tray = TrayIconBuilder::with_id("wand").tooltip("Wand").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = h.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    let s = state.clone();
    tray.on_menu_event(move |app, event| match event.id().as_ref() {
        "open" => show_settings(app),
        "pause" => {
            let paused = s.engine.lock().mode() != Mode::Paused;
            s.apply_mode(if paused { Mode::Paused } else { Mode::Normal });
        }
        "dev" => {
            let enabled = !s.config.read().dev.enabled;
            if let Err(e) = config::set_value(&s.paths.config(), "dev", "enabled", enabled.into()) {
                log::warn!("could not save dev mode: {e}");
            }
            s.reload_config();
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            show_settings(tray.app_handle());
        }
    })
    .build(h)?;
    *state.tray.lock() = Some(TrayItems { pause, dev });
    Ok(())
}

// ------------------------------------------------------------------ plumbing

fn dispatch(state: Arc<AppState>, rx: mpsc::Receiver<EngineEvent>) {
    while let Ok(first) = rx.recv() {
        // Take everything that is already waiting so overlay moves coalesce.
        let mut events = vec![first];
        while let Ok(more) = rx.try_recv() {
            events.push(more);
        }
        let mut overlay = Vec::new();
        for ev in events {
            match ev {
                EngineEvent::Fire { action, label, .. } => drop(state.actions.send((action, label))),
                EngineEvent::Rumble(kind) => drop(state.joycon.send(Control::Rumble(kind))),
                EngineEvent::Overlay(m) => overlay.push(m),
                EngineEvent::Recorded { points } => drop(state.handle.emit_to("settings", "wand://recorded", points)),
                EngineEvent::Outcome(o) => {
                    log::info!(
                        "stroke {:?} {}: gesture={:?} score={:.3} runner_up={:.3} closest={:?} len={:.0}px {:.0}ms pts={}",
                        o.source, o.kind, o.gesture, o.score, o.runner_up, o.closest, o.length_px, o.duration_ms, o.points
                    );
                    save_stroke(&state.paths, &o);
                    let mut h = state.history.lock();
                    h.push_front(o.clone());
                    h.truncate(30);
                    drop(h);
                    let _ = state.handle.emit_to("settings", "wand://outcome", o);
                }
                EngineEvent::Recentered => drop(state.handle.emit_to("settings", "wand://recentered", ())),
                EngineEvent::GyroCalibrated(result) => {
                    if let (Ok(bias), Some(serial)) = (&result, state.serial.lock().clone()) {
                        state.persisted.lock().devices.entry(serial).or_default().gyro_bias = *bias;
                        state.save_persisted();
                    }
                    let _ = state.handle.emit_to("settings", "wand://gyro-calibrated", result.map(|_| ()));
                }
            }
        }
        if !overlay.is_empty() {
            let batch = engine::coalesce(overlay);
            state.overlays.lock().retain(|(_, ch)| ch.send(batch.clone()).is_ok());
        }
    }
}

/// Append the stroke to strokes.jsonl: the path the recogniser was given and
/// what it made of it. The file is the answer to "why did that not match?".
fn save_stroke(paths: &Paths, o: &Outcome) {
    use std::io::Write;
    if o.path.len() < 2 {
        return;
    }
    let file = paths.dir.join("strokes.jsonl");
    // Start over at 4 MB, keeping the previous file once.
    if std::fs::metadata(&file).map(|m| m.len() > 4_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&file, paths.dir.join("strokes.1.jsonl"));
    }
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = serde_json::json!({
        "at": at, "kind": o.kind, "gesture": o.gesture_id, "closest": o.closest, "score": o.score,
        "runner_up": o.runner_up, "source": o.source, "length_px": o.length_px, "ms": o.duration_ms, "path": o.path,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        let _ = writeln!(f, "{line}");
    }
}

fn watch(state: Arc<AppState>) {
    let path = state.paths.config();
    let stamp = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut last = stamp(&path);
    let mut ticks = 0u32;
    loop {
        std::thread::sleep(Duration::from_millis(700));
        let now = stamp(&path);
        if now != last {
            last = now;
            // Editors write in several steps; let the file settle.
            std::thread::sleep(Duration::from_millis(150));
            state.reload_config();
        }
        ticks += 1;
        if ticks % 4 == 0 {
            sync_overlays(&state);
        }
    }
}

impl AppState {
    fn on_report(&self, raw: &[u8], report: &InputReport, t: f64) {
        if let Some((rec, _)) = self.recorder.lock().as_mut() {
            let _ = rec.push(t, raw);
        }
        // A replay owns the fusion state while it runs.
        if !self.replaying.load(Ordering::Relaxed) {
            self.engine.lock().on_report(report, t);
        }
    }

    fn set_status(&self, status: DeviceStatus) {
        let tip = match (status.state, status.battery_percent) {
            (LinkState::Connected, Some(p)) => format!("Wand: {} connected, {p}%", status.name),
            (LinkState::Connected, None) => format!("Wand: {} connected", status.name),
            (LinkState::BluetoothOff, _) => "Wand: Bluetooth is off".to_string(),
            _ => "Wand: no controller".to_string(),
        };
        if let Some(tray) = self.handle.tray_by_id("wand") {
            let _ = tray.set_tooltip(Some(tip));
        }
        {
            let mut current = self.status.lock();
            if current.state != status.state || current.detail != status.detail {
                log::info!("device: {:?}: {}", status.state, status.detail);
            }
            // Only when the rate really moves, not on every one-report wobble.
            if (current.report_hz - status.report_hz).abs() > 15.0 || current.interval_ms != status.interval_ms {
                log::info!("link: {} reports/s, interval {:?} ms, battery {:?}% ({} mV)", status.report_hz, status.interval_ms, status.battery_percent, status.battery_mv);
            }
            *current = status.clone();
        }
        let _ = self.handle.emit_to("settings", "wand://status", status);
    }

    fn on_connected(&self, serial: &str) {
        *self.serial.lock() = Some(serial.to_string());
        if let Some(cal) = self.persisted.lock().devices.get(serial) {
            let mut e = self.engine.lock();
            e.set_gyro_bias(cal.gyro_bias);
            if let Some(mag) = cal.mag {
                e.set_mag_cal(mag);
            }
        }
    }

    fn save_persisted(&self) {
        if let Ok(text) = serde_json::to_string_pretty(&*self.persisted.lock()) {
            let _ = std::fs::write(self.paths.state(), text);
        }
    }

    fn changed(&self) {
        let _ = self.handle.emit_to("settings", "wand://changed", ());
    }

    fn apply_mode(&self, mode: Mode) {
        self.engine.lock().set_mode(mode);
        if let Some(t) = self.tray.lock().as_ref() {
            let _ = t.pause.set_checked(mode == Mode::Paused);
        }
        self.changed();
    }

    fn reload_config(&self) {
        let (cfg, err) = config::load_or_init(&self.paths.config());
        if let Some(e) = &err {
            log::warn!("{e}");
        }
        let dev_err = self.dev.configure(cfg.dev.enabled, &cfg.dev.hold_key).err();
        self.engine.lock().set_config(cfg.clone());
        if let Some(t) = self.tray.lock().as_ref() {
            let _ = t.dev.set_checked(cfg.dev.enabled);
        }
        let _ = self.handle.emit("wand://overlay-config", cfg.overlay.clone());
        *self.config.write() = cfg;
        *self.config_error.lock() = err.or(dev_err);
        self.changed();
    }

    fn save_library(&self, lib: Library) -> Result<Library, String> {
        lib.save(&self.paths.gestures()).map_err(|e| format!("Could not save gestures.json: {e}"))?;
        *self.library_error.lock() = None;
        self.engine.lock().set_library(lib.clone());
        self.changed();
        Ok(lib)
    }

    /// Feed reports through the engine at their recorded pace.
    fn play(self: &Arc<Self>, reports: Vec<(u64, InputReport)>, demo: bool) {
        let gen = self.replay_gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.replaying.store(true, Ordering::SeqCst);
        self.demo.store(demo, Ordering::SeqCst);
        self.engine.lock().source_lost(Source::Replay);
        let _ = self.handle.emit_to("settings", "wand://replay", true);
        let start = Instant::now();
        let first = reports.first().map_or(0, |r| r.0);
        native_timer(true);
        for (us, report) in &reports {
            if self.replay_gen.load(Ordering::SeqCst) != gen {
                break;
            }
            let due = Duration::from_micros(us - first);
            if let Some(wait) = due.checked_sub(start.elapsed()) {
                std::thread::sleep(wait);
            }
            self.engine.lock().on_report_from(report, self.t0.elapsed().as_secs_f64(), Source::Replay);
        }
        native_timer(false);
        if self.replay_gen.load(Ordering::SeqCst) == gen {
            self.engine.lock().source_lost(Source::Replay);
            self.replaying.store(false, Ordering::SeqCst);
            // Let the action thread pick up a final Fire before demo mode ends.
            std::thread::sleep(Duration::from_millis(120));
            self.demo.store(false, Ordering::SeqCst);
            let _ = self.handle.emit_to("settings", "wand://replay", false);
        }
    }
}

fn native_timer(on: bool) {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
        if on {
            timeBeginPeriod(1);
        } else {
            timeEndPeriod(1);
        }
    }
    #[cfg(not(windows))]
    let _ = on;
}

// ------------------------------------------------------------------ commands

#[derive(Serialize)]
struct Calibration {
    gyro_bias_dps: [f32; 3],
    mag_valid: bool,
    mag_field_ut: f32,
}

#[derive(Serialize)]
struct Snapshot {
    version: &'static str,
    config: Config,
    config_error: Option<String>,
    library_error: Option<String>,
    config_path: String,
    data_dir: String,
    library: Library,
    status: DeviceStatus,
    mode: Mode,
    autostart: bool,
    dry_run: bool,
    imu_recording: Option<String>,
    replaying: bool,
    history: Vec<Outcome>,
    calibration: Calibration,
    media_keys: Vec<(&'static str, &'static str)>,
    monitors: Vec<Monitor>,
}

#[tauri::command]
fn snapshot(state: State) -> Snapshot {
    use tauri_plugin_autostart::ManagerExt;
    let (library, mode, pose, mag) = {
        let e = state.engine.lock();
        (e.library().clone(), e.mode(), e.pose(), e.mag_cal())
    };
    let cfg = state.config.read().clone();
    Snapshot {
        version: env!("CARGO_PKG_VERSION"),
        dry_run: state.force_dry_run || cfg.dev.dry_run,
        config: cfg,
        config_error: state.config_error.lock().clone(),
        library_error: state.library_error.lock().clone(),
        config_path: state.paths.config().display().to_string(),
        data_dir: state.paths.dir.display().to_string(),
        library,
        status: state.status.lock().clone(),
        mode,
        autostart: state.handle.autolaunch().is_enabled().unwrap_or(false),
        imu_recording: state.recorder.lock().as_ref().map(|(_, p)| p.display().to_string()),
        replaying: state.replaying.load(Ordering::Relaxed),
        history: state.history.lock().iter().cloned().collect(),
        calibration: Calibration { gyro_bias_dps: pose.gyro_bias_dps, mag_valid: mag.valid, mag_field_ut: mag.field_ut },
        media_keys: gestures::MEDIA_KEYS.to_vec(),
        monitors: state.monitors.lock().clone(),
    }
}

#[derive(Serialize)]
struct OverlayInit {
    monitor: Monitor,
    cfg: config::OverlayCfg,
}

#[tauri::command]
fn overlay_init(window: tauri::WebviewWindow, state: State) -> Result<OverlayInit, String> {
    // Labels are "overlay-<display>-<generation>".
    let index: usize = window
        .label()
        .strip_prefix("overlay-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|i| i.parse().ok())
        .ok_or("not an overlay window")?;
    let monitor = state.monitors.lock().get(index).copied().ok_or("no such display")?;
    Ok(OverlayInit { monitor, cfg: state.config.read().overlay.clone() })
}

#[tauri::command]
fn overlay_subscribe(window: tauri::WebviewWindow, state: State, channel: Channel<Vec<OverlayMsg>>) {
    let label = window.label().to_string();
    let mut overlays = state.overlays.lock();
    overlays.retain(|(l, _)| *l != label);
    overlays.push((label, channel));
}

/// Frame pacing measured by an overlay window over one stroke.
#[tauri::command]
fn overlay_report(window: tauri::WebviewWindow, stats: serde_json::Value) {
    log::info!("overlay frames {}: {stats}", window.label());
}

/// Stream the fused orientation at 60 Hz while the Device page is open.
#[tauri::command]
fn pose_subscribe(state: State, channel: Channel<Pose>) {
    let gen = state.pose_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let s = state.inner().clone();
    std::thread::spawn(move || {
        while s.pose_gen.load(Ordering::SeqCst) == gen {
            let pose = s.engine.lock().pose();
            if channel.send(pose).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    });
}

#[tauri::command]
fn pose_unsubscribe(state: State) {
    state.pose_gen.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
fn set_mode(state: State, mode: String) -> Result<(), String> {
    let mode = match mode.as_str() {
        "normal" => Mode::Normal,
        "recording" => Mode::Recording,
        "paused" => Mode::Paused,
        other => return Err(format!("unknown mode {other}")),
    };
    state.apply_mode(mode);
    Ok(())
}

#[tauri::command]
fn save_gesture(state: State, gesture: Gesture) -> Result<Library, String> {
    actions::validate(&gesture.action)?;
    if gesture.name.trim().is_empty() {
        return Err("Give the gesture a name.".into());
    }
    let mut lib = state.engine.lock().library().clone();
    match lib.gestures.iter_mut().find(|g| g.id == gesture.id) {
        Some(existing) => *existing = gesture,
        None => return Err("That gesture no longer exists.".into()),
    }
    state.save_library(lib)
}

#[tauri::command]
fn create_gesture(
    state: State,
    name: String,
    strokes: Vec<Vec<Pt>>,
    free_rotation: bool,
    action: Action,
    label: String,
    icon: String,
    replace_id: Option<String>,
) -> Result<Library, String> {
    actions::validate(&action)?;
    if name.trim().is_empty() {
        return Err("Give the gesture a name.".into());
    }
    if strokes.len() < 3 || strokes.iter().any(|s| s.len() < 2) {
        return Err("Draw the gesture three times first.".into());
    }
    let mut lib = state.engine.lock().library().clone();
    let rotation = if free_rotation { Rotation::Free } else { Rotation::Bounded };
    let mut gesture = gestures::from_recordings(&lib, &name, &strokes, rotation, action, &label, &icon);
    // Re-recording keeps the gesture's identity and place; only its shape changes.
    match replace_id.and_then(|id| lib.gestures.iter().position(|g| g.id == id)) {
        Some(at) => {
            let old = &lib.gestures[at];
            gesture.id = old.id.clone();
            gesture.builtin = old.builtin;
            gesture.enabled = old.enabled;
            lib.gestures[at] = gesture;
        }
        None => lib.gestures.push(gesture),
    }
    state.save_library(lib)
}

#[tauri::command]
fn delete_gesture(state: State, id: String) -> Result<Library, String> {
    let mut lib = state.engine.lock().library().clone();
    lib.gestures.retain(|g| g.id != id);
    state.save_library(lib)
}

#[tauri::command]
fn reset_gestures(state: State) -> Result<Library, String> {
    // Built-ins go back to their shipped shape and binding; recorded gestures stay.
    let current = state.engine.lock().library().clone();
    let mut lib = gestures::defaults();
    lib.gestures.extend(current.gestures.into_iter().filter(|g| !g.builtin));
    state.save_library(lib)
}

#[tauri::command]
fn check_recordings(state: State, strokes: Vec<Vec<Pt>>, ignore_id: Option<String>) -> RecordingCheck {
    let e = state.engine.lock();
    gestures::check_recordings(e.library(), &strokes, &e.config().recognizer, ignore_id.as_deref())
}

#[tauri::command]
fn set_config_value(state: State, section: String, key: String, value: serde_json::Value) -> Result<(), String> {
    let v: toml_edit::Value = match value {
        serde_json::Value::Bool(b) => b.into(),
        serde_json::Value::Number(n) if n.is_i64() => n.as_i64().unwrap().into(),
        serde_json::Value::Number(n) => n.as_f64().ok_or("bad number")?.into(),
        serde_json::Value::String(s) => s.into(),
        _ => return Err("Only booleans, numbers and text can be set here.".into()),
    };
    config::set_value(&state.paths.config(), &section, &key, v).map_err(|e| format!("Could not write wand.toml: {e}"))?;
    state.reload_config();
    Ok(())
}

#[tauri::command]
fn set_autostart(state: State, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let auto = state.handle.autolaunch();
    let result = if enabled { auto.enable() } else { auto.disable() };
    result.map_err(|e| format!("Windows would not change the startup entry: {e}"))?;
    state.changed();
    Ok(())
}

#[tauri::command]
fn open_config(state: State) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    state
        .handle
        .opener()
        .open_path(state.paths.config().display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_data_dir(state: State) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    state.handle.opener().open_path(state.paths.dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
async fn pick_program(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("Choose a program or file to open")
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
}

#[tauri::command]
fn device_action(state: State, action: String) -> Result<Option<MagCal>, String> {
    let connected = state.status.lock().state == LinkState::Connected || state.replaying.load(Ordering::Relaxed);
    let need = |what: &str| if connected { Ok(()) } else { Err(format!("Connect a controller to {what}.")) };
    match action.as_str() {
        "recenter" => {
            need("recentre")?;
            state.engine.lock().recenter();
        }
        "calibrate_gyro" => {
            need("calibrate")?;
            state.engine.lock().start_gyro_calibration();
        }
        "mag_start" => {
            need("calibrate")?;
            if !state.status.lock().has_magnetometer && !state.replaying.load(Ordering::Relaxed) {
                return Err("This controller is not sending magnetometer data.".into());
            }
            state.engine.lock().start_mag_calibration();
        }
        "mag_finish" => {
            let cal = state.engine.lock().finish_mag_calibration()?;
            if let Some(serial) = state.serial.lock().clone() {
                state.persisted.lock().devices.entry(serial).or_default().mag = Some(cal);
                state.save_persisted();
            }
            state.changed();
            return Ok(Some(cal));
        }
        "mag_cancel" => state.engine.lock().cancel_mag_calibration(),
        "mag_clear" => {
            state.engine.lock().set_mag_cal(MagCal::default());
            if let Some(serial) = state.serial.lock().clone() {
                state.persisted.lock().devices.entry(serial).or_default().mag = None;
                state.save_persisted();
            }
            state.changed();
        }
        "rumble_match" => drop(state.joycon.send(Control::Rumble(Rumble::Match))),
        "rumble_no_match" => drop(state.joycon.send(Control::Rumble(Rumble::NoMatch))),
        "disconnect" => drop(state.joycon.send(Control::Disconnect)),
        other => return Err(format!("unknown device action {other}")),
    }
    Ok(None)
}

#[tauri::command]
fn mag_progress(state: State) -> Option<MagCalProgress> {
    state.engine.lock().mag_calibration_progress()
}

#[tauri::command]
fn imu_record_start(state: State) -> Result<String, String> {
    if state.status.lock().state != LinkState::Connected {
        return Err("Connect a controller first: recordings capture what it sends.".into());
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = state.paths.recordings().join(format!("session-{secs}.wandrec"));
    let header = Header { wandrec: 1, source: "joycon".into(), product_id: 0, note: state.status.lock().name.clone() };
    let rec = Recorder::create(&path, &header).map_err(|e| format!("Could not create the recording: {e}"))?;
    *state.recorder.lock() = Some((rec, path.clone()));
    state.changed();
    Ok(path.display().to_string())
}

#[tauri::command]
fn imu_record_stop(state: State) -> Result<Option<String>, String> {
    let Some((rec, path)) = state.recorder.lock().take() else { return Ok(None) };
    let n = rec.finish().map_err(|e| e.to_string())?;
    state.changed();
    Ok(Some(format!("{} ({n} reports)", path.display())))
}

#[derive(Serialize)]
struct RecordingInfo {
    path: String,
    name: String,
    source: String,
    note: String,
    seconds: f64,
    reports: usize,
}

#[tauri::command]
fn list_recordings(state: State) -> Vec<RecordingInfo> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(state.paths.recordings()) else { return out };
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wandrec") {
            continue;
        }
        if let Ok(rec) = Recording::load(&path) {
            out.push(RecordingInfo {
                name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                path: path.display().to_string(),
                source: rec.header.source.clone(),
                note: rec.header.note.clone(),
                seconds: rec.duration_s(),
                reports: rec.reports.len(),
            });
        }
    }
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

#[tauri::command]
fn replay_file(state: State, path: String, run_actions: bool) -> Result<(), String> {
    let rec = Recording::load(std::path::Path::new(&path)).map_err(|e| format!("Could not read the recording: {e}"))?;
    let reports: Vec<_> = rec.parsed().collect();
    if reports.is_empty() {
        return Err("The recording holds no input reports.".into());
    }
    let s = state.inner().clone();
    std::thread::spawn(move || s.play(reports, !run_actions));
    Ok(())
}

/// Play a built-in or recorded gesture through the whole IMU pipeline with
/// generated sensor data. Never runs the action.
#[tauri::command]
fn replay_gesture(state: State, id: String) -> Result<(), String> {
    let (shape, px_per_degree) = {
        let e = state.engine.lock();
        let g = e.library().gestures.iter().find(|g| g.id == id).ok_or("That gesture no longer exists.")?;
        (g.variants[0].ideal.clone(), e.config().pointing.px_per_degree)
    };
    let opts = SynthOptions {
        px_per_degree,
        size_px: 620.0,
        duration_s: 0.75,
        lead_in_s: 0.5,
        seed: fastrand::u64(..),
        tilt_deg: fastrand::f32() * 10.0 - 5.0,
        ..Default::default()
    };
    let reports = synth::stroke_reports(&shape, &opts, 0, 0);
    let s = state.inner().clone();
    std::thread::spawn(move || s.play(reports, true));
    Ok(())
}

#[tauri::command]
fn replay_stop(state: State) {
    state.replay_gen.fetch_add(1, Ordering::SeqCst);
    state.replaying.store(false, Ordering::SeqCst);
    state.demo.store(false, Ordering::SeqCst);
    state.engine.lock().source_lost(Source::Replay);
    let _ = state.handle.emit_to("settings", "wand://replay", false);
}
