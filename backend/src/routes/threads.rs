use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, Message},
    routes::response::ApiResponse,
    state::AppState,
};

const THREAD_IDLE_DAYS: i64 = 1;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads", get(list_threads))
        .route(
            "/api/channels/{parent_id}/threads",
            get(list_channel_threads).post(create_channel_thread),
        )
        .route("/api/threads/{id}/join", post(join_channel_thread))
        .route("/api/threads/{id}/leave", delete(leave_channel_thread))
        .route("/api/threads/{id}/archive", post(archive_channel_thread))
        .route("/api/threads/{id}/open", post(open_channel_thread))
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

#[derive(Debug, Deserialize)]
struct CreateChannelThread {
    title: String,
    initial_post: String,
}

#[derive(Default, Deserialize)]
struct ChannelThreadQuery {
    #[serde(default)]
    archived: bool,
    search: Option<String>,
}

async fn list_channel_threads(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(parent_id): Path<String>,
    Query(query): Query<ChannelThreadQuery>,
) -> Result<Json<Value>, AppError> {
    Uuid::parse_str(&parent_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let _parent = Channel::get_by_id(&mut tx, &parent_id)
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound("Channel not found".into())
            } else {
                AppError::from(err)
            }
        })?;
    ensure_parent_thread_access(&mut tx, &parent_id, &auth_user.id).await?;
    let rows = crate::apps::query(
        &mut tx,
        "SELECT json_build_object('channel_id',t.channel_id,'parent_channel_id',t.parent_channel_id,'title',t.title,'source_message_id',t.source_message_id,'archived_at',t.archived_at,'activity_at',t.activity_at,'created_at',t.created_at,'member_count',(SELECT count(*) FROM channel_user cu WHERE cu.channel_id=t.channel_id),'is_member',EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=t.channel_id AND cu.user_id=$2),'last_message',(SELECT left(m.content,160) FROM messages m WHERE m.channel_id=t.channel_id AND m.deleted_at IS NULL ORDER BY m.created_at DESC,m.id DESC LIMIT 1))::text FROM threads t WHERE t.parent_channel_id=$1 AND $3::boolean=(t.archived_at IS NOT NULL) AND ($4='' OR t.title ILIKE '%'||$4||'%') ORDER BY t.activity_at DESC,t.channel_id DESC LIMIT 100",
        &[&parent_id, &auth_user.id, if query.archived { "true" } else { "false" }, query.search.as_deref().unwrap_or("")],
    ).await?;
    tx.commit().await?;
    Ok(Json(json!({"items":rows})))
}

async fn create_channel_thread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(parent_id): Path<String>,
    Json(payload): Json<CreateChannelThread>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    Uuid::parse_str(&parent_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    if payload.title.trim().is_empty() || payload.title.chars().count() > 100 {
        return Err(AppError::BadRequest(
            "Thread title must be between 1 and 100 characters".into(),
        ));
    }
    crate::validation::validate_message_content_with_options(&payload.initial_post, false)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let parent = Channel::get_by_id(&mut tx, &parent_id)
        .await
        .map_err(|err| {
            if err.is_record_not_found() {
                AppError::NotFound("Channel not found".into())
            } else {
                AppError::from(err)
            }
        })?;
    ensure_parent_thread_access(&mut tx, &parent_id, &auth_user.id).await?;
    if parent.is_dm || parent.archive_at.is_some() || parent.parent_channel_id.is_some() {
        return Err(AppError::BadRequest(
            "Threads cannot be created in this channel".into(),
        ));
    }
    crate::apps::events::posting(
        &mut tx,
        &auth_user.id,
        &parent_id,
        state.config.dev_auth_enabled,
    )
    .await?;

    let channel_id = Uuid::new_v4().to_string();
    let now = crate::models::now_utc_millis();
    let channel_name = format!("thread-{channel_id}");
    toasty::create!(Channel {
        id: channel_id.as_str(),
        channel_name: channel_name.as_str(),
        description: None,
        created_user_id: auth_user.id.as_str(),
        is_private: parent.is_private,
        is_nsfw: parent.is_nsfw,
        is_dm: false,
        created_at: now,
        updated_at: None,
        archive_at: None,
        deleted_at: None,
        parent_channel_id: Some(parent_id.clone()),
        thread_title: Some(payload.title.trim().to_owned()),
        thread_archived_at: None,
        thread_activity_at: Some(now),
    })
    .exec(&mut tx)
    .await?;
    toasty::sql::statement(
        "UPDATE channels SET posting_policy=(SELECT posting_policy FROM channels WHERE id=$1) WHERE id=$2",
    )
    .bind(parent_id.as_str())
    .bind(channel_id.as_str())
    .exec(&mut tx)
    .await?;
    toasty::create!(ChannelUser {
        user_id: auth_user.id.as_str(),
        channel_id: channel_id.as_str(),
        joined_at: now
    })
    .exec(&mut tx)
    .await?;
    toasty::sql::statement("INSERT INTO threads(channel_id,parent_channel_id,title,activity_at,created_at) VALUES($1,$2,$3,now() AT TIME ZONE 'UTC',now() AT TIME ZONE 'UTC')")
        .bind(channel_id.as_str()).bind(parent_id.as_str()).bind(payload.title.trim()).exec(&mut tx).await?;
    let first_message_id = Uuid::new_v4().to_string();
    toasty::create!(Message {
        id: first_message_id.as_str(),
        user_id: auth_user.id.as_str(),
        channel_id: channel_id.as_str(),
        content: payload.initial_post.as_str(),
        card: None,
        apub: false,
        apub_visibility: None,
        created_at: now,
        updated_at: None,
        deleted_at: None,
        parent_id: None,
        reply_to_message_id: None,
    })
    .exec(&mut tx)
    .await?;
    let announcement_id = Uuid::new_v4().to_string();
    let announcement = format!("🧵 {}\n/channels/{}", payload.title.trim(), channel_id);
    toasty::create!(Message {
        id: announcement_id.as_str(),
        user_id: auth_user.id.as_str(),
        channel_id: parent_id.as_str(),
        content: announcement.as_str(),
        card: None,
        apub: false,
        apub_visibility: None,
        created_at: now,
        updated_at: None,
        deleted_at: None,
        parent_id: None,
        reply_to_message_id: None,
    })
    .exec(&mut tx)
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"channel_id":channel_id,"parent_channel_id":parent_id,"title":payload.title.trim(),"first_message_id":first_message_id,"announcement_message_id":announcement_id}),
        txid,
    )))
}

