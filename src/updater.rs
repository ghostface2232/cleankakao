// GitHub Releases update checker and Windows toast notification.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use log::{debug, info, warn};
use semver::Version;
use serde::Deserialize;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::core::HSTRING;

use crate::config::Config;

pub const DEFAULT_REPO_OWNER: &str = "ghostface2232";
pub const DEFAULT_REPO_NAME: &str = "cleankakao";
pub const DEFAULT_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

const USER_AGENT: &str = concat!("CleanKakao/", env!("CARGO_PKG_VERSION"));
const RELEASES_URL: &str = "https://github.com/ghostface2232/cleankakao/releases";
const TOAST_APP_ID: &str = "CleanKakao";

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: Version,
    pub download_url: String,
    pub release_notes: String,
    pub published_at: String,
}

#[derive(Clone)]
pub struct UpdateChecker {
    pub repo_owner: &'static str,
    pub repo_name: &'static str,
    pub current_version: Version,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    published_at: Option<String>,
}

impl UpdateChecker {
    pub fn current() -> Self {
        let current_version = Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|e| {
            warn!("failed to parse current version: {e}");
            Version::new(0, 0, 0)
        });

        Self::new(DEFAULT_REPO_OWNER, DEFAULT_REPO_NAME, current_version)
    }

    pub fn new(
        repo_owner: &'static str,
        repo_name: &'static str,
        current_version: Version,
    ) -> Self {
        Self {
            repo_owner,
            repo_name,
            current_version,
            client: reqwest::Client::new(),
        }
    }

    pub async fn check_latest_version(&self) -> Option<ReleaseInfo> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            self.repo_owner, self.repo_name
        );

        let response = match self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
        {
            Ok(response) => response,
            Err(e) => {
                warn!("update check request failed: {e}");
                return None;
            }
        };

        if !response.status().is_success() {
            warn!("update check returned HTTP {}", response.status());
            return None;
        }

        let latest = match response.json::<GitHubRelease>().await {
            Ok(release) => release,
            Err(e) => {
                warn!("failed to parse update response: {e}");
                return None;
            }
        };

        parse_release_info(latest, &self.current_version)
    }

    pub fn notify_update(&self, info: &ReleaseInfo) {
        let note = release_note_preview(&info.release_notes);
        let body = if note.is_empty() {
            format!(
                "새 버전 {}이 있습니다. 클릭해서 GitHub Releases 페이지를 엽니다.",
                info.version
            )
        } else {
            format!(
                "새 버전 {}이 있습니다. {}\n클릭해서 GitHub Releases 페이지를 엽니다.",
                info.version, note
            )
        };

        if let Err(e) = show_windows_toast("CleanKakao 업데이트", &body, &info.download_url) {
            warn!("failed to show update toast: {e}");
        }
    }

    pub fn start_periodic_check(
        self,
        interval: Duration,
        config: Arc<RwLock<Config>>,
        app_running: Arc<AtomicBool>,
    ) -> thread::JoinHandle<()> {
        thread::Builder::new()
            .name("update-checker".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        warn!("failed to create tokio runtime for updater: {e}");
                        return;
                    }
                };

                while app_running.load(Ordering::Acquire) {
                    let check_enabled = config
                        .read()
                        .map(|config| config.check_update)
                        .unwrap_or_else(|e| {
                            warn!("failed to read update config: {e}");
                            false
                        });

                    if check_enabled {
                        if let Some(info) = runtime.block_on(self.check_latest_version()) {
                            info!(
                                "update available: {}, published_at={}",
                                info.version, info.published_at
                            );
                            self.notify_update(&info);
                        }
                    } else {
                        debug!("automatic update check skipped by config");
                    }

                    wait_for_next_check(interval, &app_running);
                }
            })
            .expect("failed to spawn update-checker thread")
    }
}

