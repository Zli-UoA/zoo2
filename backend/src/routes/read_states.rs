use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    middleware,
    response::Response,
    routing::get,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelUser, Message},
    routes::response::ApiResponse,
    state::AppState,
};

#[derive(Deserialize)]
pub struct MarkReadRequest {
    message_id: String,
}

#[derive(Deserialize)]
pub struct DetailsQuery {
    details: Option<String>,
}

impl DetailsQuery {
    fn requested(&self) -> bool {
        matches!(self.details.as_deref(), Some("1" | "true"))
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channel-read-states", get(list))
        .route(
            "/api/channels/{channel_id}/read-state",
            get(show).post(mark_read),
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

async fn authorize(
    tx: &mut toasty::Transaction<'_>,
    actor: &str,
    channel_id: &str,
) -> Result<(), AppError> {
    Uuid::parse_str(channel_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    let channel = Channel::filter(Channel::fields().id().eq(channel_id))
        .first()
        .exec(&mut *tx)
        .await?
        .filter(|c| c.deleted_at.is_none())
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;
    if channel.is_private || channel.is_dm || crate::apps::events::guest(tx, actor).await? {
        let member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(channel_id)
                .and(ChannelUser::fields().user_id().eq(actor)),
        )
        .first()
        .exec(&mut *tx)
        .await?;
        if member.is_none() {
            return Err(AppError::NotFound("Channel not found".into()));
        }
    }
    Ok(())
}

/// Unread authors inspected per channel when the catch-up screen asks for details.
pub const UNREAD_AUTHOR_LIMIT: i64 = 100;
/// Root messages counted per unread author. One extra row signals "1000+".
pub const UNREAD_PER_AUTHOR_SCAN: i64 = 1_001;
/// Largest exact unread count reported by the API.
pub const UNREAD_COUNT_MAX: i64 = 1_000;

/// The production SQL is shared with bounded-work regression tests.
///
/// `details = false` only uses the author watermarks and never scans messages;
/// the 10-second sidebar poll and the mark-read response use that form. The
/// catch-up screen asks for `details = true` once when it opens the queue.
#[doc(hidden)]
pub fn states_sql(single: bool, details: bool) -> String {
    let scope = match single {
        true => "c.id = $2".to_string(),
        false => {
            "EXISTS (SELECT 1 FROM channel_user cu WHERE cu.channel_id = c.id AND cu.user_id = $1)"
                .to_string()
        }
    };
    let (detail_fields, detail_join) = if details {
        (
            format!(
                r#",'unread_count', LEAST(COALESCE(u.n, 0), {UNREAD_COUNT_MAX}),
        'unread_count_truncated', COALESCE(u.n, 0) > {UNREAD_COUNT_MAX}
            OR COALESCE(u.authors, 0) >= {UNREAD_AUTHOR_LIMIT},
        'first_unread_at', u.first_at"#
            ),
            format!(
                r#"LEFT JOIN LATERAL (
            SELECT COALESCE(sum(per_author.n), 0)::int AS n,
                   min(per_author.first_at) AS first_at,
                   count(*)::int AS authors
            FROM (
                SELECT a.user_id,
                    (SELECT count(*)::int FROM (
                        SELECT 1 FROM messages m
                        WHERE m.channel_id = a.channel_id AND m.user_id = a.user_id
                          AND m.parent_id IS NULL AND m.deleted_at IS NULL
                          AND m.created_at > COALESCE(r.last_read_at, '-infinity'::timestamp)
                        LIMIT {UNREAD_PER_AUTHOR_SCAN}
                    ) counted) AS n,
                    (SELECT m.created_at FROM messages m
                     WHERE m.channel_id = a.channel_id AND m.user_id = a.user_id
                       AND m.parent_id IS NULL AND m.deleted_at IS NULL
                       AND m.created_at > COALESCE(r.last_read_at, '-infinity'::timestamp)
                     ORDER BY m.created_at LIMIT 1) AS first_at
                FROM channel_message_authors a
                WHERE a.channel_id = c.id AND a.user_id <> $1
                  AND a.latest_at > COALESCE(r.last_read_at, '-infinity'::timestamp)
                ORDER BY a.latest_at DESC, a.user_id DESC LIMIT {UNREAD_AUTHOR_LIMIT}
            ) per_author
        ) u ON h.has_unread"#
            ),
        )
    } else {
        (String::new(), String::new())
    };
    format!(
        r#"SELECT json_build_object(
        'channel_id', c.id, 'last_read_at', r.last_read_at,
        'has_unread', h.has_unread{detail_fields}
        )::text FROM channels c LEFT JOIN channel_read_states r
        ON r.channel_id = c.id AND r.user_id = $1
        LEFT JOIN LATERAL (
            SELECT EXISTS (SELECT 1 FROM (
                SELECT a.user_id FROM channel_message_authors a WHERE a.channel_id=c.id
                  AND a.latest_at>COALESCE(r.last_read_at,'-infinity'::timestamp)
                ORDER BY a.latest_at DESC,a.user_id DESC LIMIT 2
            ) latest WHERE latest.user_id<>$1) AS has_unread
        ) h ON true
        {detail_join}
        WHERE c.deleted_at IS NULL AND {scope}"#
    )
}

async fn states(
    tx: &mut toasty::Transaction<'_>,
    actor: &str,
    channel: Option<&str>,
    details: bool,
) -> Result<Vec<Value>, AppError> {
    let sql = states_sql(channel.is_some(), details);
    let mut query = toasty::sql::query(sql).bind(actor);
    if let Some(id) = channel {
        query = query.bind(id);
    }
    let rows = query.exec(&mut *tx).await?;
    let mut result = Vec::new();
    for row in rows {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
        {
            result.push(serde_json::from_str(&json).map_err(|e| AppError::Internal(e.into()))?);
        }
    }
    Ok(result)
}

async fn list(
    State(state): State<AppState>,
    actor: AuthUser,
    Query(query): Query<DetailsQuery>,
) -> Result<Json<Vec<Value>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(
        states(&mut tx, &actor.id, None, query.requested()).await?,
    ))
}

async fn show(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id).await?;
    let state = states(&mut tx, &actor.id, Some(&channel_id), false)
        .await?
        .pop()
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;
    Ok(Json(state))
}

async fn mark_read(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Json(payload): Json<MarkReadRequest>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    Uuid::parse_str(&payload.message_id)
        .map_err(|_| AppError::BadRequest("Invalid message ID".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id).await?;
    let message = Message::filter(Message::fields().id().eq(payload.message_id.as_str()))
        .first()
        .exec(&mut tx)
        .await?
        .filter(|m| m.channel_id == channel_id && m.parent_id.is_none() && m.deleted_at.is_none())
        .ok_or_else(|| AppError::NotFound("Message not found".into()))?;
    // An atomic maximum prevents a delayed request from another device moving the cursor backwards.
    let sql = "INSERT INTO channel_read_states (user_id, channel_id, last_read_at)
        VALUES ($1, $2, $3::text::timestamp)
        ON CONFLICT (user_id, channel_id) DO UPDATE
        SET last_read_at = GREATEST(channel_read_states.last_read_at, EXCLUDED.last_read_at)";
    toasty::sql::statement(sql)
        .bind(actor.id.as_str())
        .bind(channel_id.as_str())
        .bind(message.created_at.to_string())
        .exec(&mut tx)
        .await?;
    let result = states(&mut tx, &actor.id, Some(&channel_id), false)
        .await?
        .pop()
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(result, txid)))
}
