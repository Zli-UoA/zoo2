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
    auth::AuthUser,
    db::get_current_txid,
    error::AppError,
    models::{Channel, ChannelNote, ChannelUser},
    routes::response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct NoteResponse {
    pub id: String,
    pub channel_id: String,
    pub title: String,
    pub content: String,
    pub version: i32,
    pub created_user_id: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ChannelNote> for NoteResponse {
    fn from(note: ChannelNote) -> Self {
        Self {
            id: note.id,
            channel_id: note.channel_id,
            title: note.title,
            content: note.content,
            version: note.version,
            created_user_id: note.created_user_id,
            created_at: note.created_at.to_string(),
            updated_at: note.updated_at.to_string(),
        }
    }
}

#[derive(Deserialize)]
pub struct NoteInput {
    title: String,
    content: String,
}

#[derive(Deserialize)]
pub struct UpdateNoteInput {
    title: String,
    content: String,
    version: i32,
}

#[derive(Deserialize)]
pub struct DeleteNoteInput {
    version: i32,
}

#[derive(Deserialize)]
pub struct ReorderNotesInput {
    expected_note_ids: Vec<String>,
    note_ids: Vec<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels/{channel_id}/notes", get(list).post(create))
        .route(
            "/api/channels/{channel_id}/notes/deleted",
            get(list_deleted),
        )
        .route(
            "/api/channels/{channel_id}/notes/order",
            axum::routing::patch(reorder),
        )
        .route(
            "/api/channels/{channel_id}/notes/{note_id}",
            get(show).patch(update).delete(remove),
        )
        .route(
            "/api/channels/{channel_id}/notes/{note_id}/restore",
            axum::routing::post(restore),
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

/// Delete only expired tombstones, in bounded batches shared safely by workers.
pub async fn purge_expired_notes(db: &toasty::Db) -> Result<u64, AppError> {
    let db = db.clone();
    let mut cleanup_db = db.clone();
    let mut tx = cleanup_db.transaction().await?;
    let cursor_rows = toasty::sql::query(
        "SELECT last_channel_id FROM channel_notes_cleanup_cursor WHERE id = true FOR UPDATE",
    )
    .exec(&mut tx)
    .await?;
    let mut last_channel = match cursor_rows.into_iter().next() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.into_iter().next() {
            Some(toasty::stmt::Value::String(value)) => Some(value),
            Some(toasty::stmt::Value::Null) => None,
            _ => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected cleanup cursor"
                )));
            }
        },
        _ => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Missing cleanup cursor"
            )));
        }
    };
    let mut unlocked_channels = Vec::with_capacity(100);
    let mut seen_channels = std::collections::HashSet::new();
    let mut scanned = 0usize;
    let mut wrapped = false;
    while unlocked_channels.len() < 100 && scanned < 500 {
        // Page over the channels primary key rather than grouping every
        // expired note. This bounds each lookup to 100 channels and uses the
        // channel_notes expiry index for an existence probe per channel.
        let page_rows = toasty::sql::query(
            "WITH page AS MATERIALIZED (\
                 SELECT id FROM channels WHERE id > $1 ORDER BY id LIMIT $2\
             ) \
             SELECT page.id, expired.id IS NOT NULL AS eligible FROM page \
             LEFT JOIN LATERAL (\
                 SELECT id FROM channel_notes \
                 WHERE channel_id = page.id \
                   AND deleted_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
                 ORDER BY deleted_at, id LIMIT 1\
             ) expired ON true ORDER BY page.id",
        )
        .bind(last_channel.clone().unwrap_or_default())
        .bind((100 - unlocked_channels.len()).min(500 - scanned) as i32)
        .exec(&mut tx)
        .await?;
        let mut candidates = Vec::with_capacity(page_rows.len());
        let mut page_last = None;
        for value in &page_rows {
            if let toasty::stmt::Value::Record(record) = value {
                let mut fields = record.fields.iter();
                if let (Some(toasty::stmt::Value::String(id)), Some(eligible)) =
                    (fields.next(), fields.next())
                {
                    page_last = Some(id.clone());
                    if matches!(eligible, toasty::stmt::Value::Bool(true)) {
                        candidates.push(id.clone());
                    }
                }
            }
        }
        let Some(page_last) = page_last else {
            if wrapped || last_channel.is_none() {
                break;
            }
            toasty::sql::statement(
                "UPDATE channel_notes_cleanup_cursor SET last_channel_id = NULL WHERE id = true",
            )
            .exec(&mut tx)
            .await?;
            last_channel = None;
            wrapped = true;
            continue;
        };
        toasty::sql::statement(
            "UPDATE channel_notes_cleanup_cursor SET last_channel_id = $1 WHERE id = true",
        )
        .bind(page_last.clone())
        .exec(&mut tx)
        .await?;
        last_channel = Some(page_last);

        let selected = serde_json::to_string(&candidates)
            .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
        let lock_rows = toasty::sql::query(
            "SELECT channel_id, pg_try_advisory_xact_lock(\
                 hashtextextended('channel-notes:' || channel_id, 0)\
             ) AS locked \
             FROM jsonb_array_elements_text($1::jsonb) AS channel_id",
        )
        .bind(selected)
        .exec(&mut tx)
        .await?;
        for row in lock_rows {
            if let toasty::stmt::Value::Record(record) = row {
                let mut fields = record.fields.into_iter();
                if let (
                    Some(toasty::stmt::Value::String(channel_id)),
                    Some(toasty::stmt::Value::Bool(true)),
                ) = (fields.next(), fields.next())
                {
                    if seen_channels.insert(channel_id.clone()) {
                        unlocked_channels.push(channel_id);
                    }
                }
            }
        }
        scanned += page_rows.len();
    }
    let mut count = 0u64;
    let channel_count = unlocked_channels.len();
    while count < 100 && channel_count > 0 {
        let before_pass = count;
        for (index, channel_id) in unlocked_channels.iter().enumerate() {
            let remaining = (100 - count) as usize;
            if remaining == 0 {
                break;
            }
            // Share the bounded batch across channels so a single large backlog
            // cannot consume every pass while older rows in other channels wait.
            let channels_remaining = channel_count - index;
            let per_channel = remaining.div_ceil(channels_remaining);
            count += toasty::sql::statement(
                "DELETE FROM channel_notes WHERE id IN (
                SELECT id FROM channel_notes
                WHERE channel_id = $1
                  AND deleted_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours'
                ORDER BY deleted_at, id
                LIMIT $2 FOR UPDATE SKIP LOCKED
            )",
            )
            .bind(channel_id.as_str())
            .bind(per_channel as i32)
            .exec(&mut tx)
            .await?;
        }
        if count == before_pass {
            break;
        }
    }
    tx.commit().await?;
    Ok(count)
}

