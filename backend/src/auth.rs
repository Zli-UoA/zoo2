use std::time::Duration;

use axum::{extract::FromRequestParts, http::request::Parts};
use uuid::Uuid;

use crate::{
    auth::oidc::OidcClaims,
    error::AppError,
    models::{User, UserIdentity},
    state::{AppState, UserResolutionCache},
    validation::{
        MAX_DISPLAY_NAME_LENGTH, MAX_USER_NAME_LENGTH, RESERVED_USER_NAMES, validate_tenant_id,
    },
};

pub mod oidc;
pub mod rbac;

pub const DEFAULT_DEV_USER_ID: &str = "00000000-0000-0000-0000-000000000001";
pub const DEFAULT_TENANT_ID: &str = "default";
const JIT_CACHE_TTL: Duration = Duration::from_secs(300); // 5分

/// ユーザー名用のサニタイズ関数
pub(crate) fn sanitize_for_user_name(candidate: &str, fallback_uuid: &str) -> String {
    let lower = candidate.to_ascii_lowercase();
    let filtered: String = lower
        .chars()
        .filter(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '_'
        })
        .take(MAX_USER_NAME_LENGTH)
        .collect();

    if filtered.is_empty() || RESERVED_USER_NAMES.contains(&filtered.as_str()) {
        let short_uuid = fallback_uuid.replace('-', "");
        let prefix = &short_uuid[..8.min(short_uuid.len())];
        format!("user_{prefix}")
    } else {
        filtered
    }
}

/// 表示名用のサニタイズ関数 (マルチバイト文字境界を意識した安全なスライス)
pub(crate) fn sanitize_for_display_name(candidate: &str, fallback: &str) -> String {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        fallback.chars().take(MAX_DISPLAY_NAME_LENGTH).collect()
    } else {
        trimmed.chars().take(MAX_DISPLAY_NAME_LENGTH).collect()
    }
}

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
pub fn dev_auth_enabled() -> bool {
    let dev_auth = std::env::var("DEV_AUTH_ENABLED").ok();
    let env = std::env::var("ENVIRONMENT").ok();
    #[cfg(test)]
    let is_test = true;
    #[cfg(not(test))]
    let is_test = false;

    is_dev_auth_enabled_pure(dev_auth.as_deref(), env.as_deref(), is_test)
}

