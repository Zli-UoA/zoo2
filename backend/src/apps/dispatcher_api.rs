//! backend と zoo-dispatcher の内部 API。
//!
//! 外向き HTTP の出口を dispatcher に一本化するため、endpoint の challenge 検証
//! （`actions/verify-endpoint`）は `DISPATCHER_URL` が設定されていれば dispatcher に
//! 依頼する。未設定なら従来どおり backend 自身が送る（移行中・ローカル開発）。
//! 認証は共有トークン（`DISPATCHER_TOKEN`、`Authorization: Bearer`）。
use crate::error::AppError;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

/// dispatcher の外向き送信は最大 3 秒（DNS 含む）。backend 側はそれより長く待つ。
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
struct VerifyRequest {
    url: String,
    signing: String,
}

/// dispatcher 側。`token` が空なら内部 API は公開しない（route を作らない）。
pub fn router(token: &str) -> Router {
    if token.is_empty() {
        return Router::new();
    }
    Router::new()
        .route("/internal/verify-endpoint", post(verify))
        // URL と署名鍵だけの小さな本文。認証前に大きな本文を読ませない。
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(token.to_owned())
}

fn token_matches(headers: &HeaderMap, expected: &str) -> bool {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    // 長さ以外の情報を漏らさない比較。
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

async fn verify(
    State(token): State<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, AppError> {
    // 本文の解析より先に認証する（本文の形でルートの存在を探らせない）。
    if !token_matches(&headers, &token) {
        return Err(AppError::Unauthorized("Invalid dispatcher token".into()));
    }
    let req: VerifyRequest = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Invalid request body".into()))?;
    super::delivery::verify_endpoint(&req.url, &req.signing).await?;
    Ok(Json(json!({"verified": true})))
}

/// `DISPATCHER_URL` と `DISPATCHER_TOKEN` が両方あるときだけ委譲する。
fn target() -> Option<(String, String)> {
    let url = std::env::var("DISPATCHER_URL").ok()?;
    let token = std::env::var("DISPATCHER_TOKEN").ok()?;
    let url = url.trim().trim_end_matches('/').to_owned();
    (!url.is_empty() && !token.is_empty()).then_some((url, token))
}

/// challenge 検証。dispatcher が設定されていればそこで、無ければここで送る。
pub async fn verify_endpoint(url: &str, signing: &str) -> Result<(), AppError> {
    match target() {
        Some((base, token)) => verify_via(&base, &token, url, signing).await,
        None => super::delivery::verify_endpoint(url, signing).await,
    }
}

pub async fn verify_via(base: &str, token: &str, url: &str, signing: &str) -> Result<(), AppError> {
    let client = reqwest::Client::builder()
        .timeout(CLIENT_TIMEOUT)
        .build()
        .map_err(|e| AppError::Internal(e.into()))?;
    let response = client
        .post(format!("{base}/internal/verify-endpoint"))
        .bearer_auth(token)
        .json(&json!({"url":url,"signing":signing}))
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error=%e,"Dispatcher is unreachable");
            AppError::ServiceUnavailable("Event dispatcher is unavailable".into())
        })?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let message = response
        .json::<Value>()
        .await
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "Event dispatcher failed".into());
    Err(match status {
        StatusCode::BAD_REQUEST => AppError::BadRequest(message),
        StatusCode::BAD_GATEWAY => AppError::BadGateway(message),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            tracing::error!("Dispatcher rejected the backend token");
            AppError::ServiceUnavailable("Event dispatcher is misconfigured".into())
        }
        _ => AppError::ServiceUnavailable("Event dispatcher is unavailable".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_requires_exact_bearer_match() {
        let mut h = HeaderMap::new();
        assert!(!token_matches(&h, "secret"));
        h.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(token_matches(&h, "secret"));
        assert!(!token_matches(&h, "secreT"));
        assert!(!token_matches(&h, "secrets"));
        h.insert(header::AUTHORIZATION, "secret".parse().unwrap());
        assert!(!token_matches(&h, "secret"));
    }
}
