use axum::{
    extract::FromRequestParts,
    http::request::Parts,
};
use uuid::Uuid;

use crate::error::AppError;

pub const DEFAULT_DEV_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

/// 認証済みユーザー情報（ローカル先行開発時は X-Dev-User または環境変数で解決）
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
        // 1. X-Dev-User ヘッダの確認
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

        // 2. Authorization ヘッダの確認 (将来の IdP 連携やテスト用)
        if let Some(auth_val) = parts.headers.get("Authorization") {
            if let Ok(auth_str) = auth_val.to_str() {
                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    // トークン自体が UUID 文字列の場合はそのままユーザー ID として許容
                    if let Ok(uuid) = Uuid::parse_str(token) {
                        return Ok(AuthUser {
                            id: uuid.to_string(),
                        });
                    }
                }
            }
        }

        // 3. 開発用デフォルトユーザーへのフォールバック
        let default_id = std::env::var("DEV_DEFAULT_USER_ID")
            .unwrap_or_else(|_| DEFAULT_DEV_USER_ID.to_string());

        let uuid = Uuid::parse_str(&default_id).map_err(|_| {
            AppError::BadRequest(format!(
                "DEV_DEFAULT_USER_ID must be a valid UUID: {default_id}"
            ))
        })?;

        Ok(AuthUser {
            id: uuid.to_string(),
        })
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
    async fn test_auth_user_default_fallback() {
        let req = Request::builder().body(()).unwrap();
        let (mut parts, _) = req.into_parts();

        let auth_user = AuthUser::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(auth_user.id, DEFAULT_DEV_USER_ID);
    }
}
