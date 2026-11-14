//! E2EE の直前の再認証（`X-Zoo-Reauth`）。仕様は `docs/e2ee.md` の「クロス署名」の再認証の確かめ方。
//!
//! クライアントは `max_age=0` で AuthZ にログインし直して受け取った ID トークンを、再認証が要る要求の
//! `X-Zoo-Reauth` に入れる。サーバーは署名、`iss`、`aud`（`OIDC_REAUTH_AUDIENCES`）、`exp`、`sub` と、
//! AuthZ が直前に認証した時刻の claim `authenticated_at`（AuthZ#97）が 5 分以内であることを確かめる。

use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;

use crate::{auth::AuthUser, error::AppError, routes::e2ee::coded, state::AppState};

/// 再認証の ID トークンを入れるヘッダー。
pub const REAUTH_HEADER: &str = "x-zoo-reauth";
/// `authenticated_at` から再認証として認める時間（秒）。
pub const REAUTH_MAX_AGE_SECS: i64 = 5 * 60;
/// `authenticated_at` が今より先でも認める時計のずれ（秒）。
const CLOCK_SKEW_SECS: i64 = 60;

pub(crate) fn reauth_required() -> AppError {
    coded(
        StatusCode::FORBIDDEN,
        "E2EE_REAUTH_REQUIRED",
        "Recent re-authentication is required",
        Value::Null,
    )
}

/// `authenticated_at` が `now`（秒）から見て再認証として有効か。
pub(crate) fn fresh_authentication(authenticated_at: Option<i64>, now: i64) -> bool {
    authenticated_at
        .is_some_and(|at| at <= now + CLOCK_SKEW_SECS && now - at <= REAUTH_MAX_AGE_SECS)
}

/// 要求の再認証の結果。要るかどうかは DB の状態で決まるので、ハンドラーはトランザクションを始める前に
/// これを作り（ID トークンの検証で別の接続を使うため。ユーザー単位のロックを持ったまま待たない）、
/// 要ると分かった時点で [`Reauth::require`] を呼ぶ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reauth(bool);

impl Reauth {
    /// Bot の端末のクロス署名鍵の登録（#366）。再認証を求めない（`CrossSigningAuth::BotDevice` の説明）。
    pub(crate) fn bot_device() -> Self {
        Self(true)
    }

    /// 直前の再認証がなければ 403 `E2EE_REAUTH_REQUIRED`。
    pub(crate) fn require(self) -> Result<(), AppError> {
        if self.0 {
            Ok(())
        } else {
            Err(reauth_required())
        }
    }
}

/// `X-Zoo-Reauth` を確かめる。ヘッダーがない、または検証できなければ「再認証なし」。
///
/// OIDC を設定していない開発環境（`DEV_AUTH_ENABLED` が有効で、IdP がない）では、常に「再認証あり」とする。
/// 本番は OIDC の設定が必須で、`DEV_AUTH_ENABLED` も効かないので、この分岐には入らない。
pub(crate) async fn reauth(
    state: &AppState,
    headers: &HeaderMap,
    actor: &AuthUser,
) -> Result<Reauth, AppError> {
    let Some(oidc) = state.config.oidc.as_ref() else {
        return Ok(Reauth(state.config.dev_auth_enabled));
    };
    let Some(token) = headers
        .get(REAUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(Reauth(false));
    };
    let claims = match state
        .jwks_verifier
        .verify_id_token(token, oidc, &oidc.reauth_audiences)
        .await
    {
        Ok(claims) => claims,
        // JWKS を取得できないなど、IdP 側の障害はそのまま返す（利用者が再認証し直しても直らないため）。
        Err(err @ (AppError::BadGateway(_) | AppError::ServiceUnavailable(_))) => return Err(err),
        Err(_) => return Ok(Reauth(false)),
    };
    let now = jiff::Timestamp::now().as_second();
    if !fresh_authentication(claims.authenticated_at, now) {
        return Ok(Reauth(false));
    }
    // ID トークンの sub が、アクセストークン（または開発用の認証）のユーザーと同じ人か。
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let user = crate::routes::e2ee::first(
        &mut tx,
        "SELECT to_json(user_id)::text FROM user_identities WHERE iss = $1 AND sub = $2",
        &[&claims.iss, &claims.sub],
    )
    .await?;
    tx.commit().await?;
    Ok(Reauth(
        user.as_ref().and_then(Value::as_str) == Some(actor.id.as_str()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_must_be_within_five_minutes() {
        let now = 1_800_000_000;
        assert!(fresh_authentication(Some(now), now));
        assert!(fresh_authentication(Some(now - REAUTH_MAX_AGE_SECS), now));
        assert!(!fresh_authentication(
            Some(now - REAUTH_MAX_AGE_SECS - 1),
            now
        ));
        assert!(fresh_authentication(Some(now + CLOCK_SKEW_SECS), now));
        assert!(!fresh_authentication(Some(now + CLOCK_SKEW_SECS + 1), now));
        // QR ログインやリフレッシュで発行した ID トークンには claim がない。
        assert!(!fresh_authentication(None, now));
    }
}
