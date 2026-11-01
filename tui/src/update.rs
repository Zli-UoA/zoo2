//! GitHub Releases に新しい版が出ていないかを調べる。
//!
//! リリースは `tui-v<version>` のタグで作られる (CD TUI Release)。GitHub API の利用回数を
//! 抑えるため、結果を設定ディレクトリに覚えておき、[`CHECK_INTERVAL`] より新しければ問い合わせない。

use std::{path::Path, time::Duration};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const RELEASES_URL: &str = "https://api.github.com/repos/Zli-UoA/zoo2/releases?per_page=30";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const TAG_PREFIX: &str = "tui-v";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Cache {
    /// 最後に問い合わせた時刻 (Unix 秒)。
    checked_at: i64,
    latest: Option<String>,
}

/// 設定ディレクトリに置く、問い合わせ結果のファイル。
pub fn cache_path(dir: &Path) -> std::path::PathBuf {
    dir.join("update-check.json")
}

/// `current` より新しいリリースがあれば、その版 (`0.2.0` など) を返す。
/// `now` は Unix 秒。キャッシュが新しければ問い合わせない。
pub async fn check(
    http: &reqwest::Client,
    url: &str,
    cache: Option<&Path>,
    current: &str,
    now: i64,
) -> Result<Option<String>> {
    let cached = cache
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Cache>(&text).ok())
        .filter(|cache| {
            let age = now.saturating_sub(cache.checked_at);
            (0..CHECK_INTERVAL.as_secs() as i64).contains(&age)
        });
    let latest = match cached {
        Some(cache) => cache.latest,
        None => {
            let latest = fetch_latest(http, url).await?;
            if let Some(path) = cache {
                let cache = Cache {
                    checked_at: now,
                    latest: latest.clone(),
                };
                // 書けなくても次回また問い合わせるだけなので、通知は止めない。
                let _ = std::fs::write(path, serde_json::to_string(&cache)?);
            }
            latest
        }
    };
    Ok(latest.filter(|latest| is_newer(latest, current)))
}

/// 下書きとプレリリースを除いた、TUI のリリースのうち最も新しい版。
async fn fetch_latest(http: &reqwest::Client, url: &str) -> Result<Option<String>> {
    let releases: Vec<Release> = http
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            format!("zoo2-tui/{CURRENT_VERSION}"),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let version = release.tag_name.strip_prefix(TAG_PREFIX)?.to_string();
            Some((parse(&version)?, version))
        })
        .max()
        .map(|(_, version)| version))
}

fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.').map(|part| part.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse(latest), parse(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    use super::*;

    fn temp_cache() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("zoo2-tui-update-{}.json", uuid::Uuid::new_v4()))
    }

    async fn releases(server: &MockServer, expected_calls: u64) {
        Mock::given(method("GET"))
            .and(path("/releases"))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "tag_name": "tui-v0.10.0", "draft": true, "prerelease": false },
                { "tag_name": "tui-v0.9.0", "draft": false, "prerelease": true },
                { "tag_name": "v9.9.9", "draft": false, "prerelease": false },
                { "tag_name": "tui-v0.2.0", "draft": false, "prerelease": false },
                { "tag_name": "tui-v0.10.1-rc", "draft": false, "prerelease": false },
                { "tag_name": "tui-v0.3.0", "draft": false, "prerelease": false },
            ])))
            .expect(expected_calls)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn finds_newest_tui_release_and_caches_it() {
        let server = MockServer::start().await;
        releases(&server, 1).await;
        let url = format!("{}/releases", server.uri());
        let cache = temp_cache();
        let http = reqwest::Client::new();

        let found = check(&http, &url, Some(&cache), "0.1.0", 1_000)
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some("0.3.0"));

        // 間隔内はキャッシュを使い、問い合わせない。最新の版なら何も返さない。
        let found = check(&http, &url, Some(&cache), "0.3.0", 2_000)
            .await
            .unwrap();
        assert_eq!(found, None);
        let _ = std::fs::remove_file(cache);
    }

    #[tokio::test]
    async fn asks_again_after_the_interval() {
        let server = MockServer::start().await;
        releases(&server, 2).await;
        let url = format!("{}/releases", server.uri());
        let cache = temp_cache();
        let http = reqwest::Client::new();
        let later = 1_000 + CHECK_INTERVAL.as_secs() as i64;

        check(&http, &url, Some(&cache), "0.1.0", 1_000)
            .await
            .unwrap();
        let found = check(&http, &url, Some(&cache), "0.1.0", later)
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some("0.3.0"));
        let _ = std::fs::remove_file(cache);
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("broken", "0.1.0"));
    }
}
