use super::transport;
use crate::auth::AuthUser;
use axum::{Json, extract::Query};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const BATCH_SIZE: usize = 12;
const CACHE_SIZE: usize = 1024;
const MAX_BODY: usize = 64 * 1024;
static CACHE: LazyLock<Mutex<HashMap<String, CachedColor>>> = LazyLock::new(Mutex::default);
static LOOKUPS: Semaphore = Semaphore::const_new(8);

struct CachedColor {
    color: Option<String>,
    expires: Instant,
}

#[derive(Deserialize)]
pub(super) struct Hosts {
    hosts: String,
}

pub(super) async fn read(
    _user: AuthUser,
    Query(query): Query<Hosts>,
) -> Json<HashMap<String, Option<String>>> {
    let mut seen = HashSet::new();
    let origins: Vec<_> = query
        .hosts
        .split(',')
        .take(BATCH_SIZE)
        .filter_map(|host| {
            let origin = origin(host)?;
            seen.insert(host).then_some((host.to_owned(), origin))
        })
        .collect();
    Json(
        stream::iter(origins)
            .map(|(host, origin)| async move {
                color_for(&host, origin).await.map(|color| (host, color))
            })
            .buffer_unordered(4)
            .filter_map(|result| async move { result })
            .collect()
            .await,
    )
}

fn origin(host: &str) -> Option<reqwest::Url> {
    if host.len() > 253
        || !host.contains('.')
        || !host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return None;
    }
    let url = reqwest::Url::parse(&format!("https://{host}/")).ok()?;
    (url.domain() == Some(host)).then_some(url)
}

async fn color_for(host: &str, origin: reqwest::Url) -> Option<Option<String>> {
    let cached = || {
        CACHE
            .lock()
            .unwrap()
            .get(host)
            .filter(|entry| entry.expires > Instant::now())
            .map(|entry| entry.color.clone())
    };
    if let Some(color) = cached() {
        return Some(color);
    }
    let _permit = LOOKUPS.try_acquire().ok()?;
    let color = tokio::time::timeout(Duration::from_secs(5), lookup(origin))
        .await
        .ok()
        .flatten();
    let mut cache = CACHE.lock().unwrap();
    if cache.len() >= CACHE_SIZE && !cache.contains_key(host) {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.expires)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
    }
    let ttl = if color.is_some() {
        Duration::from_secs(3600)
    } else {
        Duration::from_secs(120)
    };
    cache.insert(
        host.to_owned(),
        CachedColor {
            color: color.clone(),
            expires: Instant::now() + ttl,
        },
    );
    Some(color)
}

async fn lookup(origin: reqwest::Url) -> Option<String> {
    let client = transport::safe_client(&origin).await.ok()?;
    let meta = client
        .post(origin.join("api/meta").ok()?)
        .json(&serde_json::json!({"detail": false}));
    if let Some(color) = read_color(meta).await {
        return Some(color);
    }
    read_color(client.get(origin.join("api/v2/instance").ok()?)).await
}

async fn read_color(request: reqwest::RequestBuilder) -> Option<String> {
    let response = request
        .header("User-Agent", "Zoo-ActivityPub/1.0")
        .send()
        .await
        .ok()?;
    let metadata = transport::read_json(response, MAX_BODY).await.ok()?;
    theme_color(&metadata)
}

fn theme_color(metadata: &Value) -> Option<String> {
    [
        metadata["themeColor"].as_str(),
        metadata["theme_color"].as_str(),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| {
        let hex = value.strip_prefix('#')?;
        if !matches!(hex.len(), 3 | 6) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        if hex.len() == 3 {
            return Some(format!(
                "#{}",
                hex.chars().flat_map(|c| [c, c]).collect::<String>()
            ));
        }
        Some(value.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::{origin, theme_color};
    use serde_json::json;

    #[test]
    fn reads_instance_hex_colors() {
        assert_eq!(
            theme_color(&json!({"themeColor":"#86b300"})).as_deref(),
            Some("#86b300")
        );
        assert_eq!(
            theme_color(&json!({"theme_color":"#abc"})).as_deref(),
            Some("#aabbcc")
        );
        assert_eq!(theme_color(&json!({"themeColor":"url(example)"})), None);
        assert_eq!(theme_color(&json!({})), None);
    }

    #[test]
    fn accepts_a_domain_without_other_url_parts() {
        assert!(origin("misskey.io").is_some());
        for host in [
            "localhost",
            "127.0.0.1",
            "misskey.io:443",
            "example.net@misskey.io",
            "misskey.io/path",
        ] {
            assert!(origin(host).is_none(), "{host}");
        }
    }
}
