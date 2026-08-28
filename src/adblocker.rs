// Core ad-blocking logic for KakaoTalk windows.
//
// The KakaoTalk main window exposes named EVA_ChildWindow panes such as
// OnlineMainView and LockModeView. Resize those known panes to cover the
// bottom ad strip, and hide only narrow, well-scoped ad windows. Avoid
// SetWindowRgn on KakaoTalk/CEF windows: clipping those compositor-backed
// children can leave the main client area black until KakaoTalk repaints or
// restarts.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Arc, RwLock};

use log::{debug, info, warn};

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RDW_UPDATENOW, REDRAW_WINDOW_FLAGS, RedrawWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumThreadWindows, EnumWindows, GWL_STYLE, GetParent, GetWindowLongW,
    GetWindowRect, GetWindowThreadProcessId, HWND_TOP, IsIconic, IsWindow, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos, ShowWindow, WS_POPUP,
};
use windows::core::BOOL;

use crate::config::Config;
use crate::constants::KAKAOTALK_EXE;
use crate::win32::{class_name, is_window_visible, process_image_is, window_pid, window_text};

// The bottom ad strip is covered by shrinking the main-view height by 31 px;
// the lock-screen view only needs the 2 px shadow compensation on width.
const LAYOUT_SHADOW_PADDING: i32 = 2;
const MAIN_VIEW_BOTTOM_PADDING: i32 = 31;

// Conservative fallback for a direct empty EVA_ChildWindow ad slot. In
// KakaoTalk 25.x this bottom banner is consistently 91 px tall across
// 1080p/1440p/2160p layouts, so keep the classifier tight to avoid hiding
// legitimate KakaoTalk panes that merely look like small empty windows.
const DIRECT_BANNER_HEIGHT: i32 = 91;
const DIRECT_BANNER_HEIGHT_TOLERANCE_PX: i32 = 2;
const DIRECT_BANNER_MIN_WIDTH_RATIO: f32 = 0.70;
const DIRECT_BANNER_BOTTOM_TOLERANCE_PX: i32 = 12;

