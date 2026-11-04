//! Shared standard shortcode catalog. Never mix Unicode entries into custom-image rows.
use crate::error::AppError;
use serde_json::{Value, json};
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
#[derive(Default)]
struct Cache {
    origin: String,
    value: Option<Value>,
    etag: Option<String>,
    checked: Option<Instant>,
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));
fn validate(value: &Value) -> Result<(), AppError> {
    let invalid = || AppError::BadGateway("Invalid shortcode catalog".into());
    let rows = value["shortcodes"].as_object().ok_or_else(invalid)?;
    for (name, unicode) in rows {
        if name.is_empty()
            || name.len() > 100
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_+-".contains(&c))
        {
            return Err(invalid());
        }
        let unicode = unicode.as_str().ok_or_else(invalid)?;
        if unicode.is_empty()
            || unicode.chars().count() > 128
            || unicode.chars().any(char::is_control)
        {
            return Err(invalid());
        }
    }
    Ok(())
}
pub async fn catalog() -> Result<Value, AppError> {
    let Some(origin) = crate::emoji_service::service_url()? else {
        return Ok(json!({"shortcodes":{}}));
    };
    let mut cache = CACHE.lock().await;
    refresh(&mut cache, origin).await
}
async fn refresh(cache: &mut Cache, origin: reqwest::Url) -> Result<Value, AppError> {
    if cache.origin != origin.as_str() {
        *cache = Cache {
            origin: origin.to_string(),
            ..Default::default()
        };
    }
    if cache
        .checked
        .is_some_and(|t| t.elapsed() < Duration::from_secs(3600))
        && let Some(value) = &cache.value
    {
        return Ok(value.clone());
    }
    if cache.value.is_none()
        && cache
            .checked
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
    {
        return Err(AppError::BadGateway("Shortcode service unavailable".into()));
    }
    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Internal(e.into()))?;
        let mut request = client.get(origin.join("/api/v1/shortcodes").unwrap());
        if let Some(etag) = &cache.etag {
            request = request.header("If-None-Match", etag);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| AppError::BadGateway("Shortcode service unavailable".into()))?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED && cache.value.is_some() {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(AppError::BadGateway("Shortcode service unavailable".into()));
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
            .map_err(|_| AppError::BadGateway("Shortcode transfer failed".into()))?
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err(AppError::BadGateway("Shortcode catalog too large".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::BadGateway("Invalid shortcode catalog".into()))?;
        validate(&value)?;
        Ok(Some((value, etag)))
    }
    .await;
    cache.checked = Some(Instant::now());
    match result {
        Ok(Some((value, etag))) => {
            cache.value = Some(value);
            cache.etag = etag;
        }
        Ok(None) => {}
        Err(error) => {
            if cache.value.is_none() {
                return Err(error);
            }
        }
    }
    Ok(cache
        .value
        .clone()
        .unwrap_or_else(|| json!({"shortcodes":{}})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn revalidates_and_retains_only_valid_catalogs() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        };
        let server = MockServer::start().await;
        let origin = reqwest::Url::parse(&server.uri()).unwrap();
        let expected = json!({"shortcodes":{"+1":"👍","woman_technologist":"👩‍💻"}});
        Mock::given(method("GET"))
            .and(path("/api/v1/shortcodes"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&expected)
                    .insert_header("etag", "\"v1\""),
            )
            .mount(&server)
            .await;
        let mut cache = Cache::default();
        assert_eq!(refresh(&mut cache, origin.clone()).await.unwrap(), expected);
        server.reset().await;
        cache.checked = None;
        Mock::given(header("If-None-Match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        assert_eq!(refresh(&mut cache, origin.clone()).await.unwrap(), expected);
        server.reset().await;
        cache.checked = None;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"shortcodes":{"bad":null}})),
            )
            .mount(&server)
            .await;
        assert_eq!(refresh(&mut cache, origin.clone()).await.unwrap(), expected);
        server.reset().await;
        cache.checked = None;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        assert_eq!(refresh(&mut cache, origin.clone()).await.unwrap(), expected);
        assert!(refresh(&mut Cache::default(), origin).await.is_err());
    }
    #[test]
    fn validates_unicode_and_shortcodes() {
        assert!(validate(&json!({"shortcodes":{"+1":"👍","woman_technologist":"👩‍💻"}})).is_ok());
        for value in [
            json!({"shortcodes":{"cat":""}}),
            json!({"shortcodes":{"../cat":"🐈"}}),
            json!({"shortcodes":{"cat":"\n"}}),
            json!({"shortcodes":[]}),
        ] {
            assert!(validate(&value).is_err());
        }
    }
}
