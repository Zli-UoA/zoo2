use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Rate limit exceeded")]
    RateLimited,
    #[error("Database error: {0}")]
    Database(toasty::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Bad gateway: {0}")]
    BadGateway(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Internal server error: {0}")]
    Internal(#[from] anyhow::Error),

    /// 機械判定用の `code` と追加のフィールドを持つエラー（docs/e2ee.md の「エラー」）。
    /// 応答は `{"error": message, "code": code, ...extra}`。429 で `extra` に `retry_after`（秒）があれば、
    /// `Retry-After` ヘッダーにも同じ値を入れる。
    #[error("{code}: {message}")]
    Coded {
        status: StatusCode,
        code: &'static str,
        message: String,
        extra: serde_json::Map<String, serde_json::Value>,
    },
}

impl AppError {
    /// `toasty::Error` からドメイン/HTTP レイヤーのエラーへ適切に分類する
    pub fn from_db_error(err: toasty::Error) -> Self {
        // 1. PostgreSQL の SQLSTATE による判定
        let mut curr: Option<&(dyn std::error::Error + 'static)> = Some(&err);
        while let Some(e) = curr {
            if let Some(code) = e
                .downcast_ref::<tokio_postgres::Error>()
                .and_then(|pg_err| pg_err.code())
            {
                if *code == tokio_postgres::error::SqlState::UNIQUE_VIOLATION {
                    tracing::warn!("Database unique constraint violated (SQLSTATE 23505)");
                    return AppError::Conflict(
                        "Resource already exists or unique constraint was violated".to_string(),
                    );
                }
                if *code == tokio_postgres::error::SqlState::CHECK_VIOLATION {
                    return AppError::BadRequest("Operation violates a resource constraint".into());
                }
                if *code == tokio_postgres::error::SqlState::FOREIGN_KEY_VIOLATION {
                    tracing::warn!("Database foreign key constraint violated (SQLSTATE 23503)");
                    return AppError::BadRequest("Invalid foreign key reference".to_string());
                }
            }
            curr = e.source();
        }

        // 2. ドライバ抽象化層やエラー文字列によるフォールバック判定
        let err_str = err.to_string();
        if err_str.contains("23505")
            || err_str.contains("unique constraint")
            || err_str.contains("UNIQUE constraint")
        {
            tracing::warn!("Database unique constraint violated (fallback match)");
            return AppError::Conflict(
                "Resource already exists or unique constraint was violated".to_string(),
            );
        }
        if err_str.contains("23503")
            || err_str.contains("foreign key constraint")
            || err_str.contains("FOREIGN KEY constraint")
        {
            tracing::warn!("Database foreign key constraint violated (fallback match)");
            return AppError::BadRequest("Invalid foreign key reference".to_string());
        }

        tracing::error!("Database internal error: {:?}", err);
        AppError::Database(err)
    }

    /// `toasty::Error` が UNIQUE 制約違反かどうか、および指定された制約名（オプション）に合致するか判定する
    pub fn is_unique_violation(err: &toasty::Error, constraint_name: Option<&str>) -> bool {
        let mut curr: Option<&(dyn std::error::Error + 'static)> = Some(err);
        while let Some(e) = curr {
            if let Some(db_err) = e
                .downcast_ref::<tokio_postgres::Error>()
                .and_then(|pg_err| pg_err.as_db_error())
                .filter(|db_err| {
                    *db_err.code() == tokio_postgres::error::SqlState::UNIQUE_VIOLATION
                })
            {
                return match (constraint_name, db_err.constraint()) {
                    (Some(expected), Some(actual)) => actual == expected,
                    (Some(_), None) => false,
                    (None, _) => true,
                };
            }
            curr = e.source();
        }

        let err_str = err.to_string();
        let is_unique = err_str.contains("23505")
            || err_str.contains("unique constraint")
            || err_str.contains("UNIQUE constraint");

        if !is_unique {
            return false;
        }

        match constraint_name {
            Some(expected) => err_str.contains(expected),
            None => true,
        }
    }
}

impl From<toasty::Error> for AppError {
    fn from(err: toasty::Error) -> Self {
        Self::from_db_error(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let AppError::Coded {
            status,
            code,
            message,
            extra,
        } = self
        {
            let retry_after = extra.get("retry_after").and_then(serde_json::Value::as_u64);
            let mut body = extra;
            body.insert("error".into(), message.into());
            body.insert("code".into(), code.into());
            let mut response = (status, Json(serde_json::Value::Object(body))).into_response();
            if status == StatusCode::TOO_MANY_REQUESTS {
                let value = retry_after.unwrap_or(60).to_string();
                if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
                    response.headers_mut().insert("retry-after", value);
                }
            }
            return response;
        }
        let (status, message) = match &self {
            AppError::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded".to_string(),
            ),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::BadGateway(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
            AppError::ServiceUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database operation failed".to_string(),
            ),
            AppError::Internal(err) => {
                tracing::error!("Internal error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::Coded { .. } => unreachable!("handled above"),
        };

        let body = Json(json!({
            "error": message,
        }));

        let mut response = (status, body).into_response();
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert("retry-after", axum::http::HeaderValue::from_static("60"));
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        let unauthorized = AppError::Unauthorized("invalid token".into()).into_response();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let bad_gateway = AppError::BadGateway("upstream down".into()).into_response();
        assert_eq!(bad_gateway.status(), StatusCode::BAD_GATEWAY);

        let not_found = AppError::NotFound("item 123".into()).into_response();
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);

        let bad_request = AppError::BadRequest("invalid".into()).into_response();
        assert_eq!(bad_request.status(), StatusCode::BAD_REQUEST);

        let conflict = AppError::Conflict("duplicate".into()).into_response();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);

        let forbidden = AppError::Forbidden("access denied".into()).into_response();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn coded_errors_carry_code_extra_fields_and_retry_after() {
        let mut extra = serde_json::Map::new();
        extra.insert("retry_after".into(), 42.into());
        let response = AppError::Coded {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "E2EE_RATE_LIMITED",
            message: "slow down".into(),
            extra,
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()["retry-after"], "42");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body,
            json!({"error": "slow down", "code": "E2EE_RATE_LIMITED", "retry_after": 42})
        );
    }
}
