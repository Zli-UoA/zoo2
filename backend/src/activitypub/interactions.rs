//! Approval and Like side effects share a transaction with their durable delivery and notification.
use super::*;
use crate::auth::AuthUser;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/activitypub/follows/respond", post(respond))
        .route("/api/activitypub/reactions", post(like).delete(unlike))
        .route("/api/activitypub/notifications", get(notifications))
        .route("/api/activitypub/notifications/read", post(read))
}

pub(crate) async fn notify(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    actor: &str,
    activity: &str,
    kind: &str,
    post: &str,
) -> Result<(), AppError> {
    toasty::sql::statement("INSERT INTO apub_notifications(id,user_id,actor_id,activity_id,kind,post_id) VALUES($1,$2,$3,$4,$5,NULLIF($6,'')) ON CONFLICT DO NOTHING")
        .bind(uuid::Uuid::new_v4().to_string()).bind(user).bind(actor).bind(activity).bind(kind).bind(post).exec(tx).await?;
    Ok(())
}
fn activity_id() -> Result<String, AppError> {
    Ok(format!(
        "{}/api/activitypub/activities/{}",
        base_url()?,
        uuid::Uuid::new_v4()
    ))
}
#[derive(Deserialize)]
struct Decision {
    actor: String,
    activity_id: String,
    accept: bool,
}
async fn respond(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Decision>,
) -> Result<Json<Value>, AppError> {
    keys(&state, &user.id).await?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let pending=one(&mut tx,"SELECT row_to_json(f)::text FROM apub_follows f WHERE user_id=$1 AND actor_id=$2 AND direction='incoming' AND activity_id=$3 AND NOT accepted FOR UPDATE", &[&user.id,&body.actor,&body.activity_id]).await?;
    let original: Value = pending["activity_json"].as_str().map(serde_json::from_str).transpose().map_err(internal)?
        .unwrap_or(json!({"type":"Follow","id":body.activity_id,"actor":body.actor,"object":actor_id(&user.id)?}));
    let response = json!({"@context":CONTEXT,"type":if body.accept{"Accept"}else{"Reject"},"id":activity_id()?,"actor":actor_id(&user.id)?,"object":original,"to":[body.actor]});
    queue(&mut tx, &user.id, &body.actor, &response).await?;
    if body.accept {
        toasty::sql::statement("UPDATE apub_follows SET accepted=true WHERE user_id=$1 AND actor_id=$2 AND direction='incoming'").bind(user.id.as_str()).bind(body.actor.as_str()).exec(&mut tx).await?;
    } else {
        toasty::sql::statement(
            "DELETE FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='incoming'",
        )
        .bind(user.id.as_str())
        .bind(body.actor.as_str())
        .exec(&mut tx)
        .await?;
    }
    toasty::sql::statement("UPDATE apub_notifications SET is_read=true WHERE user_id=$1 AND activity_id=$2 AND kind='follow_request'").bind(user.id).bind(body.activity_id).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct Reaction {
    post_id: String,
    reaction: Option<String>,
    emoji_id: Option<String>,
}
// Resolve exclusively through the same audience checks as Home, including guest restrictions.
pub(crate) async fn target(state: &AppState, user: &AuthUser, id: &str) -> Result<Value, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let guest = crate::apps::events::guest(&mut tx, &user.id).await?;
    one(&mut tx,"SELECT json_build_object('id',p.id,'author',p.user_id,'apub',p.apub,'visibility',p.visibility,'remote',false)::text FROM home_posts p LEFT JOIN channels c ON c.id=p.channel_id WHERE p.id=$1 AND p.deleted_at IS NULL AND (p.visibility IN ('local','public') OR p.user_id=$2 OR p.reply_user_id=$2) AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL) AND (NOT $3::boolean OR p.user_id=$2 OR p.reply_user_id=$2 OR EXISTS(SELECT 1 FROM channel_user WHERE channel_id=p.channel_id AND user_id=$2)) UNION ALL SELECT json_build_object('id',p.id,'author',p.actor_id,'apub',true,'visibility',p.visibility,'remote',true)::text FROM apub_remote_posts p JOIN apub_actors a ON a.id=p.actor_id WHERE p.id=$1 AND p.deleted_at IS NULL AND NOT a.blocked AND ((EXISTS(SELECT 1 FROM apub_remote_recipients WHERE post_id=p.id AND user_id=$2) AND (EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$2 AND actor_id=p.actor_id AND direction='outgoing' AND accepted) OR EXISTS(SELECT 1 FROM home_posts parent WHERE parent.id=p.parent_id AND parent.user_id=$2))) OR (p.visibility IN ('public','unlisted') AND EXISTS(SELECT 1 FROM apub_reposts r WHERE r.post_id=p.id AND ((r.user_id IS NOT NULL AND r.visibility IN ('public','local') AND (NOT $3::boolean OR r.user_id=$2)) OR (r.user_id IS NULL AND EXISTS(SELECT 1 FROM apub_repost_recipients WHERE activity_id=r.activity_id AND user_id=$2) AND EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$2 AND actor_id=r.actor_id AND direction='outgoing' AND accepted))))))", &[id,&user.id,if guest{"true"}else{"false"}]).await
}
pub(super) async fn undo_reaction(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    row: &Value,
) -> Result<(), AppError> {
    let activity: Value = serde_json::from_str(string(row, "activity_json")?).map_err(internal)?;
    if activity["actor"]
        .as_str()
        .is_some_and(|v| v.starts_with("https://") || v.starts_with("http://"))
    {
        let author = activity["to"][0]
            .as_str()
            .ok_or_else(|| AppError::BadRequest("Missing reaction recipient".into()))?;
        queue(tx,user,author,&json!({"@context":CONTEXT,"id":activity_id()?,"type":"Undo","actor":activity["actor"],"object":activity,"to":[author]})).await?;
    }
    toasty::sql::statement(
        "UPDATE apub_notifications SET is_read=true WHERE activity_id=$1 AND kind='like'",
    )
    .bind(string(row, "activity_id")?)
    .exec(tx)
    .await?;
    Ok(())
}
async fn like(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Reaction>,
) -> Result<Json<Value>, AppError> {
    let target = target(&state, &user, &body.post_id).await?;
    let remote = target["remote"] == true;
    let actor = if remote {
        keys(&state, &user.id).await?;
        actor_id(&user.id)?
    } else {
        format!("local:{}", user.id)
    };
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended($1||':'||$2,0))) locked",&[&user.id,&body.post_id]).await?;
    let requested = body.reaction.as_deref().unwrap_or("❤️");
    let mut selected = None;
    if requested.starts_with(':') {
        let image = body
            .emoji_id
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("Missing custom emoji identity".into()))?;
        let origin = if remote {
            reqwest::Url::parse(string(&target, "author")?)
                .map_err(internal)?
                .origin()
                .ascii_serialization()
        } else {
            String::new()
        };
        let catalog=query(&mut tx,"SELECT e::text FROM apub_emoji_catalog c CROSS JOIN LATERAL jsonb_array_elements(c.emojis) e WHERE c.origin=$1 AND e->>'image_id'=$2 AND e->>'name'=$3 UNION ALL SELECT emoji::text FROM apub_reactions WHERE post_id=$4 AND emoji->>'image_id'=$2 AND emoji->>'name'=$3 UNION ALL SELECT e::text FROM apub_remote_posts p CROSS JOIN LATERAL jsonb_array_elements(p.emojis) e WHERE p.id=$4 AND e->>'image_id'=$2 AND e->>'name'=$3 LIMIT 1", &[&origin,image,requested,&body.post_id]).await?;
        selected = catalog.into_iter().next();
        if selected.is_none() {
            return Err(AppError::BadRequest(
                "Unknown or unavailable custom emoji".into(),
            ));
        }
    }
    let normalized = if selected.is_some() {
        requested.to_owned()
    } else {
        emojis::get(requested)
            .filter(|_| requested.chars().count() <= 32)
            .ok_or_else(|| AppError::BadRequest("Invalid emoji".into()))?
            .as_str()
            .to_owned()
    };
    let old=query(&mut tx,"SELECT row_to_json(r)::text FROM apub_reactions r WHERE post_id=$1 AND user_id=$2 FOR UPDATE",&[&body.post_id,&user.id]).await?;
    if let Some(old) = old.first() {
        if old["reaction"] == normalized && old["emoji"] == json!(selected) {
            return Ok(Json(json!({"ok":true})));
        }
        undo_reaction(&mut tx, &user.id, old).await?;
        toasty::sql::statement("DELETE FROM apub_reactions WHERE post_id=$1 AND user_id=$2")
            .bind(body.post_id.as_str())
            .bind(user.id.as_str())
            .exec(&mut tx)
            .await?;
    }
    let id = if remote {
        activity_id()?
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let mut activity = json!({"@context":[CONTEXT,{"Emoji":"http://joinmastodon.org/ns#Emoji","_misskey_reaction":"https://misskey-hub.net/ns#_misskey_reaction"}],"id":id,"type":"Like","actor":actor,"object":body.post_id,"content":normalized,"_misskey_reaction":normalized,"to":[target["author"]]});
    if let Some(e) = &selected {
        activity["tag"] = json!([emoji::as_tag(e)]);
    }
    toasty::sql::statement("INSERT INTO apub_reactions(post_id,actor_id,user_id,activity_id,activity_json,reaction,emoji) VALUES($1,$2,$3,$4,$5,$6,$7::jsonb)").bind(body.post_id.as_str()).bind(actor).bind(user.id.as_str()).bind(id.as_str()).bind(activity.to_string()).bind(normalized.as_str()).bind(json!(selected).to_string()).exec(&mut tx).await?;
    if remote {
        queue(&mut tx, &user.id, string(&target, "author")?, &activity).await?;
    } else if target["author"] != user.id {
        notify(
            &mut tx,
            string(&target, "author")?,
            activity["actor"].as_str().unwrap(),
            &id,
            "like",
            &body.post_id,
        )
        .await?;
        toasty::sql::statement(
            "UPDATE apub_notifications SET reaction=$2,emoji=$3::jsonb WHERE activity_id=$1",
        )
        .bind(id)
        .bind(normalized)
        .bind(json!(selected).to_string())
        .exec(&mut tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn unlike(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Reaction>,
) -> Result<Json<Value>, AppError> {
    // The owner can always retract their Like, even after unfollowing or a Note deletion.
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"DELETE FROM apub_reactions WHERE post_id=$1 AND user_id=$2 RETURNING row_to_json(apub_reactions)::text",&[&body.post_id,&user.id]).await?;
    for row in rows {
        undo_reaction(&mut tx, &user.id, &row).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn notifications(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT json_build_object('id',n.id,'actor_id',n.actor_id,'display_name',COALESCE(a.display_name,u.display_name,n.actor_id),'actor_emojis',COALESCE(a.emojis,'[]'::jsonb),'kind',n.kind,'activity_id',n.activity_id,'post_id',n.post_id,'reaction',n.reaction,'emoji',n.emoji,'content',CASE WHEN p.id IS NOT NULL AND p.deleted_at IS NULL THEN p.content WHEN rp.id IS NOT NULL AND rp.deleted_at IS NULL THEN rp.content ELSE NULL END,'is_read',n.is_read,'created_at',n.created_at,'pending',EXISTS(SELECT 1 FROM apub_follows f WHERE f.user_id=n.user_id AND f.actor_id=n.actor_id AND f.activity_id=n.activity_id AND f.direction='incoming' AND NOT f.accepted))::text FROM apub_notifications n LEFT JOIN apub_actors a ON a.id=n.actor_id LEFT JOIN users u ON 'local:' || u.id=n.actor_id LEFT JOIN home_posts p ON p.id=n.post_id AND (p.user_id=n.user_id OR p.reply_user_id=n.user_id) AND (p.channel_id IS NULL OR EXISTS(SELECT 1 FROM channels c WHERE c.id=p.channel_id AND NOT c.is_private AND c.deleted_at IS NULL)) LEFT JOIN apub_remote_posts rp ON rp.id=n.post_id AND EXISTS(SELECT 1 FROM apub_remote_recipients recipient WHERE recipient.post_id=rp.id AND recipient.user_id=n.user_id) WHERE n.user_id=$1 ORDER BY EXISTS(SELECT 1 FROM apub_follows f WHERE f.user_id=n.user_id AND f.actor_id=n.actor_id AND f.activity_id=n.activity_id AND f.direction='incoming' AND NOT f.accepted) DESC,n.created_at DESC,n.id DESC LIMIT 100",&[&user.id]).await?;
    let count=one(&mut tx,"SELECT json_build_object('count',count(*))::text FROM apub_notifications WHERE user_id=$1 AND NOT is_read",&[&user.id]).await?;
    Ok(Json(json!({"items":rows,"unread_count":count["count"]})))
}
#[derive(Deserialize)]
struct Read {
    id: Option<String>,
}
async fn read(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Read>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    toasty::sql::statement("UPDATE apub_notifications SET is_read=true WHERE user_id=$1 AND ($2='' OR id=$2) AND NOT is_read").bind(user.id).bind(body.id.unwrap_or_default()).exec(&mut db).await?;
    Ok(Json(json!({"ok":true})))
}
