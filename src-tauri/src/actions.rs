//! What a recognised gesture does: press a key combination, press a media
//! key, open something, or run a command line.

use crate::gestures::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub vk: u16,
    /// Navigation, media and Windows keys need the extended-key flag.
    pub extended: bool,
}

const fn k(vk: u16) -> Key {
    Key { vk, extended: false }
}
const fn e(vk: u16) -> Key {
    Key { vk, extended: true }
}

/// Key names accepted in combos, lower-case. Includes the names browsers use
/// in `KeyboardEvent.code`, so what the settings window records parses here.
fn named_key(name: &str) -> Option<Key> {
    Some(match name {
        "ctrl" | "control" | "leftctrl" | "controlleft" => k(0x11),
        "rightctrl" | "controlright" => e(0xA3),
        "shift" | "leftshift" | "shiftleft" => k(0x10),
        "rightshift" | "shiftright" => k(0xA1),
        "alt" | "leftalt" | "altleft" => k(0x12),
        "rightalt" | "altright" | "altgr" => e(0xA5),
        "win" | "super" | "meta" | "cmd" | "windows" | "metaleft" | "metaright" => e(0x5B),
        "enter" | "return" => k(0x0D),
        "esc" | "escape" => k(0x1B),
        "space" | "spacebar" => k(0x20),
        "tab" => k(0x09),
        "backspace" => k(0x08),
        "delete" | "del" => e(0x2E),
        "insert" | "ins" => e(0x2D),
        "home" => e(0x24),
        "end" => e(0x23),
        "pageup" | "pgup" => e(0x21),
        "pagedown" | "pgdn" => e(0x22),
        "left" | "arrowleft" => e(0x25),
        "up" | "arrowup" => e(0x26),
        "right" | "arrowright" => e(0x27),
        "down" | "arrowdown" => e(0x28),
        "printscreen" | "prtsc" | "prtscn" | "snapshot" => e(0x2C),
        "pause" => k(0x13),
        "capslock" => k(0x14),
        "scrolllock" => k(0x91),
        "plus" | "equal" | "=" => k(0xBB),
        "minus" | "-" => k(0xBD),
        "comma" | "," => k(0xBC),
        "period" | "." => k(0xBE),
        "slash" | "/" => k(0xBF),
        "semicolon" | ";" => k(0xBA),
        "quote" | "'" => k(0xDE),
        "backquote" | "`" => k(0xC0),
        "bracketleft" | "[" => k(0xDB),
        "bracketright" | "]" => k(0xDD),
        "backslash" | "\\" => k(0xDC),
        "volumeup" => e(0xAF),
        "volumedown" => e(0xAE),
        "volumemute" => e(0xAD),
        "medianext" => e(0xB0),
        "mediaprev" => e(0xB1),
        "mediastop" => e(0xB2),
        "mediaplaypause" => e(0xB3),
        _ => return None,
    })
}

pub fn parse_key(name: &str) -> Option<Key> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() {
        return None;
    }
    if let Some(key) = named_key(&n) {
        return Some(key);
    }
    let bytes = n.as_bytes();
    if bytes.len() == 1 {
        return match bytes[0] {
            c @ b'a'..=b'z' => Some(k((c - b'a') as u16 + 0x41)),
            c @ b'0'..=b'9' => Some(k((c - b'0') as u16 + 0x30)),
            _ => None,
        };
    }
    // KeyboardEvent.code forms.
    if let Some(letter) = n.strip_prefix("key") {
        return (letter.len() == 1).then(|| parse_key(letter)).flatten();
    }
    if let Some(digit) = n.strip_prefix("digit") {
        return (digit.len() == 1).then(|| parse_key(digit)).flatten();
    }
    if let Some(num) = n.strip_prefix('f') {
        if let Ok(i) = num.parse::<u16>() {
            if (1..=24).contains(&i) {
                return Some(k(0x70 + i - 1));
            }
        }
    }
    None
}