/// JIT (Just-In-Time) ユーザー解決および自動プロビジョニング
pub async fn resolve_or_provision_user(
    db: &toasty::Db,
    user_cache: &UserResolutionCache,
    claims: &OidcClaims,
    tenant_id: &str,
) -> Result<AuthUser, AppError> {
    // 1. インメモリキャッシュの確認 (全リクエストでの DB 問い合わせ負荷を低減)
    if let Some(cached_user) = user_cache.get(&claims.iss, &claims.sub).await {
        return Ok(cached_user);
    }

    let mut db_client = db.clone();

    // 2. user_identities テーブルによる既存ユーザーの検索
    let existing_identity = UserIdentity::filter(
        UserIdentity::fields()
            .iss()
            .eq(&claims.iss)
            .and(UserIdentity::fields().sub().eq(&claims.sub)),
    )
    .first()
    .exec(&mut db_client)
    .await?;

    if let Some(identity) = existing_identity {
        let user = User::get_by_id(&mut db_client, &identity.user_id).await?;
        if user.tenant_id != tenant_id {
            return Err(AppError::Forbidden(
                "Authenticated user does not belong to requested tenant".to_string(),
            ));
        }

        let auth_user = AuthUser {
            id: user.id,
            tenant_id: user.tenant_id,
            user_name: user.user_name,
            display_name: user.display_name,
        };

        user_cache
            .insert(
                claims.iss.clone(),
                claims.sub.clone(),
                auth_user.clone(),
                JIT_CACHE_TTL,
            )
            .await;

        return Ok(auth_user);
    }

    // 3. 新規ユーザーおよび Identity の作成 (JIT プロビジョニング)
    let new_user_id = Uuid::new_v4().to_string();
    let preferred_name = claims
        .preferred_username
        .as_deref()
        .and_then(|u| u.split('@').next())
        .or_else(|| claims.email.as_deref().and_then(|e| e.split('@').next()))
        .unwrap_or("user");

    let base_username = sanitize_for_user_name(preferred_name, &new_user_id);
    let display_name = sanitize_for_display_name(
        claims.name.as_deref().unwrap_or(&base_username),
        &base_username,
    );

    // 衝突時のリトライループ (usernameの重複や、並行リクエストによる同一 (iss, sub) の同時作成)
    let mut attempt = 0;
    loop {
        attempt += 1;
        let candidate_username = if attempt == 1 {
            base_username.clone()
        } else {
            let suffix = &Uuid::new_v4().simple().to_string()[..6];
            let truncated: String = base_username.chars().take(70).collect();
            format!("{truncated}_{suffix}")
        };

        let identity_id = Uuid::new_v4().to_string();
        let mut tx = db_client.transaction().await?;
        let now = crate::models::now_utc_millis();

        let create_user_res = toasty::create!(User {
            id: new_user_id.as_str(),
            tenant_id: tenant_id,
            user_name: candidate_username.as_str(),
            display_name: display_name.as_str(),
            created_at: now,
            updated_at: None,
        })
        .exec(&mut tx)
        .await;

        let created_user = match create_user_res {
            Ok(user) => user,
            Err(err) => {
                let _ = tx.rollback().await;

                // 並行リクエストにより既に作成されたか確認
                let check_existing = UserIdentity::filter(
                    UserIdentity::fields()
                        .iss()
                        .eq(&claims.iss)
                        .and(UserIdentity::fields().sub().eq(&claims.sub)),
                )
                .first()
                .exec(&mut db_client)
                .await?;

                if let Some(identity) = check_existing {
                    let user = User::get_by_id(&mut db_client, &identity.user_id).await?;
                    if user.tenant_id != tenant_id {
                        return Err(AppError::Forbidden(
                            "Authenticated user does not belong to requested tenant".to_string(),
                        ));
                    }
                    let auth_user = AuthUser {
                        id: user.id,
                        tenant_id: user.tenant_id,
                        user_name: user.user_name,
                        display_name: user.display_name,
                    };
                    user_cache
                        .insert(
                            claims.iss.clone(),
                            claims.sub.clone(),
                            auth_user.clone(),
                            JIT_CACHE_TTL,
                        )
                        .await;
                    return Ok(auth_user);
                }

                if attempt >= 5 {
                    return Err(AppError::from(err));
                }
                continue;
            }
        };

        let create_identity_res = toasty::create!(UserIdentity {
            id: identity_id.as_str(),
            user_id: created_user.id.as_str(),
            iss: claims.iss.as_str(),
            sub: claims.sub.as_str(),
            created_at: now,
            updated_at: None,
        })
        .exec(&mut tx)
        .await;

        match create_identity_res {
            Ok(_) => {
                tx.commit().await?;
                let auth_user = AuthUser {
                    id: created_user.id,
                    tenant_id: tenant_id.to_string(),
                    user_name: created_user.user_name,
                    display_name: created_user.display_name,
                };
                user_cache
                    .insert(
                        claims.iss.clone(),
                        claims.sub.clone(),
                        auth_user.clone(),
                        JIT_CACHE_TTL,
                    )
                    .await;
                return Ok(auth_user);
            }
            Err(err) => {
                let _ = tx.rollback().await;

                // 並行リクエストで UserIdentity の (iss, sub) 重複が起きたか確認
                let check_existing = UserIdentity::filter(
                    UserIdentity::fields()
                        .iss()
                        .eq(&claims.iss)
                        .and(UserIdentity::fields().sub().eq(&claims.sub)),
                )
                .first()
                .exec(&mut db_client)
                .await?;

                if let Some(identity) = check_existing {
                    let user = User::get_by_id(&mut db_client, &identity.user_id).await?;
                    if user.tenant_id != tenant_id {
                        return Err(AppError::Forbidden(
                            "Authenticated user does not belong to requested tenant".to_string(),
                        ));
                    }
                    let auth_user = AuthUser {
                        id: user.id,
                        tenant_id: user.tenant_id,
                        user_name: user.user_name,
                        display_name: user.display_name,
                    };
                    user_cache
                        .insert(
                            claims.iss.clone(),
                            claims.sub.clone(),
                            auth_user.clone(),
                            JIT_CACHE_TTL,
                        )
                        .await;
                    return Ok(auth_user);
                }

                if attempt >= 5 {
                    return Err(AppError::from(err));
                }
            }
        }
    }
}