pub(crate) async fn ensure_parent_thread_access(
    tx: &mut toasty::Transaction<'_>,
    parent_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    crate::apps::one(tx, "SELECT '{}'::text FROM channels c WHERE c.id=$1 AND c.deleted_at IS NULL AND NOT c.is_dm AND ((NOT c.is_private AND NOT EXISTS(SELECT 1 FROM users WHERE id=$2 AND is_guest)) OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=$2))", &[parent_id,user_id]).await?;
    Ok(())
}

async fn join_channel_thread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid thread ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let row=crate::apps::one(&mut tx,"SELECT json_build_object('parent_channel_id',parent_channel_id)::text FROM threads WHERE channel_id=$1",&[&id]).await?;
    ensure_parent_thread_access(
        &mut tx,
        row["parent_channel_id"].as_str().unwrap_or_default(),
        &auth_user.id,
    )
    .await?;
    if crate::apps::events::guest(&mut tx, &auth_user.id).await? {
        return Err(AppError::Forbidden("Guests cannot join threads".into()));
    }
    toasty::sql::statement("INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now() AT TIME ZONE 'UTC') ON CONFLICT DO NOTHING").bind(auth_user.id.as_str()).bind(id.as_str()).exec(&mut tx).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"channel_id":id,"user_id":auth_user.id}),
        txid,
    )))
}

async fn leave_channel_thread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid thread ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let rows = crate::apps::query(
        &mut tx,
        "SELECT json_build_object('parent_channel_id',parent_channel_id)::text FROM threads WHERE channel_id=$1",
        &[&id],
    )
    .await?;
    if rows.is_empty() {
        return Err(AppError::NotFound("Thread not found".into()));
    }
    let row = &rows[0];
    ensure_parent_thread_access(
        &mut tx,
        row["parent_channel_id"].as_str().unwrap_or_default(),
        &auth_user.id,
    )
    .await?;
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1 AND user_id=$2")
        .bind(id.as_str())
        .bind(auth_user.id.as_str())
        .exec(&mut tx)
        .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        json!({"channel_id":id,"user_id":auth_user.id}),
        txid,
    )))
}

