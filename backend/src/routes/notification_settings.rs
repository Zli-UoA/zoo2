//! チャンネルごとの通知設定 (#620)。
//!
//! 設定は `user_channel_notification_settings` が正本で、本人の行だけを読み書きする。
//! `channel_user.notification_level` は旧アプリ互換用の列で、実際の設定は読まない。
//! チャンネルが存在し未削除で、自分の参加行がある場合だけ応答し、それ以外は 404
//! (非公開チャンネルや他人の設定の存在も推測させない)。

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser, db::get_current_txid, error::AppError, routes::response::ApiResponse,
    state::AppState,
};

const LEVELS: [&str; 3] = ["all", "mentions", "mute"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettingsResponse {
    pub level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateNotificationSettingsRequest {
    pub level: String,
}

fn validate_level(level: &str) -> Result<(), AppError> {
    if LEVELS.contains(&level) {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "level must be one of all, mentions, mute".into(),
        ))
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/channels/{id}/notification-settings",
            get(get_notification_settings).patch(update_notification_settings),
        )
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

fn not_found(channel_id: &str) -> AppError {
    AppError::NotFound(format!("Channel not found with id: {channel_id}"))
}

/// 参加を確認しつつ、設定テーブルの主キー `(user_id, channel_id)` を1回の参照で読む。
/// 削除済みチャンネルは `channels` の主キー参照を JOIN して除く。設定行が無ければ
/// 既定値の `mentions` を返す (ローリング更新中に移行前のメンバーが残っていても読めるようにする)。
///
/// 参加行を `FOR UPDATE` でロックし、トランザクション終了まで退出を待たせる。
/// ロックの acquiring と設定値の読み取りを分けることで、同時 PATCH はロック解放後に
/// 現在の設定値を読み、同値 no-op を誤って返さない。
async fn lock_membership(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
) -> Result<bool, AppError> {
    let rows = crate::apps::query(
        tx,
        "SELECT '{}'::text FROM channel_user cu \
         JOIN channels c ON c.id = cu.channel_id \
         WHERE cu.user_id = $1 AND cu.channel_id = $2 AND c.deleted_at IS NULL \
         FOR UPDATE OF cu",
        &[user_id, channel_id],
    )
    .await?;
    Ok(!rows.is_empty())
}

/// 現在の通知レベルを読む。PATCH では `lock_membership` の後に呼ぶ。
async fn current_level(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
) -> Result<Option<String>, AppError> {
    let rows = crate::apps::query(
        tx,
        "SELECT json_build_object('level', COALESCE(s.notification_level, 'mentions'))::text \
         FROM channel_user cu \
         JOIN channels c ON c.id = cu.channel_id \
         LEFT JOIN user_channel_notification_settings s \
           ON s.user_id = cu.user_id AND s.channel_id = cu.channel_id \
         WHERE cu.user_id = $1 AND cu.channel_id = $2 AND c.deleted_at IS NULL",
        &[user_id, channel_id],
    )
    .await?;
    Ok(rows
        .first()
        .and_then(|row| row["level"].as_str())
        .map(str::to_owned))
}

async fn get_notification_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<NotificationSettingsResponse>, AppError> {
    let channel_id = Uuid::parse_str(&channel_id)
        .map_err(|_| not_found(&channel_id))?
        .hyphenated()
        .to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let Some(level) = current_level(&mut tx, &auth_user.id, &channel_id).await? else {
        let _ = tx.rollback().await;
        return Err(not_found(&channel_id));
    };
    tx.commit().await?;
    Ok(Json(NotificationSettingsResponse { level }))
}

async fn update_notification_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(channel_id): Path<String>,
    Json(payload): Json<UpdateNotificationSettingsRequest>,
) -> Result<Json<ApiResponse<NotificationSettingsResponse>>, AppError> {
    validate_level(&payload.level)?;
    let channel_id = Uuid::parse_str(&channel_id)
        .map_err(|_| not_found(&channel_id))?
        .hyphenated()
        .to_string();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // まず所属行をロックし、この tx が終わるまで退出と並列 PATCH を待たせる。
    // ロック取得後に別 SQL で設定値を読むことで、待っていた PATCH の最新値を反映する。
    if !lock_membership(&mut tx, &auth_user.id, &channel_id).await? {
        let _ = tx.rollback().await;
        return Err(not_found(&channel_id));
    }
    let Some(current) = current_level(&mut tx, &auth_user.id, &channel_id).await? else {
        let _ = tx.rollback().await;
        return Err(not_found(&channel_id));
    };

    // 同値なら書き込まない。WAL を増やさず、Shape の購読者にも変更を知らせない。
    // 早期 return でも接続をプールへ返すため、明示的にロールバックする。
    if current == payload.level {
        let _ = tx.rollback().await;
        return Ok(Json(ApiResponse::new(
            NotificationSettingsResponse { level: current },
            None,
        )));
    }

    // `ON CONFLICT ... WHERE notification_level <> $3` を付けることで、同時に同じ値へ更新された
    // ときの二重書き込み (WAL と Shape の refresh) を避ける。参加時に設定行が作られていない
    // 場合だけ INSERT する。0 件なら現在値を読み直し、同値なら txid: null の no-op、
    // 参加行が消えた場合だけ 404 にする。
    for _ in 0..3 {
        let updated = toasty::sql::statement(
            "INSERT INTO user_channel_notification_settings (user_id, channel_id, notification_level) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (user_id, channel_id) DO UPDATE SET notification_level = $3 \
             WHERE user_channel_notification_settings.notification_level <> $3",
        )
        .bind(auth_user.id.as_str())
        .bind(channel_id.as_str())
        .bind(payload.level.as_str())
        .exec(&mut tx)
        .await?;
        if updated > 0 {
            let txid = get_current_txid(&mut tx).await?;
            tx.commit().await?;
            return Ok(Json(ApiResponse::new(
                NotificationSettingsResponse {
                    level: payload.level,
                },
                txid,
            )));
        }
        // 所属行は最初の確認でこの tx がロック済みなので、読み直しはロックなしでよい。
        match current_level(&mut tx, &auth_user.id, &channel_id).await? {
            Some(level) if level == payload.level => {
                let _ = tx.rollback().await;
                return Ok(Json(ApiResponse::new(
                    NotificationSettingsResponse { level },
                    None,
                )));
            }
            None => {
                let _ = tx.rollback().await;
                return Err(not_found(&channel_id));
            }
            // ほかの値へ変わったなら、もう一度だけ更新を試みる。
            Some(_) => {}
        }
    }
    let _ = tx.rollback().await;
    Err(AppError::Conflict(
        "Notification level was changed concurrently".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_documented_levels() {
        for level in LEVELS {
            assert!(validate_level(level).is_ok());
        }
        for level in ["", "all ", "ALL", "loud", "mentions,mute"] {
            assert!(matches!(
                validate_level(level),
                Err(AppError::BadRequest(_))
            ));
        }
    }
}
