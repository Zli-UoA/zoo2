use axum::{extract::FromRequestParts, http::request::Parts};
use uuid::Uuid;

use crate::error::AppError;

pub const DEFAULT_DEV_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

/// 開発用認証 (Dev Auth) が有効かどうかを判定
///
/// セキュリティのため fail-closed (デフォルト無効) とし、
/// DEV_AUTH_ENABLED=true (または 1) が明示的に設定されている場合のみ有効化する。
/// ※テスト実行時 (cfg!(test)) はテストの利便性のため ENVIRONMENT!=production の場合に有効化。
pub fn dev_auth_enabled() -> bool {
    if let Ok(val) = std::env::var("DEV_AUTH_ENABLED") {
        return val == "true" || val == "1";
    }

    #[cfg(test)]
    {
        std::env::var("ENVIRONMENT").unwrap_or_default() != "production"
    }

    #[cfg(not(test))]
    {
        false
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
        // Dev Auth が有効な環境のみ、開発用認証方式 (Bearer <UUID>, X-Dev-User, デフォルトユーザー) を許可
        // 本番環境では正式な IdP による JWT 等の署名・クレーム検証を行う
        if dev_auth_enabled() {
            // 1. Authorization ヘッダ (Bearer <UUID>)
            if let Some(auth_val) = parts.headers.get("Authorization")
                && let Ok(auth_str) = auth_val.to_str()
                && let Some(token) = auth_str.strip_prefix("Bearer ")
                && let Ok(uuid) = Uuid::parse_str(token)
            {
                return Ok(AuthUser {
                    id: uuid.to_string(),
                });
            }

            // 2. X-Dev-User ヘッダ
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

            // 3. デフォルト開発ユーザーへのフォールバック
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
    use tokio::sync::Mutex;

    static TEST_ENV_MUTEX: Mutex<()> = Mutex::const_new(());

    #[tokio::test]
    async fn test_auth_user_from_x_dev_user_header() {
        let _guard = TEST_ENV_MUTEX.lock().await;
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
    async fn test_auth_user_from_bearer_uuid_in_dev() {
        let _guard = TEST_ENV_MUTEX.lock().await;
        let test_uuid = "22222222-2222-2222-2222-222222222222";
        let req = Request::builder()
            .header("Authorization", format!("Bearer {test_uuid}"))
            .body(())
            .unwrap();
        let (mut parts, _) = req.into_parts();

        let auth_user = AuthUser::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(auth_user.id, test_uuid);
    }

    #[tokio::test]
    async fn test_auth_user_invalid_uuid() {
        let _guard = TEST_ENV_MUTEX.lock().await;
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
        let _guard = TEST_ENV_MUTEX.lock().await;
        let req = Request::builder().body(()).unwrap();
        let (mut parts, _) = req.into_parts();

        let auth_user = AuthUser::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(auth_user.id, DEFAULT_DEV_USER_ID);
    }

    #[tokio::test]
    async fn test_auth_user_rejected_in_production() {
        let _guard = TEST_ENV_MUTEX.lock().await;
        unsafe {
            std::env::set_var("ENVIRONMENT", "production");
            std::env::remove_var("DEV_AUTH_ENABLED");
        }

        let test_uuid = "33333333-3333-3333-3333-333333333333";

        // 1. Bearer UUID は本番で 403 Forbidden になること (認証バイパス防止)
        let req1 = Request::builder()
            .header("Authorization", format!("Bearer {test_uuid}"))
            .body(())
            .unwrap();
        let (mut parts1, _) = req1.into_parts();
        let res1 = AuthUser::from_request_parts(&mut parts1, &()).await;
        assert!(matches!(res1, Err(AppError::Forbidden(_))));

        // 2. X-Dev-User は本番で 403 Forbidden になること
        let req2 = Request::builder()
            .header("X-Dev-User", test_uuid)
            .body(())
            .unwrap();
        let (mut parts2, _) = req2.into_parts();
        let res2 = AuthUser::from_request_parts(&mut parts2, &()).await;
        assert!(matches!(res2, Err(AppError::Forbidden(_))));

        // 3. ヘッダなし (デフォルトユーザー) も本番で 403 Forbidden になること
        let req3 = Request::builder().body(()).unwrap();
        let (mut parts3, _) = req3.into_parts();
        let res3 = AuthUser::from_request_parts(&mut parts3, &()).await;
        assert!(matches!(res3, Err(AppError::Forbidden(_))));

        // クリーンアップ
        unsafe {
            std::env::remove_var("ENVIRONMENT");
        }
    }
}
