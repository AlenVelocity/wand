//! Bluetooth LE driver for the Joy-Con 2.
//!
//! One background thread owns the radio. It scans for a controller that is
//! either in pairing mode or advertising to reconnect to this PC, connects,
//! runs Nintendo's pairing if needed, switches on the sensors, then streams
//! input reports to the hooks until the link drops, and starts over. That
//! loop is the auto-reconnect.
//!
//! Sequence and byte values follow Nadeflore/switch2-controllers and
//! CareyScott/switch2controllerpc; see `protocol.rs` for the formats.

use crate::config::RumbleCfg;
use crate::engine::Rumble;
use crate::protocol::{self as proto, ControllerInfo, InputReport, Response, RumbleSample};
use anyhow::{anyhow, bail, Context};
use btleplug::api::{
    Central, CentralEvent, CentralState, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use futures::{Stream, StreamExt};
use serde::Serialize;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    BluetoothOff,
    #[default]
    Searching,
    Connecting,
    Pairing,
    Connected,
    Reconnecting,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct DeviceStatus {
    pub state: LinkState,
    pub name: String,
    /// "right", "left", "pro" or empty.
    pub side: String,
    pub serial: String,
    pub battery_percent: Option<u8>,
    pub battery_mv: u16,
    pub charging: bool,
    pub body_color: String,
    pub button_color: String,
    /// The bright colour of the rail and the stick's ring.
    pub accent_color: String,
    pub has_magnetometer: bool,
    /// The optical sensor in the rail is switched on.
    pub has_mouse: bool,
    pub interval_ms: Option<f32>,
    pub report_hz: f32,
    /// One sentence about the current state or the last failure.
    pub detail: String,
}

pub struct Hooks {
    /// Raw report, parsed report, monotonic seconds since the app started.
    pub report: Box<dyn Fn(&[u8], &InputReport, f64) + Send + Sync>,
    pub status: Box<dyn Fn(&DeviceStatus) + Send + Sync>,
    pub connected: Box<dyn Fn(&ControllerInfo) + Send + Sync>,
    pub lost: Box<dyn Fn() + Send + Sync>,
    pub rumble_cfg: Box<dyn Fn() -> RumbleCfg + Send + Sync>,
    /// Whether to switch the optical sensor on when a controller connects.
    pub mouse_enabled: Box<dyn Fn() -> bool + Send + Sync>,
}

pub enum Control {
    Rumble(Rumble),
    /// Switch the optical sensor on or off on the connected controller.
    Mouse(bool),
    Disconnect,
    Shutdown,
}

/// Start the driver thread. Dropping the sender, or sending `Shutdown`, ends it.
pub fn spawn(hooks: Hooks, t0: Instant) -> UnboundedSender<Control> {
    let (tx, rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("wand-joycon".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(driver(hooks, t0, rx));
        })
        .expect("spawn joycon thread");
    tx
}

enum End {
    Lost,
    Shutdown,
}

async fn driver(hooks: Hooks, t0: Instant, mut ctl: UnboundedReceiver<Control>) {
    let mut status = DeviceStatus::default();
    let mut had_link = false;
    loop {
        let result = session(&hooks, t0, &mut ctl, &mut status, had_link).await;
        (hooks.lost)();
        match result {
            Ok(End::Shutdown) => return,
            Ok(End::Lost) => {
                had_link = true;
                status.detail = "The controller disconnected. Press any button on it to reconnect.".into();
            }
            Err(e) => {
                log::warn!("joycon session ended: {e:#}");
                status.detail = format!("{e:#}");
            }
        }
        if status.state != LinkState::BluetoothOff {
            status.state = if had_link { LinkState::Reconnecting } else { LinkState::Searching };
        }
        status.battery_percent = None;
        status.interval_ms = None;
        status.report_hz = 0.0;
        (hooks.status)(&status);
        // Back off briefly, still listening for shutdown.
        let pause = tokio::time::sleep(Duration::from_millis(if status.state == LinkState::BluetoothOff { 3000 } else { 700 }));
        tokio::pin!(pause);
        loop {
            tokio::select! {
                _ = &mut pause => break,
                c = ctl.recv() => match c {
                    Some(Control::Shutdown) | None => return,
                    _ => {}
                }
            }
        }
    }
}

type Notifications = Pin<Box<dyn Stream<Item = ValueNotification> + Send>>;

struct Link {
    p: Peripheral,
    command: Characteristic,
    notifications: Notifications,
}

fn write_type(c: &Characteristic, prefer_no_response: bool) -> WriteType {
    let with = c.properties.contains(CharPropFlags::WRITE);
    let without = c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE);
    match (with, without, prefer_no_response) {
        (_, true, true) | (false, true, _) => WriteType::WithoutResponse,
        _ => WriteType::WithResponse,
    }
}

