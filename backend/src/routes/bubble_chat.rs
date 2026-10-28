//! Ephemeral bubble chat room presence: active typing text is shared while non-empty and not expired.
use crate::{
    auth::AuthUser, error::AppError, state::AppState, validation::validate_bubble_chat_content,
};
use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct UpdateBubbleChat {
    session_id: uuid::Uuid,
    content: String,
    #[serde(default)]
    seq: Option<i64>,
}

#[derive(Serialize)]
struct BubbleChatEntry {
    user_id: String,
    user_name: String,
    display_name: String,
    content: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/bubble", get(list).post(update))
        .route("/api/bubble-chat", get(list).post(update))
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<UpdateBubbleChat>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_bubble_chat_content(&body.content)?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;

    // Bounded cleanup, skipping rows being refreshed on other instances.
    toasty::sql::statement(
        "DELETE FROM bubble_chat_presence WHERE ctid IN (
            SELECT ctid FROM bubble_chat_presence
            WHERE expires_at <= statement_timestamp()
            LIMIT 1000 FOR UPDATE SKIP LOCKED
        )",
    )
    .exec(&mut tx)
    .await?;

    let session = body.session_id.to_string();
    let seq = body.seq.unwrap_or(0);
    let content = if body.content.trim().is_empty() {
        String::new()
    } else {
        body.content
    };

    // Database time avoids clock skew between backend instances.
    // An empty content row acts as a tombstone retaining the latest seq for the TTL window,
    // ensuring older out-of-order writes or delayed retries cannot resurrect cleared text.
    toasty::sql::statement(
        "INSERT INTO bubble_chat_presence (user_id, session_id, content, seq, updated_at, expires_at)
         VALUES ($1, $2, $3, $4, statement_timestamp(), statement_timestamp() + interval '15 seconds')
         ON CONFLICT (user_id, session_id)
         DO UPDATE SET
           content = EXCLUDED.content,
           seq = EXCLUDED.seq,
           updated_at = EXCLUDED.updated_at,
           expires_at = EXCLUDED.expires_at
         WHERE EXCLUDED.seq >= bubble_chat_presence.seq",
    )
    .bind(user.id)
    .bind(session)
    .bind(content)
    .bind(seq)
    .exec(&mut tx)
    .await?;

    tx.commit().await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<BubbleChatEntry>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = toasty::sql::query(
        "SELECT DISTINCT ON (u.id) u.id, u.user_name, u.display_name::text, p.content
         FROM bubble_chat_presence p
         JOIN users u ON u.id = p.user_id
         WHERE p.expires_at > statement_timestamp()
           AND btrim(p.content) <> ''
         ORDER BY u.id, p.updated_at DESC, p.session_id DESC",
    )
    .exec(&mut tx)
    .await?;

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        if let toasty::stmt::Value::Record(record) = row {
            let mut fields = record.fields.into_iter();
            if let (
                Some(toasty::stmt::Value::String(user_id)),
                Some(toasty::stmt::Value::String(user_name)),
                Some(toasty::stmt::Value::String(display_name)),
                Some(toasty::stmt::Value::String(content)),
            ) = (fields.next(), fields.next(), fields.next(), fields.next())
            {
                entries.push(BubbleChatEntry {
                    user_id,
                    user_name,
                    display_name,
                    content,
                });
                continue;
            }
        }
        return Err(AppError::Internal(anyhow::anyhow!(
            "Invalid bubble chat presence row"
        )));
    }
    Ok(Json(entries))
}