/// "Ctrl+Shift+S" to the keys to press, in order.
pub fn parse_combo(combo: &str) -> Result<Vec<Key>, String> {
    let combo = combo.trim();
    if combo.is_empty() {
        return Err("The key combination is empty.".into());
    }
    let mut keys = Vec::new();
    // "Ctrl++" means Ctrl and the plus key.
    let normalized = combo.replace("++", "+plus");
    for part in normalized.split('+') {
        let key = parse_key(part).ok_or_else(|| format!("\"{}\" is not a key Wand knows.", part.trim()))?;
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.len() > 6 {
        return Err("A combination can hold at most six keys.".into());
    }
    Ok(keys)
}

pub fn media_key(name: &str) -> Option<Key> {
    Some(match name {
        "volume_up" => e(0xAF),
        "volume_down" => e(0xAE),
        "mute" => e(0xAD),
        "next_track" => e(0xB0),
        "prev_track" => e(0xB1),
        "stop" => e(0xB2),
        "play_pause" => e(0xB3),
        _ => return None,
    })
}

/// Check an action before it is saved, so mistakes surface in the editor and
/// not at the moment a gesture is drawn.
pub fn validate(action: &Action) -> Result<(), String> {
    match action {
        Action::KeyCombo { keys } => parse_combo(keys).map(|_| ()),
        Action::MediaKey { key, repeat } => {
            media_key(key).ok_or_else(|| format!("\"{key}\" is not a media key."))?;
            if !(1..=20).contains(repeat) {
                return Err("Repeat must be between 1 and 20.".into());
            }
            Ok(())
        }
        Action::Launch { path, .. } if path.trim().is_empty() => Err("Choose something to open.".into()),
        Action::Shell { command } if command.trim().is_empty() => Err("The command is empty.".into()),
        _ => Ok(()),
    }
}

/// Alt+F4 and nothing else, whichever Alt it is.
fn is_close_window(combo: &[Key]) -> bool {
    const F4: u16 = 0x73;
    let alt = |k: &Key| matches!(k.vk, 0x12 | 0xA4 | 0xA5);
    combo.len() == 2 && combo.iter().any(alt) && combo.iter().any(|k| k.vk == F4)
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub dry_run: bool,
    pub shell: String,
}

/// Run the action. Returns a short description of what was done.
pub fn run(action: &Action, opts: &RunOptions) -> Result<String, String> {
    validate(action)?;
    let what = match action {
        Action::KeyCombo { keys } => format!("keys {keys}"),
        Action::MediaKey { key, repeat } => format!("media {key} x{repeat}"),
        Action::Launch { path, args } => format!("open {path} {args}").trim().to_string(),
        Action::Shell { command } => format!("{} {command}", opts.shell),
        Action::None => "nothing".to_string(),
    };
    if opts.dry_run {
        return Ok(format!("dry run: {what}"));
    }
    match action {
        Action::KeyCombo { keys } => {
            let combo = parse_combo(keys)?;
            // Alt+F4 with only the desktop or the taskbar in front does not
            // close a window: Windows answers it with the Shut Down dialog.
            if is_close_window(&combo) && platform::shell_is_in_front() {
                return Ok(format!("skipped {what}: no window is in front"));
            }
            platform::send_keys(&combo)?
        }
        Action::MediaKey { key, repeat } => {
            let key = media_key(key).unwrap();
            for i in 0..*repeat {
                if i > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(12));
                }
                platform::send_keys(&[key])?;
            }
        }
        Action::Launch { path, args } => platform::open(path, args)?,
        Action::Shell { command } => platform::shell(&opts.shell, command)?,
        Action::None => {}
    }
    Ok(what)
}

