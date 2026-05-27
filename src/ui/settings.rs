// Settings window built with iced 0.13.
//
// The window is launched from the tray thread via [`super::open_settings_window`]
// and runs an isolated iced application: closing it (via the `닫기` button or
// the X chrome button) exits the iced runtime but leaves the host tray app
// alive in `main`.

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use iced::alignment;
use iced::application::Appearance;
use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{
    Alignment, Color, Element, Length, Pixels, Settings, Size, Subscription, Task, Theme, window,
};
use log::warn;
use windows::Win32::Foundation::{HWND, LPARAM, SYSTEMTIME};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

use super::theme::{self, Mode, Tokens};
use crate::config::Config;
use crate::process_watcher;

const REPO_URL: &str = "https://github.com/ghostface2232/cleankakao";
const WINDOW_TITLE: &str = "CleanKakao 설정";
const WINDOW_SIZE: Size = Size {
    width: 400.0,
    height: 580.0,
};

const PILL_WIDTH: f32 = 42.0;
const PILL_HEIGHT: f32 = 22.0;
const PILL_THUMB: f32 = 16.0;
const PILL_PAD: u16 = ((PILL_HEIGHT - PILL_THUMB) / 2.0) as u16;
const STATUS_TICK: Duration = Duration::from_secs(1);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// How long the Mica setup thread keeps polling for our top-level HWND before
// giving up. The window normally exists within ~100 ms; 3 s is generous.
const MICA_WAIT_TIMEOUT: Duration = Duration::from_secs(3);
const MICA_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MICA_POST_CREATE_DELAY: Duration = Duration::from_millis(150);

const WINDOW_ICON_BYTES: &[u8] = include_bytes!("../../assets/icon_active.ico");

pub struct State {
    config_handle: Arc<RwLock<Config>>,
    config: Config,
    mode: Mode,
    kakaotalk_running: bool,
    last_check: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    ToggleAdBlock(bool),
    ToggleAutoStart(bool),
    ToggleCheckUpdate(bool),
    OpenRepo,
    Close,
    Tick,
}

