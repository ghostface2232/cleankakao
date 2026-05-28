// System tray icon and menu wiring.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon as NativeTrayIcon, TrayIconBuilder, TrayIconEvent,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, LR_DEFAULTCOLOR};

use crate::ico;

const ACTIVE_ICON_BYTES: &[u8] = include_bytes!("../assets/icon_active.ico");
const INACTIVE_ICON_BYTES: &[u8] = include_bytes!("../assets/icon_inactive.ico");
const ICON_RESOURCE_VERSION: u32 = 0x0003_0000;
const TRAY_ICON_SIZE: i32 = 32;
const SINGLE_CLICK_DELAY_PADDING: Duration = Duration::from_millis(50);

pub type TrayResult<T> = Result<T, String>;

pub struct Tray {
    tray_icon: NativeTrayIcon,
    blocking_item: CheckMenuItem,
    _menu: Menu,
    active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenSettings,
    ToggleBlocking,
    CheckForUpdates,
    Quit,
}

impl Tray {
    /// Build the tray icon, context menu, and event channel.
    pub fn new() -> TrayResult<(Self, Receiver<TrayEvent>)> {
        Self::with_active(true)
    }

    /// Build the tray icon with an explicit initial blocking state.
    pub fn with_active(active: bool) -> TrayResult<(Self, Receiver<TrayEvent>)> {
        let (sender, receiver) = channel();
        let sender = Arc::new(Mutex::new(sender));

        let blocking_item = CheckMenuItem::new("광고 차단", true, active, None);
        let settings_item = MenuItem::new("설정", true, None);
        let update_item = MenuItem::new("업데이트 확인", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit_item = MenuItem::new("종료", true, None);

        let blocking_id = blocking_item.id().clone();
        let settings_id = settings_item.id().clone();
        let update_id = update_item.id().clone();
        let quit_id = quit_item.id().clone();

        let menu = Menu::new();
        menu.append(&blocking_item).map_err(|e| e.to_string())?;
        menu.append(&settings_item).map_err(|e| e.to_string())?;
        menu.append(&update_item).map_err(|e| e.to_string())?;
        menu.append(&separator).map_err(|e| e.to_string())?;
        menu.append(&quit_item).map_err(|e| e.to_string())?;

        install_menu_event_handler(
            Arc::clone(&sender),
            MenuIds {
                blocking: blocking_id,
                settings: settings_id,
                update: update_id,
                quit: quit_id,
            },
        );
        install_tray_event_handler(Arc::clone(&sender));

        let tray_icon = TrayIconBuilder::new()
            .with_tooltip(tooltip_for_active(active))
            .with_icon(icon_for_active(active)?)
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;

        Ok((
            Self {
                tray_icon,
                blocking_item,
                _menu: menu,
                active,
            },
            receiver,
        ))
    }

    /// Switch between active / inactive tray icon variants.
    pub fn set_active(&mut self, active: bool) -> TrayResult<()> {
        self.tray_icon
            .set_icon(Some(icon_for_active(active)?))
            .map_err(|e| e.to_string())?;
        self.tray_icon
            .set_tooltip(Some(tooltip_for_active(active)))
            .map_err(|e| e.to_string())?;
        self.blocking_item.set_checked(active);
        self.active = active;
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

struct MenuIds {
    blocking: muda::MenuId,
    settings: muda::MenuId,
    update: muda::MenuId,
    quit: muda::MenuId,
}

fn install_menu_event_handler(sender: Arc<Mutex<Sender<TrayEvent>>>, ids: MenuIds) {
    MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let event_id = event.id();
        let tray_event = if event_id == &ids.blocking {
            Some(TrayEvent::ToggleBlocking)
        } else if event_id == &ids.settings {
            Some(TrayEvent::OpenSettings)
        } else if event_id == &ids.update {
            Some(TrayEvent::CheckForUpdates)
        } else if event_id == &ids.quit {
            Some(TrayEvent::Quit)
        } else {
            None
        };

        if let Some(event) = tray_event {
            send_event(&sender, event);
        }
    }));
}

fn install_tray_event_handler(sender: Arc<Mutex<Sender<TrayEvent>>>) {
    let click_generation = Arc::new(AtomicU64::new(0));

    TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => {
            let generation = click_generation.fetch_add(1, Ordering::AcqRel) + 1;
            let click_generation = Arc::clone(&click_generation);
            let sender = Arc::clone(&sender);

            thread::spawn(move || {
                thread::sleep(single_click_delay());
                if click_generation.load(Ordering::Acquire) == generation {
                    send_event(&sender, TrayEvent::ToggleBlocking);
                }
            });
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            click_generation.fetch_add(1, Ordering::AcqRel);
            send_event(&sender, TrayEvent::OpenSettings);
        }
        _ => {}
    }));
}

fn send_event(sender: &Arc<Mutex<Sender<TrayEvent>>>, event: TrayEvent) {
    if let Ok(sender) = sender.lock() {
        let _ = sender.send(event);
    }
}

fn single_click_delay() -> Duration {
    // SAFETY: GetDoubleClickTime has no parameters and simply reads the
    // current user double-click time from the system.
    Duration::from_millis(unsafe { GetDoubleClickTime() } as u64) + SINGLE_CLICK_DELAY_PADDING
}

fn tooltip_for_active(active: bool) -> &'static str {
    if active {
        "CleanKakao - 차단 중"
    } else {
        "CleanKakao - 비활성"
    }
}

fn icon_for_active(active: bool) -> TrayResult<Icon> {
    if active {
        icon_from_ico_bytes(ACTIVE_ICON_BYTES)
    } else {
        icon_from_ico_bytes(INACTIVE_ICON_BYTES)
    }
}

fn icon_from_ico_bytes(bytes: &'static [u8]) -> TrayResult<Icon> {
    let image = ico::select_ico_image(bytes, TRAY_ICON_SIZE as u32)?;

    // SAFETY: `image` is a slice into the embedded ICO data and contains one
    // icon image resource selected from a validated ICO directory entry.
    // The Win32 call returns an owned HICON, which `Icon::from_handle` wraps
    // and later destroys through tray-icon's RAII icon implementation.
    let hicon = unsafe {
        CreateIconFromResourceEx(
            image,
            true,
            ICON_RESOURCE_VERSION,
            TRAY_ICON_SIZE,
            TRAY_ICON_SIZE,
            LR_DEFAULTCOLOR,
        )
    }
    .map_err(|e| e.to_string())?;

    Ok(Icon::from_handle(hicon.0 as isize))
}