const POPUP_MIN_DIM: i32 = 80;
const POPUP_MAX_DIM: i32 = 900;
#[derive(Debug, Clone, Copy)]
struct WindowState {
    rect: RECT,
    visible: bool,
    kind: RestoreKind,
    main: isize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreKind {
    ResizedPane,
    HiddenWindow,
}

#[derive(Debug, Clone, Copy)]
pub enum AdWindow {
    MainView { hwnd: HWND, width: i32, height: i32 },
    BannerSlot { hwnd: HWND },
    Popup { hwnd: HWND },
}

impl AdWindow {
    pub fn hwnd(&self) -> HWND {
        match *self {
            AdWindow::MainView { hwnd, .. }
            | AdWindow::BannerSlot { hwnd }
            | AdWindow::Popup { hwnd } => hwnd,
        }
    }
}

pub struct AdBlocker {
    config: Arc<RwLock<Config>>,
    main_hwnds: Vec<HWND>,
    original: HashMap<isize, WindowState>,
}

// SAFETY: HWND values are opaque Win32 handles that may be passed between
// threads. AdBlocker mutates its cached handles and restore state only through
// external synchronization (`Mutex<AdBlocker>` in main), so moving it to the
// watcher/reapply worker threads is sound.
unsafe impl Send for AdBlocker {}

impl AdBlocker {
    pub fn new(config: Arc<RwLock<Config>>) -> Self {
        Self {
            config,
            main_hwnds: Vec::new(),
            original: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.main_hwnds.clear();
        self.original.clear();
    }

    pub fn has_pending_restore(&self) -> bool {
        !self.original.is_empty()
    }

    pub fn apply_all(&mut self) {
        let cfg = self.config.read().unwrap().clone();

        let mains = self.locate_main_windows();
        if mains.is_empty() {
            debug!("apply_all: KakaoTalk main windows not found");
            return;
        }

        for main in mains {
            self.apply_to_main(main, &cfg);
        }
    }

    fn apply_to_main(&mut self, main: HWND, cfg: &Config) {
        if is_iconic(main) {
            return;
        }

        // KakaoTalk can sit in the tray with its main window hidden but not
        // iconic. OnlineMainView is a CEF-backed compositor surface; resizing
        // it while the window is hidden leaves it black — and the hidden banner
        // slot white — once the window is later shown, because CEF only repaints
        // on its own paint cycle, not on our SetWindowPos/RedrawWindow. Defer
        // until the window is actually visible; the WinEvent foreground/show
        // hooks and the periodic fallback reapply as soon as it appears.
        if !is_window_visible(main) {
            debug!("apply_all: main window hidden (tray); deferring until visible");
            return;
        }

        let pid = window_pid(main);
        if pid == 0 || !process_image_is(pid, KAKAOTALK_EXE) {
            warn!("apply_all: main hwnd PID/image verification failed");
            return;
        }

        if cfg.ad_block_banner {
            for ad in self.find_main_window_ads(main, pid) {
                self.remove_ad(&ad, main);
            }
        }

        if cfg.ad_block_popup {
            for ad in find_popup_ads(main, pid) {
                self.remove_ad(&ad, main);
            }
        }
    }

    pub fn restore_all(&mut self) {
        if self.original.is_empty() {
            return;
        }

        let _ = self.locate_main_windows();
        let ready_keys: Vec<_> = self
            .original
            .iter()
            .filter_map(|(&key, state)| {
                let main = HWND(state.main as *mut c_void);
                if !is_window(main) || (!is_iconic(main) && is_window_visible(main)) {
                    Some(key)
                } else {
                    None
                }
            })
            .collect();
        let mut states: Vec<_> = ready_keys
            .into_iter()
            .filter_map(|key| self.original.remove(&key).map(|state| (key, state)))
            .collect();
        states.sort_by_key(|(_, state)| match state.kind {
            RestoreKind::ResizedPane => 0,
            RestoreKind::HiddenWindow => 1,
        });

        let mut restored_mains = HashSet::new();

        for (raw, state) in states {
            restored_mains.insert(state.main);
            let hwnd = HWND(raw as *mut c_void);
            if !is_window(hwnd) {
                continue;
            }

            match state.kind {
                RestoreKind::ResizedPane => {
                    let width = state.rect.right - state.rect.left;
                    let height = state.rect.bottom - state.rect.top;
                    if width > 0 && height > 0 {
                        // SAFETY: `hwnd` is live. We only restore the saved
                        // size because child-window x/y values from
                        // GetWindowRect are in screen coordinates, while
                        // SetWindowPos expects parent client coordinates for
                        // children.
                        let _ = unsafe {
                            SetWindowPos(
                                hwnd,
                                Some(HWND_TOP),
                                0,
                                0,
                                width,
                                height,
                                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE,
                            )
                        };
                    }
                }
                RestoreKind::HiddenWindow => {
                    let cmd = if state.visible { SW_SHOW } else { SW_HIDE };
                    // SAFETY: `hwnd` is live and ShowWindow accepts any valid
                    // HWND. Restore hidden ad windows only after resized panes
                    // are back to their original size, so compositor-backed
                    // ad children do not paint over the expanded main view.
                    let _ = unsafe { ShowWindow(hwnd, cmd) };
                    update_window(hwnd);
                }
            }
        }

        for raw in restored_mains {
            let main = HWND(raw as *mut c_void);
            if is_window(main) {
                update_window(main);
            }
        }
    }

    fn locate_main_windows(&mut self) -> Vec<HWND> {
        let discovered = find_kakaotalk_windows();
        for hwnd in &discovered {
            if !self.main_hwnds.iter().any(|known| known.0 == hwnd.0) {
                info!("acquired KakaoTalk main hwnd: {:?}", hwnd.0);
            }
        }
        self.main_hwnds = discovered.clone();
        discovered
    }

    fn find_main_window_ads(&self, main: HWND, pid: u32) -> Vec<AdWindow> {
        let main_rect = match window_rect(main) {
            Some(r) => r,
            None => return Vec::new(),
        };
        let main_width = main_rect.right - main_rect.left;
        let main_height = main_rect.bottom - main_rect.top;
        if main_width <= 0 || main_height <= 0 {
            return Vec::new();
        }

        let children = enum_direct_children(main);
        let mut out = Vec::new();
        let mut has_known_main_view = false;

        for &child in &children {
            if window_pid(child) != pid {
                continue;
            }

            let class = class_name(child);
            if class != "EVA_ChildWindow" {
                continue;
            }

            let text = window_text(child);
            if text.starts_with("OnlineMainView") {
                has_known_main_view = true;
                out.push(AdWindow::MainView {
                    hwnd: child,
                    width: main_width - LAYOUT_SHADOW_PADDING,
                    height: main_height - MAIN_VIEW_BOTTOM_PADDING,
                });
            } else if text.starts_with("LockModeView") {
                has_known_main_view = true;
                out.push(AdWindow::MainView {
                    hwnd: child,
                    width: main_width - LAYOUT_SHADOW_PADDING,
                    height: main_height,
                });
            }
        }

        if has_known_main_view {
            for child in children {
                if self.is_direct_banner_slot(main, child, pid, &main_rect) {
                    out.push(AdWindow::BannerSlot { hwnd: child });
                }
            }
        }

        out
    }

    fn is_direct_banner_slot(&self, main: HWND, child: HWND, pid: u32, main_rect: &RECT) -> bool {
        if window_pid(child) != pid || !is_window_visible(child) {
            return false;
        }
        if parent_hwnd(child).map(|p| p.0) != Some(main.0) {
            return false;
        }
        let class = class_name(child);
        let is_banner_class = class == "EVA_ChildWindow" || class == "EVA_Window_Dblclk";
        if !is_banner_class || !window_text(child).is_empty() {
            return false;
        }
        // KakaoTalk uses _EVA_ descendants for custom scroll surfaces. The
        // reference implementation avoids closing these, and we avoid hiding
        // them for the same false-positive reason.
        if has_descendant_class_prefix(child, "_EVA_") {
            return false;
        }

        let rect = match window_rect(child) {
            Some(r) => r,
            None => return false,
        };
        is_bottom_banner_rect(main_rect, &rect)
    }

    fn remove_ad(&mut self, ad: &AdWindow, main: HWND) {
        match *ad {
            AdWindow::MainView {
                hwnd,
                width,
                height,
            } => {
                if !is_window(hwnd) || width <= 0 || height <= 0 {
                    return;
                }

                let current = match window_rect(hwnd) {
                    Some(r) => r,
                    None => return,
                };
                let current_w = current.right - current.left;
                let current_h = current.bottom - current.top;
                if current_w == width && current_h == height {
                    return;
                }

                self.save_state(hwnd, RestoreKind::ResizedPane, main);
                update_window(hwnd);
                // SAFETY: `hwnd` is live. SWP_NOMOVE avoids mixing screen
                // and parent-client coordinates. This is a child main-view
                // window, so moving it to the top of its sibling stack lets
                // the expanded content cover the old banner slot without
                // affecting top-level KakaoTalk/chat-window ordering.
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        Some(HWND_TOP),
                        0,
                        0,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOMOVE,
                    )
                };
                update_window(main);
            }
            AdWindow::BannerSlot { hwnd } | AdWindow::Popup { hwnd } => {
                if !is_window(hwnd) || !is_window_visible(hwnd) {
                    return;
                }
                self.save_state(hwnd, RestoreKind::HiddenWindow, main);
                // SAFETY: `hwnd` is live and this is the documented way to
                // hide a child/top-level window without clipping the CEF
                // compositor tree into a black frame.
                let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
                update_window(main);
            }
        }
    }

