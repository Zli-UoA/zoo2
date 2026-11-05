use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::Result;

/// OIDC のアクセストークンを返すもの。期限が近ければ更新してから返す。
pub trait BearerSource: Send + Sync {
    fn bearer(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>>;
}

/// REST と Shape の両方に付ける認証情報。
#[derive(Clone)]
pub enum Credentials {
    /// backend の dev モード。`X-Dev-User` でユーザーを指定する。
    Dev(String),
    Bearer(Arc<dyn BearerSource>),
}

impl Credentials {
    pub async fn apply(&self, request: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        Ok(match self {
            Credentials::Dev(user_id) => request.header("X-Dev-User", user_id),
            Credentials::Bearer(source) => request.bearer_auth(source.bearer().await?),
        })
    }
}

/// 401 はトークン失効として扱い、呼び出し側で再ログインを促す。
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("認証が切れました。`zoo2-tui login` で再ログインしてください")]
    Unauthorized,
    #[error("{status}: {message}")]
    Status {
        status: reqwest::StatusCode,
        message: String,
    },
}

/// backend のエラー応答 `{"error": "..."}` から本文を取り出す。
pub async fn error_for_status(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(HttpError::Unauthorized.into());
    }
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_string))
        .unwrap_or(body);
    Err(HttpError::Status { status, message }.into())
}
