use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::PWSTR;

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is a live stack array; GetClassNameW writes at most
    // buf.len() - 1 UTF-16 code units.
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    if len == 0 {
        return String::new();
    }
    let raw = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
    normalize_sandboxie_class_name(&raw).to_owned()
}

fn normalize_sandboxie_class_name(class: &str) -> &str {
    class
        .strip_prefix("Sandbox:")
        .and_then(|rest| rest.split_once(':').map(|(_, original)| original))
        .filter(|original| !original.is_empty())
        .unwrap_or(class)
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

pub fn window_pid(hwnd: HWND) -> u32 {
    let mut pid: u32 = 0;
    // SAFETY: `pid` is a live stack u32. GetWindowThreadProcessId accepts a
    // HWND and writes the owning process id when available.
    let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

pub fn process_image_is(pid: u32, target_exe: &str) -> bool {
    // SAFETY: OpenProcess with PROCESS_QUERY_LIMITED_INFORMATION is the
    // documented minimum access for QueryFullProcessImageNameW.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        Err(_) => return false,
    };

    let mut buf = [0u16; 1024];
    let mut size: u32 = buf.len() as u32;
    // SAFETY: `handle` is live; `size` bounds the write into `buf`.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
    }
    .is_ok();
    // SAFETY: `handle` was returned by OpenProcess and is not used after
    // CloseHandle.
    let _ = unsafe { CloseHandle(handle) };

    if !ok || size == 0 {
        return false;
    }
    let full = String::from_utf16_lossy(&buf[..size as usize]);
    full.rsplit(['\\', '/'])
        .next()
        .map(|name| name.eq_ignore_ascii_case(target_exe))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_sandboxie_class_prefix() {
        assert_eq!(
            normalize_sandboxie_class_name("Sandbox:kakaotalk:EVA_Window_Dblclk"),
            "EVA_Window_Dblclk"
        );
        assert_eq!(
            normalize_sandboxie_class_name("Sandbox:box:Chrome_WidgetWin_1"),
            "Chrome_WidgetWin_1"
        );
    }

    #[test]
    fn preserves_regular_and_malformed_class_names() {
        assert_eq!(
            normalize_sandboxie_class_name("EVA_ChildWindow"),
            "EVA_ChildWindow"
        );
        assert_eq!(
            normalize_sandboxie_class_name("Sandbox:missing"),
            "Sandbox:missing"
        );
        assert_eq!(
            normalize_sandboxie_class_name("Sandbox:box:"),
            "Sandbox:box:"
        );
    }
}
