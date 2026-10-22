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
    let mut tx = db.transaction().await?;

    // 参加予定ユーザーの存在確認
    for uid in &payload.target_user_ids {
        User::get_by_id(&mut tx, uid.as_str())
            .await
            .map_err(|err| {
                if err.is_record_not_found() {
                    AppError::BadRequest(format!("Target user not found: {uid}"))
                } else {
                    AppError::from(err)
                }
            })?;
    }

    // 1対1 DM
    if payload.target_user_ids.len() == 1 {
        let target_id = &payload.target_user_ids[0];
        let (user_a, user_b) = if auth_user.id < *target_id {
            (auth_user.id.as_str(), target_id.as_str())
        } else {
            (target_id.as_str(), auth_user.id.as_str())
        };
        let channel_name = format!("dm:{user_a}:{user_b}");

        let existing = Channel::filter(
            Channel::fields()
                .channel_name()
                .eq(channel_name.as_str())
                .and(Channel::fields().is_dm().eq(true))
                .and(Channel::fields().deleted_at().is_none()),
        )
        .first()
        .exec(&mut tx)
        .await?;

        if let Some(existing_channel) = existing {
            let membership = ChannelUser::filter(
                ChannelUser::fields()
                    .channel_id()
                    .eq(existing_channel.id.as_str())
                    .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
            )
            .first()
            .exec(&mut tx)
            .await?;

            if membership.is_some() {
                // 既に所属中: WAL 書き込みなしで既存チャンネルを返却 (txid: None)
                return Ok(Json(ApiResponse::new(
                    ChannelResponse::from(existing_channel),
                    None,
                )));
            }

            // 過去に退出していた場合: channel_user に再追加して復帰 (WAL 書き込みあり)
            let now = crate::models::now_utc_millis();
            toasty::create!(ChannelUser {
                user_id: auth_user.id.as_str(),
                channel_id: existing_channel.id.as_str(),
                joined_at: now,
            })
            .exec(&mut tx)
            .await?;

            let txid = get_current_txid(&mut tx).await?;
            tx.commit().await?;

            return Ok(Json(ApiResponse::new(
                ChannelResponse::from(existing_channel),
                txid,
            )));
        }

        // 1対1 DM の新規作成
        let channel_id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
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

        toasty::create!(ChannelUser {
            user_id: auth_user.id.as_str(),
            channel_id: channel_id.as_str(),
            joined_at: now,
        })
        .exec(&mut tx)
        .await?;

        toasty::create!(ChannelUser {
            user_id: target_id.as_str(),
            channel_id: channel_id.as_str(),
            joined_at: now,
        })
        .exec(&mut tx)
        .await?;

        let txid = get_current_txid(&mut tx).await?;
        tx.commit().await?;

        return Ok(Json(ApiResponse::new(ChannelResponse::from(channel), txid)));
    }

    // グループ DM の新規作成
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
