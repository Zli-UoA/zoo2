//! Anonymous public-page metadata. DNS is checked and pinned for every redirect.
use crate::{auth::AuthUser, error::AppError, state::AppState};
use axum::{Json, Router, extract::Query, routing::get};
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

const MAX_HTML: usize = 512 * 1024;
static SLOTS: Semaphore = Semaphore::const_new(8);
static CACHE: LazyLock<Mutex<HashMap<String, (Instant, Preview)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    title: String,
    description: String,
    image: Option<String>,
}
#[derive(Deserialize)]
pub struct Params {
    url: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/link-preview", get(preview))
}
async fn preview(_user: AuthUser, Query(params): Query<Params>) -> Result<Json<Preview>, AppError> {
    let url = parse_url(&params.url)?;
    let key = url.to_string();
    if let Some((at, value)) = CACHE.lock().await.get(&key)
        && at.elapsed() < Duration::from_secs(600)
    {
        return Ok(Json(value.clone()));
    }
    let _permit = SLOTS.try_acquire().map_err(|_| AppError::RateLimited)?;
    let result = tokio::time::timeout(Duration::from_secs(10), fetch(url))
        .await
        .map_err(|_| unavailable())??;
    let mut cache = CACHE.lock().await;
    cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(600));
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), result.clone()));
    Ok(Json(result))
}
fn unavailable() -> AppError {
    AppError::BadGateway("Link preview is unavailable".into())
}
fn parse_url(raw: &str) -> Result<Url, AppError> {
    let url = Url::parse(raw).map_err(|_| AppError::BadRequest("Invalid URL".into()))?;
    if raw.len() > 4096
        || !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port(), None | Some(80 | 443))
    {
        return Err(AppError::BadRequest("Unsupported preview URL".into()));
    }
    Ok(url)
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 169 && b == 254)
                && !(a == 192 && b == 0 && c == 0)
                && !(a == 198 && (b == 18 || b == 19))
        }
        // Accept global unicast only; exclude transition and special-purpose ranges.
        IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}
async fn fetch(mut url: Url) -> Result<Preview, AppError> {
    for _ in 0..=5 {
        let host = url
            .host_str()
            .ok_or_else(unavailable)?
            .trim_matches(['[', ']']);
        let addresses: Vec<_> =
            tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443)))
                .await
                .map_err(|_| unavailable())?
                .collect();
        if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
            return Err(AppError::BadRequest("Preview target must be public".into()));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| unavailable())?;
        let response = client
            .get(url.clone())
            .header("User-Agent", "zoo2-link-preview/1.0")
            .send()
            .await
            .map_err(|_| unavailable())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(unavailable)?;
            url = parse_url(url.join(location).map_err(|_| unavailable())?.as_str())?;
            continue;
        }
        if !response.status().is_success()
            || !response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(';')
                        .next()
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/html"))
                })
        {
            return Err(unavailable());
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + chunk.len() > MAX_HTML {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(metadata(&String::from_utf8_lossy(&bytes), &url));
    }
    Err(unavailable())
}
fn metadata(html: &str, base: &Url) -> Preview {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("meta").unwrap();
    let mut tags = HashMap::new();
    for node in doc.select(&selector) {
        if let (Some(key), Some(value)) = (
            node.value()
                .attr("property")
                .or_else(|| node.value().attr("name")),
            node.value().attr("content"),
        ) {
            tags.entry(key.to_ascii_lowercase())
                .or_insert(value.trim().to_string());
        }
    }
    let first = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| tags.get(*key).filter(|v| !v.is_empty()))
            .cloned()
    };
    let title = first(&["og:title", "twitter:title"])
        .or_else(|| {
            doc.select(&Selector::parse("title").unwrap())
                .next()
                .map(|n| n.text().collect::<String>())
        })
        .unwrap_or_else(|| base.host_str().unwrap_or_default().into());
    let image = first(&["og:image", "twitter:image"])
        .and_then(|v| base.join(&v).ok())
        .filter(|u| parse_url(u.as_str()).is_ok())
        .map(|u| u.to_string());
    Preview {
        title: title.trim().chars().take(300).collect(),
        description: first(&["og:description", "description", "twitter:description"])
            .unwrap_or_default()
            .chars()
            .take(1000)
            .collect(),
        image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_internal_targets_and_unsafe_urls() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.100.100.200",
            "192.168.1.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
        assert!(public_ip("2001:4860:4860::8888".parse().unwrap()));
        for url in [
            "file:///etc/passwd",
            "https://user:pass@example.com",
            "http://example.com:8000",
        ] {
            assert!(parse_url(url).is_err());
        }
    }
    #[test]
    fn extracts_entities_relative_images_and_fallbacks() {
        let base = Url::parse("https://example.com/posts/one").unwrap();
        let value = metadata(
            r#"<title>Fallback</title><meta property="og:title" content="A &amp; B"><meta name="description" content="Summary"><meta property="og:image" content="/image.png">"#,
            &base,
        );
        assert_eq!(value.title, "A & B");
        assert_eq!(value.description, "Summary");
        assert_eq!(
            value.image.as_deref(),
            Some("https://example.com/image.png")
        );
        assert_eq!(metadata("<title>Fallback</title>", &base).title, "Fallback");
        assert!(
            metadata(
                r#"<meta property="og:image" content="javascript:alert(1)">"#,
                &base
            )
            .image
            .is_none()
        );
    }
}
