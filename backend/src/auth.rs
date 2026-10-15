use axum::{extract::FromRequestParts, http::request::Parts};
use uuid::Uuid;

use crate::error::AppError;

pub const DEFAULT_DEV_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

/// 開発用認証 (Dev Auth) が有効かどうかを判定
pub fn dev_auth_enabled() -> bool {
    match std::env::var("DEV_AUTH_ENABLED") {
        Ok(val) => val == "true" || val == "1",
        Err(_) => std::env::var("ENVIRONMENT").unwrap_or_default() != "production",
    }
}

/// 認証済みユーザー情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthUser {
    pub id: String,
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 1. Authorization ヘッダの確認 (将来の IdP 連携・トークン認証用)
        if let Some(auth_val) = parts.headers.get("Authorization")
            && let Ok(auth_str) = auth_val.to_str()
            && let Some(token) = auth_str.strip_prefix("Bearer ")
            && let Ok(uuid) = Uuid::parse_str(token)
        {
            return Ok(AuthUser {
                id: uuid.to_string(),
            });
        }

        // 2. Dev Auth が有効な環境のみ X-Dev-User およびデフォルトフォールバックを許可
        if dev_auth_enabled() {
            if let Some(header_val) = parts.headers.get("X-Dev-User") {
                let user_str = header_val.to_str().map_err(|_| {
                    AppError::BadRequest("Invalid X-Dev-User header format".to_string())
                })?;

                let uuid = Uuid::parse_str(user_str).map_err(|_| {
                    AppError::BadRequest(format!("X-Dev-User must be a valid UUID: {user_str}"))
                })?;

                return Ok(AuthUser {
                    id: uuid.to_string(),
                });
            }

            let default_id = std::env::var("DEV_DEFAULT_USER_ID")
                .unwrap_or_else(|_| DEFAULT_DEV_USER_ID.to_string());

            let uuid = Uuid::parse_str(&default_id).map_err(|_| {
                AppError::BadRequest(format!(
                    "DEV_DEFAULT_USER_ID must be a valid UUID: {default_id}"
                ))
            })?;

            return Ok(AuthUser {
                id: uuid.to_string(),
            });
        }

        Err(AppError::Forbidden("Authentication required".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    #[tokio::test]
    async fn test_auth_user_from_x_dev_user_header() {
        let test_uuid = "11111111-1111-1111-1111-111111111111";
        let req = Request::builder()
            .header("X-Dev-User", test_uuid)
            .body(())
            .unwrap();
        let (mut parts, _) = req.into_parts();

        let auth_user = AuthUser::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(auth_user.id, test_uuid);
    }

    #[tokio::test]
    async fn test_auth_user_invalid_uuid() {
        let req = Request::builder()
            .header("X-Dev-User", "not-a-uuid")
            .body(())
            .unwrap();
        let (mut parts, _) = req.into_parts();

        let res = AuthUser::from_request_parts(&mut parts, &()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_auth_user_default_fallback_in_dev() {
        let req = Request::builder().body(()).unwrap();
        let (mut parts, _) = req.into_parts();

        let auth_user = AuthUser::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(auth_user.id, DEFAULT_DEV_USER_ID);
    }
}