    fn save_state(&mut self, hwnd: HWND, kind: RestoreKind, main: HWND) {
        let key = hwnd.0 as isize;
        if self.original.contains_key(&key) {
            return;
        }

        if let Some(rect) = window_rect(hwnd) {
            self.original.insert(
                key,
                WindowState {
                    rect,
                    visible: is_window_visible(hwnd),
                    kind,
                    main: main.0 as isize,
                },
            );
        }
    }
}

pub fn find_kakaotalk_windows() -> Vec<HWND> {
    let mut candidates = MainWindowCandidates::default();
    let lparam = LPARAM(&mut candidates as *mut MainWindowCandidates as isize);
    // SAFETY: `candidates` outlives this synchronous EnumWindows call; the
    // callback only appends verified HWND values through `lparam`.
    let _ = unsafe { EnumWindows(Some(collect_main_window_candidate), lparam) };

    if candidates.exact.is_empty() {
        candidates.fallback
    } else {
        candidates.exact
    }
}

#[derive(Default)]
struct MainWindowCandidates {
    exact: Vec<HWND>,
    fallback: Vec<HWND>,
}

unsafe extern "system" fn collect_main_window_candidate(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let pid = window_pid(hwnd);
    if pid == 0 || !process_image_is(pid, KAKAOTALK_EXE) {
        return BOOL(1);
    }

    let candidates = lparam.0 as *mut MainWindowCandidates;
    let title = window_text(hwnd);
    if is_kakaotalk_main_title(&title) {
        // SAFETY: `candidates` points to the live context passed by
        // find_kakaotalk_windows during this synchronous enumeration.
        unsafe { (*candidates).exact.push(hwnd) };
        return BOOL(1);
    }

    if !is_window_visible(hwnd) {
        return BOOL(1);
    }
    if (get_style(hwnd) as u32) & WS_POPUP.0 != 0 {
        return BOOL(1);
    }
    if class_name(hwnd) != "EVA_Window_Dblclk"
        || !enum_direct_children(hwnd).into_iter().any(|child| {
            window_pid(child) == pid
                && class_name(child) == "EVA_ChildWindow"
                && matches!(
                    window_text(child).as_str(),
                    text if text.starts_with("OnlineMainView") || text.starts_with("LockModeView")
                )
        })
    {
        return BOOL(1);
    }

    // SAFETY: Same live context invariant as above.
    unsafe { (*candidates).fallback.push(hwnd) };
    BOOL(1)
}

