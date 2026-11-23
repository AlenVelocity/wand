//! What Win32 does for the app on Windows, done the macOS way: synthetic key
//! and mouse events, the Accessibility permission they need, the displays,
//! this Mac's Bluetooth address, and the overlay windows' place above
//! everything else.
//!
//! Coordinates. macOS lays displays out in points, and each display has its
//! own pixels per point. The engine wants one pixel space shared by every
//! display, as Windows gives it. Here that space is points times the largest
//! scale of any connected display (`unit_scale`): on a single Retina display
//! it is its physical pixels, and displays of mixed scale still sit side by
//! side in it the way System Settings arranges them.

use crate::pointing::Monitor;
use objc2::msg_send;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGDirectDisplayID, CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMirrorsDisplay, CGDisplayMode, CGEvent,
    CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventType,
    CGGetOnlineDisplayList, CGMainDisplayID, CGMouseButton, CGScrollEventUnit,
};
use parking_lot::Mutex;
use std::ffi::{c_void, CStr};
use std::time::{Duration, Instant};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const AnyObject;
}

// Classes looked up by name below live in these.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}
#[link(name = "IOBluetooth", kind = "framework")]
extern "C" {}
#[link(name = "CoreBluetooth", kind = "framework")]
extern "C" {}

fn class(name: &CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

// ---------------------------------------------------------------- permissions

/// Whether macOS lets this app post key and mouse events. Without it they are
/// dropped without an error.
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Ask for Accessibility access: macOS shows its own prompt, which lists the
/// app in System Settings, and the settings pane opens.
pub fn request_accessibility() -> bool {
    let trusted = prompt_for_accessibility();
    if !trusted {
        open_settings("Privacy_Accessibility");
    }
    trusted
}

/// Whether the app has Accessibility access; if not, macOS shows its own
/// prompt with a button to the settings pane.
pub fn prompt_for_accessibility() -> bool {
    autoreleasepool(|_| match prompt_options() {
        Some(options) => unsafe { AXIsProcessTrustedWithOptions(Retained::as_ptr(&options).cast()) },
        None => accessibility_trusted(),
    })
}

/// `accessibility_trusted`, asked again at most every two seconds: the
/// cursor thread wants to know on every movement.
fn accessibility_trusted_lately() -> bool {
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut last = LAST.lock();
    match *last {
        Some((at, trusted)) if at.elapsed() < Duration::from_secs(2) => trusted,
        _ => {
            let trusted = accessibility_trusted();
            *last = Some((Instant::now(), trusted));
            trusted
        }
    }
}

/// `{kAXTrustedCheckOptionPrompt: true}`.
fn prompt_options() -> Option<Retained<AnyObject>> {
    unsafe {
        let yes: Retained<AnyObject> = msg_send![class(c"NSNumber")?, numberWithBool: true];
        msg_send![class(c"NSDictionary")?, dictionaryWithObject: &*yes, forKey: kAXTrustedCheckOptionPrompt]
    }
}

/// Open a pane of System Settings > Privacy & Security.
pub fn open_settings(anchor: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    if let Err(e) = std::process::Command::new("/usr/bin/open").arg(url).spawn() {
        log::warn!("could not open System Settings: {e}");
    }
}

pub const NEEDS_ACCESSIBILITY: &str =
    "Wand needs Accessibility access to press keys and move the cursor. Allow it in System Settings > Privacy & Security > Accessibility.";

// ------------------------------------------------------------------ keyboard

/// Virtual-key codes (the Windows numbering the rest of the app uses) to
/// macOS key codes (`kVK_*` from Carbon's Events.h).
pub fn key_code(vk: u16) -> Option<u16> {
    const LETTERS: [u16; 26] = [
        0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, // A-M
        0x2D, 0x1F, 0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06, // N-Z
    ];
    const DIGITS: [u16; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
    const FKEYS: [u16; 20] = [
        0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, // F1-F12
        0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A, // F13-F20
    ];
    Some(match vk {
        0x41..=0x5A => LETTERS[(vk - 0x41) as usize],
        0x30..=0x39 => DIGITS[(vk - 0x30) as usize],
        0x70..=0x83 => FKEYS[(vk - 0x70) as usize],
        0x10 | 0xA0 => 0x38, // Shift
        0xA1 => 0x3C,        // right Shift
        0x11 | 0xA2 => 0x3B, // Control
        0xA3 => 0x3E,        // right Control
        0x12 | 0xA4 => 0x3A, // Option
        0xA5 => 0x3D,        // right Option
        0x5B => 0x37,        // Command
        0x5C => 0x36,        // right Command
        0x0D => 0x24,        // Return
        0x1B => 0x35,        // Escape
        0x20 => 0x31,        // Space
        0x09 => 0x30,        // Tab
        0x08 => 0x33,        // Delete (backspace)
        0x2E => 0x75,        // Forward Delete
        0x2D => 0x72,        // Help, where Insert sits on a PC keyboard
        0x24 => 0x73,        // Home
        0x23 => 0x77,        // End
        0x21 => 0x74,        // Page Up
        0x22 => 0x79,        // Page Down
        0x25 => 0x7B,        // Left
        0x26 => 0x7E,        // Up
        0x27 => 0x7C,        // Right
        0x28 => 0x7D,        // Down
        // A PC keyboard on a Mac sends these three as F13 to F15.
        0x2C => 0x69, // Print Screen
        0x91 => 0x6B, // Scroll Lock
        0x13 => 0x71, // Pause
        0x14 => 0x39, // Caps Lock
        0xBB => 0x18, // =
        0xBD => 0x1B, // -
        0xBC => 0x2B, // ,
        0xBE => 0x2F, // .
        0xBF => 0x2C, // /
        0xBA => 0x29, // ;
        0xDE => 0x27, // '
        0xC0 => 0x32, // `
        0xDB => 0x21, // [
        0xDD => 0x1E, // ]
        0xDC => 0x2A, // \
        _ => return None,
    })
}

/// The `NX_KEYTYPE_*` number of a media key, which macOS sends as a
/// system-defined event and not as a key code.
pub fn media_code(vk: u16) -> Option<i64> {
    Some(match vk {
        0xAF => 0,  // NX_KEYTYPE_SOUND_UP
        0xAE => 1,  // NX_KEYTYPE_SOUND_DOWN
        0xAD => 7,  // NX_KEYTYPE_MUTE
        0xB3 => 16, // NX_KEYTYPE_PLAY
        0xB0 => 17, // NX_KEYTYPE_NEXT
        0xB1 => 18, // NX_KEYTYPE_PREVIOUS
        _ => return None,
    })
}

pub fn is_media(vk: u16) -> bool {
    (0xAD..=0xB3).contains(&vk)
}

fn modifier_flag(code: u16) -> CGEventFlags {
    match code {
        0x38 | 0x3C => CGEventFlags::MaskShift,
        0x3B | 0x3E => CGEventFlags::MaskControl,
        0x3A | 0x3D => CGEventFlags::MaskAlternate,
        0x37 | 0x36 => CGEventFlags::MaskCommand,
        _ => CGEventFlags::empty(),
    }
}

/// Flags a real keyboard puts on these keys. System shortcuts match on them:
/// Mission Control's Control+Up only fires with the Fn flag set.
fn key_flags(code: u16) -> CGEventFlags {
    match code {
        0x7B..=0x7E => CGEventFlags::MaskSecondaryFn | CGEventFlags::MaskNumericPad,
        0x72..=0x79 | 0x60..=0x6F | 0x40 | 0x4F | 0x50 | 0x5A | 0x7A => CGEventFlags::MaskSecondaryFn,
        _ => CGEventFlags::empty(),
    }
}

fn source() -> Option<objc2_core_foundation::CFRetained<CGEventSource>> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
}

fn post(event: &CGEvent) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event));
}

