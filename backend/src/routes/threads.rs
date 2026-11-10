use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::{get, post},
};
use serde_json::Value;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, Message},
    routes::response::ApiResponse,
    state::AppState,
};

const THREAD_IDLE_DAYS: i64 = 7;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads", get(list_threads))
        .route("/api/threads/{id}/unarchive", post(unarchive_thread))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
}

async fn list_threads(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<Vec<Value>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let guest = crate::apps::events::guest(&mut tx, &auth_user.id).await?;
    let rows = crate::apps::query(
        &mut tx,
        "SELECT json_build_object('id',root.id,'channel_id',root.channel_id, \
         'content',left(root.content,160),'thread_archive_at',root.thread_archive_at, \
         'reply_count',(SELECT count(*) FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL))::text \
         FROM messages root JOIN channels c ON c.id=root.channel_id \
         WHERE root.deleted_at IS NULL AND c.deleted_at IS NULL AND NOT c.is_dm \
           AND EXISTS(SELECT 1 FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL) \
           AND ((NOT c.is_private AND NOT $2::boolean) OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=$1)) \
         ORDER BY GREATEST(COALESCE(root.thread_activity_at,root.created_at), \
           COALESCE((SELECT max(reply.created_at) FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL),root.created_at)) DESC",
        &[auth_user.id.as_str(), if guest { "true" } else { "false" }],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(rows))
}

async fn unarchive_thread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    uuid::Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid message ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;

    let root = Message::filter(Message::fields().id().eq(id.as_str()))
        .first()
        .exec(&mut tx)
        .await?
        .filter(|message| message.deleted_at.is_none())
        .ok_or_else(|| AppError::NotFound("Thread not found".into()))?;
    let channel_id = root.channel_id.as_str();
    let channel = Channel::get_by_id(&mut tx, channel_id)
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound(format!("Thread not found: {id}"))
            } else {
                AppError::from(err)
            }
        })?;

    let member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id)
            .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .is_some();
    if channel.deleted_at.is_some() || channel.archive_at.is_some() || !member {
        return Err(AppError::NotFound(format!("Thread not found: {id}")));
    }
    crate::apps::events::posting(
        &mut tx,
        &auth_user.id,
        channel_id,
        state.config.dev_auth_enabled,
    )
    .await?;

    let mut updated = crate::apps::query(
        &mut tx,
        "UPDATE messages SET thread_archive_at=NULL,thread_activity_at=now() AT TIME ZONE 'UTC' \
         WHERE id=$1 AND thread_archive_at IS NOT NULL \
         RETURNING json_build_object('id',id,'thread_archive_at',thread_archive_at)::text",
        &[id.as_str()],
    )
    .await?;
    let (row, txid) = if let Some(row) = updated.pop() {
        (row, Some(get_current_txid(&mut tx).await?))
    } else {
        let row = crate::apps::one(
            &mut tx,
            "SELECT json_build_object('id',id,'thread_archive_at',thread_archive_at)::text FROM messages WHERE id=$1",
            &[id.as_str()],
        ).await?;
        (row, None)
    };
    tx.commit().await?;

    Ok(Json(ApiResponse::new(row, txid)))
}

pub async fn archive_idle_threads(db: &toasty::Db) -> Result<(), AppError> {
    let mut db = db.clone();
    toasty::sql::statement(
        "WITH idle AS ( \
           SELECT root.id FROM messages root \
           WHERE root.deleted_at IS NULL \
             AND root.thread_archive_at IS NULL \
             AND EXISTS(SELECT 1 FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL) \
             AND GREATEST(COALESCE(root.thread_activity_at,root.created_at), \
                 COALESCE((SELECT max(reply.created_at) FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL),root.created_at)) \
               < (now() AT TIME ZONE 'UTC') - ($1::int * interval '1 day') \
           ORDER BY root.created_at LIMIT 1000 FOR UPDATE SKIP LOCKED \
         ) UPDATE messages root SET thread_archive_at=now() AT TIME ZONE 'UTC' FROM idle WHERE root.id=idle.id",
    )
    .bind(THREAD_IDLE_DAYS.to_string())
    .exec(&mut db)
    .await?;
    Ok(())
}

pub async fn worker(state: AppState) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        ticks.tick().await;
        if let Err(error) = archive_idle_threads(&state.db).await {
            tracing::warn!(error = %error, "Thread auto-archive worker failed");
        }
    }
}
