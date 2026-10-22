use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::AppError,
    models::{Attachment, Channel, ChannelUser, Message, now_utc_millis},
    routes::response::ApiResponse,
    state::AppState,
    storage::{generate_presigned_get_url, generate_presigned_put_url},
};

#[derive(Debug, Deserialize)]
pub struct PresignAttachmentRequest {
    pub channel_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub content_type: String,
}

#[derive(Debug, Serialize)]
pub struct PresignAttachmentResponse {
    pub attachment_id: String,
    pub upload_url: String,
    pub storage_key: String,
}

#[derive(Debug, Serialize)]
pub struct AttachmentUrlResponse {
    pub id: String,
    pub download_url: String,
    pub file_name: String,
    pub file_size: i64,
    pub content_type: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/attachments/presign", post(presign_attachment))
        .route("/api/attachments/{id}/url", get(get_attachment_url))
}

/// 危険な実行ファイル・スクリプトなどのアップロードを拒否する
fn validate_upload_mime(content_type: &str) -> Result<(), AppError> {
    let lower = content_type.to_ascii_lowercase();
    let base_mime = lower.split(';').next().unwrap_or("").trim();

    if base_mime.is_empty() {
        return Err(AppError::BadRequest("content_type cannot be empty".into()));
    }

    // 実行可能ファイルや危険な拡張フォーマットのブロック
    if matches!(
        base_mime,
        "application/x-msdownload"
            | "application/x-executable"
            | "application/x-dosexec"
            | "application/x-msdos-program"
            | "application/x-sharedlib"
            | "application/x-batch"
            | "application/x-sh"
    ) {
        return Err(AppError::BadRequest(format!(
            "Disallowed content-type: {base_mime}"
        )));
    }

    Ok(())
}

async fn presign_attachment(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<PresignAttachmentRequest>,
) -> Result<Json<ApiResponse<PresignAttachmentResponse>>, AppError> {
    // チャンネル ID 検証
    Uuid::parse_str(&payload.channel_id).map_err(|_| {
        AppError::BadRequest(format!(
            "channel_id is not a valid UUID: {}",
            payload.channel_id
        ))
    })?;

    // ファイル名バリデーション
    let file_name = payload.file_name.trim();
    if file_name.is_empty() {
        return Err(AppError::BadRequest("file_name cannot be empty".into()));
    }
    if file_name.len() > 255 {
        return Err(AppError::BadRequest(
            "file_name exceeds 255 characters".into(),
        ));
    }

    // ファイルサイズバリデーション
    if payload.file_size <= 0 {
        return Err(AppError::BadRequest("file_size must be positive".into()));
    }
    if payload.file_size as u64 > state.config.s3.max_file_size {
        return Err(AppError::BadRequest(format!(
            "file_size exceeds maximum allowed size ({} bytes)",
            state.config.s3.max_file_size
        )));
    }

    validate_upload_mime(&payload.content_type)?;

    let mut db = state.db.clone();

    // チャンネルの存在・アーカイブ・閲覧権限の確認
    let channel = Channel::get_by_id(&mut db, payload.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", payload.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            payload.channel_id
        )));
    }

    if channel.archive_at.is_some() {
        return Err(AppError::BadRequest(
            "Cannot attach files to an archived channel".into(),
        ));
    }

    // プライベートチャンネルの場合、所属していなければ 404 で存在を隠蔽
    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(payload.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut db)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!(
                "Channel not found: {}",
                payload.channel_id
            )));
        }
    }

    let attachment_id = Uuid::new_v4().to_string();
    let storage_key = format!("attachments/{}/{attachment_id}", payload.channel_id);

    let upload_url = generate_presigned_put_url(
        &state.s3_client,
        &state.config.s3.bucket,
        &storage_key,
        payload.file_size,
        &payload.content_type,
        Duration::from_secs(state.config.s3.presigned_put_expiry_secs),
    )
    .await?;

    let now = now_utc_millis();

    // 先行 DB レコード作成 (message_id は None)
    toasty::create!(Attachment {
        id: attachment_id.clone(),
        message_id: None,
        uploader_id: auth_user.id,
        channel_id: payload.channel_id,
        file_name: file_name.to_string(),
        file_size: payload.file_size,
        content_type: payload.content_type,
        storage_key: storage_key.clone(),
        created_at: now,
    })
    .exec(&mut db)
    .await?;

    // message_id IS NULL のレコードは Shape プロキシで除外されるため txid は None を返却
    Ok(Json(ApiResponse::new(
        PresignAttachmentResponse {
            attachment_id,
            upload_url,
            storage_key,
        },
        None,
    )))
}

async fn get_attachment_url(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<AttachmentUrlResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("attachment id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();

    let attachment = Attachment::get_by_id(&mut db, id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Attachment not found: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    // チャンネルの存在・未削除・閲覧権限の確認
    let channel = Channel::get_by_id(&mut db, attachment.channel_id.as_str())
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Channel not found: {}", attachment.channel_id))
            } else {
                AppError::from(err)
            }
        })?;

    if channel.deleted_at.is_some() {
        return Err(AppError::NotFound(format!(
            "Channel not found: {}",
            attachment.channel_id
        )));
    }

    if channel.is_private {
        let membership = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(attachment.channel_id.as_str())
                .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
        )
        .first()
        .exec(&mut db)
        .await?;

        if membership.is_none() {
            return Err(AppError::NotFound(format!("Attachment not found: {id}")));
        }
    }

    // 紐付け状態によるアクセス制御
    match &attachment.message_id {
        None => {
            // 未紐付けの添付（投稿前プレビュー用途）: アップロード本人のみ取得可能
            if attachment.uploader_id != auth_user.id {
                return Err(AppError::NotFound(format!("Attachment not found: {id}")));
            }
        }
        Some(msg_id) => {
            // 紐付け済み添付: 親メッセージの論理削除状態を確認
            let message = Message::get_by_id(&mut db, msg_id.as_str())
                .await
                .map_err(|err| {
                    if err.is_record_not_found() {
                        AppError::NotFound(format!("Attachment not found: {id}"))
                    } else {
                        AppError::from(err)
                    }
                })?;

            if message.deleted_at.is_some() {
                return Err(AppError::NotFound(format!("Attachment not found: {id}")));
            }
        }
    }

    let download_url = generate_presigned_get_url(
        &state.s3_client,
        &state.config.s3.bucket,
        &attachment.storage_key,
        &attachment.content_type,
        &attachment.file_name,
        Duration::from_secs(state.config.s3.presigned_get_expiry_secs),
    )
    .await?;

    Ok(Json(ApiResponse::new(
        AttachmentUrlResponse {
            id: attachment.id,
            download_url,
            file_name: attachment.file_name,
            file_size: attachment.file_size,
            content_type: attachment.content_type,
        },
        None,
    )))
}