/// 認証リクエスト情報からユーザーを解決する純粋判定ロジック (Dev Auth 用)
pub(crate) fn resolve_auth_user(
    dev_auth_enabled: bool,
    auth_header: Option<&str>,
    x_dev_user_header: Option<&str>,
    default_dev_user_id: Option<&str>,
    tenant_id: &str,
) -> Result<AuthUser, AppError> {
    if dev_auth_enabled {
        // 1. Authorization ヘッダ (Bearer <UUID>)
        if let Some(auth_str) = auth_header
            && let Some(token) = auth_str.strip_prefix("Bearer ")
            && let Ok(uuid) = Uuid::parse_str(token)
        {
            return Ok(AuthUser {
                id: uuid.to_string(),
                tenant_id: tenant_id.to_string(),
                user_name: "dev_user".to_string(),
                display_name: "Dev User".to_string(),
            });
        }

        // 2. X-Dev-User ヘッダ
        if let Some(user_str) = x_dev_user_header {
            let uuid = Uuid::parse_str(user_str).map_err(|_| {
                AppError::BadRequest(format!("X-Dev-User must be a valid UUID: {user_str}"))
            })?;

            return Ok(AuthUser {
                id: uuid.to_string(),
                tenant_id: tenant_id.to_string(),
                user_name: "dev_user".to_string(),
                display_name: "Dev User".to_string(),
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
            tenant_id: tenant_id.to_string(),
            user_name: "dev_user".to_string(),
            display_name: "Dev User".to_string(),
        });
    }

    Err(AppError::Forbidden("Authentication required".to_string()))
}

/// 認証済みユーザー情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthUser {
    pub id: String,
    pub tenant_id: String,
    pub user_name: String,
    pub display_name: String,
}