pub async fn cleanup_worker(db: toasty::Db) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        timer.tick().await;
        if let Err(error) = purge_expired_notes(&db).await {
            tracing::warn!(%error, "Expired channel note cleanup failed; will retry");
        }
    }
}

async fn authorize(
    tx: &mut toasty::Transaction<'_>,
    actor: &str,
    channel_id: &str,
    write: bool,
) -> Result<Channel, AppError> {
    Uuid::parse_str(channel_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    // AuthUser's active_user check runs in a separate transaction. Recheck and
    // hold the user row through note retrieval; guest_kind updates (including
    // deactivation) update this row before their triggers touch channel rows.
    let active_user =
        toasty::sql::query("SELECT id FROM users WHERE id = $1 AND NOT is_deactivated FOR SHARE")
            .bind(actor)
            .exec(&mut *tx)
            .await?;
    if active_user.is_empty() {
        return Err(AppError::Forbidden("Account is deactivated".into()));
    }
    let guest = crate::apps::events::guest(tx, actor).await?;

    // Lock order: users, channels, then channel_user. Keep the channel lock
    // shared for note writes: the database trigger for direct note updates
    // locks the note row before taking the same shared channel lock. A stronger
    // lock here would deadlock with that direct-SQL path.
    if write {
        toasty::sql::query("SELECT id FROM channels WHERE id = $1 FOR SHARE")
            .bind(channel_id)
            .exec(&mut *tx)
            .await?;
        // Serialize authorization with concurrent removal of this membership.
        toasty::sql::query(
            "SELECT user_id FROM channel_user WHERE channel_id = $1 AND user_id = $2 FOR UPDATE",
        )
        .bind(channel_id)
        .bind(actor)
        .exec(&mut *tx)
        .await?;
    } else {
        // Hold a shared channel lock through note retrieval so privacy and
        // membership changes which lock this row first serialize with reads.
        toasty::sql::query("SELECT id FROM channels WHERE id = $1 FOR SHARE")
            .bind(channel_id)
            .exec(&mut *tx)
            .await?;
        // App, webhook, and Slack cleanup paths can delete channel_user rows
        // without locking channels. Lock this actor's membership too, so those
        // removals cannot commit between authorization and note retrieval.
        toasty::sql::query(
            "SELECT user_id FROM channel_user WHERE channel_id = $1 AND user_id = $2 FOR SHARE",
        )
        .bind(channel_id)
        .bind(actor)
        .exec(&mut *tx)
        .await?;
    }
    let channel = Channel::filter(Channel::fields().id().eq(channel_id))
        .first()
        .exec(&mut *tx)
        .await?
        .filter(|channel| channel.deleted_at.is_none())
        .ok_or_else(|| AppError::NotFound("Channel not found".into()))?;
    let member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id)
            .and(ChannelUser::fields().user_id().eq(actor)),
    )
    .first()
    .exec(&mut *tx)
    .await?
    .is_some();

    if (channel.is_private || channel.is_dm || guest) && !member {
        return Err(AppError::NotFound("Channel not found".into()));
    }
    // Notes are stored as plaintext and cannot yet honor E2EE guarantees.
    if channel.encrypted {
        return Err(AppError::BadRequest(
            "Notes are unavailable in encrypted channels".into(),
        ));
    }
    if write {
        if !member {
            return Err(AppError::Forbidden(
                "Join the channel to manage notes".into(),
            ));
        }
        if channel.archive_at.is_some() {
            return Err(AppError::BadRequest(
                "Cannot modify notes in an archived channel".into(),
            ));
        }
        // Serialize API note mutations within this channel before reading note
        // order or locking note rows. Direct SQL writers that coordinate with
        // this API must take the same lock before their note statement.
        toasty::sql::query(
            "SELECT '{}'::text FROM pg_advisory_xact_lock(hashtextextended('channel-notes:' || $1, 0))",
        )
        .bind(channel_id)
        .exec(&mut *tx)
        .await?;
    }
    Ok(channel)
}

