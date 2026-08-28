// KakaoTalk process discovery and lifecycle watching.

use crate::adblocker::find_kakaotalk_windows;
use crate::config::Config;
use crate::constants::KAKAOTALK_EXE;
use log::{info, warn};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::win32::window_pid;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

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

    /// Register a callback fired when KakaoTalk transitions from "not running"
    /// to "running". Runs on the watcher thread repeatedly during the startup
    /// apply window so late-created ad surfaces are caught.
    pub fn on_kakaotalk_start<F>(&self, callback: F)
    where
        F: FnMut() + Send + 'static,
    {
        *self.on_start.lock().unwrap() = Some(Box::new(callback));
    }

    /// Register a callback fired after every KakaoTalk process exits. Use this
    /// to drop cached HWND and restore state that can no longer be reused.
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
                    let cached_pid = *kakaotalk_pid.lock().unwrap();
                    let running_pids = kakaotalk_processes();
                    let pid_now = select_watched_pid(
                        cached_pid,
                        &running_pids,
                        find_kakaotalk_main_process(),
                    );
                    let was_running = is_running.load(Ordering::Acquire);

                    match (was_running, pid_now) {
                        (false, None) => {}

                        // The process is only returned after a verified main
                        // window exists, so a background-only helper process
                        // cannot put the watcher into a false running state.
                        (false, Some(pid)) => {
                            info!("KakaoTalk ready (pid={pid})");
                            *kakaotalk_pid.lock().unwrap() = Some(pid);
                            is_running.store(true, Ordering::Release);

                            fire_startup_apply_window(&on_start, &app_running);
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
                                *kakaotalk_pid.lock().unwrap() = Some(pid);
                                is_running.store(true, Ordering::Release);
                                fire_startup_apply_window(&on_start, &app_running);
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

/// Locate any KakaoTalk.exe PID. This is used by the settings UI to report
/// whether KakaoTalk is running, including while its main window is hidden.
pub fn find_kakaotalk_process() -> Option<u32> {
    kakaotalk_processes().into_iter().next()
}

/// Locate the PID that owns a verified KakaoTalk main window. Starting from
/// top-level windows avoids selecting a background-only `KakaoTalk.exe`
/// process before the real UI process when both are running.
fn find_kakaotalk_main_process() -> Option<u32> {
    find_kakaotalk_main_processes().into_iter().next()
}

pub fn find_kakaotalk_main_processes() -> Vec<u32> {
    let mut pids = Vec::new();
    for hwnd in find_kakaotalk_windows() {
        let pid = window_pid(hwnd);
        if pid != 0 && !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
}

fn select_watched_pid(
    cached_pid: Option<u32>,
    running_pids: &[u32],
    main_window_pid: Option<u32>,
) -> Option<u32> {
    cached_pid
        .filter(|pid| running_pids.contains(pid))
        .or(main_window_pid)
}

fn kakaotalk_processes() -> Vec<u32> {
    // SAFETY: The returned snapshot handle is owned by this function and is
    // closed below after synchronous iteration completes.
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(handle) => handle,
        Err(error) => {
            warn!("CreateToolhelp32Snapshot failed: {error}");
            return Vec::new();
        }
    };

    // SAFETY: PROCESSENTRY32W is plain data. Win32 requires dwSize to be set
    // before Process32FirstW writes the first entry into it.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut pids = Vec::new();

    // SAFETY: `snapshot` is live and `entry` has the required dwSize.
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            if exe_name_matches(&entry.szExeFile, KAKAOTALK_EXE) {
                pids.push(entry.th32ProcessID);
            }
            // SAFETY: Same live snapshot and initialized entry invariants as
            // Process32FirstW. An error ends the snapshot iteration.
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    // SAFETY: `snapshot` is owned here and has not been closed elsewhere.
    let _ = unsafe { CloseHandle(snapshot) };
    pids
}

fn exe_name_matches(buf: &[u16], target: &str) -> bool {
    let len = buf.iter().position(|&ch| ch == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::{class_name, is_window_visible, process_image_is, window_text};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GWL_STYLE, GetWindowLongW};
    use windows::core::BOOL;

    #[derive(Debug)]
    struct LiveWindow {
        pid: u32,
        hwnd: HWND,
        visible: bool,
        style: i32,
        class: String,
        title: String,
    }

    fn kakaotalk_top_level_windows_live() -> Vec<LiveWindow> {
        let mut windows = Vec::new();
        let lparam = LPARAM(&mut windows as *mut Vec<LiveWindow> as isize);
        // SAFETY: `windows` outlives this synchronous EnumWindows call and
        // the callback only appends owned diagnostic values to it.
        let _ = unsafe { EnumWindows(Some(collect_live_window), lparam) };
        windows
    }

    unsafe extern "system" fn collect_live_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let pid = window_pid(hwnd);
        if pid == 0 || !process_image_is(pid, KAKAOTALK_EXE) {
            return BOOL(1);
        }

        let windows = lparam.0 as *mut Vec<LiveWindow>;
        // SAFETY: `windows` points to the live Vec passed by
        // kakaotalk_top_level_windows_live for this synchronous enumeration.
        unsafe {
            (*windows).push(LiveWindow {
                pid,
                hwnd,
                visible: is_window_visible(hwnd),
                style: GetWindowLongW(hwnd, GWL_STYLE),
                class: class_name(hwnd),
                title: window_text(hwnd),
            });
        }
        BOOL(1)
    }

    /// Ignored by default — only meaningful when KakaoTalk is actually
    /// running on the host. Run with:
    ///     cargo test -- --ignored --nocapture find_kakaotalk_process_live
    #[test]
    #[ignore]
    fn find_kakaotalk_process_live() {
        let all_pids = kakaotalk_processes();
        println!("kakaotalk_processes() = {all_pids:?}");
        for window in kakaotalk_top_level_windows_live() {
            println!(
                "top-level window: pid={} hwnd={:?} visible={} style={:#x} class={:?} title={:?}",
                window.pid, window.hwnd, window.visible, window.style, window.class, window.title
            );
        }

        let main_pids = find_kakaotalk_main_processes();
        println!("find_kakaotalk_main_processes() = {main_pids:?}");
        assert!(!main_pids.is_empty(), "KakaoTalk main windows not found");
        for pid in main_pids {
            assert!(all_pids.contains(&pid));
        }
    }

    #[test]
    fn watcher_selects_ui_process_and_keeps_it_while_window_is_hidden() {
        let background_pid = 10;
        let ui_pid = 20;
        let running = [background_pid, ui_pid];

        assert_eq!(
            select_watched_pid(None, &running, Some(ui_pid)),
            Some(ui_pid)
        );
        assert_eq!(
            select_watched_pid(Some(ui_pid), &running, None),
            Some(ui_pid)
        );
        assert_eq!(select_watched_pid(None, &[background_pid], None), None);
    }
}
