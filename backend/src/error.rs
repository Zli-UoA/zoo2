use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(toasty::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Internal server error: {0}")]
    Internal(#[from] anyhow::Error),
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
}

impl From<toasty::Error> for AppError {
    fn from(err: toasty::Error) -> Self {
        Self::from_db_error(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
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
        };

        let body = Json(json!({
            "error": message,
        }));

        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_status_codes() {
        let not_found = AppError::NotFound("item 123".into()).into_response();
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);

        let bad_request = AppError::BadRequest("invalid".into()).into_response();
        assert_eq!(bad_request.status(), StatusCode::BAD_REQUEST);

        let conflict = AppError::Conflict("duplicate".into()).into_response();
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
    }
}
