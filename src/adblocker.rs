// Core ad-blocking logic for KakaoTalk windows.

use windows::Win32::Foundation::HWND;

pub struct AdBlocker {
    // TODO: hold handles to discovered ad windows and timing state.
}

impl AdBlocker {
    pub fn new() -> Self {
        Self {}
    }

    /// Scan the given KakaoTalk top-level window for ad child windows.
    pub fn scan(&mut self, _kakao_hwnd: HWND) {
        // TODO: enumerate child windows and classify ad surfaces.
    }

    /// Hide or remove any ad windows discovered by the last scan.
    pub fn hide_ads(&mut self) {
        // TODO: apply hide / resize / overlay strategy.
    }
}

impl Default for AdBlocker {
    fn default() -> Self {
        Self::new()
    }
}
