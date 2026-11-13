use std::time::Duration;

use axum::{extract::FromRequestParts, http::request::Parts};
use uuid::Uuid;

use crate::{
    auth::oidc::{JwksVerifier, OidcClaims, OidcUserInfo},
    config::OidcConfig,
    error::AppError,
    models::{User, UserIdentity},
    state::{AppState, UserResolutionCache},
    validation::{MAX_DISPLAY_NAME_LENGTH, MAX_USER_NAME_LENGTH, RESERVED_USER_NAMES},
};

pub mod oidc;
pub mod rbac;

pub const DEFAULT_DEV_USER_ID: &str = "00000000-0000-0000-0000-000000000001";
const PROFILE_FETCH_TIMEOUT: Duration = Duration::from_secs(2);
const INITIAL_PROFILE_FETCH_TIMEOUT: Duration = Duration::from_secs(6);
const JIT_CACHE_TTL: Duration = Duration::from_secs(300); // 5分
/// claims / UserInfo のどちらからも表示名を取得できなかった場合のフォールバック
const FALLBACK_USER_NAME: &str = "user";

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

fn preferred_username<'a>(
    claims: &'a OidcClaims,
    user_info: Option<&'a OidcUserInfo>,
) -> Option<&'a str> {
    claims
        .preferred_username
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            user_info
                .and_then(|info| info.preferred_username.as_deref())
                .filter(|name| !name.trim().is_empty())
        })
}

fn email<'a>(claims: &'a OidcClaims, user_info: Option<&'a OidcUserInfo>) -> Option<&'a str> {
    claims
        .email
        .as_deref()
        .filter(|email| !email.trim().is_empty())
        .or_else(|| {
            user_info
                .and_then(|info| info.email.as_deref())
                .filter(|email| !email.trim().is_empty())
        })
}

/// 表示名に使う名前 (アクセストークンの claims を優先し、無ければ UserInfo)
fn profile_name<'a>(
    claims: &'a OidcClaims,
    user_info: Option<&'a OidcUserInfo>,
) -> Option<&'a str> {
    claims
        .name
        .as_deref()
        .or_else(|| user_info.and_then(|info| info.name.as_deref()))
}

/// 表示名の候補。`name` が無い場合は `preferred_username` を SPN のローカル部分として使う。
fn display_name_candidate<'a>(
    claims: &'a OidcClaims,
    user_info: Option<&'a OidcUserInfo>,
) -> Option<&'a str> {
    profile_name(claims, user_info)
        .or_else(|| preferred_username(claims, user_info).and_then(|name| name.split('@').next()))
}

fn derive_base_username(
    claims: &OidcClaims,
    user_info: Option<&OidcUserInfo>,
    new_user_id: &str,
) -> String {
    let preferred_name = preferred_username(claims, user_info)
        .or_else(|| email(claims, user_info))
        .and_then(|name| name.split('@').next())
        .unwrap_or(FALLBACK_USER_NAME);

    sanitize_for_user_name(preferred_name, new_user_id)
}

fn derive_display_name(
    claims: &OidcClaims,
    user_info: Option<&OidcUserInfo>,
    fallback: &str,
) -> String {
    display_name_candidate(claims, user_info)
        .map(|candidate| sanitize_for_display_name(candidate, fallback))
        .unwrap_or_else(|| fallback.to_string())
}

/// UserInfo を取得する。失敗・不正応答時は警告を出して `None` を返す (ログインは継続する)。
async fn fetch_userinfo_for_profile(
    jwks_verifier: &JwksVerifier,
    oidc_config: &OidcConfig,
    access_token: &str,
    claims: &OidcClaims,
    timeout: Duration,
) -> Option<OidcUserInfo> {
    // Bound Discovery + UserInfo + response decoding together, below the UI's 8s deadline.
    match tokio::time::timeout(
        timeout,
        jwks_verifier.fetch_userinfo(access_token, &claims.sub, oidc_config),
    )
    .await
    {
        Ok(Ok(user_info)) => Some(user_info),
        Ok(Err(err)) => {
            tracing::warn!(
                issuer = %claims.iss,
                sub = %claims.sub,
                "Failed to fetch UserInfo for user profile; falling back to access token claims/default profile: {err}"
            );
            None
        }
        Err(_) => {
            tracing::warn!("UserInfo profile fetch timed out; profile data unavailable");
            None
        }
    }
}

