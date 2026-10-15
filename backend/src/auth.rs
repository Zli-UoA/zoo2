use axum::{extract::FromRequestParts, http::request::Parts};
use uuid::Uuid;

use crate::error::AppError;

pub const DEFAULT_DEV_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

/// 環境変数の値とビルド種別から Dev Auth が有効かどうかを判定する純粋関数
pub(crate) fn is_dev_auth_enabled_pure(
    dev_auth_env: Option<&str>,
    environment_env: Option<&str>,
    is_test_build: bool,
) -> bool {
    // 本番環境 (ENVIRONMENT=production) では DEV_AUTH_ENABLED の指定に関わらず常に無効 (fail-closed)
    if environment_env == Some("production") {
        return false;
    }

    if let Some(val) = dev_auth_env {
        return val == "true" || val == "1";
    }

    is_test_build
}

/// 開発用認証 (Dev Auth) が有効かどうかを判定
///
/// セキュリティのため fail-closed (デフォルト無効) とし、
/// DEV_AUTH_ENABLED=true (または 1) が明示的に設定されている場合のみ有効化する。
/// ※テスト実行時 (cfg!(test)) はテストの利便性のため ENVIRONMENT!=production の場合に有効化。
pub fn dev_auth_enabled() -> bool {
    let dev_auth = std::env::var("DEV_AUTH_ENABLED").ok();
    let env = std::env::var("ENVIRONMENT").ok();
    #[cfg(test)]
    let is_test = true;
    #[cfg(not(test))]
    let is_test = false;

    is_dev_auth_enabled_pure(dev_auth.as_deref(), env.as_deref(), is_test)
}

