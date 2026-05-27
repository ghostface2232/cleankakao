// KakaoTalk process discovery and lifecycle watching.

use windows::Win32::Foundation::HWND;

pub struct ProcessWatcher {
    // TODO: cache last-known PID and HWND of KakaoTalk.
}

impl ProcessWatcher {
    pub fn new() -> Self {
        Self {}
    }

    /// Locate the running KakaoTalk main window, if any.
    pub fn find_kakao_window(&self) -> Option<HWND> {
        // TODO: walk top-level windows / process snapshot to find KakaoTalk.
        None
    }

    /// Returns true if KakaoTalk is currently running.
    pub fn is_running(&self) -> bool {
        self.find_kakao_window().is_some()
    }
}

impl Default for ProcessWatcher {
    fn default() -> Self {
        Self::new()
    }
}
