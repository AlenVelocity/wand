// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    wand_lib::app::run(wand_lib::app::Cli::parse(std::env::args()));
}