async fn set_channel_thread_archived(
    state: AppState,
    auth_user: AuthUser,
    id: String,
    archived: bool,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid thread ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    crate::apps::events::actor_context(&mut tx, &auth_user.id).await?;
    let rows = crate::apps::query(
        &mut tx,
        "SELECT json_build_object('parent_channel_id',parent_channel_id)::text FROM threads WHERE channel_id=$1",
        &[&id],
    )
    .await?;
    if rows.is_empty() {
        let legacy = crate::apps::query(
            &mut tx,
            "SELECT json_build_object('channel_id',root.channel_id)::text \
             FROM legacy_thread_roots roots JOIN messages root ON root.id=roots.root_message_id \
             WHERE root.id=$1 AND root.deleted_at IS NULL",
            &[&id],
        )
        .await?;
        let Some(legacy) = legacy.into_iter().next() else {
            return Err(AppError::NotFound("Thread not found".into()));
        };
        let channel_id = legacy["channel_id"].as_str().unwrap_or_default();
        let channel = Channel::get_by_id(&mut tx, channel_id).await?;
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
            return Err(AppError::NotFound("Thread not found".into()));
        }
        let archive_value = if archived {
            "now() AT TIME ZONE 'UTC'"
        } else {
            "NULL"
        };
        let activity_value = if archived {
            "thread_activity_at"
        } else {
            "now() AT TIME ZONE 'UTC'"
        };
        let updated = crate::apps::query(
            &mut tx,
            &format!(
                "UPDATE messages SET thread_archive_at={archive_value}, \
                 thread_activity_at={activity_value} \
                 WHERE id=$1 RETURNING json_build_object('id',id,'thread_archive_at',thread_archive_at)::text"
            ),
            &[&id],
        )
        .await?;
        let txid = get_current_txid(&mut tx).await?;
        tx.commit().await?;
        return Ok(Json(ApiResponse::new(
            updated.into_iter().next().unwrap_or(Value::Null),
            txid,
        )));
    }
    let row = &rows[0];
    ensure_parent_thread_access(
        &mut tx,
        row["parent_channel_id"].as_str().unwrap_or_default(),
        &auth_user.id,
    )
    .await?;
    let member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(id.as_str())
            .and(ChannelUser::fields().user_id().eq(auth_user.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .is_some();
    if !member {
        return Err(AppError::Forbidden(
            "Join the thread before managing it".into(),
        ));
    }
    let value = if archived {
        "now() AT TIME ZONE 'UTC'"
    } else {
        "NULL"
    };
    let activity = if archived {
        "activity_at"
    } else {
        "now() AT TIME ZONE 'UTC'"
    };
    let rows = crate::apps::query(
        &mut tx,
        &format!("UPDATE threads SET archived_at={value},activity_at={activity} WHERE channel_id=$1 RETURNING json_build_object('channel_id',channel_id,'archived_at',archived_at,'activity_at',activity_at)::text"),
        &[&id],
    )
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        rows.into_iter().next().unwrap_or(Value::Null),
        txid,
    )))
}
async fn archive_channel_thread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    set_channel_thread_archived(state, auth_user, id, true).await
}
async fn open_channel_thread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    set_channel_thread_archived(state, auth_user, id, false).await
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
        &format!("WITH candidates AS ( \
           SELECT t.channel_id AS id,t.parent_channel_id AS channel_id,t.title AS content, \
             t.archived_at AS thread_archive_at,t.activity_at, \
             (SELECT count(*) FROM messages m WHERE m.channel_id=t.channel_id AND m.deleted_at IS NULL) AS reply_count, \
             false AS legacy \
           FROM threads t \
           JOIN channels child ON child.id=t.channel_id \
           JOIN channels parent ON parent.id=t.parent_channel_id \
           WHERE child.deleted_at IS NULL AND parent.deleted_at IS NULL AND NOT parent.is_dm \
             AND ($2='' OR parent.id=$2) \
             AND ((NOT parent.is_private AND NOT EXISTS(SELECT 1 FROM users WHERE id=$1 AND is_guest)) \
               OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=parent.id AND cu.user_id=$1)) \
             AND t.archived_at {archive_filter} \
           UNION ALL \
           SELECT root.id,root.channel_id,left(root.content,160),root.thread_archive_at, \
             COALESCE(root.thread_activity_at,root.created_at), \
             (SELECT count(*) FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL), \
             true AS legacy \
           FROM legacy_thread_roots legacy \
           JOIN messages root ON root.id=legacy.root_message_id \
           JOIN channels parent ON parent.id=root.channel_id \
           WHERE root.deleted_at IS NULL AND parent.deleted_at IS NULL AND NOT parent.is_dm \
             AND ($2='' OR parent.id=$2) \
             AND EXISTS(SELECT 1 FROM messages reply WHERE reply.parent_id=root.id AND reply.deleted_at IS NULL) \
             AND EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=parent.id AND cu.user_id=$1) \
             AND root.thread_archive_at {archive_filter} \
             AND NOT EXISTS(SELECT 1 FROM threads migrated WHERE migrated.source_message_id=root.id) \
         ), page AS ( \
           SELECT * FROM candidates \
           WHERE (activity_at,id)<((COALESCE(NULLIF($3,'')::timestamptz,'infinity'::timestamptz) AT TIME ZONE 'UTC'),$4) \
             AND (activity_at,id)>=((COALESCE(NULLIF($6,'')::timestamptz,'-infinity'::timestamptz) AT TIME ZONE 'UTC'),$7) \
           ORDER BY activity_at DESC,id DESC LIMIT $5::int \
         ) SELECT json_build_object('id',page.id,'channel_id',page.channel_id, \
           'content',page.content,'thread_archive_at',page.thread_archive_at, \
           '_activity_at',page.activity_at,'reply_count',page.reply_count,'legacy',page.legacy)::text \
         FROM page ORDER BY page.activity_at DESC,page.id DESC"),
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
    toasty::sql::statement(
        "WITH idle AS ( \
           SELECT channel_id FROM threads \
           WHERE archived_at IS NULL AND activity_at < (now() AT TIME ZONE 'UTC') - ($1::int * interval '1 day') \
           ORDER BY activity_at,channel_id LIMIT 1000 FOR UPDATE SKIP LOCKED \
         ) UPDATE threads SET archived_at=now() AT TIME ZONE 'UTC' FROM idle WHERE threads.channel_id=idle.channel_id",
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
