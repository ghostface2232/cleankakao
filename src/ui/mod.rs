// UI module: iced-based settings window.
//
// The settings window is launched as a child process because iced 0.14/winit
// 0.30 require the EventLoop to live on the host's main thread on Windows,
// and our main thread is already committed to the tray's Win32 message loop.
// The child writes config changes straight to
// `%LocalAppData%\cleankakao\config.toml`; the tray process watches that file
// and applies changes while the settings window is still open.

pub mod settings;
pub mod theme;

use std::io::{BufRead, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use log::{info, warn};

use crate::SETTINGS_SUBPROCESS_FLAG;
use crate::config::Config;

static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
static SETTINGS_INPUT: Mutex<Option<ChildStdin>> = Mutex::new(None);
static UPDATE_CHECK_REQUESTED: AtomicBool = AtomicBool::new(false);
const CHECK_UPDATES_REQUEST: &str = "check-updates";

fn send_update_request(input: &mut impl Write) -> std::io::Result<()> {
    writeln!(input, "{CHECK_UPDATES_REQUEST}")?;
    input.flush()
}

fn read_update_requests(input: impl BufRead, pending: &AtomicBool) {
    for line in input.lines() {
        match line {
            Ok(line) if line == CHECK_UPDATES_REQUEST => pending.store(true, Ordering::Release),
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

pub(super) fn listen_for_update_requests() {
    if let Err(error) = thread::Builder::new()
        .name("settings-requests".into())
        .spawn(|| {
            read_update_requests(std::io::stdin().lock(), &UPDATE_CHECK_REQUESTED);
        })
    {
        warn!("settings: failed to start request listener: {error}");
    }
}

pub(super) fn take_update_request() -> bool {
    UPDATE_CHECK_REQUESTED.swap(false, Ordering::AcqRel)
}

/// Spawn settings or forward an explicit check to the existing child over its
/// stdin pipe. The pipe remains usable while the child starts its iced runtime.
pub fn open_settings_window(config: Arc<RwLock<Config>>, check_updates: bool) {
    if SETTINGS_OPEN.swap(true, Ordering::AcqRel) {
        if check_updates {
            match SETTINGS_INPUT.lock() {
                Ok(mut input) => {
                    if let Some(input) = input.as_mut()
                        && let Err(error) = send_update_request(input)
                    {
                        warn!("settings: failed to forward update check: {error}");
                    }
                }
                Err(error) => warn!("settings: failed to lock request pipe: {error}"),
            }
        } else {
            info!("settings window already open; ignoring duplicate request");
        }
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

    // SAFETY: GetCurrentThreadId has no preconditions; called on the tray thread.
    let host_thread = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    let mut command = Command::new(&exe);
    command.arg(SETTINGS_SUBPROCESS_FLAG);
    if check_updates {
        command.arg("--check-updates");
    }
    let mut child = match command
        .arg("--host-pid")
        .arg(std::process::id().to_string())
        .arg("--host-thread")
        .arg(host_thread.to_string())
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            warn!("settings: failed to spawn subprocess: {e}");
            SETTINGS_OPEN.store(false, Ordering::Release);
            return;
        }
    };

    match SETTINGS_INPUT.lock() {
        Ok(mut input) => *input = child.stdin.take(),
        Err(error) => warn!("settings: failed to store request pipe: {error}"),
    }

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

            if let Ok(mut input) = SETTINGS_INPUT.lock() {
                *input = None;
            }
            SETTINGS_OPEN.store(false, Ordering::Release);
        });

    if let Err(e) = spawn_result {
        warn!("settings: failed to spawn watcher thread: {e}");
        SETTINGS_OPEN.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    #[test]
    fn receive_forwarded_check_in_child() {
        if std::env::var_os("CLEANKAKAO_SETTINGS_PIPE_TEST_CHILD").is_some() {
            let pending = AtomicBool::new(false);
            read_update_requests(std::io::stdin().lock(), &pending);
            assert!(pending.load(Ordering::Acquire));
        }
    }

    #[test]
    fn forwards_checks_through_settings_stdin_even_before_listener_starts() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ui::tests::receive_forwarded_check_in_child"])
            .env("CLEANKAKAO_SETTINGS_PIPE_TEST_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        send_update_request(&mut input).unwrap();
        drop(input);
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn ignores_unknown_pipe_messages_and_exits_on_eof() {
        let pending = AtomicBool::new(false);
        read_update_requests(std::io::Cursor::new(b"unknown\n\n"), &pending);
        assert!(!pending.load(Ordering::Acquire));
    }
}
