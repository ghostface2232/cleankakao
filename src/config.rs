// Application configuration: load/save TOML, defaults, paths.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub auto_start: bool,
    pub block_ads: bool,
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_start: false,
            block_ads: true,
            check_updates: true,
        }
    }
}

impl Config {
    /// Resolve the on-disk config file path (per-user app data).
    pub fn path() -> Option<PathBuf> {
        // TODO: dirs::config_dir().join("cleankakao/config.toml").
        None
    }

    /// Load config from disk, falling back to defaults on error.
    pub fn load() -> Self {
        Self::default()
    }

    /// Persist config to disk.
    pub fn save(&self) -> std::io::Result<()> {
        // TODO: serialize to TOML and write atomically.
        Ok(())
    }
}
