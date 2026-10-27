use super::*;
use crate::routes::{channels as c, dms, messages as m};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use futures_util::StreamExt;
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/v1/auth", get(auth))
        .route("/api/apps/v1/messages", post(create_message))
        .route(
            "/api/apps/v1/messages/{id}",
            patch(update_message).delete(delete_message),
        )
        .route(
            "/api/apps/v1/messages/{id}/reactions",
            post(add_reaction).delete(delete_reaction),
        )
        .route("/api/apps/v1/channels", post(create_channel))
        .route(
            "/api/apps/v1/channels/{id}",
            patch(update_channel).delete(delete_channel),
        )
        .route("/api/apps/v1/channels/{id}/join", post(join_channel))
        .route("/api/apps/v1/channels/{id}/leave", post(leave_channel))
        .route("/api/apps/v1/channels/{id}/archive", post(archive_channel))
        .route(
            "/api/apps/v1/channels/{id}/unarchive",
            post(unarchive_channel),
        )
        .route("/api/apps/v1/dms", post(dm))
        .route("/api/apps/v1/shapes/{shape}", get(shape))
        .layer(axum::middleware::map_response(super::no_store))
}
async fn auth(p: Principal) -> Json<Value> {
    Json(
        json!({"app_id":p.app_id,"installation_id":p.installation_id,"user_id":p.user_id,"bot_user_id":p.bot_user_id,"scopes":p.scopes,"delegated":p.delegated}),
    )
}
async fn check(
    state: &AppState,
    p: &Principal,
    channel: &str,
    scope: &str,
    write: bool,
) -> Result<(), AppError> {
    p.require(scope)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    p.channel(&mut tx, channel, write).await
}
async fn message_check(
    state: &AppState,
    p: &Principal,
    id: &str,
    scope: &str,
) -> Result<(), AppError> {
    p.require(scope)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let message = crate::models::Message::get_by_id(&mut tx, id).await?;
    p.channel(&mut tx, &message.channel_id, true).await
}
async fn create_message(
    State(state): State<AppState>,
    p: Principal,
    Json(body): Json<m::CreateMessageRequest>,
) -> Result<Json<crate::routes::response::ApiResponse<m::MessageResponse>>, AppError> {
    check(&state, &p, &body.channel_id, "messages:write", true).await?;
    // A supplied UUID is the idempotency key; never return another author's write.
    if let Some(id) = &body.id {
        let mut db = state.db.clone();
        let existing =
            crate::models::Message::filter(crate::models::Message::fields().id().eq(id.as_str()))
                .first()
                .exec(&mut db)
                .await?;
        if let Some(message) = existing {
            if message.user_id == p.user_id
                && message.channel_id == body.channel_id
                && message.content == body.content
                && message.parent_id == body.parent_id
                && message.deleted_at.is_none()
            {
                return Ok(Json(crate::routes::response::ApiResponse::new(
                    message.into(),
                    None,
                )));
            }
            return Err(AppError::Conflict("Idempotency key already used".into()));
        }
    }
    m::create_message_impl(state, p.actor(), body, true, None).await
}
async fn update_message(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<m::UpdateMessageRequest>,
) -> Result<Json<crate::routes::response::ApiResponse<m::MessageResponse>>, AppError> {
    message_check(&s, &p, &id, "messages:write").await?;
    m::update_message(State(s), p.actor(), Path(id), Json(b)).await
}
async fn delete_message(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<crate::routes::response::ApiResponse<m::MessageResponse>>, AppError> {
    message_check(&s, &p, &id, "messages:write").await?;
    m::delete_message(State(s), p.actor(), Path(id)).await
}
async fn add_reaction(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<m::AddReactionRequest>,
) -> Result<Json<crate::routes::response::ApiResponse<m::ReactionResponse>>, AppError> {
    message_check(&s, &p, &id, "reactions:write").await?;
    m::add_reaction(State(s), p.actor(), Path(id), Json(b)).await
}
async fn delete_reaction(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Query(b): Query<m::DeleteReactionQuery>,
) -> Result<Json<crate::routes::response::ApiResponse<m::ReactionResponse>>, AppError> {
    message_check(&s, &p, &id, "reactions:write").await?;
    m::delete_reaction(State(s), p.actor(), Path(id), Query(b)).await
}
async fn create_channel(
    State(s): State<AppState>,
    p: Principal,
    Json(b): Json<c::CreateChannelRequest>,
) -> Result<Json<crate::routes::response::ApiResponse<c::ChannelResponse>>, AppError> {
    p.require("channels:manage")?;
    c::create_channel(State(s), p.actor(), Json(b)).await
}
async fn update_channel(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<c::UpdateChannelRequest>,
) -> Result<Json<crate::routes::response::ApiResponse<c::ChannelResponse>>, AppError> {
    check(&s, &p, &id, "channels:manage", true).await?;
    c::update_channel(State(s), p.actor(), Path(id), Json(b)).await
}
async fn delete_channel(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<crate::routes::response::ApiResponse<c::ChannelResponse>>, AppError> {
    check(&s, &p, &id, "channels:manage", true).await?;
    c::delete_channel(State(s), p.actor(), Path(id)).await
}
async fn join_channel(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<crate::routes::response::ApiResponse<c::ChannelUserResponse>>, AppError> {
    p.require("channels:manage")?;
    c::join_channel(State(s), p.actor(), Path(id)).await
}
async fn leave_channel(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<crate::routes::response::ApiResponse<c::LeaveChannelResponse>>, AppError> {
    check(&s, &p, &id, "channels:manage", false).await?;
    c::leave_channel(State(s), p.actor(), Path(id)).await
}
async fn archive_channel(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<crate::routes::response::ApiResponse<c::ChannelResponse>>, AppError> {
    check(&s, &p, &id, "channels:manage", true).await?;
    c::archive_channel(State(s), p.actor(), Path(id)).await
}
async fn unarchive_channel(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
) -> Result<Json<crate::routes::response::ApiResponse<c::ChannelResponse>>, AppError> {
    check(&s, &p, &id, "channels:manage", false).await?;
    c::unarchive_channel(State(s), p.actor(), Path(id)).await
}
async fn dm(
    State(s): State<AppState>,
    p: Principal,
    Json(b): Json<dms::CreateDmRequest>,
) -> Result<Json<crate::routes::response::ApiResponse<c::ChannelResponse>>, AppError> {
    p.require("dm:write")?;
    if !p.manifest.dm_enabled {
        return Err(AppError::Forbidden("App DMs are disabled".into()));
    }
    dms::create_or_get_dm(State(s), p.actor(), Json(b)).await
}
async fn shape(
    State(s): State<AppState>,
    p: Principal,
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<axum::response::Response, AppError> {
    use crate::routes::shapes;
    let guard_state = s.clone();
    let guard_principal = p.clone();
    let guard_channel = params.get("channel_id").cloned();
    let response = match name.as_str() {
        "messages" | "message_reactions" => {
            let channel = params
                .get("channel_id")
                .ok_or_else(|| AppError::BadRequest("channel_id is required".into()))?;
            check(&s, &p, channel, "messages:read", false).await?;
            if name == "messages" {
                shapes::messages_shape(State(s), p.actor(), Query(params)).await
            } else {
                shapes::message_reactions_shape(State(s), p.actor(), Query(params)).await
            }
        }
        "channels" => {
            p.require("channels:read")?;
            shapes::app_channels_shape(s, &p, params).await
        }
        "users" => {
            p.require("users:read")?;
            shapes::users_shape(State(s), p.actor(), Query(params)).await
        }
        "channel_users" => {
            let channel = params
                .get("channel_id")
                .ok_or_else(|| AppError::BadRequest("channel_id is required".into()))?;
            check(&s, &p, channel, "channels:read", false).await?;
            shapes::channel_users_shape(State(s), p.actor(), Query(params)).await
        }
        _ => Err(AppError::NotFound("Unknown app shape".into())),
    }?;
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        "cache-control",
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    let interval = tokio::time::interval(std::time::Duration::from_secs(1));
    let stream = futures_util::stream::unfold(
        (
            body.into_data_stream(),
            interval,
            guard_state,
            guard_principal,
            guard_channel,
        ),
        |(mut source, mut ticks, state, p, channel)| async move {
            loop {
                tokio::select! {
                 _=ticks.tick()=>{
                  if !super::delivery::active(&state,&p.credential_id).await.unwrap_or(false){return None;}
                  if let Some(id)=&channel {let mut db=state.db.clone();let Ok(mut tx)=db.transaction().await else{return None;};if p.channel(&mut tx,id,false).await.is_err(){return None;}}
                 },
                 data=source.next()=>return data.map(|d|(d,(source,ticks,state,p,channel)))
                }
            }
        },
    );
    Ok(axum::response::Response::from_parts(
        parts,
        axum::body::Body::from_stream(stream),
    ))
}
