use std::collections::HashSet;

use axum::{Json, Router, extract::State, routing::post};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, User},
    routes::{channels::ChannelResponse, response::ApiResponse},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateDmRequest {
    pub id: Option<String>,
    pub target_user_ids: Vec<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/dms", post(create_or_get_dm))
}

async fn try_create_or_get_1on1_dm(
    db: &mut toasty::Db,
    auth_user_id: &str,
    target_id: &str,
    channel_name: &str,
    custom_id: Option<String>,
) -> Result<Option<ApiResponse<ChannelResponse>>, AppError> {
    let mut tx = db.transaction().await?;

    let existing = Channel::filter(
        Channel::fields()
            .channel_name()
            .eq(channel_name)
            .and(Channel::fields().is_dm().eq(true))
            .and(Channel::fields().deleted_at().is_none()),
    )
    .first()
    .exec(&mut tx)
    .await?;

    if let Some(existing_channel) = existing {
        let mut added_any = false;
        let now = crate::models::now_utc_millis();

        // 1対1 DM の両者について membership を確認し、退出済みのユーザーをすべて復帰させる
        for uid in [auth_user_id, target_id] {
            let membership = ChannelUser::filter(
                ChannelUser::fields()
                    .channel_id()
                    .eq(existing_channel.id.as_str())
                    .and(ChannelUser::fields().user_id().eq(uid)),
            )
            .first()
            .exec(&mut tx)
            .await?;

            if membership.is_none() {
                let res = toasty::create!(ChannelUser {
                    user_id: uid,
                    channel_id: existing_channel.id.as_str(),
                    joined_at: now,
                })
                .exec(&mut tx)
                .await;

                if let Err(err) = res {
                    let _ = tx.rollback().await;
                    // channel_user_pkey 衝突の場合は並行追加による競合なのでリトライ
                    if AppError::is_unique_violation(&err, Some("channel_user_pkey")) {
                        tracing::warn!(
                            "Concurrent channel_user insert conflict (channel_user_pkey), retrying: {err}"
                        );
                        return Ok(None);
                    }
                    return Err(AppError::from(err));
                }
                added_any = true;
            }
        }

        if !added_any {
            // 両者とも既に所属中: WAL 書き込みなしで既存チャンネルを返却 (txid: None)
            return Ok(Some(ApiResponse::new(
                ChannelResponse::from(existing_channel),
                None,
            )));
        }

        let txid = get_current_txid(&mut tx).await?;
        tx.commit().await?;

        return Ok(Some(ApiResponse::new(
            ChannelResponse::from(existing_channel),
            txid,
        )));
    }

    // 1対1 DM の新規作成
    let channel_id = custom_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = crate::models::now_utc_millis();

    let channel_res = toasty::create!(Channel {
        id: channel_id.as_str(),
        channel_name: channel_name,
        description: None,
        created_user_id: auth_user_id,
        is_private: true,
        is_dm: true,
        created_at: now,
        updated_at: None,
        archive_at: None,
        deleted_at: None,
    })
    .exec(&mut tx)
    .await;

    let channel = match channel_res {
        Ok(c) => c,
        Err(err) => {
            let _ = tx.rollback().await;
            // channels_channel_name_key 衝突の場合は並行作成競合なのでリトライ
            if AppError::is_unique_violation(&err, Some("channels_channel_name_key")) {
                tracing::warn!(
                    "Concurrent 1-on-1 DM channel creation conflict (channels_channel_name_key), retrying: {err}"
                );
                return Ok(None);
            }
            return Err(AppError::from(err));
        }
    };

    if let Err(err) = toasty::create!(ChannelUser {
        user_id: auth_user_id,
        channel_id: channel_id.as_str(),
        joined_at: now,
    })
    .exec(&mut tx)
    .await
    {
        let _ = tx.rollback().await;
        if AppError::is_unique_violation(&err, Some("channel_user_pkey")) {
            tracing::warn!(
                "Concurrent channel_user insert conflict (channel_user_pkey), retrying: {err}"
            );
            return Ok(None);
        }
        return Err(AppError::from(err));
    }

    if let Err(err) = toasty::create!(ChannelUser {
        user_id: target_id,
        channel_id: channel_id.as_str(),
        joined_at: now,
    })
    .exec(&mut tx)
    .await
    {
        let _ = tx.rollback().await;
        if AppError::is_unique_violation(&err, Some("channel_user_pkey")) {
            tracing::warn!(
                "Concurrent channel_user insert conflict (channel_user_pkey), retrying: {err}"
            );
            return Ok(None);
        }
        return Err(AppError::from(err));
    }

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Some(ApiResponse::new(ChannelResponse::from(channel), txid)))
}