pub(crate) fn resolve_tenant_id(
    x_tenant_id_header: Option<&str>,
    claims_tenant_id: Option<&str>,
) -> Result<String, AppError> {
    let tenant_id = claims_tenant_id
        .or(x_tenant_id_header)
        .unwrap_or(DEFAULT_TENANT_ID);
    validate_tenant_id(tenant_id)?;
    Ok(tenant_id.to_string())
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let is_dev_enabled = state.config.dev_auth_enabled;

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
        let x_tenant_id = match parts.headers.get("X-Tenant-Id") {
            Some(v) => Some(
                v.to_str()
                    .map_err(|_| AppError::BadRequest("Invalid X-Tenant-Id header format".into()))?,
            ),
            None => None,
        };
        let tenant_id = resolve_tenant_id(x_tenant_id, None)?;

        // 1. Dev Auth 有効環境の処理
        if is_dev_enabled {
            if let Some(auth_str) = auth_header
                && let Some(token) = auth_str.strip_prefix("Bearer ")
            {
                // (a) UUID の場合は開発用 Bearer
                if let Ok(uuid) = Uuid::parse_str(token) {
                    return Ok(AuthUser {
                        id: uuid.to_string(),
                        tenant_id,
                        user_name: "dev_user".to_string(),
                        display_name: "Dev User".to_string(),
                    });
                }

                // (b) UUID 形式でない場合、OIDC 設定があれば OIDC トークンとして検証
                if let Some(ref oidc_config) = state.config.oidc {
                    let claims = state.jwks_verifier.verify_token(token, oidc_config).await?;
                    let tenant_id =
                        resolve_tenant_id(x_tenant_id, claims.tenant_id.as_deref())?;
                    return resolve_or_provision_user(
                        &state.db,
                        &state.user_cache,
                        &claims,
                        &tenant_id,
                    )
                    .await;
                }
            }

            // X-Dev-User ヘッダまたはデフォルト開発ユーザーへのフォールバック
            let default_id = std::env::var("DEV_DEFAULT_USER_ID").ok();
            return resolve_auth_user(
                is_dev_enabled,
                auth_header,
                x_dev_user,
                default_id.as_deref(),
                tenant_id.as_str(),
            );
        }

        // 2. 本番環境 (is_dev_enabled == false) の処理 (fail-closed)
        // X-Dev-User や開発用デフォルトへのフォールバックは絶対禁止
        let auth_str = auth_header.ok_or_else(|| {
            AppError::Unauthorized("Authorization header is required".to_string())
        })?;

        let token = auth_str.strip_prefix("Bearer ").ok_or_else(|| {
            AppError::Unauthorized("Authorization header must be Bearer token".to_string())
        })?;

        let oidc_config =
            state.config.oidc.as_ref().ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("OIDC configuration is missing"))
            })?;

        let claims = state.jwks_verifier.verify_token(token, oidc_config).await?;

        let tenant_id = resolve_tenant_id(x_tenant_id, claims.tenant_id.as_deref())?;
        resolve_or_provision_user(&state.db, &state.user_cache, &claims, &tenant_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dev_auth_enabled_pure() {
        assert!(is_dev_auth_enabled_pure(Some("true"), None, false));
        assert!(is_dev_auth_enabled_pure(Some("1"), None, false));
        assert!(!is_dev_auth_enabled_pure(Some("false"), None, false));
        assert!(!is_dev_auth_enabled_pure(Some("0"), None, false));
        assert!(!is_dev_auth_enabled_pure(Some("invalid"), None, false));

        assert!(!is_dev_auth_enabled_pure(None, None, false));
        assert!(!is_dev_auth_enabled_pure(None, Some("development"), false));
        assert!(!is_dev_auth_enabled_pure(None, Some("production"), false));

        assert!(is_dev_auth_enabled_pure(None, None, true));
        assert!(is_dev_auth_enabled_pure(None, Some("development"), true));
        assert!(is_dev_auth_enabled_pure(None, Some("test"), true));
        assert!(!is_dev_auth_enabled_pure(
            Some("true"),
            Some("production"),
            false
        ));
        assert!(!is_dev_auth_enabled_pure(Some("true"), Some("production"), true));
        assert!(!is_dev_auth_enabled_pure(None, Some("production"), true));
    }

    #[test]
    fn test_resolve_auth_user_when_dev_auth_disabled() {
        let valid_uuid = "11111111-1111-1111-1111-111111111111";

        assert!(matches!(
            resolve_auth_user(
                false,
                Some(&format!("******", valid_uuid)),
                None,
                None,
                DEFAULT_TENANT_ID
            ),
            Err(AppError::Forbidden(_))
        ));

        assert!(matches!(
            resolve_auth_user(false, None, Some(valid_uuid), None, DEFAULT_TENANT_ID),
            Err(AppError::Forbidden(_))
        ));

        assert!(matches!(
            resolve_auth_user(false, None, None, None, DEFAULT_TENANT_ID),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn test_resolve_auth_user_when_dev_auth_enabled() {
        let bearer_uuid = "11111111-1111-1111-1111-111111111111";
        let x_dev_uuid = "22222222-2222-2222-2222-222222222222";
        let custom_default_uuid = "33333333-3333-3333-3333-333333333333";

        let user = resolve_auth_user(
            true,
            Some(&format!("******", bearer_uuid)),
            Some(x_dev_uuid),
            Some(custom_default_uuid),
            DEFAULT_TENANT_ID,
        )
        .unwrap();
        assert_eq!(user.id, bearer_uuid);
        assert_eq!(user.tenant_id, DEFAULT_TENANT_ID);

        let user = resolve_auth_user(
            true,
            None,
            Some(x_dev_uuid),
            Some(custom_default_uuid),
            DEFAULT_TENANT_ID,
        )
        .unwrap();
        assert_eq!(user.id, x_dev_uuid);

        let user = resolve_auth_user(
            true,
            None,
            None,
            Some(custom_default_uuid),
            DEFAULT_TENANT_ID,
        )
        .unwrap();
        assert_eq!(user.id, custom_default_uuid);

        let user = resolve_auth_user(true, None, None, None, DEFAULT_TENANT_ID).unwrap();
        assert_eq!(user.id, DEFAULT_DEV_USER_ID);

        let err = resolve_auth_user(true, None, Some("not-a-uuid"), None, DEFAULT_TENANT_ID);
        assert!(matches!(err, Err(AppError::BadRequest(_))));

        let err = resolve_auth_user(
            true,
            None,
            None,
            Some("invalid-default-uuid"),
            DEFAULT_TENANT_ID,
        );
        assert!(matches!(err, Err(AppError::BadRequest(_))));

        let user = resolve_auth_user(
            true,
            Some("******"),
            Some(x_dev_uuid),
            None,
            DEFAULT_TENANT_ID,
        )
        .unwrap();
        assert_eq!(user.id, x_dev_uuid);
    }

    #[test]
    fn test_auth_user_sanitization() {
        assert_eq!(
            sanitize_for_user_name("Alice.Test_01", "default-uuid"),
            "alice.test_01"
        );
        assert_eq!(
            sanitize_for_user_name("admin", "12345678-0000-0000-0000-000000000000"),
            "user_12345678"
        );
        assert_eq!(
            sanitize_for_user_name("", "abcdef12-0000-0000-0000-000000000000"),
            "user_abcdef12"
        );

        assert_eq!(
            sanitize_for_display_name("  Alice Wonderland  ", "fallback"),
            "Alice Wonderland"
        );
        assert_eq!(sanitize_for_display_name("   ", "fallback"), "fallback");
    }

    #[test]
    fn test_resolve_tenant_id() {
        assert_eq!(
            resolve_tenant_id(Some("tenant-a"), None).unwrap(),
            "tenant-a".to_string()
        );
        assert_eq!(
            resolve_tenant_id(None, Some("tenant-b")).unwrap(),
            "tenant-b".to_string()
        );
        assert_eq!(
            resolve_tenant_id(Some("tenant-a"), Some("tenant-b")).unwrap(),
            "tenant-b".to_string()
        );
        assert_eq!(
            resolve_tenant_id(None, None).unwrap(),
            DEFAULT_TENANT_ID.to_string()
        );
        assert!(resolve_tenant_id(Some("Tenant"), None).is_err());
    }
}
