// UI module: iced-based settings window.
//
// The settings window is launched as a child process because iced 0.13/winit
// 0.30 require the EventLoop to live on the host's main thread on Windows,
// and our main thread is already committed to the tray's Win32 message loop.
// The child writes config changes straight to
// `%LocalAppData%\cleankakao\config.toml`; the tray process watches that file
// and applies changes while the settings window is still open.

pub mod settings;
pub mod theme;

use std::process::Command;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use log::{info, warn};

use crate::SETTINGS_SUBPROCESS_FLAG;
use crate::config::Config;

static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);

/// Spawn the settings window as a child process. No-op when an instance is
/// already running. Returns immediately so the tray event loop is never
/// blocked.
pub fn open_settings_window(config: Arc<RwLock<Config>>) {
    if SETTINGS_OPEN.swap(true, Ordering::AcqRel) {
        info!("settings window already open; ignoring duplicate request");
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            warn!("settings: failed to resolve current_exe: {e}");
            SETTINGS_OPEN.store(false, Ordering::Release);
            return;
        }
    };

    let child = match Command::new(&exe).arg(SETTINGS_SUBPROCESS_FLAG).spawn() {
        Ok(child) => child,
        Err(e) => {
            warn!("settings: failed to spawn subprocess: {e}");
            SETTINGS_OPEN.store(false, Ordering::Release);
            return;
        }
    };

    let spawn_result = thread::Builder::new()
        .name("settings-watcher".into())
        .spawn(move || {
            let mut child = child;
            if let Err(e) = child.wait() {
                warn!("settings: child wait failed: {e}");
            }

            // The tray process also watches config changes while the window
            // is open. This final reload covers the quiet path where the user
            // closes settings without changing anything after a transient
            // file read failed.
            let reloaded = Config::load();
            match config.write() {
                Ok(mut shared) => *shared = reloaded,
                Err(e) => warn!("settings: failed to refresh shared config: {e}"),
            }

            SETTINGS_OPEN.store(false, Ordering::Release);
        });

    if let Err(e) = spawn_result {
        warn!("settings: failed to spawn watcher thread: {e}");
        SETTINGS_OPEN.store(false, Ordering::Release);
    }
}
