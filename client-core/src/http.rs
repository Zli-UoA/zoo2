use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::Result;

/// OIDC のアクセストークンを返すもの。期限が近ければ更新してから返す。
pub trait BearerSource: Send + Sync {
    fn bearer(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>>;

    fn persistence_report(
        &self,
    ) -> Pin<Box<dyn Future<Output = TokenPersistenceReport> + Send + '_>> {
        Box::pin(async {
            TokenPersistenceReport {
                status: TokenPersistenceStatus::Unknown,
                reason: None,
            }
        })
    }

    /// Stop admitting token operations, then wait for every registered owner task.
    fn finish_token_tasks(
        &self,
    ) -> Pin<Box<dyn Future<Output = TokenPersistenceReport> + Send + '_>> {
        self.persistence_report()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPersistenceStatus {
    Pending,
    Saved,
    SessionChanged,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenPersistenceReport {
    pub status: TokenPersistenceStatus,
    pub reason: Option<&'static str>,
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

    pub async fn finish_token_tasks(&self) -> TokenPersistenceReport {
        match self {
            Credentials::Dev(_) => TokenPersistenceReport {
                status: TokenPersistenceStatus::Saved,
                reason: None,
            },
            Credentials::Bearer(source) => source.finish_token_tasks().await,
        }
    }

    pub async fn persistence_report(&self) -> TokenPersistenceReport {
        match self {
            Credentials::Dev(_) => TokenPersistenceReport {
                status: TokenPersistenceStatus::Saved,
                reason: None,
            },
            Credentials::Bearer(source) => source.persistence_report().await,
        }
    }
}

/// 401 はトークン失効として扱い、呼び出し側で再ログインを促す。
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("認証が切れました。再ログインしてください")]
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

/// E2EE の API のエラー応答（`{"error": 本文, "code": コード, ...追加のフィールド}`）。
///
/// 初回設定と解錠（#360）は、`E2EE_REAUTH_REQUIRED`、`E2EE_SSSS_CONFLICT` などのコードで分岐するので、
/// 本文全体を持つ。ほかの API は [`HttpError`] のまま。
#[derive(Debug, thiserror::Error)]
#[error("{status}: {message}")]
pub struct ApiError {
    pub status: reqwest::StatusCode,
    pub code: Option<String>,
    pub message: String,
    pub body: serde_json::Value,
}

impl ApiError {
    /// `error` が [`ApiError`] で、そのコードが `code` か。
    pub fn has_code(error: &anyhow::Error, code: &str) -> bool {
        error
            .downcast_ref::<ApiError>()
            .is_some_and(|e| e.code.as_deref() == Some(code))
    }

    /// `error` の HTTP の状態（[`ApiError`] か [`HttpError::Status`] のとき）。
    pub fn status_of(error: &anyhow::Error) -> Option<reqwest::StatusCode> {
        if let Some(e) = error.downcast_ref::<ApiError>() {
            return Some(e.status);
        }
        match error.downcast_ref::<HttpError>() {
            Some(HttpError::Status { status, .. }) => Some(*status),
            _ => None,
        }
    }
}

/// [`error_for_status`] と同じだが、401 以外のエラーを [`ApiError`] にする。
pub async fn error_for_status_coded(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(HttpError::Unauthorized.into());
    }
    let text = response.text().await.unwrap_or_default();
    let body = serde_json::from_str::<serde_json::Value>(&text).unwrap_or(serde_json::Value::Null);
    let message = body
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or(text);
    let code = body
        .get("code")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Err(ApiError {
        status,
        code,
        message,
        body,
    }
    .into())
}
