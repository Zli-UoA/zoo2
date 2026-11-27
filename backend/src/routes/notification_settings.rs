//! チャンネルごとの通知設定 (#620)。
//!
//! 設定は `channel_user.notification_level` が正本で、本人の行だけを読み書きする。
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

/// 参加行の通知レベルを主キー `(user_id, channel_id)` の1回の参照で読む。
/// 削除済みチャンネルは `channels` の主キー参照を JOIN して除く。
async fn current_level(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
    channel_id: &str,
) -> Result<Option<String>, AppError> {
    let rows = crate::apps::query(
        tx,
        "SELECT json_build_object('level', cu.notification_level)::text \
         FROM channel_user cu JOIN channels c ON c.id = cu.channel_id \
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

    let updated = toasty::sql::statement(
        "UPDATE channel_user SET notification_level = $3 \
         WHERE user_id = $1 AND channel_id = $2",
    )
    .bind(auth_user.id.as_str())
    .bind(channel_id.as_str())
    .bind(payload.level.as_str())
    .exec(&mut tx)
    .await?;
    if updated == 0 {
        let _ = tx.rollback().await;
        return Err(not_found(&channel_id));
    }
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        NotificationSettingsResponse {
            level: payload.level,
        },
        txid,
    )))
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
