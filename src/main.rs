#![windows_subsystem = "windows"]

// Entry point: tray app + event loop.

mod adblocker;
mod config;
mod process_watcher;
mod tray;
mod ui;
mod updater;

fn main() {
    env_logger::init();

    // TODO: load config, spawn process watcher, build tray, run event loop.
}
