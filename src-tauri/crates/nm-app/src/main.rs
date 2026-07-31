//! Executable entry point. Keeps no logic of its own beyond reporting a startup failure.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = nm_app::run() {
        eprintln!("network-monitor failed to start: {error}");
        std::process::exit(1);
    }
}
