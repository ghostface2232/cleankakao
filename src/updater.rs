// Self-update: check remote manifest, download, swap binary.

use semver::Version;

pub struct Updater {
    pub current: Version,
}

#[derive(Debug)]
pub struct UpdateInfo {
    pub version: Version,
    pub download_url: String,
}

impl Updater {
    pub fn new(current: Version) -> Self {
        Self { current }
    }

    /// Query the release manifest and return an UpdateInfo if newer.
    pub async fn check(&self) -> Option<UpdateInfo> {
        // TODO: reqwest GET manifest, compare versions.
        None
    }

    /// Download and apply the given update.
    pub async fn apply(&self, _info: &UpdateInfo) -> std::io::Result<()> {
        // TODO: download to temp, verify, swap, relaunch.
        Ok(())
    }
}