fn validate(input: &NoteInput) -> Result<(), AppError> {
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(AppError::BadRequest(
            "Note title must be between 1 and 200 characters".into(),
        ));
    }
    if input.title.contains('\0') || input.content.contains('\0') {
        return Err(AppError::BadRequest(
            "Note title and content must not contain NUL".into(),
        ));
    }
    if input.content.chars().count() > 100_000 {
        return Err(AppError::BadRequest("Note content is too long".into()));
    }
    Ok(())
}

fn note_path(channel_id: &str, note_id: &str) -> Result<(), AppError> {
    Uuid::parse_str(channel_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    Uuid::parse_str(note_id).map_err(|_| AppError::BadRequest("Invalid note ID".into()))?;
    Ok(())
}

async fn list(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<NoteResponse>>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, false).await?;
    let mut result = ChannelNote::filter(
        ChannelNote::fields()
            .channel_id()
            .eq(channel_id.as_str())
            .and(ChannelNote::fields().deleted_at().is_none()),
    )
    .exec(&mut tx)
    .await?;
    result.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));
    let result = result.into_iter().map(NoteResponse::from).collect();
    Ok(Json(ApiResponse::new(result, None::<String>)))
}

async fn list_deleted(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<NoteResponse>>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, false).await?;
    let is_member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id.as_str())
            .and(ChannelUser::fields().user_id().eq(actor.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .is_some();
    if !is_member {
        return Err(AppError::Forbidden(
            "Join the channel to view deleted notes".into(),
        ));
    }
    // The eligibility predicate and response body must share one PostgreSQL
    // statement snapshot. Separate SELECTs under READ COMMITTED can combine a
    // pre-delete body with a post-delete eligibility check.
    let rows = toasty::sql::query(
        "SELECT id, channel_id, title, content, version, created_user_id, \
                to_char(created_at, 'YYYY-MM-DD') || 'T' || to_char(created_at, 'HH24:MI:SS.MS'), \
                to_char(updated_at, 'YYYY-MM-DD') || 'T' || to_char(updated_at, 'HH24:MI:SS.MS') \
         FROM channel_notes \
         WHERE channel_id = $1 AND deleted_at IS NOT NULL \
           AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours' \
         ORDER BY deleted_at, id",
    )
    .bind(channel_id.as_str())
    .exec(&mut tx)
    .await?;
    let result: Vec<NoteResponse> = rows
        .into_iter()
        .map(|row| {
            let toasty::stmt::Value::Record(record) = row else {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected deleted channel note row"
                )));
            };
            let mut fields = record.fields.into_iter();
            let (
                Some(toasty::stmt::Value::String(id)),
                Some(toasty::stmt::Value::String(channel_id)),
                Some(toasty::stmt::Value::String(title)),
                Some(toasty::stmt::Value::String(content)),
                Some(toasty::stmt::Value::I32(version)),
                Some(toasty::stmt::Value::String(created_user_id)),
                Some(toasty::stmt::Value::String(created_at)),
                Some(toasty::stmt::Value::String(updated_at)),
            ) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            )
            else {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Unexpected deleted channel note columns"
                )));
            };
            let created_at = created_at
                .parse::<jiff::civil::DateTime>()
                .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
                .to_string();
            let updated_at = updated_at
                .parse::<jiff::civil::DateTime>()
                .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
                .to_string();
            Ok(NoteResponse {
                id,
                channel_id,
                title,
                content,
                version,
                created_user_id,
                created_at,
                updated_at,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(Json(ApiResponse::new(result, None::<String>)))
}

async fn show(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    note_path(&channel_id, &note_id)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, false).await?;
    let note = find(&mut tx, &channel_id, &note_id).await?;
    Ok(Json(ApiResponse::new(
        NoteResponse::from(note),
        None::<String>,
    )))
}

