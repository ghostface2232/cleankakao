// Application configuration: load/save TOML, defaults, paths.

use log::{error, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ad_block_banner: bool,
    pub ad_block_popup: bool,
    pub auto_start: bool,
    pub check_update: bool,
    pub poll_interval_ms: u64,
    pub startup_delay_ms: u64,
    pub whitelist_keywords: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ad_block_banner: true,
            ad_block_popup: true,
            auto_start: false,
            check_update: true,
            poll_interval_ms: 1000,
            startup_delay_ms: 1000,
            whitelist_keywords: [
                "생일",
                "투표",
                "일정",
                "설정",
                "프로필",
                "알림",
                "동영상",
                "영상통화",
                "보이스톡",
                "페이스톡",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cleankakao")
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();

        if !path.exists() {
            let cfg = Self::default();
            if let Err(e) = cfg.save() {
                warn!(
                    "failed to write default config to {}: {}",
                    path.display(),
                    e
                );
            }
            return cfg;
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<Config>(&contents) {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!(
                        "failed to parse config {}: {} — using defaults",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(e) => {
                error!(
                    "failed to read config {}: {} — using defaults",
                    path.display(),
                    e
                );
                Self::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;

        let path = config_path();
        let contents = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, contents)
    }
}