impl Link {
    /// Write a command and wait for the response that names the same command.
    async fn request(&mut self, bytes: Vec<u8>) -> anyhow::Result<Response> {
        let (cmd, sub) = (bytes[0], bytes[3]);
        self.p.write(&self.command, &bytes, write_type(&self.command, false)).await?;
        let deadline = tokio::time::sleep(Duration::from_secs(2));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                n = self.notifications.next() => {
                    let n = n.ok_or_else(|| anyhow!("the controller disconnected"))?;
                    if n.uuid != proto::CHAR_RESPONSE {
                        continue;
                    }
                    if let Some(r) = proto::parse_response(&n.value) {
                        if r.cmd == cmd && r.sub == sub {
                            return Ok(r);
                        }
                    }
                }
                _ = &mut deadline => bail!("the controller did not answer command {cmd:#04x}/{sub:#04x}"),
            }
        }
    }

    async fn read_memory(&mut self, len: u8, address: u32) -> anyhow::Result<Vec<u8>> {
        let r = self.request(proto::cmd_read_memory(len, address)).await?;
        proto::memory_payload(&r.data, len, address)
            .map(|d| d.to_vec())
            .ok_or_else(|| anyhow!("unexpected answer to a memory read at {address:#x}"))
    }

    /// Nintendo's pairing over the command channel. Afterwards the controller
    /// remembers this PC and advertises to it when a button is pressed.
    async fn pair(&mut self, host: u64) -> anyhow::Result<()> {
        self.request(proto::cmd_pair_addresses(host)).await.context("pairing: address exchange")?;
        let a1: [u8; 16] = std::array::from_fn(|_| fastrand::u8(..));
        let keys = self.request(proto::cmd_pair_keys(&a1)).await.context("pairing: key exchange")?;
        let a2: [u8; 16] = std::array::from_fn(|_| fastrand::u8(..));
        let confirm = self.request(proto::cmd_pair_confirm(&a2)).await.context("pairing: challenge")?;
        if let (Some(b1), Some(b2)) = (keys.data.get(1..17), confirm.data.get(1..17)) {
            let b1: [u8; 16] = b1.try_into().unwrap();
            if proto::expected_confirm(&a1, &b1, &a2) != b2 {
                // Not fatal: the link is not encrypted with this key on Windows.
                log::warn!("pairing: the controller's challenge response did not verify");
            }
        }
        self.request(proto::cmd_pair_finish()).await.context("pairing: finish")?;
        Ok(())
    }
}

