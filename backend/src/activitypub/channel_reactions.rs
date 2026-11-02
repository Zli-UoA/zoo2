//! Channel-linked posts use native local reactions and retain federated actor identities.
use super::*;
use crate::auth::AuthUser;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/channels/{id}/federated-reactions", get(list))
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // Match channel reaction Shape access, including guests and soft-deleted channels.
    one(&mut tx,"SELECT '{}'::text FROM channels c WHERE c.id=$1 AND c.deleted_at IS NULL AND (NOT c.is_private AND NOT EXISTS(SELECT 1 FROM users WHERE id=$2 AND is_guest) OR EXISTS(SELECT 1 FROM channel_user WHERE channel_id=c.id AND user_id=$2))",&[&id,&user.id]).await?;
    let mut rows = query(&mut tx,"SELECT json_build_object('id',r.activity_id,'message_id',p.id,'user_id',r.actor_id,'emoji',r.reaction,'created_at',r.created_at,'custom_emoji',r.emoji,'display_name',a.display_name,'user_name',a.user_name || '@' || split_part(a.id,'/',3))::text FROM apub_reactions r JOIN home_posts p ON p.id=r.post_id JOIN messages m ON m.id=p.id JOIN apub_actors a ON a.id=r.actor_id WHERE p.channel_id=$1 AND p.deleted_at IS NULL AND m.deleted_at IS NULL AND r.user_id IS NULL AND NOT a.blocked ORDER BY r.created_at,r.activity_id",&[&id]).await?;
    let registered = emoji::workspace_options(&mut tx).await?;
    for row in &mut rows {
        if row["custom_emoji"].is_object() {
            let descriptor = &row["custom_emoji"];
            row["emoji"] = if let Some(local) = registered
                .iter()
                .find(|e| e["image_id"] == descriptor["image_id"])
            {
                local["name"].clone()
            } else {
                json!(format!(
                    "apub:{}:{}",
                    string(descriptor, "image_id")?,
                    string(descriptor, "name")?
                ))
            };
        }
    }
    remember_display_emojis(
        &mut tx,
        &rows
            .iter()
            .map(|row| json!({"emoji":row["custom_emoji"]}))
            .collect::<Vec<_>>(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!(rows)))
}

pub(crate) async fn decorate(
    tx: &mut toasty::Transaction<'_>,
    posts: &mut [Value],
) -> Result<(), AppError> {
    let registered = emoji::workspace_options(tx).await?;
    for post in posts {
        let Some(reactions) = post["reactions"].as_array_mut() else {
            continue;
        };
        let mut merged: Vec<Value> = vec![];
        for mut reaction in std::mem::take(reactions) {
            if !reaction["emoji"].is_object() {
                if let Some(emoji) = registered
                    .iter()
                    .find(|emoji| emoji["name"] == reaction["reaction"])
                {
                    reaction["emoji"] = emoji.clone();
                }
            }
            if reaction["emoji"].is_object() {
                if let Some(local) = registered
                    .iter()
                    .find(|e| e["image_id"] == reaction["emoji"]["image_id"])
                {
                    reaction["emoji"] = local.clone();
                    reaction["reaction"] = local["name"].clone();
                }
            }
            if let Some(existing) = merged.iter_mut().find(|r| {
                r["reaction"] == reaction["reaction"]
                    && r["emoji"]["image_id"] == reaction["emoji"]["image_id"]
            }) {
                existing["count"] = json!(
                    existing["count"].as_i64().unwrap_or(0)
                        + reaction["count"].as_i64().unwrap_or(0)
                );
                existing["selected"] =
                    json!(existing["selected"] == true || reaction["selected"] == true);
            } else {
                merged.push(reaction);
            }
        }
        *reactions = merged;
    }
    Ok(())
}