/// Press macOS key codes in order, then release them in reverse. Modifier
/// state is set on every event explicitly: apps read the flags, not which
/// keys went down before.
pub fn send_key_codes(codes: &[u16]) -> Result<(), String> {
    if !accessibility_trusted() {
        return Err(NEEDS_ACCESSIBILITY.into());
    }
    let src = source();
    let mut held = CGEventFlags::empty();
    let send = |code: u16, down: bool, flags: CGEventFlags| -> Result<(), String> {
        let event = CGEvent::new_keyboard_event(src.as_deref(), code, down).ok_or("macOS would not create a key event.")?;
        CGEvent::set_flags(Some(&event), flags | key_flags(code));
        post(&event);
        // A breath between events, as a hand would leave; some system
        // shortcuts ignore a combination that arrives all at once.
        std::thread::sleep(Duration::from_millis(3));
        Ok(())
    };
    for &code in codes {
        held |= modifier_flag(code);
        send(code, true, held)?;
    }
    for &code in codes.iter().rev() {
        held &= !modifier_flag(code);
        send(code, false, held)?;
    }
    Ok(())
}

/// Tap a media key: volume, mute, play/pause, next, previous.
pub fn send_media(nx_code: i64) -> Result<(), String> {
    if !accessibility_trusted() {
        return Err(NEEDS_ACCESSIBILITY.into());
    }
    for down in [true, false] {
        autoreleasepool(|_| -> Result<(), String> {
            let event = media_event(nx_code, down).ok_or("macOS would not create a media key event.")?;
            let cg: *mut CGEvent = unsafe { msg_send![&*event, CGEvent] };
            if let Some(cg) = unsafe { cg.as_ref() } {
                post(cg);
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// The system-defined event a keyboard's media key sends:
/// NSEventTypeSystemDefined, subtype NX_SUBTYPE_AUX_CONTROL_BUTTONS.
fn media_event(nx_code: i64, down: bool) -> Option<Retained<AnyObject>> {
    let state: i64 = if down { 0xA } else { 0xB };
    unsafe {
        msg_send![
            class(c"NSEvent")?,
            otherEventWithType: 14usize,
            location: CGPoint::new(0.0, 0.0),
            modifierFlags: (state << 8) as usize,
            timestamp: 0.0f64,
            windowNumber: 0isize,
            context: std::ptr::null_mut::<AnyObject>(),
            subtype: 8i16,
            data1: ((nx_code << 16) | (state << 8)) as isize,
            data2: -1isize
        ]
    }
}

/// Whether a key is held right now, by its macOS key code.
pub fn key_down(code: u16) -> bool {
    CGEventSource::key_state(CGEventSourceStateID::CombinedSessionState, code)
}

// ------------------------------------------------------------------- displays

#[derive(Clone, Copy, Debug)]
pub struct Display {
    /// In points, origin at the top left of the main display, y down.
    pub bounds: CGRect,
    pub scale: f64,
    pub main: bool,
}

/// Every connected display, asleep or awake, so the overlays are in place
/// before a sleeping display wakes. A display mirroring another shows the same
/// pixels and gets no overlay of its own.
fn read_displays() -> Vec<Display> {
    let mut ids: [CGDirectDisplayID; 16] = [0; 16];
    let mut count = 0u32;
    let err = unsafe { CGGetOnlineDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if err.0 != 0 {
        return Vec::new();
    }
    let main = CGMainDisplayID();
    ids[..count as usize]
        .iter()
        .filter(|&&id| CGDisplayMirrorsDisplay(id) == 0)
        .map(|&id| {
            let bounds = CGDisplayBounds(id);
            let scale = CGDisplayCopyDisplayMode(id)
                .map(|mode| {
                    let (px, pt) = (CGDisplayMode::pixel_width(Some(&mode)), CGDisplayMode::width(Some(&mode)));
                    if pt > 0 { px as f64 / pt as f64 } else { 1.0 }
                })
                .unwrap_or(1.0);
            Display { bounds, scale: scale.max(1.0), main: id == main }
        })
        .collect()
}

static DISPLAYS: Mutex<Option<(Instant, Vec<Display>)>> = Mutex::new(None);

/// The connected displays, read again at most once a second: the cursor
/// thread asks on every movement.
pub fn displays() -> Vec<Display> {
    let mut cache = DISPLAYS.lock();
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(1) && !list.is_empty() {
            return list.clone();
        }
    }
    let list = read_displays();
    *cache = Some((Instant::now(), list.clone()));
    list
}

/// Engine pixels per point.
pub fn unit_scale(displays: &[Display]) -> f64 {
    displays.iter().map(|d| d.scale).fold(1.0, f64::max)
}

/// The displays as the engine and the overlays see them. Every overlay draws
/// at `unit_scale`, so a stroke has the same size in points on every display.
pub fn monitors() -> Vec<Monitor> {
    let ds = displays();
    let u = unit_scale(&ds);
    let mut list: Vec<Monitor> = ds
        .iter()
        .map(|d| Monitor {
            x: (d.bounds.origin.x * u) as f32,
            y: (d.bounds.origin.y * u) as f32,
            w: (d.bounds.size.width * u) as f32,
            h: (d.bounds.size.height * u) as f32,
            scale: u as f32,
            primary: d.main,
        })
        .collect();
    list.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.x.total_cmp(&b.x)).then(a.y.total_cmp(&b.y)));
    list
}

pub fn contains(r: &CGRect, p: CGPoint) -> bool {
    p.x >= r.origin.x && p.y >= r.origin.y && p.x < r.origin.x + r.size.width && p.y < r.origin.y + r.size.height
}

/// Where the cursor is, in points.
pub fn cursor_points() -> Option<CGPoint> {
    let event = CGEvent::new(None)?;
    Some(CGEvent::location(Some(&event)))
}

/// Where the cursor is, in engine pixels.
pub fn cursor_units() -> Option<(f32, f32)> {
    let p = cursor_points()?;
    let u = unit_scale(&displays());
    Some(((p.x * u) as f32, (p.y * u) as f32))
}

// ---------------------------------------------------------------------- mouse

pub fn mouse_event(kind: CGEventType, at: CGPoint, button: CGMouseButton) -> Option<objc2_core_foundation::CFRetained<CGEvent>> {
    CGEvent::new_mouse_event(source().as_deref(), kind, at, button)
}

pub fn set_field(event: &CGEvent, field: CGEventField, value: i64) {
    CGEvent::set_integer_value_field(Some(event), field, value);
}

/// Post a mouse event. Without Accessibility access macOS would drop it
/// without a word, so the log says so, once until access comes.
pub fn post_event(event: &CGEvent) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !accessibility_trusted_lately() {
        if !WARNED.swap(true, Ordering::Relaxed) {
            log::warn!("the cursor cannot move: {NEEDS_ACCESSIBILITY}");
        }
        return;
    }
    WARNED.store(false, Ordering::Relaxed);
    post(event);
}

pub fn scroll_event(pixels: i32) -> Option<objc2_core_foundation::CFRetained<CGEvent>> {
    CGEvent::new_scroll_wheel_event2(source().as_deref(), CGScrollEventUnit::Pixel, 1, pixels, 0, 0)
}

/// The user's double-click interval, in seconds.
pub fn double_click_interval() -> f64 {
    class(c"NSEvent").map(|c| unsafe { msg_send![c, doubleClickInterval] }).unwrap_or(0.5)
}

// ------------------------------------------------------------------ Bluetooth

/// CBManagerAuthorization: 0 not determined, 1 restricted, 2 denied, 3 allowed.
fn bluetooth_authorization() -> Option<isize> {
    let manager = class(c"CBManager")?;
    manager.class_method(Sel::register(c"authorization"))?;
    Some(unsafe { msg_send![manager, authorization] })
}

/// Bluetooth permission is refused or blocked by a profile.
pub fn bluetooth_denied() -> bool {
    matches!(bluetooth_authorization(), Some(1 | 2))
}

/// macOS is still asking whether the app may use Bluetooth.
pub fn bluetooth_undecided() -> bool {
    bluetooth_authorization() == Some(0)
}

/// This Mac's Bluetooth address, as the 48-bit number the controller stores
/// at pairing. CoreBluetooth keeps it to itself; IOBluetooth tells.
///
/// IOBluetooth's first call waits on the main thread's run loop, and on the
/// Bluetooth permission while macOS asks for it. It runs once, on its own
/// thread; callers wait up to `wait` and get `None` if it has not answered.
pub fn bluetooth_address_within(wait: Duration) -> Option<u64> {
    static ANSWER: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    static ASK: std::sync::Once = std::sync::Once::new();
    ASK.call_once(|| {
        let _ = std::thread::Builder::new().name("wand-bt-address".into()).spawn(|| {
            let _ = ANSWER.set(read_bluetooth_address());
        });
    });
    let until = Instant::now() + wait;
    loop {
        if let Some(answer) = ANSWER.get() {
            return *answer;
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn read_bluetooth_address() -> Option<u64> {
    autoreleasepool(|_| unsafe {
        let controller: Option<Retained<AnyObject>> = msg_send![class(c"IOBluetoothHostController")?, defaultController];
        let text: Option<Retained<AnyObject>> = msg_send![&*controller?, addressAsString];
        let utf8: *const std::ffi::c_char = msg_send![&*text?, UTF8String];
        if utf8.is_null() {
            return None;
        }
        parse_address(&CStr::from_ptr(utf8).to_string_lossy())
    })
}

/// A request for a short connection interval, held for as long as this lives.
pub struct LowLatency {
    _manager: Retained<AnyObject>,
    _peripheral: Retained<AnyObject>,
}

// The two objects are only kept, never used, after the request is made.
unsafe impl Send for LowLatency {}

/// Ask macOS for a low-latency link to the peripheral CoreBluetooth knows as
/// `id`.
///
/// A Mac opens a link to the controller with 30 ms between connection events
/// and the controller never asks for less, so it sends half the reports it
/// sends over Windows' link. CoreBluetooth has no public way for a central to
/// ask for more: `-[CBCentralManager setDesiredConnectionLatency:forPeripheral:]`
/// is the private twin of the public call on `CBPeripheralManager`. btleplug
/// keeps its central manager to itself, so this makes one of its own; every
/// manager in the process shares the one link.
///
/// What it buys is up to the Bluetooth chip. bluetoothd turns "low" into an
/// interval of 10 to 30 ms (its log says so), and on the Mac this was tried
/// on (macOS 27, M-series) the chip kept 30 ms. Other Macs, with fewer links
/// to schedule, may pick less.
///
/// The manager reports on the main queue: the main thread must be running a
/// run loop, and this must not be called on it.
pub fn request_low_latency(id: &str, wait: Duration) -> Option<LowLatency> {
    let until = Instant::now() + wait;
    let poll = |done: &dyn Fn() -> bool| -> bool {
        while !done() {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    };
    let (manager, peripheral) = autoreleasepool(|_| unsafe {
        let set_latency = Sel::register(c"setDesiredConnectionLatency:forPeripheral:");
        let cls = class(c"CBCentralManager")?;
        if !cls.instance_method(set_latency).is_some() {
            log::info!("this macOS has no connection latency request");
            return None;
        }
        let nil: Option<&AnyObject> = None;
        let manager: Option<Retained<AnyObject>> =
            msg_send![msg_send![cls, alloc], initWithDelegate: nil, queue: nil, options: nil];
        let manager = manager?;
        let text: Retained<AnyObject> = msg_send![class(c"NSString")?, stringWithUTF8String: std::ffi::CString::new(id).ok()?.as_ptr()];
        let uuid: Option<Retained<AnyObject>> = msg_send![msg_send![class(c"NSUUID")?, alloc], initWithUUIDString: &*text];
        let ids: Retained<AnyObject> = msg_send![class(c"NSArray")?, arrayWithObject: &*uuid?];
        Some((manager, ids))
    })
    .map(|(manager, ids)| {
        // Powered on (5) before it will look anything up.
        let on = poll(&|| { let state: isize = unsafe { msg_send![&*manager, state] }; state == 5 });
        (manager, ids, on)
    })
    .and_then(|(manager, ids, on)| {
        if !on {
            log::info!("connection latency: Bluetooth did not come up in time");
            return None;
        }
        let peripheral = autoreleasepool(|_| unsafe {
            let found: Retained<AnyObject> = msg_send![&*manager, retrievePeripheralsWithIdentifiers: &*ids];
            let first: Option<Retained<AnyObject>> = msg_send![&*found, firstObject];
            first
        })?;
        Some((manager, peripheral))
    })?;
    // Its own claim on the link: connecting to a peripheral that is already
    // connected only adds this manager as a user of the link.
    let nil: Option<&AnyObject> = None;
    unsafe {
        let _: () = msg_send![&*manager, connectPeripheral: &*peripheral, options: nil];
    }
    // CBPeripheralStateConnected.
    if !poll(&|| { let state: isize = unsafe { msg_send![&*peripheral, state] }; state == 2 }) {
        log::info!("connection latency: could not share the link");
        return None;
    }
    unsafe {
        // CBPeripheralManagerConnectionLatencyLow.
        let _: () = msg_send![&*manager, setDesiredConnectionLatency: 0isize, forPeripheral: &*peripheral];
    }
    log::info!("asked macOS for a low-latency link");
    Some(LowLatency { _manager: manager, _peripheral: peripheral })
}

/// "48-f1-eb-85-11-5f" or "48:F1:EB:85:11:5F" to 0x48f1eb85115f.
pub fn parse_address(text: &str) -> Option<u64> {
    let hex: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return None;
    }
    u64::from_str_radix(&hex, 16).ok().filter(|&a| a != 0)
}

// -------------------------------------------------------------------- windows

/// Make an overlay window sit above everything, on every Space and over
/// full-screen apps, out of window cycling and app hiding, and cover `frame`
/// (points, top-left origin). Main thread only.
///
/// # Safety
/// `ns_window` must be a live `NSWindow`.
pub unsafe fn overlay_window(ns_window: *mut c_void, frame: CGRect) {
    let Some(window) = (ns_window as *mut AnyObject).as_ref() else { return };
    // NSPopUpMenuWindowLevel: over the menu bar, the Dock and menus.
    let _: () = msg_send![window, setLevel: 101isize];
    // CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenAuxiliary.
    let _: () = msg_send![window, setCollectionBehavior: (1usize << 0) | (1 << 4) | (1 << 6) | (1 << 8)];
    let _: () = msg_send![window, setCanHide: false];
    let _: () = msg_send![window, setHidesOnDeactivate: false];
    // AppKit counts y up from the bottom of the main display.
    let main_h = CGDisplayBounds(CGMainDisplayID()).size.height;
    let flipped = CGRect::new(
        CGPoint::new(frame.origin.x, main_h - frame.origin.y - frame.size.height),
        CGSize::new(frame.size.width, frame.size.height),
    );
    let _: () = msg_send![window, setFrame: flipped, display: true];
    let _: () = msg_send![window, orderFrontRegardless];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_address("48-f1-eb-85-11-5f"), Some(0x48f1_eb85_115f));
        assert_eq!(parse_address("48:F1:EB:85:11:5F"), Some(0x48f1_eb85_115f));
        assert_eq!(parse_address("00-00-00-00-00-00"), None);
        assert_eq!(parse_address("48-f1-eb"), None);
    }

    /// Debug builds check every message's argument and return types against
    /// the Objective-C runtime, so these catch a wrong signature without
    /// pressing anything.
    #[test]
    fn messages_match_their_signatures() {
        autoreleasepool(|_| {
            for down in [true, false] {
                let event = media_event(16, down).expect("a media key event");
                let cg: *mut CGEvent = unsafe { msg_send![&*event, CGEvent] };
                assert!(!cg.is_null());
            }
            assert!(prompt_options().is_some());
        });
        assert!(double_click_interval() > 0.0);
        assert!(bluetooth_authorization().is_some());
        let _ = key_down(0x3D);
        let _ = accessibility_trusted();
    }

    #[test]
    fn every_key_wand_parses_has_a_mac_key() {
        use crate::actions::parse_key;
        let names = [
            "ctrl", "rightctrl", "shift", "rightshift", "alt", "rightalt", "cmd", "enter", "esc", "space", "tab",
            "backspace", "delete", "insert", "home", "end", "pageup", "pagedown", "left", "up", "right", "down",
            "printscreen", "pause", "capslock", "scrolllock", "=", "-", ",", ".", "/", ";", "'", "`", "[", "]", "\\",
            "a", "z", "0", "9", "f1", "f12", "f20",
        ];
        for n in names {
            let key = parse_key(n).unwrap_or_else(|| panic!("{n} does not parse"));
            assert!(key_code(key.vk).is_some(), "{n} has no macOS key code");
        }
        for n in ["volumeup", "volumedown", "volumemute", "medianext", "mediaprev", "mediaplaypause"] {
            assert!(media_code(parse_key(n).unwrap().vk).is_some(), "{n}");
        }
        assert_eq!(key_code(parse_key("cmd").unwrap().vk), Some(0x37));
        assert_eq!(key_code(parse_key("z").unwrap().vk), Some(0x06));
        assert!(key_code(parse_key("f21").unwrap().vk).is_none());
    }
}