fn hex_color(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

async fn session(
    hooks: &Hooks,
    t0: Instant,
    ctl: &mut UnboundedReceiver<Control>,
    status: &mut DeviceStatus,
    had_link: bool,
) -> anyhow::Result<End> {
    let manager = Manager::new().await.context("Bluetooth is not available")?;
    let Some(adapter) = manager.adapters().await?.into_iter().next() else {
        status.state = LinkState::BluetoothOff;
        bail!("This PC has no Bluetooth adapter.");
    };
    if matches!(adapter.adapter_state().await, Ok(CentralState::PoweredOff)) {
        status.state = LinkState::BluetoothOff;
        bail!("Bluetooth is turned off. Turn it on in Windows settings.");
    }
    let host: u64 = adapter.adapter_address().await.ok().flatten().map(u64::from).unwrap_or(0);

    let mut events = adapter.events().await?;
    adapter.start_scan(ScanFilter::default()).await.context("could not start scanning")?;
    status.state = if had_link { LinkState::Reconnecting } else { LinkState::Searching };
    if status.detail.is_empty() || !had_link {
        status.detail = "Hold the small sync button on the Joy-Con's rail until its lights chase. A controller already paired with this PC reconnects when you press any button.".into();
    }
    (hooks.status)(status);

    // --- wait for a controller that wants this PC
    let (id, advert) = loop {
        tokio::select! {
            ev = events.next() => match ev {
                Some(CentralEvent::ManufacturerDataAdvertisement { id, manufacturer_data }) => {
                    let Some(a) = manufacturer_data.get(&proto::NINTENDO_COMPANY_ID).and_then(|d| proto::parse_advert(d)) else { continue };
                    if !proto::is_supported(a.product_id) {
                        continue;
                    }
                    if a.host_address == 0 || host == 0 || a.host_address == host {
                        break (id, a);
                    }
                    let hint = format!("A {} nearby is paired with another device. Hold its sync button to pair it with this PC.", proto::controller_name(a.product_id));
                    if status.detail != hint {
                        log::info!("saw a {} that wants host {:012x}; this PC is {:012x}", proto::controller_name(a.product_id), a.host_address, host);
                        status.detail = hint;
                        (hooks.status)(status);
                    }
                }
                Some(CentralEvent::StateUpdate(CentralState::PoweredOff)) => {
                    status.state = LinkState::BluetoothOff;
                    bail!("Bluetooth was turned off.");
                }
                Some(_) => {}
                None => bail!("the Bluetooth adapter stopped reporting"),
            },
            c = ctl.recv() => match c {
                Some(Control::Shutdown) | None => {
                    let _ = adapter.stop_scan().await;
                    return Ok(End::Shutdown);
                }
                _ => {}
            }
        }
    };
    let _ = adapter.stop_scan().await;

    let pairing = advert.host_address == 0;
    log::info!(
        "found {} ({}); its host field is {:012x}, this PC is {:012x}",
        proto::controller_name(advert.product_id),
        if pairing { "pairing mode" } else { "reconnecting" },
        advert.host_address,
        host
    );
    status.state = if pairing { LinkState::Pairing } else { LinkState::Connecting };
    status.name = proto::controller_name(advert.product_id).to_string();
    status.detail = format!("Found a {}.", status.name);
    (hooks.status)(status);

    let p = adapter.peripheral(&id).await?;
    tokio::time::timeout(Duration::from_secs(15), p.connect())
        .await
        .map_err(|_| anyhow!("connecting to the controller timed out"))?
        .context("could not connect to the controller")?;

    // Everything after this point must disconnect on failure.
    let result = run_link(hooks, t0, ctl, status, &p, host, pairing).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), p.disconnect()).await;
    result
}

