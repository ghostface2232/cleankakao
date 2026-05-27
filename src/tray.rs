// System tray icon and menu wiring.

pub struct Tray {
    // TODO: hold TrayIcon and Menu handles.
}

#[derive(Debug, Clone, Copy)]
pub enum TrayEvent {
    OpenSettings,
    ToggleBlocking,
    Quit,
}

impl Tray {
    /// Build the tray icon, menu, and event channel.
    pub fn new() -> Self {
        Self {}
    }

    /// Switch between active / inactive tray icon variants.
    pub fn set_active(&mut self, _active: bool) {
        // TODO: swap icon resource.
    }
}

impl Default for Tray {
    fn default() -> Self {
        Self::new()
    }
}
