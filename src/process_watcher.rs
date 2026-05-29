// KakaoTalk process discovery and lifecycle watching.

use crate::config::Config;
use crate::constants::KAKAOTALK_EXE;
use log::{debug, info, warn};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
use windows::core::{PCWSTR, w};

const MAIN_HWND_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const MAIN_HWND_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const STARTUP_APPLY_WINDOW: Duration = Duration::from_secs(5);
const STARTUP_APPLY_INTERVAL: Duration = Duration::from_millis(100);
const SHUTDOWN_CHECK_INTERVAL: Duration = Duration::from_millis(100);

type LifecycleCallback = Box<dyn FnMut() + Send + 'static>;
type CallbackSlot = Arc<Mutex<Option<LifecycleCallback>>>;

pub struct ProcessWatcher {
    poll_interval: Duration,
    is_running: Arc<AtomicBool>,
    kakaotalk_pid: Arc<Mutex<Option<u32>>>,
    on_start: CallbackSlot,
    on_stop: CallbackSlot,
}

impl ProcessWatcher {
    pub fn new(poll_interval: Duration, _startup_delay: Duration) -> Self {
        Self {
            poll_interval,
            is_running: Arc::new(AtomicBool::new(false)),
            kakaotalk_pid: Arc::new(Mutex::new(None)),
            on_start: Arc::new(Mutex::new(None)),
            on_stop: Arc::new(Mutex::new(None)),
        }
    }

    pub fn from_config(cfg: &Config) -> Self {
        Self::new(
            Duration::from_millis(cfg.poll_interval_ms),
            Duration::from_millis(cfg.startup_delay_ms),
        )
    }

    /// Shareable handle to the running flag — useful for the tray icon, which
    /// needs to observe state from another thread after `start` consumes self.
    pub fn is_running_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.is_running)
    }

    /// Shareable handle to the cached PID. Read inside callbacks or from the
    /// tray thread to look up the current KakaoTalk process.
    pub fn pid_handle(&self) -> Arc<Mutex<Option<u32>>> {
        Arc::clone(&self.kakaotalk_pid)
    }

    /// Register a callback fired when KakaoTalk transitions from "not running"
    /// to "running". Runs on the watcher thread repeatedly during the startup
    /// apply window so late-created ad surfaces are caught.
    pub fn on_kakaotalk_start<F>(&self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        *self.on_start.lock().unwrap() = Some(Box::new(callback));
    }

    /// Register a callback fired when KakaoTalk exits. Use this to drop any
    /// cached HWND / WindowState / AdBlocker state; otherwise a relaunch under
    /// a new PID will reuse stale handles and silently fail to block ads.
    pub fn on_kakaotalk_stop<F>(&self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        *self.on_stop.lock().unwrap() = Some(Box::new(callback));
    }

    pub fn start(self, app_running: Arc<AtomicBool>) -> JoinHandle<()> {
        let Self {
            poll_interval,
            is_running,
            kakaotalk_pid,
            on_start,
            on_stop,
        } = self;

        thread::Builder::new()
            .name("process-watcher".into())
            .spawn(move || {
                while app_running.load(Ordering::Acquire) {
                    let pid_now = find_kakaotalk_process();
                    let was_running = is_running.load(Ordering::Acquire);
                    let cached_pid = *kakaotalk_pid.lock().unwrap();

                    match (was_running, pid_now) {
                        (false, None) => {}

                        // Fresh launch candidate. The PID may exist before the
                        // main window does (e.g. Windows logon launches this
                        // app before KakaoTalk finishes startup), so wait for
                        // the HWND for a bounded window before committing to
                        // "running".
                        (false, Some(pid)) => {
                            if wait_for_kakaotalk_main_hwnd(pid, &app_running).is_some() {
                                info!("KakaoTalk ready (pid={pid})");
                                *kakaotalk_pid.lock().unwrap() = Some(pid);
                                is_running.store(true, Ordering::Release);

                                fire_startup_apply_window(&on_start, &app_running);
                            } else {
                                debug!(
                                    "KakaoTalk pid {pid} present but main window not ready within {:?}",
                                    MAIN_HWND_WAIT_TIMEOUT
                                );
                            }
                        }

                        // Exit. Clear the cached PID and notify downstream
                        // listeners so they drop any per-process state.
                        (true, None) => {
                            info!("KakaoTalk exited (was pid={cached_pid:?})");
                            *kakaotalk_pid.lock().unwrap() = None;
                            is_running.store(false, Ordering::Release);
                            fire(&on_stop);
                        }

                        // Still running. Detect a fast restart that swapped
                        // PIDs between two polls — without this, the cached
                        // HWND in downstream modules would point at a dead
                        // process and ad blocking would silently break.
                        (true, Some(pid)) => {
                            if cached_pid != Some(pid) {
                                warn!(
                                    "KakaoTalk pid changed without observed exit: {cached_pid:?} -> {pid}"
                                );
                                fire(&on_stop);

                                if wait_for_kakaotalk_main_hwnd(pid, &app_running).is_some() {
                                    *kakaotalk_pid.lock().unwrap() = Some(pid);
                                    is_running.store(true, Ordering::Release);
                                    fire_startup_apply_window(&on_start, &app_running);
                                } else {
                                    *kakaotalk_pid.lock().unwrap() = None;
                                    is_running.store(false, Ordering::Release);
                                }
                            }
                        }
                    }

                    if !sleep_while_running(poll_interval, &app_running) {
                        break;
                    }
                }
            })
            .expect("failed to spawn process-watcher thread")
    }
}