async fn run_link(
    hooks: &Hooks,
    t0: Instant,
    ctl: &mut UnboundedReceiver<Control>,
    status: &mut DeviceStatus,
    p: &Peripheral,
    host: u64,
    pairing: bool,
) -> anyhow::Result<End> {
    // Windows defaults a BLE link to a slow connection interval and the
    // controller never asks for a faster one itself. Hold a throughput request
    // open for as long as the link lives.
    let address: u64 = p.address().into();
    let _fast = tokio::task::spawn_blocking(move || fast_link::request(address)).await.ok().flatten();

    log::info!("link up, reading services");
    p.discover_services().await.context("could not read the controller's services")?;
    let chars = p.characteristics();
    let find = |uuid| chars.iter().find(|c| c.uuid == uuid).cloned();
    let input = find(proto::CHAR_INPUT).ok_or_else(|| anyhow!("this device is not a Switch 2 controller (no input characteristic)"))?;
    let command = find(proto::CHAR_COMMAND).ok_or_else(|| anyhow!("no command characteristic"))?;
    let response = find(proto::CHAR_RESPONSE).ok_or_else(|| anyhow!("no response characteristic"))?;

    log::info!("characteristics: input {:?}, command {:?}, response {:?}", input.properties, command.properties, response.properties);
    let notifications = p.notifications().await?;
    // Windows sometimes refuses the first subscription right after connecting
    // (the reference drivers retry here too).
    let mut attempt = 0;
    loop {
        match p.subscribe(&response).await {
            Ok(()) => break,
            Err(e) if attempt < 3 => {
                attempt += 1;
                log::warn!("subscribing to command responses failed ({e}); retry {attempt}");
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
            Err(e) => return Err(e).context("could not subscribe to command responses"),
        }
    }
    let mut link = Link { p: p.clone(), command, notifications };

    let info = proto::parse_controller_info(&link.read_memory(0x40, proto::ADDR_CONTROLLER_INFO).await?)
        .ok_or_else(|| anyhow!("could not read the controller's identity"))?;
    let pid = info.product_id;
    let rumble = proto::rumble_char(pid).and_then(|u| chars.iter().find(|c| c.uuid == u).cloned());
    log::info!("controller info: serial {:?}, product {:#06x}, rumble characteristic {}", info.serial, pid, if rumble.is_some() { "found" } else { "missing" });

    if pairing {
        if host == 0 {
            log::warn!("adapter address unknown: skipping pairing, the controller will not reconnect on its own");
        } else {
            link.pair(host).await?;
            log::info!("pairing exchange finished");
        }
    }
    let _ = link.request(proto::cmd_set_leds(0x01)).await;

    // Buttons, sticks, IMU and rumble always. The optical sensor (Joy-Con
    // only) and the magnetometer are asked for on top; if the controller
    // refuses a set, the next smaller one is tried, mouse before magnetometer
    // because strokes do not need the magnetometer.
    let base = proto::FEATURE_BUTTONS | proto::FEATURE_STICKS | proto::FEATURE_IMU | proto::FEATURE_RUMBLE;
    let has_sensor = matches!(pid, proto::PID_JOYCON_R | proto::PID_JOYCON_L);
    let want_mouse = has_sensor && (hooks.mouse_enabled)();
    let mut sets = vec![base | proto::FEATURE_MAGNETOMETER, base];
    if want_mouse {
        sets = vec![base | proto::FEATURE_MOUSE | proto::FEATURE_MAGNETOMETER, base | proto::FEATURE_MOUSE, base | proto::FEATURE_MAGNETOMETER, base];
    }
    let mut features = 0;
    for (i, set) in sets.iter().enumerate() {
        let selected = link.request(proto::cmd_set_feature_mask(*set)).await;
        let enabled = match selected {
            Ok(_) => link.request(proto::cmd_enable_features(*set)).await,
            Err(e) => Err(e),
        };
        match enabled {
            Ok(_) => {
                features = *set;
                break;
            }
            Err(e) if i + 1 < sets.len() => log::warn!("the controller refused feature set {set:#04x} ({e:#}); trying a smaller one"),
            Err(e) => return Err(e).context("could not enable the motion sensors"),
        }
    }

    (hooks.connected)(&info);
    p.subscribe(&input).await.context("could not subscribe to input reports")?;

    status.state = LinkState::Connected;
    status.name = proto::controller_name(pid).to_string();
    status.side = match pid {
        proto::PID_JOYCON_R => "right",
        proto::PID_JOYCON_L => "left",
        proto::PID_PRO => "pro",
        _ => "",
    }
    .into();
    status.serial = info.serial.clone();
    status.body_color = hex_color(info.body_color);
    status.button_color = hex_color(info.button_color);
    status.accent_color = hex_color(info.accent_color());
    log::info!("colours stored on the controller: {}", info.colors.map(hex_color).join(" "));
    status.has_magnetometer = false;
    status.has_mouse = features & proto::FEATURE_MOUSE != 0;
    status.interval_ms = p.connection_parameters().await.ok().flatten().map(|c| c.interval_us as f32 / 1000.0);
    status.detail = if pairing { "Paired.".into() } else { "Connected.".into() };
    (hooks.status)(status);
    log::info!("connected: {} serial {} features {features:#04x} interval {:?} ms", status.name, status.serial, status.interval_ms);

    let seq = Arc::new(AtomicU8::new(0));
    let mut last_report = tokio::time::Instant::now();
    let mut last_status = Instant::now();
    let mut window_reports = 0u32;
    let mut mv_smooth = 0.0f32;
    let mut total_reports = 0u64;
    let mut next_summary = Instant::now() + Duration::from_secs(1);

    loop {
        tokio::select! {
            n = link.notifications.next() => {
                let Some(n) = n else { return Ok(End::Lost) };
                if n.uuid != proto::CHAR_INPUT {
                    continue;
                }
                let Some(report) = InputReport::parse(&n.value) else { continue };
                (hooks.report)(&n.value, &report, t0.elapsed().as_secs_f64());
                last_report = tokio::time::Instant::now();
                window_reports += 1;
                total_reports += 1;
                if total_reports == 1 {
                    let hex: String = n.value.iter().map(|b| format!("{b:02x}")).collect();
                    log::info!("first input report ({} bytes): {hex}", n.value.len());
                }
                // A line of raw sensor values now and then: what tells apart a
                // wrong axis convention from a wrong byte offset.
                if Instant::now() >= next_summary {
                    next_summary = Instant::now() + Duration::from_secs(if total_reports < 2000 { 3 } else { 60 });
                    let g = report.gyro_rad_s().map(f32::to_degrees);
                    log::info!(
                        "sensors: accel {:+.2?} g, gyro {:+.1?} dps, mag {:?} uT, mouse {:?}, sticks {:?} {:?}, {} mV, charge {:#04x}, buttons {:#010x}, t {} us",
                        report.accel_g(), g, report.mag_ut().map(|m| m.map(|v| v.round())), report.mouse, report.left_stick, report.right_stick,
                        report.battery_mv, report.charge_state, report.buttons, report.motion_time_us
                    );
                }
                if report.battery_mv > 0 {
                    mv_smooth = if mv_smooth == 0.0 { report.battery_mv as f32 } else { mv_smooth + (report.battery_mv as f32 - mv_smooth) * 0.02 };
                }
                let elapsed = last_status.elapsed();
                if elapsed >= Duration::from_secs(1) {
                    let mut next = status.clone();
                    next.report_hz = (window_reports as f32 / elapsed.as_secs_f32()).round();
                    next.battery_mv = mv_smooth.round() as u16;
                    next.battery_percent = (mv_smooth > 0.0).then(|| proto::battery_percent(next.battery_mv));
                    next.charging = report.charging();
                    next.has_magnetometer = features & proto::FEATURE_MAGNETOMETER != 0 && report.mag_ut().is_some();
                    if next.interval_ms.is_none() {
                        next.interval_ms = p.connection_parameters().await.ok().flatten().map(|c| c.interval_us as f32 / 1000.0);
                    }
                    window_reports = 0;
                    last_status = Instant::now();
                    if next != *status {
                        *status = next;
                        (hooks.status)(status);
                    }
                }
            }
            c = ctl.recv() => match c {
                Some(Control::Rumble(kind)) => {
                    let cfg = (hooks.rumble_cfg)();
                    if let (true, Some(ch)) = (cfg.enabled, rumble.clone()) {
                        tokio::spawn(play(p.clone(), ch, pid, rumble_steps(kind, &cfg), seq.clone()));
                    }
                }
                Some(Control::Mouse(on)) => {
                    if has_sensor && on != (features & proto::FEATURE_MOUSE != 0) {
                        let next = if on { features | proto::FEATURE_MOUSE } else { features & !proto::FEATURE_MOUSE };
                        let result = if on {
                            match link.request(proto::cmd_set_feature_mask(next)).await {
                                Ok(_) => link.request(proto::cmd_enable_features(next)).await,
                                Err(e) => Err(e),
                            }
                        } else {
                            link.request(proto::cmd_disable_features(proto::FEATURE_MOUSE)).await
                        };
                        match result {
                            Ok(_) => {
                                features = next;
                                status.has_mouse = on;
                                (hooks.status)(status);
                                log::info!("optical sensor switched {}; features {features:#04x}", if on { "on" } else { "off" });
                            }
                            // The engine ignores the sensor while the setting is off,
                            // so a refusal here only costs a little battery.
                            Err(e) => log::warn!("could not switch the optical sensor {}: {e:#}", if on { "on" } else { "off" }),
                        }
                        last_report = tokio::time::Instant::now();
                    }
                }
                Some(Control::Disconnect) => return Ok(End::Lost),
                Some(Control::Shutdown) | None => return Ok(End::Shutdown),
            },
            // No report for four seconds: the controller is gone even if
            // Windows has not noticed yet.
            _ = tokio::time::sleep_until(last_report + Duration::from_secs(4)) => return Ok(End::Lost),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Set(RumbleSample),
    Wait(u32),
}

/// The haptic vocabulary: one crisp tick for a match, two soft low pulses for
/// a miss, a faint tick when a sample is captured or pointing is recentred.
pub fn rumble_steps(kind: Rumble, cfg: &RumbleCfg) -> Vec<Step> {
    use Step::*;
    let off = Set(RumbleSample::SILENT);
    match kind {
        Rumble::Match => vec![Set(RumbleSample::tick(cfg.match_strength)), Wait(cfg.match_ms), off],
        Rumble::NoMatch => {
            let thud = Set(RumbleSample::thud(cfg.no_match_strength));
            vec![thud, Wait(cfg.no_match_ms), off, Wait(cfg.no_match_gap_ms), thud, Wait(cfg.no_match_ms), off]
        }
        Rumble::Captured => vec![Set(RumbleSample::tick(cfg.match_strength * 0.5)), Wait(cfg.match_ms), off],
        Rumble::Recenter => vec![Set(RumbleSample::tick(cfg.match_strength * 0.35)), Wait(cfg.match_ms.min(10)), off],
    }
}

async fn play(p: Peripheral, ch: Characteristic, pid: u16, steps: Vec<Step>, seq: Arc<AtomicU8>) {
    let wt = write_type(&ch, true);
    for step in steps {
        match step {
            Step::Wait(ms) => tokio::time::sleep(Duration::from_millis(ms as u64)).await,
            Step::Set(s) => {
                let n = seq.fetch_add(1, Ordering::Relaxed);
                let result = if pid == proto::PID_PRO {
                    p.write(&ch, &proto::rumble_packet_pro(n, [s; 3]), wt).await
                } else {
                    p.write(&ch, &proto::rumble_packet(n, [s; 3]), wt).await
                };
                if let Err(e) = result {
                    log::debug!("rumble write failed: {e}");
                    return;
                }
            }
        }
    }
}

#[cfg(windows)]
mod fast_link {
    use windows::Devices::Bluetooth::{
        BluetoothLEDevice, BluetoothLEPreferredConnectionParameters, BluetoothLEPreferredConnectionParametersRequest,
    };

    /// Keeps the throughput-optimised request alive; Windows drops back to its
    /// default interval when the request object is released.
    pub struct FastLink {
        _device: BluetoothLEDevice,
        _request: BluetoothLEPreferredConnectionParametersRequest,
    }

    pub fn request(address: u64) -> Option<FastLink> {
        let run = || -> windows::core::Result<FastLink> {
            let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;
            let params = BluetoothLEPreferredConnectionParameters::ThroughputOptimized()?;
            let request = device.RequestPreferredConnectionParameters(&params)?;
            log::info!("connection parameters request: {:?}", request.Status().map(|s| s.0));
            Ok(FastLink { _device: device, _request: request })
        };
        match run() {
            Ok(link) => Some(link),
            Err(e) => {
                log::warn!("could not request a fast connection interval: {e}");
                None
            }
        }
    }
}

#[cfg(not(windows))]
mod fast_link {
    pub struct FastLink;
    pub fn request(_: u64) -> Option<FastLink> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn haptic_patterns() {
        let cfg = Config::default().rumble;
        let m = rumble_steps(Rumble::Match, &cfg);
        assert_eq!(m.len(), 3);
        assert!(matches!(m[0], Step::Set(s) if s.hf_amp > 600 && s.lf_amp < 100), "a match is a high, sharp pulse");
        assert_eq!(m[2], Step::Set(RumbleSample::SILENT));

        let n = rumble_steps(Rumble::NoMatch, &cfg);
        let pulses = n.iter().filter(|s| matches!(s, Step::Set(x) if x.lf_amp > 0)).count();
        assert_eq!(pulses, 2, "a miss is a double pulse");
        assert!(matches!(n[0], Step::Set(s) if s.lf_amp < 500 && s.hf_amp < 300), "and a soft one");
        assert_eq!(*n.last().unwrap(), Step::Set(RumbleSample::SILENT), "every pattern ends silent");
        let total: u32 = n.iter().map(|s| if let Step::Wait(ms) = s { *ms } else { 0 }).sum();
        assert!(total < 200, "the whole miss signal fits well inside the 400 ms confirmation");
    }

    #[test]
    fn write_type_follows_properties() {
        let mk = |p| Characteristic {
            uuid: proto::CHAR_COMMAND,
            service_uuid: proto::CHAR_COMMAND,
            properties: p,
            descriptors: Default::default(),
        };
        assert_eq!(write_type(&mk(CharPropFlags::WRITE), true), WriteType::WithResponse);
        assert_eq!(write_type(&mk(CharPropFlags::WRITE_WITHOUT_RESPONSE), false), WriteType::WithoutResponse);
        let both = mk(CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE);
        assert_eq!(write_type(&both, true), WriteType::WithoutResponse);
        assert_eq!(write_type(&both, false), WriteType::WithResponse);
    }
}