/// UserInfo の最新値を使う。トークンの picture は新規作成時の初期値に限る。
/// 外側の None は同期対象外を表し、保存済みの画像を維持する。
fn profile_picture(
    claims: &OidcClaims,
    user_info: Option<&OidcUserInfo>,
    new_user: bool,
) -> Option<Option<String>> {
    if !claims.has_scope("profile") {
        return None;
    }
    let value = match user_info {
        Some(info) => info.picture.as_deref(),
        None if new_user => Some(claims.picture.as_deref()?),
        None => return None,
    };
    Some(value.and_then(|value| {
        let url = reqwest::Url::parse(value).ok()?;
        let normalized = url.to_string();
        (value.len() <= 2048
            && normalized.len() <= 2048
            && url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none())
        .then_some(normalized)
    }))
}

async fn sync_profile_picture(
    db: &toasty::Db,
    user_id: &str,
    claims: &OidcClaims,
    user_info: Option<&OidcUserInfo>,
    new_user: bool,
) -> Result<(), AppError> {
    if let Some(picture) = profile_picture(claims, user_info, new_user) {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement("UPDATE users SET avatar_url=NULLIF($2,'') WHERE id=$1 AND avatar_url IS DISTINCT FROM NULLIF($2,'')")
            .bind(user_id).bind(picture.as_deref().unwrap_or(""))
            .exec(&mut tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

/// 修正前に JIT で作成された `display_name = "user"` のユーザーを UserInfo から補正する。
///
/// ユーザー本人が `PATCH /api/users/me` で編集した表示名を上書きしないよう、
/// 未編集 (`updated_at` が NULL) かつ旧フォールバック値のままの場合だけ更新する。
async fn repair_legacy_display_name(
    db: &toasty::Db,
    user_cache: &UserResolutionCache,
    claims: &OidcClaims,
    user: User,
    user_info: Option<&OidcUserInfo>,
) -> Result<User, AppError> {
    if user.display_name != FALLBACK_USER_NAME || user.updated_at.is_some() {
        return Ok(user);
    }

    if !user_cache.should_attempt_profile_repair(&user.id).await {
        return Ok(user);
    }

    let Some(candidate) = display_name_candidate(claims, user_info) else {
        // UserInfo が成功しても名前が無い場合は、しばらく再試行を抑止する
        if user_info.is_some() {
            user_cache.mark_profile_repair_attempted(&user.id).await;
        }
        return Ok(user);
    };

    let new_display_name = sanitize_for_display_name(candidate, &user.user_name);
    if new_display_name == user.display_name || new_display_name == FALLBACK_USER_NAME {
        if user_info.is_some() {
            user_cache.mark_profile_repair_attempted(&user.id).await;
        }
        return Ok(user);
    }

    let mut db_client = db.clone();
    let mut tx = db_client.transaction().await?;
    let now = crate::models::now_utc_millis();
    // 読み取り後に本人が PATCH /api/users/me で編集している可能性があるため、
    // 旧フォールバックのまま未編集であることを UPDATE の条件にも含める。
    User::filter(
        User::fields()
            .id()
            .eq(user.id.as_str())
            .and(User::fields().display_name().eq(FALLBACK_USER_NAME))
            .and(User::fields().updated_at().is_none()),
    )
    .update()
    .display_name(new_display_name.as_str())
    .updated_at(Some(now))
    .exec(&mut tx)
    .await?;
    let updated_user = User::get_by_id(&mut tx, user.id.as_str()).await?;
    tx.commit().await?;

    Ok(updated_user)
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
    jwks_verifier: &JwksVerifier,
    oidc_config: &OidcConfig,
    access_token: &str,
    claims: &OidcClaims,
) -> Result<AuthUser, AppError> {
    // 1. インメモリキャッシュの確認 (全リクエストでの DB 問い合わせ負荷を低減)
    if let Some(cached_user) = user_cache.get(&claims.iss, &claims.sub).await {
        return Ok(cached_user);
    }

    let _resolution = user_cache.lock_resolution(&claims.iss, &claims.sub).await;
    if let Some(cached_user) = user_cache.get(&claims.iss, &claims.sub).await {
        return Ok(cached_user);
    }
    if user_cache
        .profile_retry_limited(&claims.iss, &claims.sub)
        .await
    {
        return Err(AppError::ServiceUnavailable(
            "User profile is temporarily unavailable; please retry login".into(),
        ));
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
        let needs_name_repair = user.display_name == FALLBACK_USER_NAME
            && user.updated_at.is_none()
            && user_cache.should_attempt_profile_repair(&user.id).await;
        let user_info = if claims.has_scope("profile") || needs_name_repair {
            fetch_userinfo_for_profile(
                jwks_verifier,
                oidc_config,
                access_token,
                claims,
                PROFILE_FETCH_TIMEOUT,
            )
            .await
        } else {
            None
        };
        sync_profile_picture(db, &user.id, claims, user_info.as_ref(), false).await?;
        let user =
            repair_legacy_display_name(db, user_cache, claims, user, user_info.as_ref()).await?;

        let auth_user = AuthUser {
            id: user.id,
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
    //    AuthZ などの access token にはプロフィール属性が含まれないため、
    //    初回ログイン時に UserInfo から名前を取得する。候補の取得に失敗した場合は作成前に再試行を返す。
    let new_user_id = Uuid::new_v4().to_string();
    let user_info = fetch_userinfo_for_profile(
        jwks_verifier,
        oidc_config,
        access_token,
        claims,
        INITIAL_PROFILE_FETCH_TIMEOUT,
    )
    .await;

    if user_info.is_none()
        && preferred_username(claims, None)
            .or_else(|| email(claims, None))
            .is_none()
    {
        user_cache
            .mark_profile_unavailable(&claims.iss, &claims.sub)
            .await;
        return Err(AppError::ServiceUnavailable(
            "User profile is temporarily unavailable; please retry login".into(),
        ));
    }

    let base_username = derive_base_username(claims, user_info.as_ref(), &new_user_id);
    let display_name = derive_display_name(claims, user_info.as_ref(), &base_username);

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
            is_bot: false,
            id: new_user_id.as_str(),
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
                    let auth_user = AuthUser {
                        id: user.id,
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
                sync_profile_picture(db, &created_user.id, claims, user_info.as_ref(), true)
                    .await?;
                let auth_user = AuthUser {
                    id: created_user.id,
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
                    let auth_user = AuthUser {
                        id: user.id,
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
) -> Result<AuthUser, AppError> {
    if dev_auth_enabled {
        // 1. Authorization ヘッダ (Bearer <UUID>)
        if let Some(auth_str) = auth_header
            && let Some(token) = auth_str.strip_prefix("Bearer ")
            && let Ok(uuid) = Uuid::parse_str(token)
        {
            return Ok(AuthUser {
                id: uuid.to_string(),
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
    pub user_name: String,
    pub display_name: String,
}

impl AuthUser {
    async fn resolve_request_with_mode(
        parts: &mut Parts,
        state: &AppState,
    ) -> (&'static str, Result<Self, AppError>) {
        let is_dev_enabled = state.config.dev_auth_enabled;

        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok());

        let x_dev_user = match parts.headers.get("X-Dev-User") {
            Some(v) => match v.to_str() {
                Ok(s) => Some(s),
                Err(_) => {
                    let mode = if is_dev_enabled { "dev" } else { "oidc" };
                    return (
                        mode,
                        Err(AppError::BadRequest(
                            "Invalid X-Dev-User header format".into(),
                        )),
                    );
                }
            },
            None => None,
        };

        // 1. Dev Auth 有効環境の処理
        if is_dev_enabled {
            if let Some(auth_str) = auth_header
                && let Some(token) = auth_str.strip_prefix("Bearer ")
            {
                // (a) UUID の場合は開発用 Bearer
                if let Ok(uuid) = Uuid::parse_str(token) {
                    return (
                        "dev",
                        Ok(AuthUser {
                            id: uuid.to_string(),
                            user_name: "dev_user".to_string(),
                            display_name: "Dev User".to_string(),
                        }),
                    );
                }

                // (b) UUID 形式でない場合、OIDC 設定があれば OIDC トークンとして検証
                if let Some(ref oidc_config) = state.config.oidc {
                    let claims = match state.jwks_verifier.verify_token(token, oidc_config).await {
                        Ok(c) => c,
                        Err(e) => return ("oidc", Err(e)),
                    };
                    let res = resolve_or_provision_user(
                        &state.db,
                        &state.user_cache,
                        &state.jwks_verifier,
                        oidc_config,
                        token,
                        &claims,
                    )
                    .await;
                    return ("oidc", res);
                }
            }

            // X-Dev-User ヘッダまたはデフォルト開発ユーザーへのフォールバック
            let default_id = std::env::var("DEV_DEFAULT_USER_ID").ok();
            let res = resolve_auth_user(
                is_dev_enabled,
                auth_header,
                x_dev_user,
                default_id.as_deref(),
            );
            return ("dev", res);
        }

        // 2. 本番環境 (is_dev_enabled == false) の処理 (fail-closed)
        // X-Dev-User や開発用デフォルトへのフォールバックは絶対禁止
        let Some(auth_str) = auth_header else {
            return (
                "oidc",
                Err(AppError::Unauthorized(
                    "Authorization header is required".to_string(),
                )),
            );
        };

        let Some(token) = auth_str.strip_prefix("Bearer ") else {
            return (
                "oidc",
                Err(AppError::Unauthorized(
                    "Authorization header must be Bearer token".to_string(),
                )),
            );
        };

        let Some(oidc_config) = state.config.oidc.as_ref() else {
            return (
                "oidc",
                Err(AppError::Internal(anyhow::anyhow!(
                    "OIDC configuration is missing"
                ))),
            );
        };

        let claims = match state.jwks_verifier.verify_token(token, oidc_config).await {
            Ok(c) => c,
            Err(e) => return ("oidc", Err(e)),
        };

        let res = resolve_or_provision_user(
            &state.db,
            &state.user_cache,
            &state.jwks_verifier,
            oidc_config,
            token,
            &claims,
        )
        .await;
        ("oidc", res)
    }

    async fn resolve_request(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let (mode, res) = Self::resolve_request_with_mode(parts, state).await;
        crate::metrics::record_auth_verification(mode, if res.is_ok() { "ok" } else { "err" });
        res
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let actor = Self::resolve_request(parts, state).await?;
        crate::apps::events::active_user(state, &actor.id).await?;
        Ok(actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_auth_user_from_x_dev_user_header() {
        let test_uuid = "11111111-1111-1111-1111-111111111111";
        let user = resolve_auth_user(true, None, Some(test_uuid), None).unwrap();
        assert_eq!(user.id, test_uuid);
    }

    #[test]
    fn test_auth_user_from_bearer_uuid_in_dev() {
        let test_uuid = "22222222-2222-2222-2222-222222222222";
        let user =
            resolve_auth_user(true, Some(&format!("Bearer {test_uuid}")), None, None).unwrap();
        assert_eq!(user.id, test_uuid);
    }

    #[test]
    fn test_auth_user_invalid_uuid() {
        let res = resolve_auth_user(true, None, Some("not-a-uuid"), None);
        assert!(matches!(res, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_auth_user_default_fallback_in_dev() {
        let user = resolve_auth_user(true, None, None, None).unwrap();
        assert_eq!(user.id, DEFAULT_DEV_USER_ID);
    }

    #[test]
    fn test_auth_user_sanitization() {
        assert_eq!(
            sanitize_for_user_name("Alice.Test_01", "default-uuid"),
            "alice.test_01"
        );
        // 予約語は fallback
        assert_eq!(
            sanitize_for_user_name("admin", "12345678-0000-0000-0000-000000000000"),
            "user_12345678"
        );
        // 空文字も fallback
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
    fn test_sanitize_for_user_name_preserves_valid_names() {
        for name in [
            "a",
            "0",
            ".",
            "-",
            "_",
            "john.doe",
            "alice_123",
            "bob-smith",
            "general",
        ] {
            assert_eq!(sanitize_for_user_name(name, "unused"), name);
        }

        let exact = "a".repeat(MAX_USER_NAME_LENGTH);
        assert_eq!(sanitize_for_user_name(&exact, "unused"), exact);
    }

    #[test]
    fn test_sanitize_for_user_name_replaces_reserved_names() {
        for reserved in RESERVED_USER_NAMES {
            let sanitized =
                sanitize_for_user_name(reserved, "12345678-0000-0000-0000-000000000000");
            assert_eq!(sanitized, "user_12345678");
            assert!(crate::validation::validate_user_name(&sanitized).is_ok());
        }
    }

    #[test]
    fn test_sanitize_for_user_name_always_returns_valid_name() {
        let fallback = "abcdef12-0000-0000-0000-000000000000";
        let too_long = "a".repeat(MAX_USER_NAME_LENGTH + 1);

        for candidate in [
            "",
            "   ",
            "あ",
            "Ａ",
            "user name",
            "user@example.com",
            "😀",
            too_long.as_str(),
        ] {
            let sanitized = sanitize_for_user_name(candidate, fallback);
            assert!(
                crate::validation::validate_user_name(&sanitized).is_ok(),
                "{candidate:?} produced invalid username {sanitized:?}"
            );
        }

        assert_eq!(sanitize_for_user_name("", fallback), "user_abcdef12");
        assert_eq!(sanitize_for_user_name("あ😀", fallback), "user_abcdef12");
        assert_eq!(
            sanitize_for_user_name(&too_long, fallback).chars().count(),
            MAX_USER_NAME_LENGTH
        );
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picture_uses_fresh_userinfo_and_keeps_existing_on_failure() {
        let mut claims: OidcClaims = serde_json::from_value(json!({
            "sub":"user", "iss":"https://id.example", "aud":"zoo", "exp":9999999999u64, "scope":"openid profile",
            "picture":"https://avater.zliusercontent.com/avatars/old.png"
        }))
        .unwrap();
        let mut info: OidcUserInfo = serde_json::from_value(json!({
            "sub":"user", "picture":"https://avater.zliusercontent.com/avatars/new.png"
        }))
        .unwrap();
        assert_eq!(
            profile_picture(&claims, Some(&info), false),
            Some(info.picture.clone())
        );
        assert_eq!(
            profile_picture(&claims, None, true),
            Some(claims.picture.clone())
        );
        info.picture = None;
        assert_eq!(profile_picture(&claims, Some(&info), false), Some(None));
        assert_eq!(profile_picture(&claims, None, false), None);
        claims.picture = None;
        assert_eq!(profile_picture(&claims, None, true), None);
        for url in [
            "javascript:alert(1)",
            "data:image/png;base64,xxx",
            "/relative.png",
            "https://user:password@example.test/image.png",
            "http://example.test/image.png",
        ] {
            info.picture = Some(url.into());
            assert_eq!(profile_picture(&claims, Some(&info), false), Some(None));
        }
    }
    #[test]
    fn picture_checks_input_and_normalized_byte_boundaries() {
        let claims: OidcClaims = serde_json::from_value(json!({
            "sub":"user", "iss":"https://id.example", "aud":"zoo", "exp":9999999999u64, "scope":"openid profile",
        }))
        .unwrap();
        let prefix = "https://example.test/";
        for (path, valid) in [
            ("a".repeat(2048 - prefix.len()), true),
            ("a".repeat(2049 - prefix.len()), false),
            ("é".repeat(400), false),
            ("é".repeat(100), true),
        ] {
            let info: OidcUserInfo =
                serde_json::from_value(json!({"sub":"user", "picture":format!("{prefix}{path}")}))
                    .unwrap();
            assert_eq!(
                profile_picture(&claims, Some(&info), false)
                    .flatten()
                    .is_some(),
                valid
            );
        }
    }
    #[test]
    fn picture_deletion_requires_verified_profile_scope() {
        for (scope, allowed) in [
            (json!("openid profile"), true),
            (json!(["openid", "profile"]), true),
            (json!("openid offline_access"), false),
            (json!(["openid"]), false),
            (json!("openid myprofile"), false),
            (json!(null), false),
            (json!({"profile":true}), false),
        ] {
            let claims: OidcClaims = serde_json::from_value(json!({
                "sub":"user", "iss":"https://id.example", "aud":"zoo", "exp":9999999999u64,
                "scope":scope, "picture":"https://images.example/token.png",
            }))
            .unwrap();
            let info: OidcUserInfo = serde_json::from_value(json!({"sub":"user"})).unwrap();
            assert_eq!(claims.has_scope("profile"), allowed);
            assert_eq!(
                profile_picture(&claims, Some(&info), false),
                if allowed { Some(None) } else { None }
            );
            if !allowed {
                assert_eq!(profile_picture(&claims, None, true), None);
            }
        }
    }
    #[test]
    fn blank_token_fields_use_available_verified_username_candidates() {
        let claims: OidcClaims = serde_json::from_value(json!({
            "sub":"user", "iss":"https://id.example", "aud":"zoo", "exp":9999999999u64,
            "preferred_username":"  ", "email":"alice@example.test",
        }))
        .unwrap();
        assert_eq!(derive_base_username(&claims, None, "unused"), "alice");
        let info: OidcUserInfo =
            serde_json::from_value(json!({"sub":"user","preferred_username":"actual_user"}))
                .unwrap();
        assert_eq!(
            derive_base_username(&claims, Some(&info), "unused"),
            "actual_user"
        );
    }
}