fn is_kakaotalk_main_title(title: &str) -> bool {
    matches!(title, "카카오톡" | "KakaoTalk" | "カカオトーク")
}

fn enum_descendants(parent: HWND) -> Vec<HWND> {
    let mut buf: Vec<HWND> = Vec::new();
    let lparam = LPARAM(&mut buf as *mut Vec<HWND> as isize);
    // SAFETY: `buf` outlives this synchronous EnumChildWindows call; the
    // callback writes into the live Vec via `lparam`.
    let _ = unsafe { EnumChildWindows(Some(parent), Some(collect_descendant), lparam) };
    buf
}

unsafe extern "system" fn collect_descendant(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let buf = lparam.0 as *mut Vec<HWND>;
    // SAFETY: `buf` points to the live Vec passed by enum_descendants.
    unsafe { (*buf).push(hwnd) };
    BOOL(1)
}

fn enum_direct_children(parent: HWND) -> Vec<HWND> {
    enum_descendants(parent)
        .into_iter()
        .filter(|h| parent_hwnd(*h).map(|p| p.0) == Some(parent.0))
        .collect()
}

fn has_descendant_class_prefix(hwnd: HWND, prefix: &str) -> bool {
    enum_descendants(hwnd)
        .into_iter()
        .any(|child| class_name(child).starts_with(prefix))
}

fn has_chrome_descendant(hwnd: HWND) -> bool {
    enum_descendants(hwnd).into_iter().any(|child| {
        let text = window_text(child);
        text == "Chrome Legacy Window"
    })
}

