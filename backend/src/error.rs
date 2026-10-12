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
    Database(#[from] toasty::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Internal server error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(err) => {
                let err_str = err.to_string();
                if err_str.contains("unique constraint") || err_str.contains("UNIQUE constraint") {
                    tracing::warn!("Unique constraint violation: {}", err_str);
                    (
                        StatusCode::CONFLICT,
                        "Resource already exists or unique constraint was violated".to_string(),
                    )
                } else if err_str.contains("foreign key constraint")
                    || err_str.contains("FOREIGN KEY constraint")
                {
                    tracing::warn!("Foreign key violation: {}", err_str);
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid foreign key reference".to_string(),
                    )
                } else {
                    tracing::error!("Database error: {:?}", err);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Database operation failed".to_string(),
                    )
                }
            }
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
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