fn wait_for_next_check(interval: Duration, app_running: &AtomicBool) {
    let mut remaining = interval;
    while app_running.load(Ordering::Acquire) && !remaining.is_zero() {
        let step = remaining.min(Duration::from_secs(60));
        thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
}

fn show_windows_toast(title: &str, body: &str, launch_url: &str) -> windows::core::Result<()> {
    let xml = format!(
        r#"<toast activationType="protocol" launch="{url}">
    <visual>
        <binding template="ToastGeneric">
            <text>{title}</text>
            <text>{body}</text>
        </binding>
    </visual>
    <actions>
        <action content="GitHub Releases 열기" arguments="{url}" activationType="protocol"/>
    </actions>
</toast>"#,
        title = escape_xml(title),
        body = escape_xml(body),
        url = escape_xml(launch_url)
    );

    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(TOAST_APP_ID))?;
    notifier.Show(&toast)
}

fn parse_release_info(latest: GitHubRelease, current_version: &Version) -> Option<ReleaseInfo> {
    let tag = latest.tag_name.trim_start_matches(['v', 'V']);
    let version = match Version::parse(tag) {
        Ok(version) => version,
        Err(e) => {
            warn!(
                "failed to parse latest release tag '{}': {e}",
                latest.tag_name
            );
            return None;
        }
    };

    if version <= *current_version {
        debug!(
            "no update available: current={}, latest={}",
            current_version, version
        );
        return None;
    }

    Some(ReleaseInfo {
        version,
        download_url: if latest.html_url.is_empty() {
            RELEASES_URL.to_string()
        } else {
            latest.html_url
        },
        release_notes: latest.body.unwrap_or_default(),
        published_at: latest.published_at.unwrap_or_default(),
    })
}

fn release_note_preview(release_notes: &str) -> String {
    release_notes
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(90)
        .collect()
}

fn escape_xml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_newer_release_info() {
        let info = parse_release_info(
            GitHubRelease {
                tag_name: "v1.2.3".to_string(),
                html_url: "https://example.test/release".to_string(),
                body: Some("notes".to_string()),
                published_at: Some("2026-05-28T00:00:00Z".to_string()),
            },
            &Version::parse("1.2.2").unwrap(),
        )
        .unwrap();

        assert_eq!(info.version, Version::parse("1.2.3").unwrap());
        assert_eq!(info.download_url, "https://example.test/release");
        assert_eq!(info.release_notes, "notes");
        assert_eq!(info.published_at, "2026-05-28T00:00:00Z");
    }

    #[test]
    fn ignores_same_or_older_release() {
        let current = Version::parse("1.2.3").unwrap();

        assert!(parse_release_info(release("1.2.3"), &current).is_none());
        assert!(parse_release_info(release("1.2.2"), &current).is_none());
    }

    #[test]
    fn ignores_invalid_release_tag() {
        assert!(parse_release_info(release("latest"), &Version::parse("1.0.0").unwrap()).is_none());
    }

    #[test]
    fn falls_back_to_releases_url_when_html_url_is_empty() {
        let mut release = release("1.0.1");
        release.html_url.clear();

        let info = parse_release_info(release, &Version::parse("1.0.0").unwrap()).unwrap();

        assert_eq!(info.download_url, RELEASES_URL);
    }

    #[test]
    fn previews_first_non_empty_release_note_line() {
        assert_eq!(
            release_note_preview("\n  \n  First line\nSecond line"),
            "First line"
        );

        let long = "a".repeat(120);
        assert_eq!(release_note_preview(&long).chars().count(), 90);
    }

    #[test]
    fn escapes_xml_special_characters() {
        assert_eq!(
            escape_xml("<tag a=\"b\">Tom & 'Jerry'</tag>"),
            "&lt;tag a=&quot;b&quot;&gt;Tom &amp; &apos;Jerry&apos;&lt;/tag&gt;"
        );
    }

    fn release(tag_name: &str) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag_name.to_string(),
            html_url: "https://example.test/release".to_string(),
            body: None,
            published_at: None,
        }
    }
}