impl State {
    fn new(handle: Arc<RwLock<Config>>) -> Self {
        let config = handle.read().map(|cfg| cfg.clone()).unwrap_or_default();
        Self {
            config_handle: handle,
            config,
            mode: detect_system_mode(),
            kakaotalk_running: process_watcher::find_kakaotalk_process().is_some(),
            last_check: now_hhmm(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleAdBlock(v) => {
                // Treat banner + popup as one switch: the user has no reason
                // to keep one kind of ad and not the other.
                self.config.ad_block_banner = v;
                self.config.ad_block_popup = v;
                self.persist();
            }
            Message::ToggleAutoStart(v) => {
                self.config.auto_start = v;
                self.persist();
            }
            Message::ToggleCheckUpdate(v) => {
                self.config.check_update = v;
                self.persist();
            }
            Message::OpenRepo => open_url(REPO_URL),
            Message::Close => return iced::exit(),
            Message::Tick => {
                self.kakaotalk_running = process_watcher::find_kakaotalk_process().is_some();
                self.last_check = now_hhmm();
            }
        }
        Task::none()
    }

    fn persist(&mut self) {
        match self.config_handle.write() {
            Ok(mut shared) => *shared = self.config.clone(),
            Err(e) => warn!("settings: failed to take config write lock: {e}"),
        }
        if let Err(e) = self.config.save() {
            warn!("settings: failed to save config: {e}");
        }
    }

    fn theme(&self) -> Theme {
        theme::theme_for(self.mode)
    }

    /// Application-level appearance. The background must be fully
    /// transparent so the DWM Mica backdrop renders behind iced's content.
    fn style(&self, _theme: &Theme) -> Appearance {
        let tokens = Tokens::for_mode(self.mode);
        Appearance {
            background_color: Color::TRANSPARENT,
            text_color: tokens.text_primary,
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(STATUS_TICK).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Message> {
        let tokens = Tokens::for_mode(self.mode);

        let header = column![
            text("CleanKakao")
                .font(theme::HEADING_FONT)
                .size(theme::HEADING_SIZE)
                .color(tokens.text_primary),
            text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(theme::CAPTION_SIZE)
                .color(tokens.text_secondary),
        ]
        .spacing(2);

        let ad_block_on = self.config.ad_block_banner || self.config.ad_block_popup;
        let ad_block = section(
            tokens,
            theme::ICON_SHIELD,
            "광고 차단",
            toggle_row(
                tokens,
                theme::ICON_EYE_OFF,
                "배너 · 팝업 광고 숨기기",
                ad_block_on,
                Message::ToggleAdBlock,
            ),
        );

        let general = section(
            tokens,
            theme::ICON_SETTINGS,
            "일반",
            column![
                toggle_row(
                    tokens,
                    theme::ICON_ROCKET,
                    "Windows 시작 시 실행",
                    self.config.auto_start,
                    Message::ToggleAutoStart,
                ),
                toggle_row(
                    tokens,
                    theme::ICON_ARROW_SYNC,
                    "자동 업데이트 확인",
                    self.config.check_update,
                    Message::ToggleCheckUpdate,
                ),
            ]
            .spacing(10)
            .into(),
        );

        let blocking_active = self.config.ad_block_banner || self.config.ad_block_popup;
        let status = section(
            tokens,
            theme::ICON_INFO,
            "상태",
            column![
                status_row(
                    tokens,
                    "카카오톡",
                    if self.kakaotalk_running {
                        "실행 중"
                    } else {
                        "미실행"
                    },
                    self.kakaotalk_running,
                ),
                status_row(
                    tokens,
                    "차단 상태",
                    if blocking_active {
                        "활성"
                    } else {
                        "비활성"
                    },
                    blocking_active,
                ),
                status_text_row(tokens, "마지막 확인", &self.last_check),
            ]
            .spacing(8)
            .into(),
        );

        let footer = row![
            button(
                row![
                    text(theme::ICON_OPEN)
                        .font(theme::ICON_FONT)
                        .size(theme::ICON_SIZE)
                        .color(tokens.text_primary),
                    text("GitHub").size(theme::BODY_SIZE),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([6, 12])
            .style(theme::secondary_button(tokens))
            .on_press(Message::OpenRepo),
            Space::with_width(Length::Fill),
            button(text("닫기").size(theme::BODY_SIZE))
                .padding([6, 16])
                .style(theme::primary_button(tokens))
                .on_press(Message::Close),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let body = column![
            header,
            ad_block,
            general,
            status,
            Space::with_height(Length::Fill),
            footer,
        ]
        .spacing(18);

        container(body)
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::root_container(tokens))
            .into()
    }
}

fn section<'a>(
    tokens: Tokens,
    icon: &'static str,
    title: &'static str,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let header = row![
        text(icon)
            .font(theme::ICON_FONT)
            .size(theme::SECTION_TITLE_SIZE)
            .color(tokens.accent),
        text(title)
            .font(theme::HEADING_FONT)
            .size(theme::SECTION_TITLE_SIZE)
            .color(tokens.text_primary),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let card = container(body)
        .padding(14)
        .width(Length::Fill)
        .style(theme::card_container(tokens));

    column![header, card].spacing(6).into()
}

fn toggle_row<'a>(
    tokens: Tokens,
    icon: &'static str,
    label: &'static str,
    value: bool,
    to_message: fn(bool) -> Message,
) -> Element<'a, Message> {
    row![
        text(icon)
            .font(theme::ICON_FONT)
            .size(theme::ICON_SIZE)
            .color(tokens.text_secondary),
        text(label)
            .size(theme::BODY_SIZE)
            .color(tokens.text_primary)
            .width(Length::Fill),
        pill_toggle(tokens, value, to_message),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

/// True pill-shaped toggle. iced 0.13's built-in `toggler` hard-codes its
/// corner radius at `height / 2.46`, which renders as a soft-cornered
/// rectangle, so we draw the track + thumb ourselves with `container`s whose
/// border radius is exactly `height / 2`.
fn pill_toggle<'a>(
    tokens: Tokens,
    value: bool,
    to_message: fn(bool) -> Message,
) -> Element<'a, Message> {
    let thumb = container(Space::new(0.0, 0.0))
        .width(Length::Fixed(PILL_THUMB))
        .height(Length::Fixed(PILL_THUMB))
        .style(theme::pill_thumb(PILL_THUMB));

    let thumb_alignment = if value {
        alignment::Horizontal::Right
    } else {
        alignment::Horizontal::Left
    };

    let track = container(thumb)
        .width(Length::Fixed(PILL_WIDTH))
        .height(Length::Fixed(PILL_HEIGHT))
        .padding([0u16, PILL_PAD])
        .align_x(thumb_alignment)
        .align_y(alignment::Vertical::Center)
        .style(theme::pill_track(tokens, value, PILL_HEIGHT));

    mouse_area(track).on_press(to_message(!value)).into()
}

fn status_row<'a>(
    tokens: Tokens,
    label: &'static str,
    value: &str,
    positive: bool,
) -> Element<'a, Message> {
    let dot_color = if positive {
        tokens.success
    } else {
        tokens.danger
    };
    row![
        text(label)
            .size(theme::BODY_SIZE)
            .color(tokens.text_primary)
            .width(Length::Fill),
        text(value.to_string())
            .size(theme::BODY_SIZE)
            .color(tokens.text_secondary),
        text(theme::ICON_CIRCLE)
            .font(theme::ICON_FONT)
            .size(theme::STATUS_DOT_SIZE)
            .color(dot_color),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn status_text_row<'a>(tokens: Tokens, label: &'static str, value: &str) -> Element<'a, Message> {
    row![
        text(label)
            .size(theme::BODY_SIZE)
            .color(tokens.text_primary)
            .width(Length::Fill),
        text(value.to_string())
            .size(theme::BODY_SIZE)
            .color(tokens.text_secondary),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// Read the current Windows app theme. Returns `Mode::Dark` if the registry
/// value is missing or unreadable, matching the WinUI3 default look.
fn detect_system_mode() -> Mode {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
        RegQueryValueExW,
    };
    use windows::core::w;

    let mut key = HKEY::default();
    // SAFETY: HKEY_CURRENT_USER is always valid; the subkey path is a static
    // null-terminated UTF-16 literal; `key` is a live stack HKEY out-pointer.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            Some(0),
            KEY_READ,
            &mut key,
        )
    };
    if status.is_err() {
        return Mode::Dark;
    }

    let mut value: u32 = 1;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    let mut ty = REG_VALUE_TYPE::default();
    // SAFETY: `key` is the just-opened HKCU subkey; the value name is a static
    // UTF-16 literal; `value`/`size`/`ty` are live stack locals sized for a
    // REG_DWORD read.
    let read_status = unsafe {
        RegQueryValueExW(
            key,
            w!("AppsUseLightTheme"),
            None,
            Some(&mut ty),
            Some(&mut value as *mut u32 as *mut u8),
            Some(&mut size),
        )
    };
    // SAFETY: `key` was returned by RegOpenKeyExW and is not used after this
    // call.
    let _ = unsafe { RegCloseKey(key) };

    if read_status.is_err() {
        return Mode::Dark;
    }

    if value == 0 { Mode::Dark } else { Mode::Light }
}

