//! Dev mode: hold a key and move the mouse to draw.
//!
//! Polls the key and the cursor from a thread of its own, at 1 kHz while the
//! key is down and 250 Hz otherwise, and feeds the same engine entry point the
//! projected wand position goes through. Nothing is hooked or intercepted:
//! the key still reaches whatever has focus, so the default is a key that does
//! nothing on its own (Right Ctrl).

use crate::actions::parse_key;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct DevMouse {
    pub enabled: AtomicBool,
    pub vk: AtomicU16,
    stop: AtomicBool,
}

impl DevMouse {
    pub fn new() -> Arc<DevMouse> {
        Arc::new(DevMouse { enabled: AtomicBool::new(false), vk: AtomicU16::new(0xA3), stop: AtomicBool::new(false) })
    }

    pub fn configure(&self, enabled: bool, hold_key: &str) -> Result<(), String> {
        let key = parse_key(hold_key).ok_or_else(|| format!("dev.hold_key: \"{hold_key}\" is not a key Wand knows."));
        if let Ok(k) = &key {
            self.vk.store(k.vk, Ordering::Relaxed);
        }
        self.enabled.store(enabled, Ordering::Relaxed);
        key.map(|_| ())
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Run until `stop()`. `sink(seconds, x, y, down)` gets physical pixels of
    /// the virtual desktop.
    pub fn run(self: Arc<Self>, t0: Instant, mut sink: impl FnMut(f64, f32, f32, bool)) {
        let mut was_down = false;
        while !self.stop.load(Ordering::Relaxed) {
            if !self.enabled.load(Ordering::Relaxed) {
                if was_down {
                    if let Some((x, y)) = platform::cursor() {
                        sink(t0.elapsed().as_secs_f64(), x, y, false);
                    }
                    was_down = false;
                    platform::fine_timer(false);
                }
                std::thread::sleep(Duration::from_millis(120));
                continue;
            }
            let down = platform::key_down(self.vk.load(Ordering::Relaxed));
            if down != was_down {
                platform::fine_timer(down);
            }
            if down || was_down {
                if let Some((x, y)) = platform::cursor() {
                    sink(t0.elapsed().as_secs_f64(), x, y, down);
                }
            }
            was_down = down;
            std::thread::sleep(Duration::from_millis(if down { 1 } else { 4 }));
        }
        if was_down {
            platform::fine_timer(false);
        }
    }
}

pub fn cursor() -> Option<(f32, f32)> {
    platform::cursor()
}

#[cfg(windows)]
mod platform {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    pub fn key_down(vk: u16) -> bool {
        (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
    }

    pub fn cursor() -> Option<(f32, f32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x as f32, p.y as f32))
    }

    /// 1 ms sleeps while a stroke is being drawn; the default 15.6 ms timer
    /// otherwise, so an idle app costs nothing.
    pub fn fine_timer(on: bool) {
        unsafe {
            if on {
                timeBeginPeriod(1);
            } else {
                timeEndPeriod(1);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use crate::macos;

    pub fn key_down(vk: u16) -> bool {
        macos::key_code(vk).is_some_and(macos::key_down)
    }

    pub fn cursor() -> Option<(f32, f32)> {
        macos::cursor_units()
    }

    pub fn fine_timer(_: bool) {}
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub fn key_down(_: u16) -> bool {
        false
    }
    pub fn cursor() -> Option<(f32, f32)> {
        None
    }
    pub fn fine_timer(_: bool) {}
}
