// Settings window built with iced 0.13.
//
// The window is launched from the tray thread via [`super::open_settings_window`]
// and runs an isolated iced application: closing it (via the `닫기` button or
// the X chrome button) exits the iced runtime but leaves the host tray app
// alive in `main`.

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{Arc, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use iced::alignment;
use iced::application::Appearance;
use iced::widget::{
    Space, button, column, container, image as image_widget, mouse_area, row, text,
};
use iced::{
    Alignment, Color, ContentFit, Element, Length, Pixels, Settings, Size, Subscription, Task,
    Theme, window,
};
use log::warn;
use windows::Win32::Foundation::{HWND, LPARAM, SYSTEMTIME, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, EnumWindows, GetWindowTextW, GetWindowThreadProcessId, ICON_BIG,
    ICON_SMALL, ICON_SMALL2, IsWindowVisible, LR_DEFAULTCOLOR, SendMessageW, WM_SETICON,
};
use windows::core::BOOL;

use super::theme::{self, Mode, Tokens};
use crate::config::Config;
use crate::process_watcher;

const REPO_URL: &str = "https://github.com/ghostface2232/cleankakao";
const WINDOW_TITLE: &str = " ";
const WINDOW_SIZE: Size = Size {
    width: 400.0,
    height: 585.0,
};

const PILL_WIDTH: f32 = 42.0;
const PILL_HEIGHT: f32 = 22.0;
const PILL_THUMB: f32 = 16.0;
const PILL_PAD: u16 = ((PILL_HEIGHT - PILL_THUMB) / 2.0) as u16;
const PILL_TRAVEL: f32 = PILL_WIDTH - PILL_THUMB - (PILL_PAD as f32 * 2.0);
const TOGGLE_ANIMATION_SPEED: f32 = 14.0;
const ANIMATION_TICK: Duration = Duration::from_millis(16);
const STATUS_TICK: Duration = Duration::from_secs(1);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const LOGO_SIZE: f32 = 90.0;
const ICON_RESOURCE_VERSION: u32 = 0x0003_0000;
const SETTINGS_TASKBAR_ICON_SIZE: i32 = 32;

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
    toggle_animation: ToggleAnimation,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    ToggleAdBlock(bool),
    ToggleAutoStart(bool),
    ToggleCheckUpdate(bool),
    OpenRepo,
    Close,
    StatusTick,
    AnimationTick(Instant),
}

