//! Per-user federation display names; local chat names remain independent.
use super::*;
use crate::auth::AuthUser;
use axum::{Json, Router, extract::State, routing::get};
use serde::Deserialize;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/activitypub/profile", get(read).put(update))
        .route("/api/activitypub/accounts", get(account))
}
#[derive(Deserialize)]
struct ProfileInput {
    display_name: String,
    #[serde(default)]
    summary: String,
}
fn display_name(value: &str) -> Result<&str, AppError> {
    let value = value.trim();
    if value.chars().count() > 100 || value.chars().any(char::is_control) {
        return Err(AppError::BadRequest(
            "ActivityPub display name must be at most 100 characters without control characters"
                .into(),
        ));
    }
    Ok(value)
}
async fn read(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(one(&mut tx,"SELECT json_build_object('display_name',COALESCE(p.display_name,''),'default_display_name',u.display_name,'summary',COALESCE(p.summary,''))::text FROM users u LEFT JOIN apub_profiles p ON p.user_id=u.id WHERE u.id=$1",&[&user.id]).await?))
}
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Json(input): Json<ProfileInput>,
) -> Result<Json<Value>, AppError> {
    let name = display_name(&input.display_name)?;
    if input.summary.chars().count() > 1000 || input.summary.contains('\0') {
        return Err(AppError::BadRequest(
            "Bio must be at most 1000 characters".into(),
        ));
    }
    // Key generation uses its own transaction. Complete it before locking the profile.
    let key = if base_url().is_ok() {
        Some(keys(&state, &user.id).await?)
    } else {
        None
    };
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("INSERT INTO apub_profiles(user_id,display_name,summary) VALUES($1,NULLIF($2,''),$3) ON CONFLICT(user_id) DO UPDATE SET display_name=EXCLUDED.display_name,summary=EXCLUDED.summary")
        .bind(user.id.as_str()).bind(name).bind(input.summary.as_str()).exec(&mut tx).await?;
    if let Some(key) = key {
        let actor = actor_id(&user.id)?;
        let account = one(&mut tx,"SELECT json_build_object('user_name',u.user_name,'display_name',COALESCE(p.display_name,u.display_name),'summary',COALESCE(p.summary,''))::text FROM users u LEFT JOIN apub_profiles p ON p.user_id=u.id WHERE u.id=$1 AND NOT u.is_bot",&[&user.id]).await?;
        let followers = query(&mut tx,"SELECT json_build_object('id',f.actor_id)::text FROM apub_follows f JOIN apub_actors a ON a.id=f.actor_id WHERE f.user_id=$1 AND f.direction='incoming' AND f.accepted AND NOT a.blocked",&[&user.id]).await?;
        let activity = json!({"@context":CONTEXT,"id":format!("{}/api/activitypub/activities/{}",base_url()?,uuid::Uuid::new_v4()),"type":"Update","actor":actor,"object":routes::actor_value(&account,&key,&actor),"to":[format!("{actor}/followers")]});
        for follower in followers {
            queue(&mut tx, &user.id, string(&follower, "id")?, &activity).await?;
        }
    }
    tx.commit().await?;
    read(State(state), user).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_trim_reset_and_validate_unicode_length() {
        assert_eq!(display_name("  APub名  ").unwrap(), "APub名");
        assert_eq!(display_name("　 ").unwrap(), "");
        assert!(display_name(&"猫".repeat(100)).is_ok());
        assert!(display_name(&"猫".repeat(101)).is_err());
        assert!(display_name("bad\nname").is_err());
    }
}

#[derive(Deserialize)]
struct AccountQuery {
    actor: String,
}
async fn account(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Query(input): axum::extract::Query<AccountQuery>,
) -> Result<Json<Value>, AppError> {
    let local_prefix = base_url()
        .ok()
        .map(|base| format!("{base}/api/activitypub/users/"));
    let id = local_prefix
        .as_ref()
        .and_then(|prefix| input.actor.strip_prefix(prefix))
        .unwrap_or(&input.actor);
    if id.starts_with("https://") {
        let remote = remote_actor(&state, Some(&user.id), id, false).await?;
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        if remote["avatar"].is_object() {
            emoji::remember(&mut tx, &[remote["avatar"].clone()]).await?;
        }
        emoji::remember(
            &mut tx,
            remote["emojis"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )
        .await?;
        let follow = query(&mut tx,"SELECT json_build_object('accepted',accepted)::text FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing'",&[&user.id,id]).await?;
        tx.commit().await?;
        return Ok(Json(
            json!({"id":id,"user_name":remote["user_name"],"display_name":remote["display_name"],"summary":remote["summary"],"avatar":remote["avatar"],"emojis":remote["emojis"],"remote":true,"url":id,"following":follow.first().map(|v|v["accepted"].clone())}),
        ));
    }
    let local = routes::local_user(&state, id.trim_start_matches("local:")).await?;
    Ok(Json(
        json!({"id":id.trim_start_matches("local:"),"user_name":local["user_name"],"display_name":local["display_name"],"summary":local["summary"],"remote":false,"url":actor_id(id).ok()}),
    ))
}
