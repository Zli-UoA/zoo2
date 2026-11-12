use crate::error::AppError;
use serde_json::{Value, json};
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
#[derive(serde::Deserialize, serde::Serialize)]
struct AccountProfile {
    username: String,
    display_name: String,
    avatar_url: Option<String>,
}

struct CacheEntry {
    source: String,
    checked: Instant,
    value: Option<Value>,
}
static CACHE: LazyLock<Mutex<Option<CacheEntry>>> = LazyLock::new(|| Mutex::new(None));

pub async fn catalog() -> Result<Value, AppError> {
    let Some(source) = std::env::var("EMOJI_ACCOUNT_PROFILE_URL")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        return Ok(json!({"accounts":[]}));
    };
    cached_catalog(&source, &CACHE).await
}

async fn cached_catalog(
    source: &str,
    cache: &Mutex<Option<CacheEntry>>,
) -> Result<Value, AppError> {
    let url = reqwest::Url::parse(source)
        .map_err(|_| AppError::BadGateway("Invalid account directory configuration".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::BadGateway(
            "Invalid account directory configuration".into(),
        ));
    }
    let mut cache = cache.lock().await;
    if let Some(entry) = cache.as_ref()
        && entry.source == source
        && entry.checked.elapsed() < Duration::from_secs(60)
    {
        return entry
            .value
            .clone()
            .ok_or_else(|| AppError::BadGateway("Account directory unavailable".into()));
    }
    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| AppError::BadGateway("Account directory unavailable".into()))?;
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|_| AppError::BadGateway("Account directory unavailable".into()))?;
        if !response.status().is_success() {
            return Err(AppError::BadGateway("Account directory unavailable".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AppError::BadGateway("Account directory unavailable".into()))?
        {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(AppError::BadGateway("Account directory too large".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::BadGateway("Invalid account directory".into()))?;
        let rows = value["accounts"]
            .as_array()
            .ok_or_else(|| AppError::BadGateway("Invalid account directory".into()))?;
        let profiles = rows
            .iter()
            .map(|row| serde_json::from_value::<AccountProfile>(row.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::BadGateway("Invalid account directory".into()))?;
        Ok(json!({"accounts":profiles}))
    }
    .await;
    match result {
        Ok(value) => {
            *cache = Some(CacheEntry {
                source: source.to_owned(),
                checked: Instant::now(),
                value: Some(value.clone()),
            });
            Ok(value)
        }
        Err(error) => {
            *cache = Some(CacheEntry {
                source: source.to_owned(),
                checked: Instant::now(),
                value: None,
            });
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    #[tokio::test]
    async fn failure_is_shared_without_serving_expired_profiles() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).set_delay(Duration::from_millis(50)))
            .expect(1)
            .mount(&server)
            .await;
        let cache = Mutex::new(Some(CacheEntry {
            source: server.uri(),
            checked: Instant::now() - Duration::from_secs(61),
            value: Some(json!({"accounts":[{"username":"old"}]})),
        }));
        let source = server.uri();
        let (a, b, c) = tokio::join!(
            cached_catalog(&source, &cache),
            cached_catalog(&source, &cache),
            cached_catalog(&source, &cache)
        );
        assert!(a.is_err() && b.is_err() && c.is_err());
        assert!(cache.lock().await.as_ref().unwrap().value.is_none());
        server.verify().await;
        cache.lock().await.as_mut().unwrap().checked -= Duration::from_secs(61);
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"accounts":[]})))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            cached_catalog(&source, &cache).await.unwrap(),
            json!({"accounts":[]})
        );
    }
}