fn fire(slot: &CallbackSlot) {
    if let Some(cb) = slot.lock().unwrap().as_mut() {
        cb();
    }
}

fn fire_startup_apply_window(slot: &CallbackSlot, app_running: &AtomicBool) {
    let deadline = Instant::now() + STARTUP_APPLY_WINDOW;
    while app_running.load(Ordering::Acquire) {
        fire(slot);

        let now = Instant::now();
        if now >= deadline {
            break;
        }

        if !sleep_while_running((deadline - now).min(STARTUP_APPLY_INTERVAL), app_running) {
            break;
        }
    }
}

fn wait_for_kakaotalk_main_hwnd(pid: u32, app_running: &AtomicBool) -> Option<HWND> {
    let deadline = Instant::now() + MAIN_HWND_WAIT_TIMEOUT;
    while app_running.load(Ordering::Acquire) {
        if find_kakaotalk_process() != Some(pid) {
            return None;
        }

        if let Some(hwnd) = find_kakaotalk_main_hwnd(pid) {
            return Some(hwnd);
        }

        let now = Instant::now();
        if now >= deadline {
            return None;
        }

        if !sleep_while_running((deadline - now).min(MAIN_HWND_RETRY_INTERVAL), app_running) {
            return None;
        }
    }

    None
}

fn sleep_while_running(duration: Duration, app_running: &AtomicBool) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        if !app_running.load(Ordering::Acquire) {
            return false;
        }

        let now = Instant::now();
        if now >= deadline {
            return true;
        }

        thread::sleep((deadline - now).min(SHUTDOWN_CHECK_INTERVAL));
    }
}

/// Locate the KakaoTalk.exe PID via a Toolhelp32 process snapshot. Returns
/// `None` when KakaoTalk is not running or the snapshot call fails.
pub fn find_kakaotalk_process() -> Option<u32> {
    // SAFETY: `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)` is documented
    // to return a system-wide process snapshot HANDLE that the caller owns
    // and must release with CloseHandle. We close it unconditionally below
    // regardless of which path the function takes.
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(h) => h,
        Err(e) => {
            warn!("CreateToolhelp32Snapshot failed: {e}");
            return None;
        }
    };

    // SAFETY: PROCESSENTRY32W is a C plain-old-data struct; zero-initialising
    // it is valid. The Win32 contract requires `dwSize` to be set to the
    // struct size before Process32FirstW reads from `entry`, which we do
    // immediately on the next line.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let mut found: Option<u32> = None;

    // SAFETY: `snapshot` is a valid HANDLE returned by
    // CreateToolhelp32Snapshot above and is not closed until after this
    // block. `entry` is fully initialised with the correct `dwSize`.
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            if exe_name_matches(&entry.szExeFile, KAKAOTALK_EXE) {
                found = Some(entry.th32ProcessID);
                break;
            }
            // SAFETY: Same invariants as Process32FirstW. The iteration
            // terminates when Process32NextW returns Err (typically
            // ERROR_NO_MORE_FILES at the end of the snapshot).
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    // SAFETY: `snapshot` was returned by CreateToolhelp32Snapshot and has not
    // been closed elsewhere; any CloseHandle error is non-recoverable and
    // does not affect the returned PID.
    let _ = unsafe { CloseHandle(snapshot) };

    found
}

fn exe_name_matches(buf: &[u16], target: &str) -> bool {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let name = String::from_utf16_lossy(&buf[..len]);
    name.eq_ignore_ascii_case(target)
}

/// Find the KakaoTalk main window by one of its localised titles and confirm
/// it belongs to `pid`. Returns `None` if the window has not been created yet
/// (process exists, UI not up) or if the matching window belongs to some
/// other process that happens to share the title.
fn find_kakaotalk_main_hwnd(pid: u32) -> Option<HWND> {
    let titles: [PCWSTR; 3] = [w!("카카오톡"), w!("KakaoTalk"), w!("カカオトーク")];

    for title in titles {
        // SAFETY: `title` is a static null-terminated UTF-16 literal produced
        // by the `w!` macro and lives for the whole program. `PCWSTR::null()`
        // for the class name is the documented "any class" sentinel for
        // FindWindowW. The call has no aliasing or mutation requirements.
        let hwnd = match unsafe { FindWindowW(PCWSTR::null(), title) } {
            Ok(h) => h,
            Err(_) => continue,
        };

        let mut owner_pid: u32 = 0;
        // SAFETY: `hwnd` was just returned non-null by FindWindowW and is
        // valid for the duration of this call. `owner_pid` is a live stack
        // u32 that GetWindowThreadProcessId writes through the optional out
        // pointer.
        let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner_pid)) };

        if owner_pid == pid {
            return Some(hwnd);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ignored by default — only meaningful when KakaoTalk is actually
    /// running on the host. Run with:
    ///     cargo test -- --ignored --nocapture find_kakaotalk_process_live
    #[test]
    #[ignore]
    fn find_kakaotalk_process_live() {
        let pid = find_kakaotalk_process();
        println!("find_kakaotalk_process() = {pid:?}");
        let pid = pid.expect("KakaoTalk.exe not found — is it running?");

        let hwnd = find_kakaotalk_main_hwnd(pid);
        println!("find_kakaotalk_main_hwnd({pid}) = {hwnd:?}");
        assert!(hwnd.is_some(), "no main HWND for pid {pid}");
    }
}
