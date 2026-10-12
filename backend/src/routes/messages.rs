use axum::{
    Json, Router,
    extract::{Query, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::AppError, models::Message, state::AppState};

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub id: Option<String>,
    pub user_id: String,
    pub channel_id: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub channel_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub id: String,
    pub user_id: String,
    pub channel_id: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl From<Message> for MessageResponse {
    fn from(m: Message) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            channel_id: m.channel_id,
            content: m.content,
            created_at: m.created_at.to_string(),
            updated_at: m.updated_at.map(|t| t.to_string()),
            deleted_at: m.deleted_at.map(|t| t.to_string()),
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/messages", post(create_message).get(list_messages))
}

async fn create_message(
    State(state): State<AppState>,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    let mut db = state.db.clone();
    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = jiff::Zoned::now().datetime();

    let message = toasty::create!(Message {
        id: id.as_str(),
        user_id: payload.user_id.as_str(),
        channel_id: payload.channel_id.as_str(),
        content: payload.content.as_str(),
        created_at: now,
        updated_at: None,
        deleted_at: None,
    })
    .exec(&mut db)
    .await?;

    Ok(Json(MessageResponse::from(message)))
}

async fn list_messages(
    State(state): State<AppState>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<Vec<MessageResponse>>, AppError> {
    let mut db = state.db.clone();
    let messages = if let Some(channel_id) = query.channel_id {
        Message::filter(Message::fields().channel_id().eq(channel_id.as_str()))
            .exec(&mut db)
            .await?
    } else {
        Message::all().exec(&mut db).await?
    };

    let res = messages.into_iter().map(MessageResponse::from).collect();
    Ok(Json(res))
}