fn now_hhmm() -> String {
    // SAFETY: GetLocalTime takes no parameters and returns a SYSTEMTIME by
    // value, so there are no aliasing or initialization requirements.
    let st: SYSTEMTIME = unsafe { GetLocalTime() };
    format!("{:02}:{:02}", st.wHour, st.wMinute)
}

/// Hand the URL off to the Windows shell so the user's default browser opens
/// it. `rundll32 url.dll,FileProtocolHandler` avoids the brief console flash
/// that `cmd /C start` would cause for a `#![windows_subsystem = "windows"]`
/// process.
fn open_url(url: &str) {
    if let Err(e) = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        warn!("failed to open url {url}: {e}");
    }
}

/// Run the settings window. Blocks until the user closes the window. Intended
/// to be called from the main thread of the `--settings` subprocess (iced 0.13
/// / winit 0.30 require the EventLoop to live on the main thread on Windows).
pub fn run(config_handle: Arc<RwLock<Config>>) -> iced::Result {
    let mode = detect_system_mode();
    force_settings_renderer();
    // iced exposes no way to grab the underlying HWND, so the Mica setup
    // thread waits for the window to appear by title and then talks to DWM
    // directly. Spawned before `iced::application().run_with(...)` because
    // that call takes over the main thread.
    spawn_mica_setup(WINDOW_TITLE, mode);

    iced::application(WINDOW_TITLE, State::update, State::view)
        .theme(State::theme)
        .style(State::style)
        .subscription(State::subscription)
        .settings(Settings {
            id: Some("cleankakao.settings".to_string()),
            fonts: vec![
                theme::FLUENT_ICONS_BYTES.into(),
                theme::PRETENDARD_MEDIUM_BYTES.into(),
                theme::PRETENDARD_SEMIBOLD_BYTES.into(),
            ],
            default_font: theme::BODY_FONT,
            default_text_size: Pixels(theme::BODY_SIZE),
            antialiasing: true,
        })
        .window(window::Settings {
            size: WINDOW_SIZE,
            min_size: Some(WINDOW_SIZE),
            max_size: Some(WINDOW_SIZE),
            resizable: false,
            transparent: true,
            icon: load_window_icon(),
            ..Default::default()
        })
        .run_with(move || (State::new(config_handle.clone()), Task::none()))
}

/// Decode the embedded ICO once and hand the largest decoded frame to iced
/// for the title bar and taskbar entry. The `image` crate's ICO loader picks
/// the highest-resolution sub-image automatically.
fn load_window_icon() -> Option<window::Icon> {
    match image::load_from_memory_with_format(WINDOW_ICON_BYTES, image::ImageFormat::Ico) {
        Ok(decoded) => {
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            match window::icon::from_rgba(rgba.into_raw(), width, height) {
                Ok(icon) => Some(icon),
                Err(e) => {
                    warn!("settings: window icon rgba conversion failed: {e}");
                    None
                }
            }
        }
        Err(e) => {
            warn!("settings: window icon decode failed: {e}");
            None
        }
    }
}