async fn find(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    note_id: &str,
) -> Result<ChannelNote, AppError> {
    ChannelNote::filter(
        ChannelNote::fields()
            .id()
            .eq(note_id)
            .and(ChannelNote::fields().channel_id().eq(channel_id))
            .and(ChannelNote::fields().deleted_at().is_none()),
    )
    .first()
    .exec(tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Note not found".into()))
}

async fn reorder(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Json(input): Json<ReorderNotesInput>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    let mut active = ChannelNote::filter(
        ChannelNote::fields()
            .channel_id()
            .eq(channel_id.as_str())
            .and(ChannelNote::fields().deleted_at().is_none()),
    )
    .exec(&mut tx)
    .await?;
    active.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));
    let active_order: Vec<_> = active.iter().map(|note| note.id.clone()).collect();
    if active_order != input.expected_note_ids {
        return Err(AppError::Conflict(
            "Note order changed; reload before reordering".into(),
        ));
    }
    let mut active_ids: Vec<_> = active.into_iter().map(|note| note.id).collect();
    active_ids.sort();
    let mut requested = input.note_ids.clone();
    requested.sort();
    if active_ids != requested
        || input
            .note_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.note_ids.len()
    {
        return Err(AppError::BadRequest(
            "Order must contain every active note exactly once".into(),
        ));
    }
    for (position, id) in input.note_ids.iter().enumerate() {
        toasty::sql::query(
            "UPDATE channel_notes SET position = $1 WHERE id = $2 AND channel_id = $3",
        )
        .bind(position as i32)
        .bind(id.as_str())
        .bind(channel_id.as_str())
        .exec(&mut tx)
        .await?;
    }
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        serde_json::json!({"updated":true}),
        txid,
    )))
}

async fn restore(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    note_path(&channel_id, &note_id)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    let rows = toasty::sql::query(
        "UPDATE channel_notes          SET deleted_at = NULL, version = version + 1, updated_at = statement_timestamp() AT TIME ZONE 'UTC'          WHERE id = $2 AND channel_id = $1 AND deleted_at IS NOT NULL            AND deleted_at >= (statement_timestamp() AT TIME ZONE 'UTC') - interval '24 hours'          RETURNING position",
    )
    .bind(channel_id.as_str())
    .bind(note_id.as_str())
    .exec(&mut tx)
    .await?;
    let Some(toasty::stmt::Value::Record(record)) = rows.first() else {
        return Err(AppError::NotFound("Restorable note not found".into()));
    };
    let Some(toasty::stmt::Value::I32(restored_position)) = record.fields.first() else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Unexpected restored channel note position"
        )));
    };
    toasty::sql::query(
        "UPDATE channel_notes SET position = position + 1          WHERE channel_id = $1 AND deleted_at IS NULL AND position >= $2 AND id <> $3",
    )
    .bind(channel_id.as_str())
    .bind(*restored_position)
    .bind(note_id.as_str())
    .exec(&mut tx)
    .await?;
    let note = find(&mut tx, &channel_id, &note_id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(NoteResponse::from(note), txid)))
}

