//! Short-lived presence only: drafts are never stored or transmitted.
use crate::{
    auth::AuthUser,
    error::AppError,
    models::{Channel, ChannelUser, Message},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
#[derive(Deserialize)]
struct Scope {
    parent_id: Option<String>,
}
#[derive(Deserialize)]
struct Update {
    parent_id: Option<String>,
    session_id: uuid::Uuid,
    active: bool,
}
#[derive(Serialize)]
struct TypingUser {
    id: String,
    display_name: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/channels/{id}/typing", get(list).post(update))
}

async fn authorize(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
    parent: &Option<String>,
    user: &AuthUser,
) -> Result<(), AppError> {
    let channel = Channel::get_by_id(&mut *tx, channel_id)
        .await
        .map_err(|e| {
            if e.is_record_not_found() {
                AppError::NotFound("Channel not found".into())
            } else {
                e.into()
            }
        })?;
    let member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id)
            .and(ChannelUser::fields().user_id().eq(user.id.as_str())),
    )
    .first()
    .exec(&mut *tx)
    .await?;
    if channel.deleted_at.is_some() || channel.archive_at.is_some() || member.is_none() {
        return Err(AppError::NotFound("Channel not found".into()));
    }
    if let Some(parent) = parent {
        let root = Message::get_by_id(&mut *tx, parent.as_str())
            .await
            .map_err(|e| {
                if e.is_record_not_found() {
                    AppError::NotFound("Thread not found".into())
                } else {
                    e.into()
                }
            })?;
        if root.channel_id != channel_id || root.deleted_at.is_some() || root.parent_id.is_some() {
            return Err(AppError::NotFound("Thread not found".into()));
        }
    }
    Ok(())
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<Update>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &id, &body.parent_id, &user).await?;
    // Bounded cleanup, skipping rows being refreshed on other instances.
    toasty::sql::statement(
        "DELETE FROM typing_presence WHERE ctid IN (
            SELECT ctid FROM typing_presence
            WHERE expires_at <= statement_timestamp()
            LIMIT 1000 FOR UPDATE SKIP LOCKED
        )",
    )
    .exec(&mut tx)
    .await?;
    let scope = body.parent_id.unwrap_or_default();
    let session = body.session_id.to_string();
    if body.active {
        // Database time avoids clock skew between backend instances.
        toasty::sql::statement(
            "INSERT INTO typing_presence (channel_id, parent_id, user_id, session_id, expires_at)
             VALUES ($1, NULLIF($2, ''), $3, $4, statement_timestamp() + interval '6 seconds')
             ON CONFLICT (channel_id, user_id, session_id)
             DO UPDATE SET parent_id = EXCLUDED.parent_id, expires_at = EXCLUDED.expires_at",
        )
        .bind(id)
        .bind(scope)
        .bind(user.id)
        .bind(session)
        .exec(&mut tx)
        .await?;
    } else {
        toasty::sql::statement(
            "DELETE FROM typing_presence
             WHERE channel_id = $1 AND parent_id IS NOT DISTINCT FROM NULLIF($2, '')
               AND user_id = $3 AND session_id = $4",
        )
        .bind(id)
        .bind(scope)
        .bind(user.id)
        .bind(session)
        .exec(&mut tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(scope): Query<Scope>,
) -> Result<Json<Vec<TypingUser>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    authorize(&mut tx, &id, &scope.parent_id, &user).await?;
    let rows = toasty::sql::query(
        "SELECT DISTINCT u.id, u.display_name::text
         FROM typing_presence p
         JOIN users u ON u.id = p.user_id
         JOIN channel_user m ON m.user_id = p.user_id AND m.channel_id = p.channel_id
         WHERE p.channel_id = $1 AND p.parent_id IS NOT DISTINCT FROM NULLIF($2, '')
           AND p.user_id <> $3 AND p.expires_at > statement_timestamp()
         ORDER BY u.id",
    )
    .bind(id)
    .bind(scope.parent_id.unwrap_or_default())
    .bind(user.id)
    .exec(&mut tx)
    .await?;
    let mut users = Vec::with_capacity(rows.len());
    for row in rows {
        if let toasty::stmt::Value::Record(record) = row {
            let mut fields = record.fields.into_iter();
            if let (
                Some(toasty::stmt::Value::String(id)),
                Some(toasty::stmt::Value::String(display_name)),
            ) = (fields.next(), fields.next())
            {
                users.push(TypingUser { id, display_name });
                continue;
            }
        }
        return Err(AppError::Internal(anyhow::anyhow!(
            "Invalid typing presence row"
        )));
    }
    Ok(Json(users))
}
