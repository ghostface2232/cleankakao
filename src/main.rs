#![windows_subsystem = "windows"]

// Entry point: tray app + event loop.

mod adblocker;
mod config;
mod constants;
mod ico;
mod process_watcher;
mod tray;
mod ui;
mod updater;
mod win32;
mod window_events;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use adblocker::AdBlocker;
use config::Config;
use log::{error, info, warn};
use process_watcher::ProcessWatcher;
use tray::{Tray, TrayEvent};
use updater::{DEFAULT_UPDATE_CHECK_INTERVAL, UpdateChecker};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HANDLE, HWND,
    WAIT_FAILED, WAIT_TIMEOUT,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW,
    PostQuitMessage, QS_ALLINPUT, SW_SHOW, SetForegroundWindow, ShowWindow, TranslateMessage,
    WM_QUIT,
};
use windows::core::{PCWSTR, w};

const SINGLE_INSTANCE_MUTEX: PCWSTR = w!("CleanKakao");
const RUN_REGISTRY_PATH: PCWSTR = w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_REGISTRY_VALUE: PCWSTR = w!("CleanKakao");
const REAPPLY_INTERVAL: Duration = Duration::from_millis(1000);
const CONFIG_RELOAD_INTERVAL: Duration = Duration::from_millis(250);
const MESSAGE_LOOP_WAIT_MS: u32 = 100;
pub const SETTINGS_SUBPROCESS_FLAG: &str = "--settings";

fn main() {
    env_logger::init();

    // iced/winit insists the EventLoop be built on the main thread (panics
    // otherwise on Windows). Our main thread already owns the tray Win32
    // message pump, so the settings window runs in a dedicated child process
    // whose main thread is iced's. This branch is that child.
    if std::env::args().any(|arg| arg == SETTINGS_SUBPROCESS_FLAG) {
        run_settings_subprocess();
        return;
    }

    let _single_instance = match SingleInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(e) => {
            error!("failed to create single-instance mutex: {e}");
            return;
        }
    };

    let loaded_config = Config::load();
    if let Err(e) = sync_auto_start(loaded_config.auto_start) {
        warn!("failed to sync auto-start registration: {e}");
    }

    let config = Arc::new(RwLock::new(loaded_config.clone()));
    let blocking_enabled = Arc::new(AtomicBool::new(
        loaded_config.ad_block_banner || loaded_config.ad_block_popup,
    ));
    let app_running = Arc::new(AtomicBool::new(true));
    let adblocker = Arc::new(Mutex::new(AdBlocker::new(Arc::clone(&config))));
    let (config_updates, config_update_events) = channel();

    let (mut tray, tray_events) = match Tray::with_active(blocking_enabled.load(Ordering::Acquire))
    {
        Ok(tray) => tray,
        Err(e) => {
            error!("failed to initialize tray: {e}");
            return;
        }
    };

    let watcher = ProcessWatcher::from_config(&loaded_config);
    let kakaotalk_running = watcher.is_running_handle();
    let kakaotalk_pid = watcher.pid_handle();

    {
        let adblocker = Arc::clone(&adblocker);
        let blocking_enabled = Arc::clone(&blocking_enabled);
        watcher.on_kakaotalk_start(move || {
            if blocking_enabled.load(Ordering::Acquire) {
                apply_adblocker(&adblocker);
            }
        });
    }

    {
        let adblocker = Arc::clone(&adblocker);
        watcher.on_kakaotalk_stop(move || {
            if let Ok(mut adblocker) = adblocker.lock() {
                adblocker.reset();
            }
        });
    }

    let _process_watcher = watcher.start(Arc::clone(&app_running));
    let _periodic_reapply = start_periodic_reapply_worker(
        Arc::clone(&adblocker),
        Arc::clone(&blocking_enabled),
        Arc::clone(&kakaotalk_running),
        Arc::clone(&app_running),
    );
    let _window_event_reapply = window_events::start_reapply_worker(
        Arc::clone(&adblocker),
        Arc::clone(&blocking_enabled),
        Arc::clone(&kakaotalk_running),
        Arc::clone(&kakaotalk_pid),
        Arc::clone(&app_running),
    );
    let _config_reload_worker =
        start_config_reload_worker(loaded_config, config_updates, Arc::clone(&app_running));
    let _update_checker = UpdateChecker::current().start_periodic_check(
        DEFAULT_UPDATE_CHECK_INTERVAL,
        Arc::clone(&config),
        Arc::clone(&app_running),
    );

    run_message_loop(
        &tray_events,
        &config_update_events,
        &mut tray,
        config,
        adblocker,
        blocking_enabled,
        app_running,
    );
}

struct SingleInstance(HANDLE);