impl State {
    fn new(handle: Arc<RwLock<Config>>) -> Self {
        let config = handle.read().map(|cfg| cfg.clone()).unwrap_or_default();
        let ad_block_on = config.ad_block_banner || config.ad_block_popup;
        let auto_start = config.auto_start;
        let check_update = config.check_update;
        Self {
            config_handle: handle,
            config,
            mode: detect_system_mode(),
            kakaotalk_running: process_watcher::find_kakaotalk_process().is_some(),
            last_check: now_hhmm(),
            toggle_animation: ToggleAnimation::new(ad_block_on, auto_start, check_update),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleAdBlock(v) => {
                // Treat banner + popup as one switch: the user has no reason
                // to keep one kind of ad and not the other.
                self.config.ad_block_banner = v;
                self.config.ad_block_popup = v;
                self.toggle_animation.ad_block.set_target(v);
                self.persist();
            }
            Message::ToggleAutoStart(v) => {
                self.config.auto_start = v;
                self.toggle_animation.auto_start.set_target(v);
                self.persist();
            }
            Message::ToggleCheckUpdate(v) => {
                self.config.check_update = v;
                self.toggle_animation.check_update.set_target(v);
                self.persist();
            }
            Message::OpenRepo => open_url(REPO_URL),
            Message::Close => return iced::exit(),
            Message::StatusTick => {
                self.kakaotalk_running = process_watcher::find_kakaotalk_process().is_some();
                self.last_check = now_hhmm();
            }
            Message::AnimationTick(now) => {
                self.toggle_animation.update(now);
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
        Subscription::batch([
            iced::time::every(STATUS_TICK).map(|_| Message::StatusTick),
            iced::time::every(ANIMATION_TICK).map(Message::AnimationTick),
        ])
    }

    fn view(&self) -> Element<'_, Message> {
        let tokens = Tokens::for_mode(self.mode);

        let logo = logo_image_handle().map(|handle| {
            image_widget(handle)
                .width(Length::Fixed(LOGO_SIZE))
                .height(Length::Fixed(LOGO_SIZE))
                .content_fit(ContentFit::Contain)
        });

        let header_text = column![
            text("CleanKakao")
                .font(theme::HEADING_FONT)
                .size(theme::HEADING_SIZE)
                .color(tokens.text_primary),
            text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(theme::CAPTION_SIZE)
                .color(tokens.text_secondary),
        ]
        .spacing(2);

        let header = column![
            logo.map(Element::from)
                .unwrap_or_else(|| Space::new(LOGO_SIZE, LOGO_SIZE).into()),
            header_text,
        ]
        .spacing(8);

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
                self.toggle_animation.ad_block.progress,
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
                    self.toggle_animation.auto_start.progress,
                    Message::ToggleAutoStart,
                ),
                toggle_row(
                    tokens,
                    theme::ICON_ARROW_SYNC,
                    "자동 업데이트 확인",
                    self.config.check_update,
                    self.toggle_animation.check_update.progress,
                    Message::ToggleCheckUpdate,
                ),
            ]
            .spacing(10)
            .into(),
        );

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

        let body = column![header, ad_block, general, status, footer,].spacing(18);

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
    progress: f32,
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
        pill_toggle(tokens, value, progress, to_message),
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
    progress: f32,
    to_message: fn(bool) -> Message,
) -> Element<'a, Message> {
    let thumb = container(Space::new(0.0, 0.0))
        .width(Length::Fixed(PILL_THUMB))
        .height(Length::Fixed(PILL_THUMB))
        .style(theme::pill_thumb(PILL_THUMB));

    let thumb_offset = PILL_TRAVEL * progress.clamp(0.0, 1.0);
    let thumb_row = row![Space::with_width(Length::Fixed(thumb_offset)), thumb];

    let track = container(thumb_row)
        .width(Length::Fixed(PILL_WIDTH))
        .height(Length::Fixed(PILL_HEIGHT))
        .padding([0u16, PILL_PAD])
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Center)
        .style(theme::pill_track(tokens, value, PILL_HEIGHT));

    mouse_area(track).on_press(to_message(!value)).into()
}

struct ToggleAnimation {
    ad_block: AnimatedToggle,
    auto_start: AnimatedToggle,
    check_update: AnimatedToggle,
    last_frame: Option<Instant>,
}

impl ToggleAnimation {
    fn new(ad_block: bool, auto_start: bool, check_update: bool) -> Self {
        Self {
            ad_block: AnimatedToggle::new(ad_block),
            auto_start: AnimatedToggle::new(auto_start),
            check_update: AnimatedToggle::new(check_update),
            last_frame: None,
        }
    }

    fn update(&mut self, now: Instant) {
        let dt = self
            .last_frame
            .map(|last| now.saturating_duration_since(last).as_secs_f32())
            .unwrap_or_default()
            .min(0.05);
        self.last_frame = Some(now);

        self.ad_block.update(dt);
        self.auto_start.update(dt);
        self.check_update.update(dt);
    }
}

struct AnimatedToggle {
    progress: f32,
    target: f32,
}

impl AnimatedToggle {
    fn new(value: bool) -> Self {
        let progress = f32::from(value);
        Self {
            progress,
            target: progress,
        }
    }

    fn set_target(&mut self, value: bool) {
        self.target = f32::from(value);
    }

    fn update(&mut self, dt: f32) {
        if (self.progress - self.target).abs() <= f32::EPSILON {
            return;
        }

        let step = TOGGLE_ANIMATION_SPEED * dt;
        if self.progress < self.target {
            self.progress = (self.progress + step).min(self.target);
        } else {
            self.progress = (self.progress - step).max(self.target);
        }
    }
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
            ..Default::default()
        })
        .run_with(move || (State::new(config_handle.clone()), Task::none()))
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
            hide_titlebar_icon(hwnd);
            apply_taskbar_icon(hwnd);
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