async fn create(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(channel_id): Path<String>,
    Json(input): Json<NoteInput>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    validate(&input)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    let id = Uuid::new_v4().to_string();
    let now = crate::models::now_utc_millis();
    let rows = toasty::sql::query("SELECT COALESCE(MAX(position), -1) + 1 AS position FROM channel_notes WHERE channel_id = $1 AND deleted_at IS NULL")
        .bind(channel_id.as_str()).exec(&mut tx).await?;
    let position = match rows.first() {
        Some(toasty::stmt::Value::Record(record)) => match record.fields.first() {
            Some(toasty::stmt::Value::I32(value)) => *value,
            _ => 0,
        },
        _ => 0,
    };
    let note = toasty::create!(ChannelNote {
        id: id.as_str(),
        channel_id: channel_id.as_str(),
        title: input.title.trim(),
        content: input.content.as_str(),
        position,
        deleted_at: None,
        version: 1,
        created_user_id: actor.id.as_str(),
        created_at: now,
        updated_at: now,
    })
    .exec(&mut tx)
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(NoteResponse::from(note), txid)))
}

async fn update(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
    Json(input): Json<UpdateNoteInput>,
) -> Result<Json<ApiResponse<NoteResponse>>, AppError> {
    note_path(&channel_id, &note_id)?;
    validate(&NoteInput {
        title: input.title.clone(),
        content: input.content.clone(),
    })?;
    if input.version < 1 {
        return Err(AppError::BadRequest("Invalid note version".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    let rows = toasty::sql::query(
        "UPDATE channel_notes SET title = $1, content = $2, version = version + 1, updated_at = statement_timestamp() AT TIME ZONE 'UTC' \
         WHERE id = $3 AND channel_id = $4 AND version = $5 AND deleted_at IS NULL RETURNING id",
    )
    .bind(input.title.trim())
    .bind(input.content.as_str())
    .bind(note_id.as_str())
    .bind(channel_id.as_str())
    .bind(input.version)
    .exec(&mut tx)
    .await?;
    if rows.is_empty() {
        return Err(stale_or_missing(&mut tx, &channel_id, &note_id).await?);
    }
    let note = find(&mut tx, &channel_id, &note_id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(NoteResponse::from(note), txid)))
}

async fn stale_or_missing(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    note_id: &str,
) -> Result<AppError, AppError> {
    match find(tx, channel_id, note_id).await {
        Ok(_) => Ok(AppError::Conflict(
            "Note has changed; reload before editing".into(),
        )),
        Err(AppError::NotFound(_)) => Ok(AppError::NotFound("Note not found".into())),
        Err(error) => Err(error),
    }
}

async fn remove(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((channel_id, note_id)): Path<(String, String)>,
    Json(input): Json<DeleteNoteInput>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    note_path(&channel_id, &note_id)?;
    if input.version < 1 {
        return Err(AppError::BadRequest("Invalid note version".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &actor.id, &channel_id, true).await?;
    let rows = toasty::sql::query(
        "UPDATE channel_notes SET deleted_at = statement_timestamp() AT TIME ZONE 'UTC', version = version + 1, updated_at = statement_timestamp() AT TIME ZONE 'UTC' WHERE id = $1 AND channel_id = $2 AND version = $3 AND deleted_at IS NULL RETURNING id",
    )
    .bind(note_id.as_str())
    .bind(channel_id.as_str())
    .bind(input.version)
    .exec(&mut tx)
    .await?;
    if rows.is_empty() {
        return Err(stale_or_missing(&mut tx, &channel_id, &note_id).await?);
    }
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        serde_json::json!({"deleted":true}),
        txid,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_note_title_and_content_limits() {
        assert!(
            validate(&NoteInput {
                title: "  Runbook  ".into(),
                content: "body".into(),
            })
            .is_ok()
        );
        assert!(
            validate(&NoteInput {
                title: "  ".into(),
                content: String::new(),
            })
            .is_err()
        );
        assert!(
            validate(&NoteInput {
                title: "n".repeat(201),
                content: String::new(),
            })
            .is_err()
        );
        assert!(
            validate(&NoteInput {
                title: "n".into(),
                content: "x".repeat(100_001),
            })
            .is_err()
        );
    }
}
