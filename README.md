# Wand

Wand is an app for Windows and macOS that turns a Nintendo Switch 2 Joy-Con into a wand. Hold ZR, draw a shape in the air, let go, and a key combination, media key, program or command runs. A transparent overlay draws the stroke as you make it and shows which gesture it was taken for. Laid on its side on a desk, the same controller is a mouse.

Tauri v2. Rust does Bluetooth, sensor fusion, recognition and actions. TypeScript and WebGL2 draw the overlay and the settings window.

![A matched circle: the clean shape, the glint running along it, and the action's label](docs/overlay-match.png)

## Status

Tested with one right-hand Joy-Con 2 on Windows 11 and a built-in Bluetooth adapter:

- Pairing from sync mode, and reconnecting on a button press after a restart.
- Buttons, gyro, accelerometer and magnetometer arriving at about 68 reports a second (about 33 on a Mac).
- The overlay holding 240 fps on a 240 Hz display (median frame 4.2 ms).
- Recognition of the first eight gestures. In one session of 321 strokes, 283 of the 315 long enough to count matched. The misses include scribbles and shapes that are not in the library.
- The mouse on a desk. The controller sends optical data alongside the magnetometer, and the cursor and the clicks work. Lying on that desk the sensor read a distance of about 150 and a roughness between 2,100 and 3,900, with the rail 6 to 14 degrees off vertical. Lifted, it read 3,081 and 4,607.

On macOS, Wand builds, passes its tests, starts, and recognises a replayed recording. It has not been tried with a controller, or watched on screen: the overlay, the menu bar item, pairing, reconnecting, the report rate macOS gives the link, key presses and the mouse are all untested there.

Not tested: the left Joy-Con and the Pro Controller 2, how the rumble feels, magnetometer calibration on hardware, battery percentage against the console, and "Start with Windows". The mouse has not been tried on cloth. The roughness limit is 4,000 (`mouse.max_roughness`), which the desk already came close to, so a rougher surface may need it raised. The six gestures in the lower half of the table pass with generated sensor data and take nothing from 370 logged real strokes, but nobody has drawn them with a controller yet.

## Build and run

On Windows you need Rust (stable, MSVC), Node 20 or later with pnpm, and the Visual Studio C++ build tools. On macOS (11 or later) you need Rust, Node 20 or later with pnpm, and the Xcode command line tools.

```powershell
pnpm install
```

```powershell
pnpm app
```

`pnpm bundle` builds an installer into `src-tauri\target\release\bundle\nsis\` on Windows, and `Wand.app` and a disk image into `src-tauri/target/release/bundle/` on macOS. For one Mac build that runs on both Apple silicon and Intel, `rustup target add x86_64-apple-darwin` once and then `pnpm tauri build --target universal-apple-darwin`. Tests: `cargo test --manifest-path src-tauri/Cargo.toml`.

## Pairing

Joy-Con 2 use Bluetooth LE with Nintendo's own pairing, which Wand speaks itself. Do not pair the controller in Windows settings: Windows cannot do it, and the attempt makes the controller hang up. If it is listed there, remove it. On a Mac, do not pair it in System Settings either, and allow Wand to use Bluetooth when macOS asks.

1. Open Wand and go to **Device**.
2. Hold the small round sync button on the Joy-Con's rail until the lights chase.
3. Wait a few seconds for it to connect.

After that, pressing any button reconnects it. A Joy-Con paired here stops reconnecting to the Switch until you pair it with the console again.

## Gestures

Hold ZR (ZL on a left Joy-Con), draw, release. The action runs at once; the animation follows.

| Gesture | How to draw it | Windows | macOS |
| --- | --- | --- | --- |
| Circle | either way round, from any point | Screenshot (Win+PrintScreen) | Screenshot (Cmd+Shift+3) |
| Slash | a diagonal, down to the left or up to the right | Close window (Alt+F4) | Close window (Cmd+W) |
| Z | as written | Undo (Ctrl+Z) | Undo (Cmd+Z) |
| V | as written, or as a tick | Paste (Ctrl+V) | Paste (Cmd+V) |
| Flick up / down | a straight line | Volume up / down | Volume up / down |
| Flick left / right | a straight line | Previous / next track | Previous / next track |
| Chevron | `>`, top to bottom | Play / pause | Play / pause |
| Caret | `^`, left to right | Copy (Ctrl+C) | Copy (Cmd+C) |
| Infinity | from the crossing, either way | Switch window (Alt+Tab) | Switch app (Cmd+Tab) |
| Star | five points in one stroke, either way round | Task view (Win+Tab) | Mission Control (Ctrl+Up) |
| M | as written | Mute | Mute |
| S | from the top right | Save (Ctrl+S) | Save (Cmd+S) |

Wand strokes shorter than 10 degrees of travel or slower than 8 degrees a second are ignored, so pressing the trigger does not fire anything. Alt+F4 is not sent while only the desktop or the taskbar is in front, where Windows would answer it with the Shut Down dialog.

To add your own, press **Record a gesture**, draw the shape three times and bind it to a key combination, media key, program or command. Click any card to rebind it, turn it off or draw it again in your own hand. The library is `gestures.json` in the data folder: `%APPDATA%\app.wand.desktop\` on Windows, `~/Library/Application Support/app.wand.desktop/` on macOS.

## Mouse

A Joy-Con 2 has an optical sensor in its rail. Lay the controller on its side with the rail down and it moves the cursor.

| On a surface | Does |
| --- | --- |
| R (L on a left Joy-Con) | left click |
| ZR (ZL) | right click |
| Stick click | middle click |
| Stick forward / back | scroll |

Lift it and the same buttons are the wand's again. What a press means is decided when the button goes down and holds until it comes up: a drag survives lifting the controller to reposition it, and a stroke already being drawn is not cut short by setting the controller down.

Wand takes the controller to be on a surface from three readings, all shown live on the Device page: the sensor's distance and roughness values, and whether the rail faces down. The third is there because fingers wrapped around the rail look like a surface to the sensor. The limits are `mouse.max_distance`, `mouse.max_roughness` and `mouse.max_tilt_deg`.

The controller reports about 68 times a second, or about 33 on a Mac. With **Glide** on (Settings), each report's movement is spread until the next report arrives (15 ms, or 30 ms on a Mac), so the cursor moves on every frame of a fast display.

## Calibration

All on the Device page.

- **Recentre**: point at the middle of the main monitor and click the stick.
- **Gyro**: Wand learns the gyro offset whenever the controller lies still. The Calibrate button does the same on demand.
- **Magnetometer**: optional. Press Calibrate and turn the controller through every direction until the bar fills. It keeps the pointing centre from wandering over minutes. Strokes do not need it.
- **Sensitivity**: `pointing.px_per_degree`, 24 by default, which suited wrist-drawn strokes on a 1440p monitor. Lower it if strokes run off the screen, raise it if they come out small.

## Configuration

Every threshold is in `wand.toml` in the data folder, with a comment on each line. Saving the file reloads it. The ones that matter most are `recognizer.min_score` (0.80), `recognizer.min_margin` (0.04), `pointing.px_per_degree`, `stroke.min_length_deg` and `stroke.min_speed_deg_s`. The mouse has its own section, `[mouse]`: speed, scrolling, which buttons click, `invert_x` and `invert_y` if the cursor moves against your hand. A file written by an older version gets new sections appended the next time the app starts.

## Working on it without a controller

- **Dev mode** (tray menu or Settings): hold Right Ctrl (Right Option on a Mac) and move the mouse. The points go through the same recogniser and overlay.
- Dry run, in the tray menu or as `wand.exe --dry-run`, logs actions instead of running them, and keeps the mouse from moving or clicking.
- Every stroke is appended to `strokes.jsonl` in the data folder. `wandtool strokes <file>` scores a log again with the current code, which is how recogniser changes are checked against real hands. 44 such strokes are a test fixture in `src-tauri/tests/data/`.
- `wandtool selftest 200` draws every gesture with 200 generated hands and prints hit rates and margins.
- "Record controller traffic" in Settings writes raw input reports to a `.wandrec` file. `wandtool replay <file>` feeds one back through the whole pipeline. A replay never moves the real cursor.
- `wandtool sensor <file>` prints what the optical sensor read over a recording, split by whether Wand counted it as on a surface. Record the controller on a desk, on a leg and in the hand to see where the limits should sit.
- `wandtool pointer-check` moves the cursor two pixels and back to confirm the system accepts injected mouse input.
- `wandtool bluetooth` (macOS) prints this Mac's Bluetooth address, which pairing needs, and whether Bluetooth and Accessibility are allowed.
- `wandtool record <file> [seconds]` connects a controller itself (quit the app first), saves what it sends as a `.wandrec`, and prints how its report counter, motion clock and arrival times advance. `wandtool timing <file>` prints the same for any recording.

Run the tool with `cargo run --manifest-path src-tauri/Cargo.toml --bin wandtool -- <command>`.

## How it works

- `protocol.rs`, `joycon.rs`: the Joy-Con 2 protocol over btleplug (scan, pairing, feature flags, input reports, rumble).
- `fusion.rs`: a Mahony-style filter. Gravity corrects pitch and roll, magnetic heading corrects yaw only, gyro bias is learned at rest.
- `pointing.rs`: projects where the tip points onto a plane, so wrist roll does not move the point.
- `recognizer.rs`: a $1-family template matcher. Lines keep their proportions, other shapes are stretched to a unit box so an oval is still a circle, and tails or hooks around a shape are trimmed for a second try.
- `engine.rs`, `actions.rs`: strokes in, overlay messages, key presses and rumble out. The engine also decides, report by report, whether the controller is a wand or a mouse.
- `pointer.rs`: the cursor. Relative moves, clicks and wheel turns through `SendInput` on Windows and Quartz events on macOS, from a thread that sleeps unless there is movement to hand out.
- `macos.rs`: what Win32 does on Windows, done on a Mac: key and mouse events, the Accessibility check, displays, this Mac's Bluetooth address and the overlay windows' level.
- `src/overlay/`: one click-through, always-on-top window per monitor, drawn with WebGL2 into a half-float buffer so glow does not pile up where a stroke crosses itself. Nothing runs while idle.

Sample timing comes from the first four bytes of each input report, a millisecond clock on the controller (15 a report over Windows' link, 30 over a Mac's), with the tick length checked against when reports arrive. The motion timestamp further in counts about 44,700 a second, not microseconds, and is not used.

## The landing page

`site/` is the page served at [wand.alen.is](https://wand.alen.is). `pnpm site` runs it locally and `pnpm site:build` writes a static folder to `site/dist`, which is all a host needs (build command `pnpm site:build`, output directory `site/dist`).

The trail on the page is the overlay code from `src/overlay`, and strokes drawn there are matched by `site/recognizer.ts`, a TypeScript port of `recognizer.rs`. The shapes the web side uses are in `src/shared/gestures.ts`, a mirror of `defaults()` in `gestures.rs`. Change one, change the other. Opening `/og.html` on the dev server redraws the link-preview image, `site/public/og.png`.

## Limits

- One controller at a time.
- The overlay sits above full-screen games too. Some systems turn off variable refresh rate under it; quit Wand from the tray if that happens.
- About 500 MB in the tray, most of it WebView2 (one web view per monitor).
- Windows blocks injected keys and mouse input into windows that run as administrator.

## macOS

Wand lives in the menu bar and has no Dock icon. It needs two permissions, both under System Settings > Privacy & Security:

- **Bluetooth**, asked for on the first start. Without it Wand cannot see the controller.
- **Accessibility**, for pressing keys and moving the cursor. Wand asks for it each time it starts until it has it, and Settings shows a banner with an **Allow** button. Without it gestures are recognised and drawn but do nothing, and the cursor does not move.

macOS ties Accessibility to the app's signature. The build is signed ad hoc, which changes with every build, so after installing a new build the old entry in the Accessibility list no longer counts, even with its switch on. Clear Wand's entries with `tccutil reset Accessibility app.wand.desktop`, start Wand and allow it again. A Developer ID or a self-signed certificate (`bundle.macOS.signingIdentity` in `tauri.macos.conf.json`) keeps the permission across updates.

A downloaded copy that is not notarised is held back by Gatekeeper: open it once from System Settings > Privacy & Security > Open Anyway.

Differences from Windows:

- macOS gives the controller a slower Bluetooth link: 30 ms between connection events, so about 33 reports a second instead of 68. Wand asks for a low-latency link through a private CoreBluetooth call; macOS passes that on as "anything from 10 to 30 ms", and the Mac it was tried on stayed at 30. Strokes carry half as many points (a simulated star still scores 0.92 at 30 ms against 0.96 at 15), and Glide spreads each cursor movement over 30 ms instead of 15.
- Mac keyboards have no right Control key, so dev mode is held on Right Option (`dev.hold_key`).
- Shell commands run in a login zsh, so `PATH` includes what your profile adds; `actions.shell` can be `zsh`, `bash` or `sh`.
- There is no media stop key.
- macOS adds no acceleration to the mouse, so `mouse.sensitivity` is the whole speed.
- The transparent overlay uses a private macOS API, which rules out the Mac App Store.

## Sources

The Joy-Con 2 protocol comes from [ndeadly/switch2_controller_research](https://github.com/ndeadly/switch2_controller_research), [Nadeflore/switch2-controllers](https://github.com/Nadeflore/switch2-controllers) and [CareyScott/switch2controllerpc](https://github.com/CareyScott/switch2controllerpc).

## Licence

MIT. See [LICENSE](LICENSE).