fn hide_titlebar_icon(hwnd: HWND) {
    // SAFETY: `hwnd` is a live top-level window owned by this process. Passing a
    // null icon handle clears the non-client small icon slots only.
    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(0)),
        );
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL2 as usize)),
            Some(LPARAM(0)),
        );
    }
}

fn apply_taskbar_icon(hwnd: HWND) {
    let hicon = match hicon_from_ico_bytes(WINDOW_ICON_BYTES) {
        Ok(hicon) => hicon,
        Err(e) => {
            warn!("settings: failed to create taskbar icon: {e}");
            return;
        }
    };

    // SAFETY: `hwnd` is a live top-level window owned by this process. `hicon`
    // is an owned icon handle created from embedded ICO bytes and kept alive
    // for the lifetime of this short-lived settings subprocess.
    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(hicon.0 as isize)),
        );
    }
}

fn hicon_from_ico_bytes(
    bytes: &'static [u8],
) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON, String> {
    let image = select_ico_image(bytes)?;

    // SAFETY: `image` is a slice into embedded ICO data and contains one icon
    // image resource selected from a validated ICO directory entry.
    unsafe {
        CreateIconFromResourceEx(
            image,
            true,
            ICON_RESOURCE_VERSION,
            SETTINGS_TASKBAR_ICON_SIZE,
            SETTINGS_TASKBAR_ICON_SIZE,
            LR_DEFAULTCOLOR,
        )
    }
    .map_err(|e| e.to_string())
}

fn select_ico_image(bytes: &'static [u8]) -> Result<&'static [u8], String> {
    if bytes.len() < 6 {
        return Err("ICO data is too short".into());
    }

    let reserved = read_u16(bytes, 0)?;
    let image_type = read_u16(bytes, 2)?;
    let count = read_u16(bytes, 4)? as usize;

    if reserved != 0 || image_type != 1 {
        return Err("ICO header is invalid".into());
    }

    let entries_end = 6usize
        .checked_add(
            count
                .checked_mul(16)
                .ok_or_else(|| "ICO entry table is too large".to_string())?,
        )
        .ok_or_else(|| "ICO entry table overflows".to_string())?;
    if bytes.len() < entries_end {
        return Err("ICO entry table is truncated".into());
    }

    let mut best: Option<(usize, (u32, u32, u32))> = None;
    for index in 0..count {
        let offset = 6 + index * 16;
        let width = decode_ico_dimension(bytes[offset]);
        let height = decode_ico_dimension(bytes[offset + 1]);
        let size = read_u32(bytes, offset + 8)? as usize;
        let image_offset = read_u32(bytes, offset + 12)? as usize;
        let image_end = image_offset
            .checked_add(size)
            .ok_or_else(|| "ICO image range overflows".to_string())?;

        if image_offset >= bytes.len() || image_end > bytes.len() || size == 0 {
            continue;
        }

        let desired = SETTINGS_TASKBAR_ICON_SIZE as u32;
        let too_small_penalty = u32::from(width < desired || height < desired);
        let max_size_delta = width.max(height).abs_diff(desired);
        let shape_delta = width.abs_diff(desired) + height.abs_diff(desired);
        let score = (too_small_penalty, max_size_delta, shape_delta);

        if best.is_none_or(|(_, best_score)| score < best_score) {
            best = Some((index, score));
        }
    }

    let (index, _) = best.ok_or_else(|| "ICO contains no usable image".to_string())?;
    let offset = 6 + index * 16;
    let size = read_u32(bytes, offset + 8)? as usize;
    let image_offset = read_u32(bytes, offset + 12)? as usize;

    Ok(&bytes[image_offset..image_offset + size])
}

fn decode_ico_dimension(value: u8) -> u32 {
    if value == 0 { 256 } else { value as u32 }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "offset overflows".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| "unexpected end of data".to_string())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "offset overflows".to_string())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| "unexpected end of data".to_string())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn logo_image_handle() -> Option<image_widget::Handle> {
    static HANDLE: OnceLock<Option<image_widget::Handle>> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let decoded =
                image::load_from_memory_with_format(WINDOW_ICON_BYTES, image::ImageFormat::Ico)
                    .ok()?;
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            Some(image_widget::Handle::from_rgba(
                width,
                height,
                rgba.into_raw(),
            ))
        })
        .clone()
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