/// Poll for the settings window by title from a worker thread, then apply
/// the Windows 11 Mica backdrop and immersive-dark-mode attributes via DWM.
/// No-op on Windows versions that don't recognise the attribute IDs (DWM
/// silently ignores them).
fn spawn_mica_setup(title: &'static str, mode: Mode) {
    thread::Builder::new()
        .name("settings-mica".into())
        .spawn(move || {
            let Some(hwnd) = wait_for_window(title, MICA_WAIT_TIMEOUT) else {
                warn!("settings: timed out waiting for window before applying Mica");
                return;
            };
            // winit applies its Windows-specific defaults during WM_CREATE,
            // including `DWMSBT_AUTO`. Give it a moment to finish, then apply
            // our explicit Mica backdrop.
            thread::sleep(MICA_POST_CREATE_DELAY);
            apply_mica(hwnd, mode);
        })
        .map_err(|e| warn!("settings: failed to spawn Mica setup thread: {e}"))
        .ok();
}

fn wait_for_window(title: &str, timeout: Duration) -> Option<HWND> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(hwnd) = find_own_top_level_window(title) {
            return Some(hwnd);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(MICA_POLL_INTERVAL);
    }
}

fn find_own_top_level_window(title: &str) -> Option<HWND> {
    struct Ctx<'a> {
        title: &'a str,
        pid: u32,
        hwnd: Option<HWND>,
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` points to the stack-local Ctx below. EnumWindows
        // invokes this callback synchronously before the Ctx goes out of
        // scope.
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx<'_>) };

        let mut owner_pid = 0;
        // SAFETY: `owner_pid` is a live stack u32 out-parameter.
        let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner_pid)) };
        if owner_pid != ctx.pid || !is_window_visible(hwnd) {
            return BOOL(1);
        }

        if window_text(hwnd) == ctx.title {
            ctx.hwnd = Some(hwnd);
            return BOOL(0);
        }

        BOOL(1)
    }

    let mut ctx = Ctx {
        title,
        // SAFETY: GetCurrentProcessId takes no parameters.
        pid: unsafe { GetCurrentProcessId() },
        hwnd: None,
    };
    let lparam = LPARAM(&mut ctx as *mut Ctx<'_> as isize);
    // SAFETY: `ctx` outlives this synchronous EnumWindows call.
    let _ = unsafe { EnumWindows(Some(collect), lparam) };
    ctx.hwnd
}

fn apply_mica(hwnd: HWND, mode: Mode) {
    // SAFETY: `hwnd` is a live top-level window owned by this process. Each
    // DWM call takes a valid HWND and properly sized inputs.
    unsafe {
        // (1) Extend DWM's frame across the client area. Some Win32 render
        // paths only show `DWMSBT_MAINWINDOW` behind the default frame unless
        // the glass frame is explicitly extended into the client area.
        let margins = MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        };
        if let Err(e) = DwmExtendFrameIntoClientArea(hwnd, &margins) {
            warn!("settings: DwmExtendFrameIntoClientArea failed: {e}");
        }

        // (2) Sync the non-client (title bar + border) palette to the chosen
        // light/dark mode so it visually matches the Mica tint.
        let dark: i32 = match mode {
            Mode::Dark => 1,
            Mode::Light => 0,
        };
        if let Err(e) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        ) {
            warn!("settings: DWMWA_USE_IMMERSIVE_DARK_MODE failed: {e}");
        }

        // (3) Engage Mica. `DWMSBT_MAINWINDOW` asks DWM to draw the backdrop
        // behind the whole window bounds; iced's transparent clear color then
        // lets that material show through the root container. Keep winit's
        // blur-behind transparency enabled: disabling it makes the swapchain
        // alpha get composed as an opaque light client area on some systems.
        let backdrop: i32 = DWMSBT_MAINWINDOW.0;
        if let Err(e) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        ) {
            warn!("settings: DWMWA_SYSTEMBACKDROP_TYPE failed: {e}");
        }
    }
}

fn is_window_visible(hwnd: HWND) -> bool {
    // SAFETY: IsWindowVisible accepts any HWND value.
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    // SAFETY: `buf` is a live stack array; GetWindowTextW writes at most
    // buf.len() - 1 UTF-16 code units.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    if len == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len.min(buf.len())])
}

fn force_settings_renderer() {
    // iced defaults to wgpu first. On Windows 11 this can produce an opaque
    // swapchain even when the window and widgets are transparent, hiding the
    // DWM Mica material. The settings window is simple enough that the
    // software renderer is a better fit for reliable composition.
    // SAFETY: This settings subprocess is still single-threaded here; no
    // other Rust threads have been spawned and no concurrent environment
    // access is happening.
    unsafe {
        std::env::set_var("ICED_BACKEND", "tiny-skia");
    }
}
