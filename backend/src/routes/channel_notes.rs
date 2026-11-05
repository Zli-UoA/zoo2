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

async fn authorize(
    tx: &mut toasty::Transaction<'_>,
    actor: &str,
    channel_id: &str,
    write: bool,
) -> Result<Channel, AppError> {
    Uuid::parse_str(channel_id).map_err(|_| AppError::BadRequest("Invalid channel ID".into()))?;
    if write {
        toasty::sql::query("SELECT id FROM channels WHERE id = $1 FOR UPDATE")
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

    if (channel.is_private || channel.is_dm) && !member {
        return Err(AppError::NotFound("Channel not found".into()));
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
    let notes = ChannelNote::filter(ChannelNote::fields().channel_id().eq(channel_id.as_str()))
        .exec(&mut tx)
        .await?;
    let mut result: Vec<_> = notes
        .into_iter()
        .filter(|note| note.deleted_at.is_none())
        .collect();
    result.sort_by_key(|note| note.position);
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
    let notes = ChannelNote::filter(ChannelNote::fields().channel_id().eq(channel_id.as_str()))
        .exec(&mut tx)
        .await?;
    let cutoff = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_hours(24))
        .unwrap();
    let mut result: Vec<_> = notes
        .into_iter()
        .filter(|note| {
            note.deleted_at.is_some_and(|at| {
                at.to_zoned(jiff::tz::TimeZone::UTC)
                    .ok()
                    .is_some_and(|zoned| zoned.timestamp() >= cutoff)
            })
        })
        .collect();
    result.sort_by_key(|note| note.deleted_at);
    Ok(Json(ApiResponse::new(
        result.into_iter().map(NoteResponse::from).collect(),
        None::<String>,
    )))
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
    let notes = ChannelNote::filter(ChannelNote::fields().channel_id().eq(channel_id.as_str()))
        .exec(&mut tx)
        .await?;
    let mut active: Vec<_> = notes
        .into_iter()
        .filter(|note| note.deleted_at.is_none())
        .collect();
    active.sort_by_key(|note| note.position);
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
    let deleted = ChannelNote::filter(
        ChannelNote::fields()
            .id()
            .eq(note_id.as_str())
            .and(ChannelNote::fields().channel_id().eq(channel_id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?;
    let Some(deleted) = deleted.filter(|note| {
        note.deleted_at.is_some_and(|at| {
            at.to_zoned(jiff::tz::TimeZone::UTC)
                .ok()
                .is_some_and(|deleted_at| {
                    deleted_at.timestamp()
                        >= jiff::Timestamp::now()
                            .checked_sub(jiff::SignedDuration::from_hours(24))
                            .unwrap()
                })
        })
    }) else {
        return Err(AppError::NotFound("Restorable note not found".into()));
    };
    toasty::sql::query("UPDATE channel_notes SET position = position + 1 WHERE channel_id = $1 AND deleted_at IS NULL AND position >= $2")
        .bind(channel_id.as_str())
        .bind(deleted.position)
        .exec(&mut tx)
        .await?;
    let rows = toasty::sql::query("UPDATE channel_notes SET deleted_at = NULL, version = version + 1, updated_at = statement_timestamp() WHERE id = $2 AND channel_id = $1 AND deleted_at >= statement_timestamp() - interval '24 hours' RETURNING id")
        .bind(channel_id.as_str()).bind(note_id.as_str()).exec(&mut tx).await?;
    if rows.is_empty() {
        return Err(AppError::NotFound("Restorable note not found".into()));
    }
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
    let notifications = crate::notifications::create_note_notifications(
        &mut tx,
        &note,
        "",
        note.content.as_str(),
        actor.id.as_str(),
    )
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    crate::push::dispatch(state.clone(), notifications);
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
    let previous = find(&mut tx, &channel_id, &note_id).await?;
    let rows = toasty::sql::query(
        "UPDATE channel_notes SET title = $1, content = $2, version = version + 1, updated_at = statement_timestamp() \
         WHERE id = $3 AND channel_id = $4 AND version = $5 RETURNING id",
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
    let notifications = crate::notifications::create_note_notifications(
        &mut tx,
        &previous,
        previous.content.as_str(),
        input.content.as_str(),
        actor.id.as_str(),
    )
    .await?;
    let note = find(&mut tx, &channel_id, &note_id).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    crate::push::dispatch(state.clone(), notifications);
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
        "UPDATE channel_notes SET deleted_at = statement_timestamp(), version = version + 1, updated_at = statement_timestamp() WHERE id = $1 AND channel_id = $2 AND version = $3 AND deleted_at IS NULL RETURNING id",
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
