//! Wand: draw gestures in the air with a Joy-Con 2.
//!
//! The first group of modules is pure logic with no I/O and is what the tests
//! and `wandtool` exercise. The second group talks to Windows or macOS,
//! Bluetooth and Tauri.

pub mod config;
pub mod engine;
pub mod fusion;
pub mod gestures;
pub mod pointer;
pub mod pointing;
pub mod protocol;
pub mod recognizer;
pub mod replay;
pub mod synth;

pub mod actions;
pub mod app;
pub mod devmouse;
pub mod joycon;
#[cfg(target_os = "macos")]
pub mod macos;
