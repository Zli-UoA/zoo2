//! Authenticated Home/LTL and atomic channel cross-posts.
use crate::{
    activitypub,
    apps::{one, query},
    auth::AuthUser,
    error::AppError,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch},
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Default, Deserialize)]
struct Page {
    before: Option<String>,
    timeline: Option<String>,
    limit: Option<usize>,
}
#[derive(Deserialize)]
struct NewPost {
    content: String,
    #[serde(default)]
    apub: bool,
    #[serde(default = "local_visibility")]
    visibility: String,
    channel_id: Option<String>,
}
fn local_visibility() -> String {
    "local".into()
}
#[derive(Deserialize)]
struct EditPost {
    content: String,
}
#[derive(Deserialize)]
struct Settings {
    ltl_channel_pattern: String,
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/home", get(list).post(create))
        .route("/api/home/live", get(super::home_live::upgrade))
        .route("/api/home/settings", get(settings).put(update_settings))
        .route("/api/home/{id}", patch(edit).delete(delete))
}
fn validate_content(content: &str) -> Result<(), AppError> {
    if content.trim().is_empty() || content.chars().count() > 1000 || content.contains('\0') {
        return Err(AppError::BadRequest(
            "Content must contain 1–1000 characters".into(),
        ));
    }
    Ok(())
}
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<NewPost>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    validate_content(&body.content)?;
    if !matches!(
        body.visibility.as_str(),
        "local" | "public" | "unlisted" | "followers"
    ) || (body.apub && body.visibility == "local")
        || (body.channel_id.is_some() && body.visibility == "followers")
    {
        return Err(AppError::BadRequest(
            "Invalid ActivityPub audience or channel association".into(),
        ));
    }
    if body.apub {
        activitypub::base_url()?;
        activitypub::keys(&state, &user.id).await?;
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if let Some(channel) = &body.channel_id {
        let actor = crate::apps::events::Actor {
            user: user.clone(),
            principal: None,
        };
        let c = actor.channel(&mut tx, channel, true).await?;
        if c["is_private"] == true || c["is_dm"] == true {
            return Err(AppError::BadRequest(
                "Home requires a public channel".into(),
            ));
        }
        crate::apps::events::posting(&mut tx, &user.id, channel, state.config.dev_auth_enabled)
            .await?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO home_posts(id,user_id,content,apub,visibility,channel_id) VALUES($1,$2,$3,$4::boolean,$5,NULLIF($6,''))")
        .bind(id.as_str()).bind(user.id.as_str()).bind(body.content).bind(body.apub.to_string()).bind(body.visibility).bind(body.channel_id.as_deref().unwrap_or("")).exec(&mut tx).await?;
    let notifications = if body.channel_id.is_some() {
        let message = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
        crate::notifications::create_message_notifications(&mut tx, &message, &user.id, None)
            .await?
    } else {
        vec![]
    };
    tx.commit().await?;
    crate::push::dispatch(state, notifications);
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
async fn mutable_post(
    tx: &mut toasty::Transaction<'_>,
    state: &AppState,
    user: &AuthUser,
    id: &str,
) -> Result<Value, AppError> {
    let p=one(tx,"SELECT row_to_json(p)::text FROM home_posts p WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL FOR UPDATE",&[id,&user.id]).await?;
    if let Some(channel) = p["channel_id"].as_str() {
        let actor = crate::apps::events::Actor {
            user: user.clone(),
            principal: None,
        };
        actor.channel(tx, channel, true).await?;
        crate::apps::events::posting(tx, &user.id, channel, state.config.dev_auth_enabled).await?;
    }
    Ok(p)
}
async fn edit(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<EditPost>,
) -> Result<StatusCode, AppError> {
    crate::validation::validate_message_content_with_options(&body.content, false)?;
    if body.content.contains('\0') {
        return Err(AppError::BadRequest("Invalid content".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    mutable_post(&mut tx, &state, &user, &id).await?;
    toasty::sql::statement("UPDATE home_posts SET content=$2,updated_at=now() WHERE id=$1")
        .bind(id)
        .bind(body.content)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let p=one(&mut tx,"SELECT row_to_json(p)::text FROM home_posts p WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL",&[&id,&user.id]).await?;
    if p["channel_id"].is_string() {
        drop(tx);
        let _ = crate::routes::messages::delete_message(State(state), user, Path(id)).await?;
    } else {
        toasty::sql::statement("UPDATE home_posts SET deleted_at=now(),updated_at=now() WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL").bind(id).bind(user.id).exec(&mut tx).await?;
        tx.commit().await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(page): Query<Page>,
) -> Result<Json<Vec<Value>>, AppError> {
    let timeline = page.timeline.as_deref().unwrap_or("home");
    if !matches!(timeline, "home" | "local") {
        return Err(AppError::BadRequest("Unknown timeline".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let guest = crate::apps::events::guest(&mut tx, &user.id).await?;
    let rows=query(&mut tx,r#"WITH feed AS (
      SELECT p.id,p.user_id,u.user_name,u.display_name,p.content,p.created_at,p.updated_at,p.apub,p.visibility,p.channel_id,c.channel_name,false AS remote,NULL::text AS url
      FROM home_posts p JOIN users u ON u.id=p.user_id LEFT JOIN channels c ON c.id=p.channel_id
      WHERE p.deleted_at IS NULL AND (p.visibility IN ('local','public') OR p.user_id=$1 AND $3='home')
        AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL)
        AND (NOT $4::boolean OR p.user_id=$1 OR EXISTS(SELECT 1 FROM channel_user WHERE channel_id=p.channel_id AND user_id=$1))
      UNION ALL
      SELECT p.id,p.actor_id,a.user_name || '@' || split_part(a.id,'/',3),a.display_name,p.content,p.published,p.updated_at,true,p.visibility,NULL,NULL,true,p.id
      FROM apub_remote_posts p JOIN apub_actors a ON a.id=p.actor_id
      WHERE $3='home' AND p.deleted_at IS NULL AND NOT a.blocked
        AND EXISTS(SELECT 1 FROM apub_remote_recipients WHERE post_id=p.id AND user_id=$1)
        AND EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$1 AND actor_id=p.actor_id AND direction='outgoing' AND accepted)
    ) SELECT row_to_json(f)::text FROM feed f WHERE ($2='' OR (created_at,id)<(SELECT created_at,id FROM feed WHERE id=$2)) ORDER BY created_at DESC,id DESC LIMIT $5::integer"#,
    &[&user.id,page.before.as_deref().unwrap_or(""),timeline,if guest{"true"}else{"false"}, &page.limit.unwrap_or(50).clamp(1, 500).to_string()]).await?;
    Ok(Json(rows))
}
async fn settings(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let admin =
        crate::auth::rbac::is_workspace_admin(&mut tx, &user.id, state.config.dev_auth_enabled)
            .await?;
    let setting = one(
        &mut tx,
        "SELECT row_to_json(s)::text FROM activitypub_settings s",
        &[],
    )
    .await?;
    Ok(Json(
        json!({"ltl_channel_pattern":setting["ltl_channel_pattern"],"can_manage":admin}),
    ))
}
async fn update_settings(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Settings>,
) -> Result<Json<Value>, AppError> {
    if body.ltl_channel_pattern.chars().count() > 200 || body.ltl_channel_pattern.contains('\0') {
        return Err(AppError::BadRequest("Invalid channel pattern".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if !crate::auth::rbac::is_workspace_admin(&mut tx, &user.id, state.config.dev_auth_enabled)
        .await?
    {
        return Err(AppError::Forbidden("Administrator required".into()));
    }
    let updated=one(&mut tx,"UPDATE activitypub_settings SET ltl_channel_pattern=$1 RETURNING row_to_json(activitypub_settings)::text",&[&body.ltl_channel_pattern]).await?;
    tx.commit().await?;
    Ok(Json(updated))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_limits() {
        assert!(validate_content(" \n").is_err());
        assert!(validate_content("text\0").is_err());
        assert!(validate_content(&"あ".repeat(1000)).is_ok());
        assert!(validate_content(&"あ".repeat(1001)).is_err());
    }
}
