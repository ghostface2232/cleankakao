// Application configuration: load/save TOML, defaults, paths.

use log::{error, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ad_block_banner: bool,
    pub ad_block_popup: bool,
    pub auto_start: bool,
    pub check_update: bool,
    pub poll_interval_ms: u64,
    pub startup_delay_ms: u64,
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
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Self {
        if !path.exists() {
            let cfg = Self::default();
            if let Err(e) = cfg.save_to_path(path) {
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
        let path = config_path();
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        let contents = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn missing_config_writes_defaults() {
        let (_dir, path) = temp_config_path("missing");

        let loaded = Config::load_from_path(&path);

        assert_eq!(loaded, Config::default());
        assert!(path.exists());
        assert_eq!(Config::load_from_path(&path), Config::default());
    }

    #[test]
    fn partial_config_uses_defaults_for_missing_fields() {
        let (_dir, path) = temp_config_path("partial");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"
ad_block_banner = false
auto_start = true
"#,
        )
        .unwrap();

        let loaded = Config::load_from_path(&path);

        assert!(!loaded.ad_block_banner);
        assert!(loaded.ad_block_popup);
        assert!(loaded.auto_start);
        assert!(loaded.check_update);
        assert_eq!(loaded.poll_interval_ms, 1000);
        assert_eq!(loaded.startup_delay_ms, 1000);
    }

    #[test]
    fn invalid_config_falls_back_to_defaults() {
        let (_dir, path) = temp_config_path("invalid");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "ad_block_banner = nope").unwrap();

        assert_eq!(Config::load_from_path(&path), Config::default());
    }

    #[test]
    fn save_to_path_round_trips() {
        let (_dir, path) = temp_config_path("roundtrip");
        let cfg = Config {
            ad_block_banner: false,
            ad_block_popup: false,
            auto_start: true,
            check_update: false,
            poll_interval_ms: 2500,
            startup_delay_ms: 1500,
        };

        cfg.save_to_path(&path).unwrap();

        assert_eq!(Config::load_from_path(&path), cfg);
    }

    fn temp_config_path(label: &str) -> (TempDir, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "cleankakao-config-test-{}-{label}-{unique}",
            std::process::id()
        ));
        let path = dir.join("config.toml");
        (TempDir(dir), path)
    }

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