/// 認証リクエスト情報からユーザーを解決する純粋判定ロジック
///
/// Dev Auth が有効な環境のみ、開発用認証方式 (Bearer <UUID>, X-Dev-User, デフォルトユーザー) を許可。
/// dev_auth_enabled が false (本番など) の場合は、Bearer や X-Dev-User が渡されても 403 Forbidden となる。
pub(crate) fn resolve_auth_user(
    dev_auth_enabled: bool,
    auth_header: Option<&str>,
    x_dev_user_header: Option<&str>,
    default_dev_user_id: Option<&str>,
) -> Result<AuthUser, AppError> {
    if dev_auth_enabled {
        // 1. Authorization ヘッダ (Bearer <UUID>)
        if let Some(auth_str) = auth_header
            && let Some(token) = auth_str.strip_prefix("Bearer ")
            && let Ok(uuid) = Uuid::parse_str(token)
        {
            return Ok(AuthUser {
                id: uuid.to_string(),
            });
        }

        // 2. X-Dev-User ヘッダ
        if let Some(user_str) = x_dev_user_header {
            let uuid = Uuid::parse_str(user_str).map_err(|_| {
                AppError::BadRequest(format!("X-Dev-User must be a valid UUID: {user_str}"))
            })?;

            return Ok(AuthUser {
                id: uuid.to_string(),
            });
        }

        // 3. デフォルト開発ユーザーへのフォールバック
        let default_id = default_dev_user_id.unwrap_or(DEFAULT_DEV_USER_ID);
        let uuid = Uuid::parse_str(default_id).map_err(|_| {
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
        let is_dev_enabled = dev_auth_enabled();
        if !is_dev_enabled {
            return Err(AppError::Forbidden("Authentication required".to_string()));
        }

        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok());

        let x_dev_user = match parts.headers.get("X-Dev-User") {
            Some(v) => Some(
                v.to_str()
                    .map_err(|_| AppError::BadRequest("Invalid X-Dev-User header format".into()))?,
            ),
            None => None,
        };

        let default_id = std::env::var("DEV_DEFAULT_USER_ID").ok();

        resolve_auth_user(
            is_dev_enabled,
            auth_header,
            x_dev_user,
            default_id.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use tokio::sync::Mutex;

    static TEST_ENV_MUTEX: Mutex<()> = Mutex::const_new(());

    #[test]
    fn test_is_dev_auth_enabled_pure() {
        // DEV_AUTH_ENABLED が明示的に設定されている場合
        assert!(is_dev_auth_enabled_pure(Some("true"), None, false));
        assert!(is_dev_auth_enabled_pure(Some("1"), None, false));
        assert!(!is_dev_auth_enabled_pure(Some("false"), None, false));
        assert!(!is_dev_auth_enabled_pure(Some("0"), None, false));
        assert!(!is_dev_auth_enabled_pure(Some("invalid"), None, false));

        // DEV_AUTH_ENABLED 未設定 & 非テストビルド (本番ビルド) -> 常に false (fail-closed)
        assert!(!is_dev_auth_enabled_pure(None, None, false));
        assert!(!is_dev_auth_enabled_pure(None, Some("development"), false));
        assert!(!is_dev_auth_enabled_pure(None, Some("production"), false));

        // DEV_AUTH_ENABLED 未設定 & テストビルド
        assert!(is_dev_auth_enabled_pure(None, None, true));
        assert!(is_dev_auth_enabled_pure(None, Some("development"), true));
        assert!(is_dev_auth_enabled_pure(None, Some("test"), true));
        // ENVIRONMENT=production のときは DEV_AUTH_ENABLED が設定されていても常に false (fail-closed)
        assert!(!is_dev_auth_enabled_pure(
            Some("true"),
            Some("production"),
            false
        ));
        assert!(!is_dev_auth_enabled_pure(
            Some("true"),
            Some("production"),
            true
        ));
        assert!(!is_dev_auth_enabled_pure(None, Some("production"), true));
    }

    #[test]
    fn test_resolve_auth_user_when_dev_auth_disabled() {
        let valid_uuid = "11111111-1111-1111-1111-111111111111";

        // dev_auth_enabled = false のときは全て 403 Forbidden になること (認証バイパス防止)
        assert!(matches!(
            resolve_auth_user(false, Some(&format!("Bearer {valid_uuid}")), None, None),
            Err(AppError::Forbidden(_))
        ));

        assert!(matches!(
            resolve_auth_user(false, None, Some(valid_uuid), None),
            Err(AppError::Forbidden(_))
        ));

        assert!(matches!(
            resolve_auth_user(false, None, None, None),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn test_resolve_auth_user_when_dev_auth_enabled() {
        let bearer_uuid = "11111111-1111-1111-1111-111111111111";
        let x_dev_uuid = "22222222-2222-2222-2222-222222222222";
        let custom_default_uuid = "33333333-3333-3333-3333-333333333333";

        // 1. Bearer UUID が最優先
        let user = resolve_auth_user(
            true,
            Some(&format!("Bearer {bearer_uuid}")),
            Some(x_dev_uuid),
            Some(custom_default_uuid),
        )
        .unwrap();
        assert_eq!(user.id, bearer_uuid);

        // 2. Bearer がなければ X-Dev-User
        let user =
            resolve_auth_user(true, None, Some(x_dev_uuid), Some(custom_default_uuid)).unwrap();
        assert_eq!(user.id, x_dev_uuid);

        // 3. Bearer も X-Dev-User もなければカスタムデフォルト
        let user = resolve_auth_user(true, None, None, Some(custom_default_uuid)).unwrap();
        assert_eq!(user.id, custom_default_uuid);

        // 4. カスタムデフォルトがなければ定数 DEFAULT_DEV_USER_ID
        let user = resolve_auth_user(true, None, None, None).unwrap();
        assert_eq!(user.id, DEFAULT_DEV_USER_ID);

        // 5. 不正な X-Dev-User UUID は BadRequest
        let err = resolve_auth_user(true, None, Some("not-a-uuid"), None);
        assert!(matches!(err, Err(AppError::BadRequest(_))));

        // 6. 不正なカスタムデフォルト UUID も BadRequest
        let err = resolve_auth_user(true, None, None, Some("invalid-default-uuid"));
        assert!(matches!(err, Err(AppError::BadRequest(_))));

        // 7. 不正な Bearer トークンの場合、X-Dev-User にフォールバック
        let user =
            resolve_auth_user(true, Some("Bearer invalid-uuid"), Some(x_dev_uuid), None).unwrap();
        assert_eq!(user.id, x_dev_uuid);
    }

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
