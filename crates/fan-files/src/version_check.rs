use serde::Deserialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL_SECS: u64 = 86400;
const GITHUB_API: &str = "https://api.github.com/repos/AI4S-YB/fan-files/releases/latest";

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct VersionCache {
    last_check: u64,
    latest_version: String,
    release_url: String,
}

fn cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".fan-files/.version-cache")
}

pub fn spawn_check() {
    let current = crate::version::VERSION.to_string();
    std::thread::spawn(move || {
        let should_check = if let Ok(data) = std::fs::read_to_string(cache_path()) {
            if let Ok(cache) = serde_json::from_str::<VersionCache>(&data) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                now.saturating_sub(cache.last_check) >= CHECK_INTERVAL_SECS
            } else {
                true
            }
        } else {
            true
        };

        if !should_check {
            return;
        }

        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(5)))
            .timeout_global(Some(std::time::Duration::from_secs(10)))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let mut response = match agent
            .get(GITHUB_API)
            .header("User-Agent", "fan-files-version-check")
            .header("Accept", "application/vnd.github.v3+json")
            .call()
        {
            Ok(r) => r,
            Err(_) => return,
        };

        let release: GitHubRelease = match response.body_mut().read_json() {
            Ok(r) => r,
            Err(_) => return,
        };

        let cache = VersionCache {
            last_check: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            latest_version: release.tag_name.clone(),
            release_url: release.html_url,
        };
        if let Ok(json) = serde_json::to_string(&cache) {
            let _ = std::fs::create_dir_all(cache_path().parent().unwrap());
            std::fs::write(cache_path(), json).ok();
        }

        if release.tag_name != current && release.tag_name > current {
            eprintln!(
                "⚠  fan-files {} is outdated. Latest: {}. Run 'fan-files update' to upgrade.",
                current, release.tag_name
            );
        }
    });
}