async fn create_or_get_dm(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<CreateDmRequest>,
) -> Result<Json<ApiResponse<ChannelResponse>>, AppError> {
    if payload.target_user_ids.is_empty() {
        return Err(AppError::BadRequest(
            "target_user_ids must not be empty".into(),
        ));
    }

    // 自身を含め最大 10 名 (ターゲットは最大 9 名)
    if payload.target_user_ids.len() > 9 {
        return Err(AppError::BadRequest(
            "Direct message group cannot exceed 10 members (including yourself)".into(),
        ));
    }

    if let Some(ref id) = payload.id {
        Uuid::parse_str(id)
            .map_err(|_| AppError::BadRequest(format!("channel id is not a valid UUID: {id}")))?;
    }

    // 重複チェック
    let mut seen = HashSet::new();
    for uid in &payload.target_user_ids {
        Uuid::parse_str(uid)
            .map_err(|_| AppError::BadRequest(format!("user_id is not a valid UUID: {uid}")))?;

        if uid == &auth_user.id {
            return Err(AppError::BadRequest(
                "Cannot include yourself in target_user_ids".into(),
            ));
        }

        if !seen.insert(uid) {
            return Err(AppError::BadRequest(
                "target_user_ids must not contain duplicate users".into(),
            ));
        }
    }

    let mut db = state.db.clone();

    // 参加予定ユーザーの存在確認 (リード操作のためトランザクション外で実行)
    for uid in &payload.target_user_ids {
        let target = User::get_by_id(&mut db, uid.as_str())
            .await
            .map_err(|err| {
                if err.is_record_not_found() {
                    AppError::BadRequest(format!("Target user not found: {uid}"))
                } else {
                    AppError::from(err)
                }
            })?;
        if target.is_bot {
            return Err(AppError::BadRequest(
                "Cannot start a DM with an integration user".into(),
            ));
        }
    }

    // 1対1 DM
    // 並行作成・並行復帰による UNIQUE 制約違反 (channels_channel_name_key, channel_user_pkey) に備え、
    // 競合時は最大 1 回リトライして既存チャンネルの取得・復帰処理を行う。
    if payload.target_user_ids.len() == 1 {
        let target_id = &payload.target_user_ids[0];
        let (user_a, user_b) = if auth_user.id < *target_id {
            (auth_user.id.as_str(), target_id.as_str())
        } else {
            (target_id.as_str(), auth_user.id.as_str())
        };
        let channel_name = format!("dm:{user_a}:{user_b}");

        for _ in 0..2 {
            if let Some(resp) = try_create_or_get_1on1_dm(
                &mut db,
                &auth_user.id,
                target_id,
                &channel_name,
                payload.id.clone(),
            )
            .await?
            {
                return Ok(Json(resp));
            }
        }

        return Err(AppError::Conflict(
            "Failed to create or get direct message channel due to concurrent conflict. Please retry.".into(),
        ));
    }

    // グループ DM の新規作成
    let mut tx = db.transaction().await?;
    let channel_id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let group_uuid = Uuid::new_v4().to_string();
    let channel_name = format!("dm:group:{group_uuid}");
    let now = crate::models::now_utc_millis();

    let channel = toasty::create!(Channel {
        id: channel_id.as_str(),
        channel_name: channel_name.as_str(),
        description: None,
        created_user_id: auth_user.id.as_str(),
        is_private: true,
        is_dm: true,
        created_at: now,
        updated_at: None,
        archive_at: None,
        deleted_at: None,
    })
    .exec(&mut tx)
    .await?;

    // 自分を追加
    toasty::create!(ChannelUser {
        user_id: auth_user.id.as_str(),
        channel_id: channel_id.as_str(),
        joined_at: now,
    })
    .exec(&mut tx)
    .await?;

    // ターゲットユーザー全員を追加
    for uid in &payload.target_user_ids {
        toasty::create!(ChannelUser {
            user_id: uid.as_str(),
            channel_id: channel_id.as_str(),
            joined_at: now,
        })
        .exec(&mut tx)
        .await?;
    }

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(ChannelResponse::from(channel), txid)))
}
