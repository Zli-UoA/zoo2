use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

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

#[derive(Default, Deserialize)]
struct ThreadQuery {
    channel_id: Option<String>,
    #[serde(default)]
    archived: bool,
    before: Option<String>,
    through: Option<String>,
    limit: Option<usize>,
}

async fn list_threads(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(page): Query<ThreadQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = page.limit.unwrap_or(100).clamp(1, 100);
    let parse_cursor = |value: &str| -> Result<[String; 2], AppError> {
        let cursor: [String; 2] = serde_json::from_str(value)
            .map_err(|_| AppError::BadRequest("Invalid thread cursor".into()))?;
        cursor[0]
            .parse::<jiff::Timestamp>()
            .map_err(|_| AppError::BadRequest("Invalid thread cursor".into()))?;
        Ok(cursor)
    };
    let cursor = page.before.as_deref().map(parse_cursor).transpose()?;
    let through = page.through.as_deref().map(parse_cursor).transpose()?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let archive_filter = if page.archived {
        "IS NOT NULL"
    } else {
        "IS NULL"
    };
    let mut rows = crate::apps::query(
        &mut tx,
        &format!("WITH joined_channels AS MATERIALIZED ( \
           SELECT c.id FROM channel_user cu JOIN channels c ON c.id=cu.channel_id \
           WHERE cu.user_id=$1 AND c.deleted_at IS NULL AND NOT c.is_dm AND ($2='' OR c.id=$2) \
         ), candidates AS ( \
           SELECT root.* FROM joined_channels c \
           CROSS JOIN LATERAL ( \
             SELECT root.id,root.channel_id,root.content,root.thread_archive_at,root.thread_activity_at AS activity_at \
             FROM messages root WHERE root.channel_id=c.id AND root.deleted_at IS NULL \
               AND root.thread_activity_at IS NOT NULL \
               AND root.thread_archive_at {archive_filter} \
               AND (root.thread_activity_at,root.id)<((COALESCE(NULLIF($3,'')::timestamptz,'infinity'::timestamptz) AT TIME ZONE 'UTC'),$4) \
               AND (root.thread_activity_at,root.id)>=((COALESCE(NULLIF($6,'')::timestamptz,'-infinity'::timestamptz) AT TIME ZONE 'UTC'),$7) \
               AND EXISTS(SELECT 1 FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL) \
             ORDER BY root.thread_activity_at DESC,root.id DESC LIMIT $5::int \
           ) root \
         ), page AS ( \
           SELECT * FROM candidates ORDER BY activity_at DESC,id DESC LIMIT $5::int \
         ) SELECT json_build_object('id',root.id,'channel_id',root.channel_id, \
           'content',left(root.content,160),'thread_archive_at',root.thread_archive_at, \
           '_activity_at',root.activity_at, \
           'reply_count',(SELECT count(*) FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL))::text \
         FROM page root ORDER BY root.activity_at DESC,root.id DESC"),
        &[
            auth_user.id.as_str(), page.channel_id.as_deref().unwrap_or(""),
            cursor.as_ref().map(|v| v[0].as_str()).unwrap_or(""),
            cursor.as_ref().map(|v| v[1].as_str()).unwrap_or(""),
            &(limit + 1).to_string(),
            through.as_ref().map(|v| v[0].as_str()).unwrap_or(""),
            through.as_ref().map(|v| v[1].as_str()).unwrap_or(""),
        ],
    ).await?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if more {
        rows.last()
            .and_then(|row| row["_activity_at"].as_str().zip(row["id"].as_str()))
            .map(|(at, id)| json!([format!("{at}Z"), id]).to_string())
    } else {
        None
    };
    for row in &mut rows {
        if let Some(object) = row.as_object_mut() {
            object.remove("_activity_at");
        }
    }
    tx.commit().await?;
    Ok(Json(json!({"items":rows,"next_cursor":next_cursor})))
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
             AND root.thread_activity_at IS NOT NULL \
             AND root.thread_activity_at < (now() AT TIME ZONE 'UTC') - ($1::int * interval '1 day') \
           ORDER BY root.thread_activity_at,root.id LIMIT 1000 FOR UPDATE SKIP LOCKED \
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
