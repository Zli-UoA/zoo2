//! Read-only shared emoji catalog. The workspace table remains the migration fallback.
use crate::{error::AppError, state::AppState};
use serde_json::{Value, json};
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};
#[derive(Default)]
struct Cache {
    origin: String,
    etag: Option<String>,
    rows: Option<Vec<Value>>,
    checked: Option<Instant>,
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));
static SNAPSHOT: LazyLock<RwLock<Option<(String, Vec<Value>)>>> = LazyLock::new(|| RwLock::new(None));
pub fn service_url() -> Result<Option<reqwest::Url>, AppError> {
    let Some(value) = std::env::var("EMOJI_SERVICE_URL")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    let url = reqwest::Url::parse(&value)
        .map_err(|_| AppError::BadRequest("Invalid EMOJI_SERVICE_URL".into()))?;
    if url.origin().ascii_serialization() != value
        || !url.username().is_empty()
        || url.password().is_some()
        || (url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))))
    {
        return Err(AppError::BadRequest(
            "EMOJI_SERVICE_URL must be an HTTPS origin".into(),
        ));
    }
    Ok(Some(url))
}
pub fn writable() -> Result<(), AppError> {
    if service_url()?.is_some() {
        return Err(AppError::Forbidden(
            "Manage emojis in the shared emoji service".into(),
        ));
    }
    Ok(())
}
fn decode(value: &Value, origin: &reqwest::Url) -> Result<Vec<Value>, AppError> {
    let invalid = || AppError::BadGateway("Invalid shared emoji catalog".into());
    let emojis = value["emojis"].as_array().ok_or_else(invalid)?;
    let mut names = std::collections::HashSet::new();
    let name_pattern = regex::Regex::new(r"^[a-z0-9_-]{1,64}$").unwrap();
    let image_pattern = regex::Regex::new(r"^/images/[a-f0-9]{64}$").unwrap();
    emojis.iter().map(|e| {
        let name = e["name"].as_str().ok_or_else(invalid)?;
        let aliases = e["aliases"].as_array().ok_or_else(invalid)?;
        for n in std::iter::once(name).chain(aliases.iter().map(|a| a.as_str().unwrap_or(""))) {
            if !name_pattern.is_match(n) || !names.insert(n.to_owned()) { return Err(invalid()); }
        }
        let image = e["image_url"].as_str().ok_or_else(invalid)?;
        if !image_pattern.is_match(image) || !e["category"].is_string() { return Err(invalid()); }
        let url = origin.join(image).map_err(|_| invalid())?;
        Ok(json!({"name":name,"aliases":aliases,"category":e["category"],"url":url.as_str(),"updated_at":e["updated_at"],"created_by":""}))
    }).collect()
}
pub async fn catalog() -> Result<Option<Vec<Value>>, AppError> {
    let Some(origin) = service_url()? else {
        return Ok(None);
    };
    // Once loaded, reads never wait for the background worker's network request.
    if let Some((source, rows)) = &*SNAPSHOT.read().await {
        if source == origin.as_str() { return Ok(Some(rows.clone())); }
    }
    refresh_shared().await
}
async fn refresh_shared() -> Result<Option<Vec<Value>>, AppError> {
    let Some(origin) = service_url()? else { return Ok(None) };
    let mut cache = CACHE.lock().await;
    let rows = refresh(&mut cache, origin.clone(), std::env::var("EMOJI_CACHE_FILE").ok()).await?;
    if let Some(rows) = &rows { *SNAPSHOT.write().await = Some((origin.to_string(),rows.clone())); }
    Ok(rows)
}
async fn refresh(
    cache: &mut Cache,
    origin: reqwest::Url,
    disk: Option<String>,
) -> Result<Option<Vec<Value>>, AppError> {
    if cache.origin != origin.as_str() {
        *cache = Cache {
            origin: origin.to_string(),
            ..Default::default()
        };
        if let Some(path) = &disk {
            if tokio::fs::metadata(&path)
                .await
                .is_ok_and(|m| m.len() <= 8 * 1024 * 1024)
            {
                if let Ok(bytes) = tokio::fs::read(&path).await {
                    if let Ok(saved) = serde_json::from_slice::<Value>(&bytes) {
                        if saved["origin"] == origin.as_str() {
                            if let Ok(rows) = decode(&saved["catalog"], &origin) {
                                cache.rows = Some(rows);
                                cache.etag = saved["etag"].as_str().map(str::to_owned);
                            }
                        }
                    }
                }
            }
        }
    }
    if cache
        .checked
        .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
    {
        return Ok(cache.rows.clone());
    }
    cache.checked = Some(Instant::now());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(e.into()))?;
    let mut request = client.get(origin.join("/api/v1/emojis").unwrap());
    if let Some(etag) = &cache.etag {
        request = request.header("If-None-Match", etag);
    }
    let result = async {
        let mut response = request
            .send()
            .await
            .map_err(|_| AppError::BadGateway("Emoji service unavailable".into()))?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED && cache.rows.is_some() {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(AppError::BadGateway("Emoji service unavailable".into()));
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AppError::BadGateway("Emoji transfer failed".into()))?
        {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(AppError::BadGateway("Emoji catalog too large".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::BadGateway("Invalid emoji catalog".into()))?;
        Ok(Some((decode(&value, &origin)?, etag, value)))
    }
    .await;
    match result {
        Ok(Some((rows, etag, value))) => {
            if let Some(path) = &disk {
                let snapshot = json!({"origin":origin.as_str(),"etag":etag,"catalog":value});
                let temporary = format!("{path}.{}.tmp", uuid::Uuid::new_v4());
                let saved = async {
                    if let Some(parent) = std::path::Path::new(&path)
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                    {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&temporary, snapshot.to_string()).await?;
                    tokio::fs::rename(&temporary, &path).await
                }
                .await;
                if saved.is_err() {
                    let _ = tokio::fs::remove_file(temporary).await;
                    tracing::warn!("Emoji catalog disk cache write failed");
                }
            }
            cache.rows = Some(rows);
            cache.etag = etag;
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!("Shared emoji catalog refresh failed");
            if cache.rows.is_none() {
                return Err(error);
            }
        }
    }
    Ok(cache.rows.clone())
}
pub async fn worker(state: AppState) {
    if !matches!(service_url(), Ok(Some(_))) {
        return;
    }
    let mut previous: Option<Vec<Value>> = None;
    loop {
        if let Ok(Some(rows)) = refresh_shared().await {
            if let Some(old) = &previous {
                let result = async {
                    let mut db = state.db.clone();
                    let mut tx = db.transaction().await?;
                    for row in &rows {
                        let prior = old.iter().find(|v| v["name"] == row["name"]);
                        if prior != Some(row) {
                            let mut data = row.clone();
                            data["subtype"] = json!(if prior.is_some() { "change" } else { "add" });
                            crate::apps::events::emit(
                                &mut tx,
                                "emoji.changed",
                                data,
                                None,
                                None,
                                None,
                                None,
                            )
                            .await?;
                        }
                    }
                    for row in old
                        .iter()
                        .filter(|r| !rows.iter().any(|v| v["name"] == r["name"]))
                    {
                        let mut data = row.clone();
                        data["subtype"] = json!("remove");
                        crate::apps::events::emit(
                            &mut tx,
                            "emoji.changed",
                            data,
                            None,
                            None,
                            None,
                            None,
                        )
                        .await?;
                    }
                    tx.commit().await?;
                    Ok::<_, AppError>(())
                }
                .await;
                if result.is_ok() {
                    previous = Some(rows);
                } else {
                    tracing::warn!("Shared emoji change events failed");
                }
            } else {
                previous = Some(rows);
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_validates_alias_namespace_and_image_origin() {
        let origin = reqwest::Url::parse("https://emoji.test").unwrap();
        let item = json!({"name":"cat","aliases":["neko"],"category":"animals","image_url":format!("/images/{}","a".repeat(64))});
        assert_eq!(
            decode(&json!({"emojis":[item.clone()]}), &origin).unwrap()[0]["url"],
            format!("https://emoji.test/images/{}", "a".repeat(64))
        );
        assert!(decode(&json!({"emojis":[item.clone(),item.clone()]}), &origin).is_err());
        let mut bad = item;
        bad["image_url"] = json!("https://evil.test/image");
        assert!(decode(&json!({"emojis":[bad]}), &origin).is_err());
    }
    #[tokio::test]
    async fn cache_revalidates_and_retains_last_success_on_failure_and_restart() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        };
        let server = MockServer::start().await;
        let origin = reqwest::Url::parse(&server.uri()).unwrap();
        let value = json!({"emojis":[{"name":"cat","aliases":["neko"],"category":"animals","image_url":format!("/images/{}","a".repeat(64))}]});
        let file = std::env::temp_dir().join(format!("emoji-cache-{}.json", uuid::Uuid::new_v4()));
        let disk = Some(file.to_string_lossy().to_string());
        Mock::given(method("GET"))
            .and(path("/api/v1/emojis"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&value)
                    .insert_header("ETag", "\"one\""),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut cache = Cache::default();
        let expected = refresh(&mut cache, origin.clone(), disk.clone())
            .await
            .unwrap();
        assert!(expected.as_ref().is_some_and(|r| r.len() == 1));
        server.reset().await;
        cache.checked = None;
        Mock::given(method("GET"))
            .and(header("If-None-Match", "\"one\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            refresh(&mut cache, origin.clone(), disk.clone())
                .await
                .unwrap(),
            expected
        );
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        cache.checked = None;
        assert_eq!(
            refresh(&mut cache, origin.clone(), disk.clone())
                .await
                .unwrap(),
            expected
        );
        let mut restarted = Cache::default();
        assert_eq!(
            refresh(&mut restarted, origin.clone(), disk).await.unwrap(),
            expected
        );
        let mut empty = Cache::default();
        assert!(refresh(&mut empty, origin, None).await.is_err());
        let _ = tokio::fs::remove_file(file).await;
    }
}
