//! The cursor: what the engine's `PointerEvent`s become.
//!
//! One thread owns the mouse. On Windows moves are relative, sent with
//! `SendInput`, so they pass through the same path as a real mouse and
//! Windows applies the user's pointer speed. On macOS they are Quartz events
//! at the new position. The thread sleeps on its queue; it only ticks while
//! there is movement left to hand out.

use crate::engine::{MouseButton, PointerEvent};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

pub enum Cmd {
    Event(PointerEvent),
    /// `mouse.smoothing_ms` changed.
    Smoothing(f32),
    /// Reports a second the controller is sending now.
    ReportRate(f32),
}

/// How long each report's movement is spread over: what the setting asks
/// for, stretched to the time between reports when they come further apart.
/// Off stays off.
fn glide_span(chosen_ms: f32, report_hz: f32) -> f32 {
    let between_ms = if report_hz > 0.0 { (1000.0 / report_hz).min(50.0) } else { 0.0 };
    if chosen_ms > 0.0 { chosen_ms.max(between_ms) } else { 0.0 }
}

/// Hands movement out evenly over time.
///
/// The controller reports its position about 68 times a second over the link
/// Windows gives it, and about 33 over a Mac's. Sent as it arrives, the
/// cursor would hop at that rate across a display drawing 120 or 240. Each
/// report's travel is instead paid out over the next `span_ms`, one
/// millisecond at a time, which at one report interval turns the hops into a
/// steady glide. Reports that arrive in a clump queue behind one another, up
/// to twice the span, so a clump is not a lurch.
#[derive(Clone, Copy, Debug)]
pub struct Spreader {
    span_ms: f32,
    pending: (f32, f32),
    time_left_ms: f32,
    /// Fraction of a pixel not yet sent.
    carry: (f32, f32),
}

impl Spreader {
    pub fn new(span_ms: f32) -> Spreader {
        Spreader { span_ms: span_ms.max(0.0), pending: (0.0, 0.0), time_left_ms: 0.0, carry: (0.0, 0.0) }
    }

    pub fn set_span(&mut self, span_ms: f32) {
        self.span_ms = span_ms.max(0.0);
        self.time_left_ms = self.time_left_ms.min(2.0 * self.span_ms);
    }

    pub fn push(&mut self, dx: f32, dy: f32) {
        self.pending.0 += dx;
        self.pending.1 += dy;
        self.time_left_ms = (self.time_left_ms + self.span_ms).min(2.0 * self.span_ms);
    }

    /// Nothing left to hand out.
    pub fn idle(&self) -> bool {
        self.pending == (0.0, 0.0)
    }

    /// Whole pixels to send for the `dt_ms` that just passed.
    pub fn step(&mut self, dt_ms: f32) -> (i32, i32) {
        if self.idle() {
            return (0, 0);
        }
        if self.time_left_ms <= dt_ms || self.time_left_ms <= 0.0 {
            return self.flush();
        }
        let share = dt_ms / self.time_left_ms;
        let take = (self.pending.0 * share, self.pending.1 * share);
        self.pending.0 -= take.0;
        self.pending.1 -= take.1;
        self.time_left_ms -= dt_ms;
        self.whole(take)
    }

    /// Everything still owed, now. A click must land where the hand put it.
    pub fn flush(&mut self) -> (i32, i32) {
        let take = std::mem::take(&mut self.pending);
        self.time_left_ms = 0.0;
        self.whole(take)
    }

    fn whole(&mut self, add: (f32, f32)) -> (i32, i32) {
        self.carry.0 += add.0;
        self.carry.1 += add.1;
        let out = (self.carry.0.trunc(), self.carry.1.trunc());
        self.carry.0 -= out.0;
        self.carry.1 -= out.1;
        (out.0 as i32, out.1 as i32)
    }
}

/// Wheel units per notch, as Windows counts them.
const WHEEL_DELTA: f32 = 120.0;

