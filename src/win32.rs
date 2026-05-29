use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowTextW, IsWindowVisible};

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is a live stack array; GetClassNameW writes at most
    // buf.len() - 1 UTF-16 code units.
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    if len == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len.min(buf.len())])
}

pub fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    // SAFETY: `buf` is a live stack array; GetWindowTextW writes at most
    // buf.len() - 1 UTF-16 code units.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    if len == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len.min(buf.len())])
}

pub fn is_window_visible(hwnd: HWND) -> bool {
    // SAFETY: IsWindowVisible accepts any HWND value.
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}
