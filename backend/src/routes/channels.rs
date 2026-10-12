use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::AppError, models::Channel, state::AppState};

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub id: Option<String>,
    pub channel_name: String,
    pub description: Option<String>,
    pub created_user_id: String,
    pub is_private: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ChannelResponse {
    pub id: String,
    pub channel_name: String,
    pub description: Option<String>,
    pub created_user_id: String,
    pub is_private: bool,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub archive_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl From<Channel> for ChannelResponse {
    fn from(c: Channel) -> Self {
        Self {
            id: c.id,
            channel_name: c.channel_name,
            description: c.description,
            created_user_id: c.created_user_id,
            is_private: c.is_private,
            created_at: c.created_at.to_string(),
            updated_at: c.updated_at.map(|t| t.to_string()),
            archive_at: c.archive_at.map(|t| t.to_string()),
            deleted_at: c.deleted_at.map(|t| t.to_string()),
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channels", post(create_channel).get(list_channels))
        .route("/api/channels/{id}", get(get_channel_by_id))
}

async fn create_channel(
    State(state): State<AppState>,
    Json(payload): Json<CreateChannelRequest>,
) -> Result<Json<ChannelResponse>, AppError> {
    let mut db = state.db.clone();
    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = jiff::Zoned::now().datetime();

    let channel = toasty::create!(Channel {
        id: id.as_str(),
        channel_name: payload.channel_name.as_str(),
        description: payload.description,
        created_user_id: payload.created_user_id.as_str(),
        is_private: payload.is_private.unwrap_or(false),
        created_at: now,
        updated_at: None,
        archive_at: None,
        deleted_at: None,
    })
    .exec(&mut db)
    .await?;

    Ok(Json(ChannelResponse::from(channel)))
}

async fn list_channels(
    State(state): State<AppState>,
) -> Result<Json<Vec<ChannelResponse>>, AppError> {
    let mut db = state.db.clone();
    let channels = Channel::all().exec(&mut db).await?;
    let res = channels.into_iter().map(ChannelResponse::from).collect();
    Ok(Json(res))
}

async fn get_channel_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ChannelResponse>, AppError> {
    let mut db = state.db.clone();
    let channel = Channel::get_by_id(&mut db, id.as_str())
        .await
        .map_err(|_| AppError::NotFound(format!("Channel not found with id: {id}")))?;

    Ok(Json(ChannelResponse::from(channel)))
}