fn is_bottom_banner_rect(main_rect: &RECT, rect: &RECT) -> bool {
    let main_width = main_rect.right - main_rect.left;
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let near_bottom = (main_rect.bottom - rect.bottom).abs() <= DIRECT_BANNER_BOTTOM_TOLERANCE_PX;
    let wide_enough = (width as f32) >= (main_width as f32) * DIRECT_BANNER_MIN_WIDTH_RATIO;
    let exact_height = (height - DIRECT_BANNER_HEIGHT).abs() <= DIRECT_BANNER_HEIGHT_TOLERANCE_PX;

    near_bottom && wide_enough && exact_height
}

struct PopupCtx {
    main_key: isize,
    main_rect: RECT,
    pid: u32,
    seen: HashSet<isize>,
    out: Vec<AdWindow>,
}

fn find_popup_ads(main_hwnd: HWND, pid: u32) -> Vec<AdWindow> {
    let main_rect = match window_rect(main_hwnd) {
        Some(r) => r,
        None => return Vec::new(),
    };
    let mut ctx = PopupCtx {
        main_key: main_hwnd.0 as isize,
        main_rect,
        pid,
        seen: HashSet::new(),
        out: Vec::new(),
    };
    let lparam = LPARAM(&mut ctx as *mut PopupCtx as isize);
    // SAFETY: ctx outlives this synchronous EnumWindows call.
    let _ = unsafe { EnumWindows(Some(collect_popup), lparam) };
    let thread_id = window_thread_id(main_hwnd);
    if thread_id != 0 {
        // SAFETY: ctx still outlives this synchronous EnumThreadWindows call.
        let _ = unsafe { EnumThreadWindows(thread_id, Some(collect_popup), lparam) };
    }
    for hwnd in enum_related_process_windows(main_hwnd, pid) {
        let already_selected = ctx.out.iter().any(|ad| ad.hwnd().0 == hwnd.0);
        if !already_selected && is_bottom_banner_frame(hwnd, ctx.main_key, &ctx.main_rect) {
            ctx.out.push(AdWindow::BannerSlot { hwnd });
        }
    }
    ctx.out
}

fn enum_related_process_windows(main_hwnd: HWND, pid: u32) -> Vec<HWND> {
    struct Ctx {
        pid: u32,
        out: Vec<HWND>,
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam points at the live Ctx created in
        // enum_related_process_windows.
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };
        if window_pid(hwnd) == ctx.pid {
            ctx.out.push(hwnd);
        }
        BOOL(1)
    }

    let mut ctx = Ctx {
        pid,
        out: Vec::new(),
    };
    let lparam = LPARAM(&mut ctx as *mut Ctx as isize);
    // SAFETY: ctx outlives this synchronous EnumWindows call.
    let _ = unsafe { EnumWindows(Some(collect), lparam) };
    let thread_id = window_thread_id(main_hwnd);
    if thread_id != 0 {
        // SAFETY: ctx outlives this synchronous EnumThreadWindows call.
        let _ = unsafe { EnumThreadWindows(thread_id, Some(collect), lparam) };
    }
    let parents = ctx.out.clone();
    for parent in parents {
        for child in enum_descendants(parent) {
            if window_pid(child) == pid {
                ctx.out.push(child);
            }
        }
    }
    ctx.out.sort_by_key(|h| h.0 as isize);
    ctx.out.dedup_by_key(|h| h.0 as isize);
    ctx.out
}

fn is_bottom_banner_frame(hwnd: HWND, main_key: isize, main_rect: &RECT) -> bool {
    if !is_window_visible(hwnd) {
        return false;
    }
    if class_name(hwnd) != "EVA_Window_Dblclk" || !window_text(hwnd).is_empty() {
        return false;
    }
    if parent_hwnd(hwnd).map(|p| p.0 as isize) != Some(main_key) {
        return false;
    }
    let rect = match window_rect(hwnd) {
        Some(r) => r,
        None => return false,
    };
    is_bottom_banner_rect(main_rect, &rect)
}

