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

static CACHE: LazyLock<Mutex<Option<(String, Instant, Value)>>> =
    LazyLock::new(|| Mutex::new(None));

pub async fn catalog() -> Result<Value, AppError> {
    let Some(source) = std::env::var("EMOJI_ACCOUNT_PROFILE_URL")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        return Ok(json!({"accounts":[]}));
    };
    let url = reqwest::Url::parse(&source)
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
    let mut cache = CACHE.lock().await;
    if let Some((previous, checked, value)) = cache.as_ref()
        && previous == &source
        && checked.elapsed() < Duration::from_secs(60)
    {
        return Ok(value.clone());
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
            *cache = Some((source, Instant::now(), value.clone()));
            Ok(value)
        }
        Err(error) => {
            *cache = None;
            Err(error)
        }
    }
}