#[cfg(windows)]
mod platform {
    use super::Key;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, SW_SHOWNORMAL};

    /// True when the foreground window is the desktop or a taskbar, not a
    /// program's window.
    pub fn shell_is_in_front() -> bool {
        let mut name = [0u16; 64];
        let len = unsafe { GetClassNameW(GetForegroundWindow(), &mut name) };
        let class = String::from_utf16_lossy(&name[..len.max(0) as usize]);
        matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    }

    fn input(key: Key, up: bool) -> INPUT {
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if key.extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        // The scan code travels with the virtual key. Programs that read the
        // physical key (Chromium's KeyboardEvent.code, many games) see
        // nothing at all when it is left at zero.
        let scan = unsafe { MapVirtualKeyW(key.vk as u32, MAPVK_VK_TO_VSC) } as u16;
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: VIRTUAL_KEY(key.vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    /// All keys down in order, then up in reverse, as one atomic batch.
    pub fn send_keys(keys: &[Key]) -> Result<(), String> {
        let mut inputs: Vec<INPUT> = keys.iter().map(|k| input(*k, false)).collect();
        inputs.extend(keys.iter().rev().map(|k| input(*k, true)));
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            return Err("Windows refused the key press. A window running as administrator may be in front.".into());
        }
        Ok(())
    }

    pub fn open(path: &str, args: &str) -> Result<(), String> {
        let (path, args) = (HSTRING::from(path.trim()), HSTRING::from(args.trim()));
        let args_ptr = if args.is_empty() { PCWSTR::null() } else { PCWSTR(args.as_ptr()) };
        let result = unsafe {
            ShellExecuteW(None, &HSTRING::from("open"), &path, args_ptr, PCWSTR::null(), SW_SHOWNORMAL)
        };
        // ShellExecute reports success as a value above 32.
        if result.0 as usize <= 32 {
            return Err(format!("Windows could not open {path}."));
        }
        Ok(())
    }

    pub fn shell(shell: &str, command: &str) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = if shell.eq_ignore_ascii_case("powershell") {
            let mut c = std::process::Command::new("powershell.exe");
            c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
            c
        } else {
            let mut c = std::process::Command::new("cmd.exe");
            c.arg("/C").raw_arg(command);
            c
        };
        cmd.creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not start the command: {e}"))
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Key;
    pub fn shell_is_in_front() -> bool {
        false
    }

    pub fn send_keys(_: &[Key]) -> Result<(), String> {
        Err("Key presses are only implemented on Windows.".into())
    }
    pub fn open(_: &str, _: &str) -> Result<(), String> {
        Err("Opening is only implemented on Windows.".into())
    }
    pub fn shell(_: &str, _: &str) -> Result<(), String> {
        Err("Shell commands are only implemented on Windows.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gestures;

    fn vks(combo: &str) -> Vec<u16> {
        parse_combo(combo).unwrap().iter().map(|k| k.vk).collect()
    }

    #[test]
    fn combos() {
        assert_eq!(vks("Ctrl+Z"), [0x11, 0x5A]);
        assert_eq!(vks("alt + f4"), [0x12, 0x73]);
        assert_eq!(vks("Win+PrintScreen"), [0x5B, 0x2C]);
        assert_eq!(vks("Ctrl+Shift+Esc"), [0x11, 0x10, 0x1B]);
        assert_eq!(vks("Ctrl++"), [0x11, 0xBB]);
        assert_eq!(vks("Ctrl+Ctrl+A"), [0x11, 0x41]);
        // What the browser's KeyboardEvent.code yields.
        assert_eq!(vks("ControlLeft+KeyK"), [0x11, 0x4B]);
        assert_eq!(vks("Shift+Digit7"), [0x10, 0x37]);
        assert_eq!(vks("ArrowUp"), [0x26]);
        assert_eq!(vks("F24"), [0x87]);
        assert!(parse_combo("Ctrl+Hyper").is_err());
        assert!(parse_combo("").is_err());
        assert!(parse_combo("F25").is_err());
        assert!(parse_combo("Win+PrintScreen").unwrap()[1].extended);
    }

    #[test]
    fn every_default_binding_is_valid() {
        for g in gestures::defaults().gestures {
            validate(&g.action).unwrap_or_else(|e| panic!("{}: {e}", g.name));
        }
    }

    #[test]
    fn validation_messages() {
        assert!(validate(&Action::MediaKey { key: "volume_up".into(), repeat: 0 }).is_err());
        assert!(validate(&Action::MediaKey { key: "boom".into(), repeat: 1 }).is_err());
        assert!(validate(&Action::Launch { path: "  ".into(), args: String::new() }).is_err());
        assert!(validate(&Action::Shell { command: String::new() }).is_err());
        assert!(validate(&Action::None).is_ok());
    }

    #[test]
    fn only_plain_alt_f4_counts_as_close_window() {
        let combo = |s: &str| parse_combo(s).unwrap();
        assert!(is_close_window(&combo("Alt+F4")));
        assert!(is_close_window(&combo("RightAlt+F4")));
        assert!(!is_close_window(&combo("Ctrl+Alt+F4")));
        assert!(!is_close_window(&combo("Alt+F5")));
        assert!(!is_close_window(&combo("Ctrl+W")));
    }

    #[test]
    fn dry_run_touches_nothing() {
        let opts = RunOptions { dry_run: true, shell: "cmd".into() };
        assert_eq!(run(&Action::KeyCombo { keys: "Alt+F4".into() }, &opts).unwrap(), "dry run: keys Alt+F4");
        assert_eq!(
            run(&Action::Shell { command: "shutdown /s".into() }, &opts).unwrap(),
            "dry run: cmd shutdown /s"
        );
        assert!(run(&Action::KeyCombo { keys: "Ctrl+Nope".into() }, &opts).is_err());
    }
}