unsafe extern "system" fn collect_popup(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` points at the live PopupCtx on find_popup_ads's
    // stack frame; the callback runs synchronously.
    let ctx = unsafe { &mut *(lparam.0 as *mut PopupCtx) };
    let key = hwnd.0 as isize;

    if key == ctx.main_key || !ctx.seen.insert(key) {
        return BOOL(1);
    }
    if !is_window_visible(hwnd) {
        return BOOL(1);
    }
    if window_pid(hwnd) != ctx.pid {
        return BOOL(1);
    }
    if (get_style(hwnd) as u32) & WS_POPUP.0 == 0 {
        return BOOL(1);
    }

    let rect = match window_rect(hwnd) {
        Some(r) => r,
        None => return BOOL(1),
    };
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if !(POPUP_MIN_DIM..=POPUP_MAX_DIM).contains(&w)
        || !(POPUP_MIN_DIM..=POPUP_MAX_DIM).contains(&h)
    {
        return BOOL(1);
    }

    let title = window_text(hwnd);
    let class = class_name(hwnd);
    let parent = parent_hwnd(hwnd);
    let parent_key = parent.map(|p| p.0 as isize);
    let is_bottom_banner_frame = class == "EVA_Window_Dblclk"
        && title.is_empty()
        && parent_key == Some(ctx.main_key)
        && is_bottom_banner_rect(&ctx.main_rect, &rect);
    if is_bottom_banner_frame {
        debug!(
            "bottom banner frame candidate hwnd={:?} size={}x{} title={:?}",
            hwnd.0, w, h, title
        );
        ctx.out.push(AdWindow::BannerSlot { hwnd });
        return BOOL(1);
    }

    let is_kakaotalk_ad_popup_shape = match class.as_str() {
        "EVA_Window" => title.is_empty() && parent.is_none(),
        "EVA_Window_Dblclk" => title.is_empty() && parent_key == Some(ctx.main_key),
        _ => false,
    };
    if !is_kakaotalk_ad_popup_shape {
        return BOOL(1);
    }
    if !has_chrome_descendant(hwnd) {
        return BOOL(1);
    }

    debug!(
        "popup candidate hwnd={:?} size={}x{} title={:?}",
        hwnd.0, w, h, title
    );
    ctx.out.push(AdWindow::Popup { hwnd });
    BOOL(1)
}

fn window_thread_id(hwnd: HWND) -> u32 {
    // SAFETY: Passing None for the PID out parameter is documented; the
    // return value is the owning thread id or 0 for invalid HWNDs.
    unsafe { GetWindowThreadProcessId(hwnd, None) }
}

fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a live stack RECT. GetWindowRect either fills it
    // and returns Ok or leaves it untouched and returns Err.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok().map(|_| rect)
}

fn is_window(hwnd: HWND) -> bool {
    // SAFETY: IsWindow accepts any handle value and returns FALSE for
    // invalid handles.
    unsafe { IsWindow(Some(hwnd)) }.as_bool()
}

fn is_iconic(hwnd: HWND) -> bool {
    // SAFETY: IsIconic accepts any HWND value.
    unsafe { IsIconic(hwnd) }.as_bool()
}

fn get_style(hwnd: HWND) -> i32 {
    // SAFETY: GetWindowLongW reads a 32-bit per-window slot.
    unsafe { GetWindowLongW(hwnd, GWL_STYLE) }
}

fn parent_hwnd(hwnd: HWND) -> Option<HWND> {
    // SAFETY: GetParent accepts any HWND and returns no parent/invalid via
    // Err or an invalid HWND.
    match unsafe { GetParent(hwnd) } {
        Ok(p) if !p.is_invalid() => Some(p),
        _ => None,
    }
}

fn update_window(hwnd: HWND) {
    if !is_window(hwnd) {
        return;
    }
    const FLAGS: REDRAW_WINDOW_FLAGS =
        REDRAW_WINDOW_FLAGS(RDW_INVALIDATE.0 | RDW_ERASE.0 | RDW_ALLCHILDREN.0 | RDW_UPDATENOW.0);
    // SAFETY: `hwnd` was verified live; None update-rect and update-region
    // mean "the entire window".
    let _ = unsafe { RedrawWindow(Some(hwnd), None, None, FLAGS) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_banner_rect_matches_expected_geometry() {
        let main = rect(100, 100, 700, 900);
        let banner = rect(120, 809, 680, 900);

        assert!(is_bottom_banner_rect(&main, &banner));
    }

    #[test]
    fn bottom_banner_rect_allows_small_height_and_bottom_tolerance() {
        let main = rect(0, 0, 600, 800);

        assert!(is_bottom_banner_rect(&main, &rect(0, 707, 600, 798)));
        assert!(is_bottom_banner_rect(&main, &rect(0, 709, 600, 802)));
    }

    #[test]
    fn bottom_banner_rect_rejects_wrong_height_bottom_or_width() {
        let main = rect(0, 0, 600, 800);

        assert!(!is_bottom_banner_rect(&main, &rect(0, 700, 600, 800)));
        assert!(!is_bottom_banner_rect(&main, &rect(0, 709, 600, 780)));
        assert!(!is_bottom_banner_rect(&main, &rect(0, 709, 419, 800)));
    }

    #[test]
    fn main_title_matches_supported_locales_only() {
        for title in ["카카오톡", "KakaoTalk", "カカオトーク"] {
            assert!(is_kakaotalk_main_title(title));
        }

        for title in ["", "KakaoTalk.exe", "카카오톡 설정", "KakaoTalk Chat"] {
            assert!(!is_kakaotalk_main_title(title));
        }
    }

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Ignored by default; meaningful only when KakaoTalk is running on the
    /// host. Run with:
    ///     cargo test -- --ignored --nocapture adblocker_live
    #[test]
    #[ignore]
    fn adblocker_live() {
        let _ = env_logger::builder()
            .filter_module("cleankakao", log::LevelFilter::Debug)
            .is_test(true)
            .try_init();

        let cfg = Arc::new(RwLock::new(Config::default()));
        let mut blocker = AdBlocker::new(cfg);

        let hwnds = find_kakaotalk_windows();
        println!("find_kakaotalk_windows() = {hwnds:?}");
        let hwnd = hwnds
            .first()
            .copied()
            .expect("KakaoTalk main window not found - is it running?");

        let pid = window_pid(hwnd);
        println!("main hwnd pid = {pid}");
        assert!(pid != 0);

        blocker.apply_all();
        println!("saved {} window states:", blocker.original.len());
        for (raw, state) in &blocker.original {
            let h = HWND(*raw as *mut c_void);
            println!(
                "  hwnd=0x{:X} class={} title={:?} rect={:?} visible_before={}",
                *raw,
                class_name(h),
                window_text(h),
                state.rect,
                state.visible
            );
        }

        blocker.restore_all();
        assert!(blocker.original.is_empty());
    }

    /// Diagnostic test that leaves the ad-blocked layout visible for a short
    /// window so it can be checked by eye.
    #[test]
    #[ignore]
    fn adblocker_live_hold() {
        let _ = env_logger::builder()
            .filter_module("cleankakao", log::LevelFilter::Debug)
            .is_test(true)
            .try_init();

        let cfg = Arc::new(RwLock::new(Config::default()));
        let mut blocker = AdBlocker::new(cfg);

        assert!(
            !find_kakaotalk_windows().is_empty(),
            "KakaoTalk main window not found - is it running?"
        );

        blocker.apply_all();
        println!("saved {} window states:", blocker.original.len());
        for (raw, state) in &blocker.original {
            let h = HWND(*raw as *mut c_void);
            println!(
                "  before hwnd=0x{:X} class={} title={:?} rect={:?} visible={}",
                *raw,
                class_name(h),
                window_text(h),
                state.rect,
                state.visible
            );
            println!(
                "   after hwnd=0x{:X} rect={:?} visible={}",
                *raw,
                window_rect(h),
                is_window_visible(h)
            );
        }
        println!("ad-blocked layout is being held for 10 seconds");
        std::thread::sleep(std::time::Duration::from_secs(10));
        blocker.restore_all();
    }
}
