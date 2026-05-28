// Event-driven ad-block reapplication for KakaoTalk window restore/show.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use log::{debug, warn};
use windows::Win32::Foundation::{HWND, WAIT_FAILED, WAIT_TIMEOUT};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_SHOW,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, MSG, MsgWaitForMultipleObjects,
    OBJID_WINDOW, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};

use crate::adblocker::AdBlocker;

const EVENT_LOOP_WAIT: Duration = Duration::from_millis(50);
const PID_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const RESTORE_BURST_DURATION: Duration = Duration::from_millis(650);
const RESTORE_BURST_INTERVAL: Duration = Duration::from_millis(75);

static EVENT_PENDING: AtomicBool = AtomicBool::new(false);

pub fn start_reapply_worker(
    adblocker: Arc<Mutex<AdBlocker>>,
    blocking_enabled: Arc<AtomicBool>,
    kakaotalk_running: Arc<AtomicBool>,
    kakaotalk_pid: Arc<Mutex<Option<u32>>>,
    app_running: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("window-event-reapply".into())
        .spawn(move || {
            let mut hooks: Option<WinEventHooks> = None;
            let mut last_pid_refresh = Instant::now() - PID_REFRESH_INTERVAL;
            let mut burst_until: Option<Instant> = None;

            while app_running.load(Ordering::Acquire) {
                let now = Instant::now();
                if now.duration_since(last_pid_refresh) >= PID_REFRESH_INTERVAL {
                    last_pid_refresh = now;
                    let desired_pid = if blocking_enabled.load(Ordering::Acquire)
                        && kakaotalk_running.load(Ordering::Acquire)
                    {
                        *kakaotalk_pid.lock().unwrap()
                    } else {
                        None
                    };

                    if hooks.as_ref().map(|h| h.pid) != desired_pid {
                        hooks = desired_pid.and_then(install_hooks);
                        EVENT_PENDING.store(false, Ordering::Release);
                    }
                }

                let wait_for = if burst_until.is_some_and(|deadline| now < deadline) {
                    RESTORE_BURST_INTERVAL
                } else {
                    EVENT_LOOP_WAIT
                };

                pump_messages(wait_for);

                if EVENT_PENDING.swap(false, Ordering::AcqRel) {
                    debug!("KakaoTalk window event observed; reapplying ad blocker");
                    apply_if_enabled(&adblocker, &blocking_enabled, &kakaotalk_running);
                    burst_until = Some(Instant::now() + RESTORE_BURST_DURATION);
                    continue;
                }

                if burst_until.is_some_and(|deadline| Instant::now() < deadline) {
                    apply_if_enabled(&adblocker, &blocking_enabled, &kakaotalk_running);
                } else {
                    burst_until = None;
                }
            }
        })
        .expect("failed to spawn window-event-reapply thread")
}

fn apply_if_enabled(
    adblocker: &Arc<Mutex<AdBlocker>>,
    blocking_enabled: &Arc<AtomicBool>,
    kakaotalk_running: &Arc<AtomicBool>,
) {
    if !blocking_enabled.load(Ordering::Acquire) || !kakaotalk_running.load(Ordering::Acquire) {
        return;
    }

    if let Ok(mut adblocker) = adblocker.lock() {
        adblocker.apply_all();
    }
}

fn pump_messages(wait_for: Duration) {
    // SAFETY: We wait for any queued GUI message, with no object handles.
    let wait =
        unsafe { MsgWaitForMultipleObjects(None, false, wait_for.as_millis() as u32, QS_ALLINPUT) };
    if wait == WAIT_FAILED {
        warn!("window event MsgWaitForMultipleObjects failed");
        return;
    }
    if wait == WAIT_TIMEOUT {
        return;
    }

    let mut message = MSG::default();
    // SAFETY: `message` is a valid out pointer. We remove and dispatch all
    // messages on this hook thread so out-of-context WinEvent callbacks fire.
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
        // SAFETY: `message` was produced by PeekMessageW for this thread.
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}

struct WinEventHooks {
    pid: u32,
    hooks: Vec<HWINEVENTHOOK>,
}

impl Drop for WinEventHooks {
    fn drop(&mut self) {
        for hook in self.hooks.drain(..) {
            // SAFETY: Each hook in this vec was returned by SetWinEventHook
            // and is unhooked exactly once when the target PID changes or the
            // worker exits.
            let _ = unsafe { UnhookWinEvent(hook) };
        }
    }
}

fn install_hooks(pid: u32) -> Option<WinEventHooks> {
    let ranges = [
        (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
        (EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZEEND),
        (EVENT_OBJECT_CREATE, EVENT_OBJECT_SHOW),
    ];
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    let mut hooks = Vec::new();

    for (event_min, event_max) in ranges {
        // SAFETY: We install an out-of-context WinEvent hook for a specific
        // KakaoTalk PID. The callback is a static function and this worker
        // thread pumps messages until the hook is removed.
        let hook = unsafe {
            SetWinEventHook(
                event_min,
                event_max,
                None,
                Some(win_event_proc),
                pid,
                0,
                flags,
            )
        };
        if hook.is_invalid() {
            warn!("failed to install WinEvent hook {event_min}-{event_max} for pid {pid}");
        } else {
            hooks.push(hook);
        }
    }

    if hooks.is_empty() {
        None
    } else {
        debug!(
            "installed {} WinEvent hooks for KakaoTalk pid {pid}",
            hooks.len()
        );
        Some(WinEventHooks { pid, hooks })
    }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    idobject: i32,
    idchild: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if hwnd.is_invalid() {
        return;
    }
    if idobject != OBJID_WINDOW.0 || idchild != CHILDID_SELF as i32 {
        return;
    }
    if matches!(
        event,
        EVENT_SYSTEM_FOREGROUND
            | EVENT_SYSTEM_MINIMIZEEND
            | EVENT_OBJECT_CREATE
            | EVENT_OBJECT_SHOW
    ) {
        EVENT_PENDING.store(true, Ordering::Release);
    }
}