impl SingleInstance {
    fn acquire() -> Result<Option<Self>, String> {
        // SAFETY: We pass no security attributes, request initial ownership,
        // and use a static null-terminated mutex name.
        let handle = unsafe { CreateMutexW(None, true, SINGLE_INSTANCE_MUTEX) }
            .map_err(|e| e.to_string())?;

        // SAFETY: GetLastError has no preconditions and reports whether the
        // named mutex already existed after CreateMutexW returned.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            focus_existing_instance();
            // SAFETY: `handle` was returned by CreateMutexW above and is not
            // used after this close.
            let _ = unsafe { CloseHandle(handle) };
            return Ok(None);
        }

        Ok(Some(Self(handle)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: The handle is owned by this guard and is closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn focus_existing_instance() {
    // There may be no visible app window yet because CleanKakao primarily
    // lives in the tray. This is a best-effort path for future settings UI.
    // SAFETY: Both arguments are static or null PCWSTR values. FindWindowW
    // does not mutate caller-owned memory.
    let hwnd = unsafe { FindWindowW(PCWSTR::null(), w!("CleanKakao")) }.unwrap_or(HWND::default());
    if hwnd.0.is_null() {
        return;
    }

    // SAFETY: `hwnd` came from FindWindowW. Bringing it forward is best-effort.
    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(hwnd) };
}

fn start_periodic_reapply_worker(
    adblocker: Arc<Mutex<AdBlocker>>,
    blocking_enabled: Arc<AtomicBool>,
    kakaotalk_running: Arc<AtomicBool>,
    app_running: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("adblocker-reapply".into())
        .spawn(move || {
            while app_running.load(Ordering::Acquire) {
                if kakaotalk_running.load(Ordering::Acquire) {
                    if blocking_enabled.load(Ordering::Acquire) {
                        apply_adblocker(&adblocker);
                    } else {
                        restore_adblocker(&adblocker);
                    }
                }

                thread::sleep(REAPPLY_INTERVAL);
            }
        })
        .expect("failed to spawn adblocker-reapply thread")
}

fn start_config_reload_worker(
    initial_config: Config,
    updates: Sender<Config>,
    app_running: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("config-reload".into())
        .spawn(move || {
            let mut last = initial_config;
            while app_running.load(Ordering::Acquire) {
                thread::sleep(CONFIG_RELOAD_INTERVAL);

                let current = Config::load();
                if current != last {
                    last = current.clone();
                    if updates.send(current).is_err() {
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn config-reload thread")
}

fn run_message_loop(
    tray_events: &Receiver<TrayEvent>,
    config_updates: &Receiver<Config>,
    tray: &mut Tray,
    config: Arc<RwLock<Config>>,
    adblocker: Arc<Mutex<AdBlocker>>,
    blocking_enabled: Arc<AtomicBool>,
    app_running: Arc<AtomicBool>,
) {
    let mut should_quit = false;

    while !should_quit && app_running.load(Ordering::Acquire) {
        drain_tray_events(
            tray_events,
            tray,
            &config,
            &adblocker,
            &blocking_enabled,
            &app_running,
            &mut should_quit,
        );
        drain_config_updates(config_updates, tray, &config, &adblocker, &blocking_enabled);

        // SAFETY: We wait for any queued GUI message, with no object handles.
        let wait =
            unsafe { MsgWaitForMultipleObjects(None, false, MESSAGE_LOOP_WAIT_MS, QS_ALLINPUT) };
        if wait == WAIT_FAILED {
            warn!("MsgWaitForMultipleObjects failed");
            break;
        }

        if wait == WAIT_TIMEOUT {
            continue;
        }

        let mut message = MSG::default();
        // SAFETY: `message` is a valid out pointer. We remove all messages
        // for the current thread and dispatch them below.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
            if message.message == WM_QUIT {
                should_quit = true;
                break;
            }

            // SAFETY: `message` was produced by PeekMessageW for this thread.
            let _ = unsafe { TranslateMessage(&message) };
            unsafe { DispatchMessageW(&message) };
        }
    }

    app_running.store(false, Ordering::Release);
}

fn drain_config_updates(
    config_updates: &Receiver<Config>,
    tray: &mut Tray,
    config: &Arc<RwLock<Config>>,
    adblocker: &Arc<Mutex<AdBlocker>>,
    blocking_enabled: &Arc<AtomicBool>,
) {
    while let Ok(reloaded) = config_updates.try_recv() {
        let previous = match config.write() {
            Ok(mut shared) => {
                let previous = shared.clone();
                *shared = reloaded.clone();
                previous
            }
            Err(e) => {
                warn!("failed to update shared config: {e}");
                continue;
            }
        };

        if previous.auto_start != reloaded.auto_start {
            if let Err(e) = sync_auto_start(reloaded.auto_start) {
                warn!("failed to sync auto-start registration: {e}");
            }
        }

        let was_configured_active = previous.ad_block_banner || previous.ad_block_popup;
        let is_configured_active = reloaded.ad_block_banner || reloaded.ad_block_popup;
        if was_configured_active != is_configured_active {
            blocking_enabled.store(is_configured_active, Ordering::Release);
            if let Err(e) = tray.set_active(is_configured_active) {
                warn!("failed to update tray state: {e}");
            }

            if is_configured_active {
                apply_adblocker(adblocker);
            } else {
                restore_adblocker(adblocker);
            }
        } else if is_configured_active && blocking_enabled.load(Ordering::Acquire) {
            apply_adblocker(adblocker);
        }
    }
}

fn drain_tray_events(
    tray_events: &Receiver<TrayEvent>,
    tray: &mut Tray,
    config: &Arc<RwLock<Config>>,
    adblocker: &Arc<Mutex<AdBlocker>>,
    blocking_enabled: &Arc<AtomicBool>,
    app_running: &Arc<AtomicBool>,
    should_quit: &mut bool,
) {
    while let Ok(event) = tray_events.try_recv() {
        match event {
            TrayEvent::ToggleBlocking => {
                let active = !blocking_enabled.load(Ordering::Acquire);
                persist_blocking_config(config, active);
                blocking_enabled.store(active, Ordering::Release);
                if let Err(e) = tray.set_active(active) {
                    warn!("failed to update tray state: {e}");
                }

                if active {
                    apply_adblocker(adblocker);
                } else {
                    restore_adblocker(adblocker);
                }
            }
            TrayEvent::OpenSettings => ui::open_settings_window(Arc::clone(config)),
            TrayEvent::CheckForUpdates => check_for_updates(),
            TrayEvent::Quit => {
                app_running.store(false, Ordering::Release);
                restore_adblocker(adblocker);
                *should_quit = true;
                // SAFETY: Posts WM_QUIT to the current thread's message loop.
                unsafe { PostQuitMessage(0) };
            }
        }
    }
}

fn persist_blocking_config(config: &Arc<RwLock<Config>>, active: bool) {
    let next = match config.write() {
        Ok(mut shared) => {
            shared.ad_block_banner = active;
            shared.ad_block_popup = active;
            shared.clone()
        }
        Err(e) => {
            warn!("failed to update shared blocking config: {e}");
            return;
        }
    };

    if let Err(e) = next.save() {
        warn!("failed to save blocking config: {e}");
    }
}

fn apply_adblocker(adblocker: &Arc<Mutex<AdBlocker>>) {
    if let Ok(mut adblocker) = adblocker.lock() {
        adblocker.apply_all();
    }
}

fn restore_adblocker(adblocker: &Arc<Mutex<AdBlocker>>) {
    if let Ok(mut adblocker) = adblocker.lock() {
        adblocker.restore_all();
    }
}

fn run_settings_subprocess() {
    let config = Arc::new(RwLock::new(Config::load()));
    if let Err(e) = ui::settings::run(config) {
        error!("settings window failed: {e}");
    }
}

fn check_for_updates() {
    thread::spawn(|| {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(e) => {
                warn!("failed to create tokio runtime for updater: {e}");
                return;
            }
        };

        let updater = UpdateChecker::current();
        if let Some(info) = runtime.block_on(updater.check_latest_version()) {
            info!("update available: {}", info.version);
            updater.notify_update(&info);
        } else {
            info!("no update available");
        }
    });
}

fn sync_auto_start(enabled: bool) -> Result<(), String> {
    let key = open_run_registry_key()?;

    let result = if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = format!("\"{}\"", exe.display());
        let wide = encode_wide_null(&command);
        // SAFETY: `wide` is a live UTF-16 buffer with a trailing NUL. We
        // expose its bytes only for the duration of RegSetValueExW.
        let data =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };

        // SAFETY: `key` is an open HKCU Run key. The value name is static
        // UTF-16 and `data` points to a REG_SZ buffer including its NUL.
        let status = unsafe { RegSetValueExW(key, RUN_REGISTRY_VALUE, None, REG_SZ, Some(data)) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(format!("RegSetValueExW failed: {status:?}"))
        }
    } else {
        // SAFETY: `key` is an open HKCU Run key and the value name is static.
        let status = unsafe { RegDeleteValueW(key, RUN_REGISTRY_VALUE) };
        if status.is_ok() || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("RegDeleteValueW failed: {status:?}"))
        }
    };

    // SAFETY: `key` was returned by RegCreateKeyExW and is not used after
    // this close.
    let _ = unsafe { RegCloseKey(key) };
    result
}

fn open_run_registry_key() -> Result<HKEY, String> {
    let mut key = HKEY::default();
    // SAFETY: We create/open the current user's Run key with set-value
    // access. `key` is a valid out pointer for the returned HKEY.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_REGISTRY_PATH,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };

    if status.is_ok() {
        Ok(key)
    } else {
        Err(format!("RegCreateKeyExW failed: {status:?}"))
    }
}

fn encode_wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