/// Start the cursor thread. `live` is asked before anything is sent: while it
/// says no (a dry run, a demo replay) moves, presses and scrolls are dropped.
/// A release always goes through if this thread did the press, so a button
/// can never be left held. Dropping the sender ends the thread.
pub fn spawn(smoothing_ms: f32, live: impl Fn() -> bool + Send + 'static) -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    std::thread::Builder::new()
        .name("wand-pointer".into())
        .spawn(move || {
            let (mut chosen_ms, mut report_hz) = (smoothing_ms, 0.0f32);
            let mut spread = Spreader::new(glide_span(chosen_ms, report_hz));
            let mut held = [false; 3];
            let mut wheel = 0.0f32;
            let mut last = Instant::now();
            let mut fine = false;
            let slot = |b: MouseButton| match b {
                MouseButton::Left => 0,
                MouseButton::Right => 1,
                MouseButton::Middle => 2,
            };
            loop {
                let first = if spread.idle() {
                    rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                } else {
                    rx.recv_timeout(Duration::from_millis(1))
                };
                let mut next = match first {
                    Ok(cmd) => Some(cmd),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                while let Some(cmd) = next {
                    match cmd {
                        Cmd::Smoothing(ms) => {
                            chosen_ms = ms;
                            spread.set_span(glide_span(chosen_ms, report_hz));
                        }
                        Cmd::ReportRate(hz) => {
                            report_hz = hz;
                            spread.set_span(glide_span(chosen_ms, report_hz));
                        }
                        Cmd::Event(PointerEvent::Move { dx, dy }) => {
                            if live() {
                                if spread.idle() {
                                    // Time spent asleep is not time to pay out.
                                    last = Instant::now();
                                }
                                spread.push(dx, dy);
                            }
                        }
                        Cmd::Event(PointerEvent::Button { button, down }) => {
                            let i = slot(button);
                            if down != held[i] && (!down || live()) {
                                let (dx, dy) = spread.flush();
                                platform::move_by(dx, dy);
                                platform::button(button, down);
                                held[i] = down;
                            }
                        }
                        Cmd::Event(PointerEvent::Scroll { notches }) => {
                            if live() {
                                wheel += notches * WHEEL_DELTA;
                                let units = wheel.trunc();
                                wheel -= units;
                                platform::wheel(units as i32);
                            }
                        }
                    }
                    next = rx.try_recv().ok();
                }
                let now = Instant::now();
                let (dx, dy) = spread.step(now.duration_since(last).as_secs_f32() * 1000.0);
                last = now;
                platform::move_by(dx, dy);
                // One-millisecond sleeps only while the cursor is gliding.
                if fine == spread.idle() {
                    fine = !fine;
                    platform::fine_timer(fine);
                }
            }
            for (i, button) in [MouseButton::Left, MouseButton::Right, MouseButton::Middle].into_iter().enumerate() {
                if held[i] {
                    platform::button(button, false);
                }
            }
            if fine {
                platform::fine_timer(false);
            }
        })
        .expect("spawn pointer thread");
    tx
}

/// Move the cursor by `dx`, `dy` and report where it was and where it went.
/// For `wandtool pointer-check`.
pub fn nudge(dx: i32, dy: i32) -> Option<((i32, i32), (i32, i32))> {
    let before = platform::cursor()?;
    platform::move_by(dx, dy);
    std::thread::sleep(Duration::from_millis(30));
    Some((before, platform::cursor()?))
}

#[cfg(windows)]
mod platform {
    use crate::engine::MouseButton;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
        MOUSEINPUT, MOUSE_EVENT_FLAGS,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    fn send(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        // A refusal (a window running as administrator is in front) is not
        // worth a log line per report; the cursor simply does not move.
        unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    }

    pub fn move_by(dx: i32, dy: i32) {
        if dx != 0 || dy != 0 {
            send(dx, dy, 0, MOUSEEVENTF_MOVE);
        }
    }

    pub fn button(button: MouseButton, down: bool) {
        let flags = match (button, down) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        send(0, 0, 0, flags);
    }

    pub fn wheel(units: i32) {
        if units != 0 {
            send(0, 0, units, MOUSEEVENTF_WHEEL);
        }
    }

    pub fn cursor() -> Option<(i32, i32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
    }

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

/// Movement is in physical pixels, as on Windows; macOS places the cursor in
/// points, so a move is divided by the scale of the display it starts on.
/// Quartz applies no acceleration to these events.
#[cfg(target_os = "macos")]
mod platform {
    use crate::engine::MouseButton;
    use crate::macos::{self, contains};
    use objc2_core_foundation::CGPoint;
    use objc2_core_graphics::{CGEventField, CGEventType, CGMouseButton};
    use parking_lot::Mutex;
    use std::time::Instant;

    /// Pixels of scrolling per wheel notch (120 units). About three lines.
    const PIXELS_PER_NOTCH: f64 = 60.0;

    struct State {
        held: [bool; 3],
        /// Where this thread last put the cursor. macOS may round what it
        /// reports back; continuing from here keeps sub-point movement.
        last: Option<CGPoint>,
        last_down: Option<(MouseButton, Instant, CGPoint)>,
        clicks: i64,
        wheel_carry: f64,
    }

    static STATE: Mutex<State> =
        Mutex::new(State { held: [false; 3], last: None, last_down: None, clicks: 1, wheel_carry: 0.0 });

    fn slot(b: MouseButton) -> usize {
        match b {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
        }
    }

    fn here(s: &mut State) -> Option<CGPoint> {
        let read = macos::cursor_points()?;
        Some(match s.last {
            Some(l) if (l.x - read.x).abs() <= 1.0 && (l.y - read.y).abs() <= 1.0 => l,
            _ => read,
        })
    }

    pub fn move_by(dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        let mut s = STATE.lock();
        let Some(from) = here(&mut s) else { return };
        let displays = macos::displays();
        let on = displays.iter().find(|d| contains(&d.bounds, from)).or(displays.first());
        let scale = on.map_or(1.0, |d| d.scale);
        let (mx, my) = (dx as f64 / scale, dy as f64 / scale);
        let mut to = CGPoint::new(from.x + mx, from.y + my);
        // Off every display: stop at the edge of the one the cursor is on.
        if let (false, Some(d)) = (displays.iter().any(|d| contains(&d.bounds, to)), on) {
            let r = d.bounds;
            to.x = to.x.clamp(r.origin.x, r.origin.x + r.size.width - 0.5);
            to.y = to.y.clamp(r.origin.y, r.origin.y + r.size.height - 0.5);
        }
        s.last = Some(to);
        // With a button held, apps expect drags, not moves.
        let (kind, button) = match s.held {
            [true, _, _] => (CGEventType::LeftMouseDragged, CGMouseButton::Left),
            [_, true, _] => (CGEventType::RightMouseDragged, CGMouseButton::Right),
            [_, _, true] => (CGEventType::OtherMouseDragged, CGMouseButton::Center),
            _ => (CGEventType::MouseMoved, CGMouseButton::Left),
        };
        drop(s);
        if let Some(event) = macos::mouse_event(kind, to, button) {
            macos::set_field(&event, CGEventField::MouseEventDeltaX, mx.round() as i64);
            macos::set_field(&event, CGEventField::MouseEventDeltaY, my.round() as i64);
            macos::post_event(&event);
        }
    }

    /// Apps count double clicks from the click count on the event, which a
    /// real mouse gets from the system. Synthetic clicks have to carry it.
    pub fn button(button: MouseButton, down: bool) {
        let mut s = STATE.lock();
        let Some(at) = here(&mut s) else { return };
        if down {
            let now = Instant::now();
            let again = matches!(s.last_down, Some((b, t, p)) if b == button
                && now.duration_since(t).as_secs_f64() <= macos::double_click_interval()
                && (p.x - at.x).abs() <= 4.0 && (p.y - at.y).abs() <= 4.0);
            s.clicks = if again { s.clicks + 1 } else { 1 };
            s.last_down = Some((button, now, at));
        }
        s.held[slot(button)] = down;
        let clicks = s.clicks;
        drop(s);
        let (kind, cg) = match (button, down) {
            (MouseButton::Left, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
            (MouseButton::Left, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
            (MouseButton::Right, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
            (MouseButton::Right, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
            (MouseButton::Middle, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
            (MouseButton::Middle, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
        };
        if let Some(event) = macos::mouse_event(kind, at, cg) {
            macos::set_field(&event, CGEventField::MouseEventClickState, clicks);
            if button == MouseButton::Middle {
                macos::set_field(&event, CGEventField::MouseEventButtonNumber, 2);
            }
            macos::post_event(&event);
        }
    }

    pub fn wheel(units: i32) {
        let mut s = STATE.lock();
        s.wheel_carry += units as f64 / 120.0 * PIXELS_PER_NOTCH;
        let px = s.wheel_carry.trunc();
        s.wheel_carry -= px;
        drop(s);
        if px != 0.0 {
            if let Some(event) = macos::scroll_event(px as i32) {
                macos::post_event(&event);
            }
        }
    }

    /// In physical pixels of the display the cursor is on.
    pub fn cursor() -> Option<(i32, i32)> {
        let p = macos::cursor_points()?;
        let scale = macos::displays().iter().find(|d| contains(&d.bounds, p)).map_or(1.0, |d| d.scale);
        Some(((p.x * scale).round() as i32, (p.y * scale).round() as i32))
    }

    /// macOS sleeps to the millisecond already.
    pub fn fine_timer(_: bool) {}
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use crate::engine::MouseButton;
    pub fn move_by(_: i32, _: i32) {}
    pub fn button(_: MouseButton, _: bool) {}
    pub fn wheel(_: i32) {}
    pub fn cursor() -> Option<(i32, i32)> {
        None
    }
    pub fn fine_timer(_: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the spreader the way the thread does: reports every `every_ms`,
    /// a step each millisecond. Returns what was sent in each millisecond.
    fn run(span_ms: f32, every_ms: usize, reports: &[(f32, f32)], extra_ms: usize) -> Vec<(i32, i32)> {
        let mut s = Spreader::new(span_ms);
        let mut out = Vec::new();
        for ms in 0..reports.len() * every_ms + extra_ms {
            if ms % every_ms == 0 {
                if let Some(r) = reports.get(ms / every_ms) {
                    s.push(r.0, r.1);
                }
            }
            out.push(s.step(1.0));
        }
        assert!(s.idle(), "everything is paid out in the end");
        out
    }

    #[test]
    fn every_pixel_arrives_and_none_twice() {
        let reports: Vec<(f32, f32)> = (0..40).map(|i| (7.3 + (i % 5) as f32, -3.1 - (i % 3) as f32 * 0.7)).collect();
        let want = reports.iter().fold((0.0, 0.0), |a, r| (a.0 + r.0, a.1 + r.1));
        let sent = run(15.0, 15, &reports, 40).iter().fold((0, 0), |a, s| (a.0 + s.0, a.1 + s.1));
        assert!((sent.0 as f32 - want.0).abs() < 1.0 && (sent.1 as f32 - want.1).abs() < 1.0, "{sent:?} vs {want:?}");
    }

    #[test]
    fn steady_reports_become_steady_motion() {
        // 30 px every 15 ms: 2 px in every millisecond, not 30 px in one.
        let out = run(15.0, 15, &[(30.0, 0.0); 20], 30);
        let moving = &out[1..20 * 15];
        assert!(moving.iter().all(|s| (1..=3).contains(&s.0)), "{moving:?}");
        // With smoothing off the same reports are 20 hops.
        let raw = run(0.0, 15, &[(30.0, 0.0); 20], 30);
        assert_eq!(raw.iter().filter(|s| s.0 != 0).count(), 20);
        assert!(raw.iter().all(|s| s.0 == 0 || s.0 == 30));
    }

    #[test]
    fn a_clump_of_reports_is_not_a_lurch() {
        // Two reports in the same millisecond, then the gap they left.
        let mut s = Spreader::new(15.0);
        s.push(30.0, 0.0);
        s.push(30.0, 0.0);
        let sent: Vec<i32> = (0..30).map(|_| s.step(1.0).0).collect();
        assert!(sent.iter().all(|&x| (1..=3).contains(&x)), "{sent:?}");
        assert_eq!(sent.iter().sum::<i32>(), 60);
        // A flood is bounded at two spans of delay however much arrives.
        let mut s = Spreader::new(15.0);
        for _ in 0..10 {
            s.push(30.0, 0.0);
        }
        let sent: Vec<i32> = (0..30).map(|_| s.step(1.0).0).collect();
        assert_eq!(sent.iter().sum::<i32>(), 300);
    }

    #[test]
    fn slow_movement_keeps_its_fractions() {
        // 0.4 px per report is under a pixel every time and must still add up.
        let out = run(15.0, 15, &[(0.4, -0.4); 50], 30);
        let sum = out.iter().fold((0, 0), |a, s| (a.0 + s.0, a.1 + s.1));
        assert!((19..=20).contains(&sum.0) && (-20..=-19).contains(&sum.1), "{sum:?}");
        assert!(out.iter().all(|s| s.0.abs() <= 1 && s.1.abs() <= 1));
    }

    #[test]
    fn glide_lasts_until_the_next_report() {
        // Windows' link: the setting's 15 ms already covers a report interval.
        assert_eq!(glide_span(15.0, 68.0), 15.0);
        // A Mac's link sends half as often; gliding for 15 ms of every 30
        // would stutter.
        assert!((glide_span(15.0, 33.3) - 30.0).abs() < 0.1);
        assert_eq!(glide_span(0.0, 33.3), 0.0, "off stays off");
        assert_eq!(glide_span(15.0, 0.0), 15.0, "no controller yet");
        assert_eq!(glide_span(15.0, 5.0), 50.0, "a crawling link is not waited for");
    }

    #[test]
    fn a_click_flushes_what_is_owed() {
        let mut s = Spreader::new(15.0);
        s.push(40.0, 10.0);
        let a = s.step(3.0);
        let b = s.flush();
        assert_eq!((a.0 + b.0, a.1 + b.1), (40, 10));
        assert!(s.idle());
        assert_eq!(s.step(5.0), (0, 0));
    }
}
