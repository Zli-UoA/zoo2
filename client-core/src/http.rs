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
        /// 応答の `{"code": ...}`。コードを返すエラーだけ入る。
        code: Option<String>,
    },
}

/// 参加行があるスレッドへ、未参加・退出済みのまま返信したときの 403 の `code`
/// （backend の `create_message_impl`）。参加すれば同じ返信を送れる。
pub const THREAD_JOIN_REQUIRED_CODE: &str = "THREAD_JOIN_REQUIRED";

/// 同じ 403 の本文。`code` を返す前のサーバーに備えて、`code` がないときだけ本文で判定する。
pub const THREAD_JOIN_REQUIRED_MESSAGE: &str = "Join the thread before replying";

/// `error` が「スレッドに参加してから返信する」ことを求める 403 か。投稿権限がないなど、
/// ほかの 403 は含まない（参加しても送れない）。暗号化チャンネルの API は [`ApiError`]、
/// ほかは [`HttpError::Status`] でエラーを返すので、どちらも見る。`code` を優先し、
/// `code` がない応答（コードを返す前のサーバー）だけ、本文で判定する。
pub fn is_thread_join_required(error: &anyhow::Error) -> bool {
    let (status, message, code) = match (
        error.downcast_ref::<HttpError>(),
        error.downcast_ref::<ApiError>(),
    ) {
        (
            Some(HttpError::Status {
                status,
                message,
                code,
            }),
            _,
        ) => (*status, message.as_str(), code.as_deref()),
        (_, Some(api)) => (api.status, api.message.as_str(), api.code.as_deref()),
        _ => return false,
    };
    status == reqwest::StatusCode::FORBIDDEN
        && match code {
            Some(code) => code == THREAD_JOIN_REQUIRED_CODE,
            None => message.contains(THREAD_JOIN_REQUIRED_MESSAGE),
        }
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
    let json = serde_json::from_str::<serde_json::Value>(&body).ok();
    let code = json
        .as_ref()
        .and_then(|value| value.get("code")?.as_str().map(str::to_string));
    let message = json
        .and_then(|value| value.get("error")?.as_str().map(str::to_string))
        .unwrap_or(body);
    Err(HttpError::Status {
        status,
        message,
        code,
    }
    .into())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn status(status: reqwest::StatusCode, message: &str) -> anyhow::Error {
        HttpError::Status {
            status,
            message: message.to_string(),
            code: None,
        }
        .into()
    }

    fn status_with_code(status: reqwest::StatusCode, message: &str, code: &str) -> anyhow::Error {
        HttpError::Status {
            status,
            message: message.to_string(),
            code: Some(code.to_string()),
        }
        .into()
    }

    /// `code` があれば本文は見ない（本文を変更・翻訳しても判定が変わらない）。`code` がなければ、
    /// コードを返す前のサーバーの本文で判定する。
    #[test]
    fn the_code_wins_over_the_message() {
        assert!(is_thread_join_required(&status_with_code(
            reqwest::StatusCode::FORBIDDEN,
            "参加してから返信してください",
            THREAD_JOIN_REQUIRED_CODE,
        )));
        assert!(!is_thread_join_required(&status_with_code(
            reqwest::StatusCode::FORBIDDEN,
            THREAD_JOIN_REQUIRED_MESSAGE,
            "OTHER_CODE",
        )));
    }

    #[test]
    fn only_the_join_required_403_asks_for_a_join() {
        assert!(is_thread_join_required(&status(
            reqwest::StatusCode::FORBIDDEN,
            THREAD_JOIN_REQUIRED_MESSAGE,
        )));
        // 投稿権限がないなどの 403、ほかの状態、別の種類のエラーは、参加しても直らない。
        assert!(!is_thread_join_required(&status(
            reqwest::StatusCode::FORBIDDEN,
            "Only channel managers may post here",
        )));
        assert!(!is_thread_join_required(&status(
            reqwest::StatusCode::CONFLICT,
            THREAD_JOIN_REQUIRED_MESSAGE,
        )));
        assert!(!is_thread_join_required(&anyhow::anyhow!("network down")));
    }

    fn coded(status: reqwest::StatusCode, message: &str) -> anyhow::Error {
        ApiError {
            status,
            code: None,
            message: message.to_string(),
            body: serde_json::json!({ "error": message }),
        }
        .into()
    }

    /// 暗号化チャンネルの API（`error_for_status_coded`）は `ApiError` を返す。
    #[test]
    fn the_coded_error_of_the_e2ee_api_is_recognised_too() {
        assert!(is_thread_join_required(&coded(
            reqwest::StatusCode::FORBIDDEN,
            THREAD_JOIN_REQUIRED_MESSAGE,
        )));
        assert!(!is_thread_join_required(&coded(
            reqwest::StatusCode::FORBIDDEN,
            "Only channel managers may post here",
        )));
        assert!(!is_thread_join_required(&coded(
            reqwest::StatusCode::CONFLICT,
            THREAD_JOIN_REQUIRED_MESSAGE,
        )));
    }
}
