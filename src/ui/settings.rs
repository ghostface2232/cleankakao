// Settings window built with iced.

use crate::config::Config;

#[derive(Debug, Default)]
pub struct SettingsWindow {
    pub config: Config,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleAutoStart(bool),
    ToggleBlockAds(bool),
    ToggleCheckUpdates(bool),
    Save,
}

impl SettingsWindow {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn update(&mut self, _message: Message) {
        // TODO: handle messages and persist via Config::save.
    }

    // TODO: view() returning iced Element once iced is wired up.
}
