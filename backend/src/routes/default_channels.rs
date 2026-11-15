//! Default public channels for newly provisioned accounts.
use crate::{
    apps::{one, query},
    auth::AuthUser,
    error::AppError,
    state::AppState,
};
use axum::{Json, Router, extract::State, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/api/workspace/default-channels",
        get(settings).patch(update),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    channel_ids: Vec<String>,
}

async fn require_admin(
    tx: &mut toasty::Transaction<'_>,
    state: &AppState,
    user: &AuthUser,
) -> Result<(), AppError> {
    if !crate::auth::rbac::is_workspace_admin(tx, &user.id, state.config.dev_auth_enabled).await? {
        return Err(AppError::Forbidden("Administrator required".into()));
    }
    Ok(())
}
async fn read(tx: &mut toasty::Transaction<'_>) -> Result<Value, AppError> {
    let selected = one(tx, "SELECT json_build_object('channel_ids', COALESCE(preferences::jsonb->'default_channel_ids','[]'::jsonb))::text FROM workspace_settings WHERE id='workspace'", &[]).await?;
    let channels = query(tx, "SELECT json_build_object('id',id,'channel_name',channel_name)::text FROM channels WHERE NOT is_private AND NOT is_dm AND archive_at IS NULL AND deleted_at IS NULL ORDER BY channel_name", &[]).await?;
    Ok(json!({"channel_ids":selected["channel_ids"],"channels":channels}))
}
async fn settings(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &state, &user).await?;
    Ok(Json(read(&mut tx).await?))
}
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Settings>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    require_admin(&mut tx, &state, &user).await?;
    let unique: std::collections::HashSet<_> = body.channel_ids.iter().collect();
    if body.channel_ids.len() > 100 || unique.len() != body.channel_ids.len() {
        return Err(AppError::BadRequest(
            "Expected at most 100 distinct channel IDs".into(),
        ));
    }
    for id in &body.channel_ids {
        let valid = one(&mut tx, "SELECT json_build_object('valid',EXISTS(SELECT 1 FROM channels WHERE id=$1 AND NOT is_private AND NOT is_dm AND archive_at IS NULL AND deleted_at IS NULL))::text", &[id]).await?;
        if valid["valid"] != true {
            return Err(AppError::BadRequest(
                "Default channels must be active public channels".into(),
            ));
        }
    }
    toasty::sql::statement("UPDATE workspace_settings SET preferences=jsonb_set(preferences::jsonb,'{default_channel_ids}',$1::jsonb)::text WHERE id='workspace'")
        .bind(serde_json::to_string(&body.channel_ids).map_err(|e| AppError::Internal(e.into()))?).exec(&mut tx).await?;
    let result = read(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(result))
}

/// Run in the account creation transaction, never on subsequent logins.
pub(crate) async fn join(tx: &mut toasty::Transaction<'_>, user_id: &str) -> Result<(), AppError> {
    toasty::sql::statement("INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT $1,c.id,CURRENT_TIMESTAMP FROM channels c JOIN workspace_settings s ON s.id='workspace' WHERE c.id IN (SELECT jsonb_array_elements_text(COALESCE(s.preferences::jsonb->'default_channel_ids','[]'::jsonb))) AND NOT c.is_private AND NOT c.is_dm AND c.archive_at IS NULL AND c.deleted_at IS NULL ON CONFLICT DO NOTHING")
        .bind(user_id).exec(tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires migrated PostgreSQL"]
    async fn joins_only_configured_active_public_channels_atomically() -> anyhow::Result<()> {
        let config = crate::config::Config::from_env();
        let mut db = crate::db::init_db(&config.database_url).await?;
        let mut tx = db.transaction().await?;
        let user = uuid::Uuid::new_v4().to_string();
        toasty::sql::statement(
            "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'Default channel test')",
        )
        .bind(user.clone())
        .exec(&mut tx)
        .await?;
        let mut ids = Vec::new();
        for kind in [
            "public",
            "public2",
            "private",
            "dm",
            "archived",
            "deleted",
            "unselected",
        ] {
            let id = uuid::Uuid::new_v4().to_string();
            toasty::sql::statement("INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm,archive_at,deleted_at) VALUES($1,$1,$2,$3,$4,CASE WHEN $5 THEN CURRENT_TIMESTAMP END,CASE WHEN $6 THEN CURRENT_TIMESTAMP END)")
                .bind(id.clone()).bind(user.clone()).bind(kind == "private").bind(kind == "dm").bind(kind == "archived").bind(kind == "deleted").exec(&mut tx).await?;
            ids.push(id);
        }
        let configured = serde_json::to_string(&ids[..6])?;
        toasty::sql::statement("UPDATE workspace_settings SET preferences=jsonb_set(preferences::jsonb,'{default_channel_ids}',$1::jsonb)::text WHERE id='workspace'").bind(configured).exec(&mut tx).await?;
        join(&mut tx, &user).await?;
        join(&mut tx, &user).await?;
        let memberships = query(
            &mut tx,
            "SELECT json_build_object('id',channel_id)::text FROM channel_user WHERE user_id=$1",
            &[&user],
        )
        .await?;
        assert_eq!(memberships.len(), 2);
        assert!(
            memberships
                .iter()
                .all(|m| m["id"] == ids[0] || m["id"] == ids[1])
        );
        toasty::sql::statement("UPDATE workspace_settings SET preferences=jsonb_set(preferences::jsonb,'{default_channel_ids}','[]'::jsonb)::text WHERE id='workspace'").exec(&mut tx).await?;
        let other = uuid::Uuid::new_v4().to_string();
        toasty::sql::statement(
            "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'Empty defaults')",
        )
        .bind(other.clone())
        .exec(&mut tx)
        .await?;
        join(&mut tx, &other).await?;
        assert!(query(&mut tx, "SELECT json_build_object('id',channel_id)::text FROM channel_user WHERE user_id=$1", &[&other]).await?.is_empty());
        tx.rollback().await?;
        Ok(())
    }
}
