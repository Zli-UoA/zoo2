//! Authenticated operations that produce the additional native events.
use super::{
    events::{self, Actor},
    *,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{delete, get, patch, post},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users/me/profile", get(my_profile).patch(profile))
        .route("/api/users/{id}/profile", get(user_profile))
        .route("/api/users/{id}/guest", patch(guest_status))
        .route("/api/presence/heartbeat", post(heartbeat))
        .route(
            "/api/channels/{channel}/settings",
            get(channel_settings).patch(settings),
        )
        .route("/api/channels/{channel}/view", post(view))
        .route("/api/conversation-views", get(views))
        .route("/api/channels/{channel}/history", delete(clear_history))
        .route("/api/channels/{channel}/pins", get(pins).post(add_pin))
        .route(
            "/api/channels/{channel}/pins/{kind}/{id}",
            delete(remove_pin),
        )
        .route("/api/stars", get(stars).post(add_star))
        .route("/api/stars/{kind}/{id}", delete(remove_star))
        .route("/api/emojis", get(emojis).post(put_emoji))
        .route("/api/emojis/{name}", get(emoji).delete(remove_emoji))
        .route("/api/workspace", get(workspace).patch(update_workspace))
        .route("/api/usergroups", get(groups).post(create_group))
        .route("/api/usergroups/{id}", patch(update_group))
        .route(
            "/api/usergroups/{id}/members",
            get(group_members).put(set_group_members),
        )
        .route("/api/invite-requests", get(invites).post(invite))
        .route("/api/invite-requests/{id}", patch(resolve_invite))
        .route("/api/reminders", get(reminders).post(create_reminder))
        .route(
            "/api/reminders/{id}",
            patch(complete_reminder).delete(remove_reminder),
        )
        .route(
            "/api/messages/{id}/metadata",
            patch(metadata).delete(remove_metadata),
        )
        .route(
            "/api/assistant-sessions",
            get(sessions).post(create_session),
        )
        .route("/api/assistant-sessions/{id}", patch(update_session))
        .route("/api/apps/{id}/context", post(context))
        .route(
            "/api/apps/{id}/connection",
            get(connection).post(request_connection),
        )
        .route("/api/apps/{id}/functions/{name}/invoke", post(function))
        .route("/api/apps/{id}/entities/requests", post(entity))
        .route(
            "/api/apps/v1/users/me/profile",
            get(my_profile).patch(profile),
        )
        .route("/api/apps/v1/users/{id}/profile", get(user_profile))
        .route("/api/apps/v1/users/{id}/guest", patch(guest_status))
        .route("/api/apps/v1/presence/heartbeat", post(heartbeat))
        .route(
            "/api/apps/v1/channels/{channel}/settings",
            get(channel_settings).patch(settings),
        )
        .route("/api/apps/v1/channels/{channel}/view", post(view))
        .route(
            "/api/apps/v1/channels/{channel}/history",
            delete(clear_history),
        )
        .route(
            "/api/apps/v1/channels/{channel}/pins",
            get(pins).post(add_pin),
        )
        .route(
            "/api/apps/v1/channels/{channel}/pins/{kind}/{id}",
            delete(remove_pin),
        )
        .route("/api/apps/v1/stars", get(stars).post(add_star))
        .route("/api/apps/v1/stars/{kind}/{id}", delete(remove_star))
        .route("/api/apps/v1/emojis", get(emojis).post(put_emoji))
        .route(
            "/api/apps/v1/emojis/{name}",
            get(emoji).delete(remove_emoji),
        )
        .route(
            "/api/apps/v1/workspace",
            get(workspace).patch(update_workspace),
        )
        .route("/api/apps/v1/usergroups", get(groups).post(create_group))
        .route("/api/apps/v1/usergroups/{id}", patch(update_group))
        .route(
            "/api/apps/v1/usergroups/{id}/members",
            get(group_members).put(set_group_members),
        )
        .route("/api/apps/v1/invite-requests", get(invites).post(invite))
        .route(
            "/api/apps/v1/reminders",
            get(reminders).post(create_reminder),
        )
        .route(
            "/api/apps/v1/reminders/{id}",
            patch(complete_reminder).delete(remove_reminder),
        )
        .route(
            "/api/apps/v1/messages/{id}/metadata",
            patch(metadata).delete(remove_metadata),
        )
        .route(
            "/api/apps/v1/assistant-sessions",
            get(sessions).post(create_session),
        )
        .route(
            "/api/apps/v1/assistant-sessions/{id}",
            patch(update_session),
        )
        .route(
            "/api/apps/v1/function-executions/{id}/complete",
            post(complete_function),
        )
        .route(
            "/api/apps/v1/user-connections/{user}",
            post(confirm_connection),
        )
        .route(
            "/api/apps/v1/files/{id}",
            get(file).patch(update_file).delete(delete_file),
        )
        .route("/api/apps/v1/files/{id}/shares", post(share_file))
        .route(
            "/api/apps/v1/files/{id}/shares/{channel}",
            delete(unshare_file),
        )
        .route("/api/apps/v1/files/{id}/public-link", post(public_file))
        .route("/api/public-files/{token}", get(public_download))
        .layer(axum::middleware::map_response(super::no_store))
}
async fn my_profile(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("users:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let mut v=one(&mut tx,"SELECT json_build_object('id',u.id,'display_name',u.display_name,'status_text',u.status_text,'status_emoji',u.status_emoji,'status_expires_at',u.status_expires_at,'fields',u.profile_fields::jsonb,'guest_kind',u.guest_kind,'preferences',COALESCE(e.preferences,'{}')::jsonb,'presence',COALESCE(e.presence,'away'),'presence_override',COALESCE(e.presence_override,'auto'),'dnd_until',e.dnd_until)::text FROM users u LEFT JOIN user_event_settings e ON e.user_id=u.id WHERE u.id=$1",&[&a.user.id]).await?;
    if a.principal
        .as_ref()
        .is_some_and(|p| !p.scopes.iter().any(|s| s == "preferences:read"))
    {
        for key in ["preferences", "presence_override", "dnd_until"] {
            v.as_object_mut().unwrap().remove(key);
        }
    }
    Ok(Json(v))
}
async fn user_profile(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("users:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.visible_user(&mut tx, &id).await?;
    let v=one(&mut tx,"SELECT json_build_object('id',u.id,'user_name',u.user_name,'display_name',u.display_name,'avatar_url',u.avatar_url,'status_text',u.status_text,'status_emoji',u.status_emoji,'status_expires_at',u.status_expires_at,'fields',u.profile_fields::jsonb,'guest_kind',u.guest_kind,'presence',COALESCE(e.presence,'away'))::text FROM users u LEFT JOIN user_event_settings e ON e.user_id=u.id WHERE u.id=$1",&[&id]).await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    display_name: Option<String>,
    status_text: Option<String>,
    status_emoji: Option<String>,
    #[serde(default, deserialize_with = "present_value")]
    status_expires_at: Option<Value>,
    fields: Option<Value>,
    preferences: Option<Value>,
    presence: Option<String>,
    #[serde(default, deserialize_with = "present_value")]
    dnd_until: Option<Value>,
}
fn present_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}
fn timestamp(v: &Value) -> Result<String, AppError> {
    if v.is_null() {
        return Ok(String::new());
    }
    let s = v
        .as_str()
        .ok_or_else(|| AppError::BadRequest("Expected an RFC3339 timestamp or null".into()))?;
    let t: jiff::Timestamp = s
        .parse()
        .map_err(|_| AppError::BadRequest("Invalid timestamp".into()))?;
    Ok(t.to_string())
}
async fn profile(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Profile>,
) -> Result<Json<Value>, AppError> {
    if b.display_name.is_some()
        || b.status_text.is_some()
        || b.status_emoji.is_some()
        || b.status_expires_at.is_some()
        || b.fields.is_some()
    {
        a.require("users:write")?;
    }
    if b.preferences.is_some() || b.presence.is_some() || b.dnd_until.is_some() {
        a.require("preferences:write")?;
    }
    if let Some(name) = &b.display_name {
        crate::validation::validate_display_name(name)?;
    }
    for text in [&b.status_text, &b.status_emoji].into_iter().flatten() {
        events::text(text, 100)?;
    }
    if let Some(p) = &b.preferences {
        events::object(p)?;
    }
    if let Some(p) = &b.fields {
        events::object(p)?;
    }
    if b.presence
        .as_deref()
        .is_some_and(|v| !matches!(v, "auto" | "away"))
    {
        return Err(AppError::BadRequest("Presence must be auto or away".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let old = one(
        &mut tx,
        "SELECT row_to_json(u)::text FROM users u WHERE id=$1 FOR UPDATE",
        &[&a.user.id],
    )
    .await?;
    if old["is_bot"] == true && b.display_name.is_some() {
        return Err(AppError::Forbidden(
            "Manage a bot's display name through its App profile".into(),
        ));
    }
    if let Some(fields) = &b.fields {
        let definition = one(
            &mut tx,
            "SELECT profile_fields::text FROM workspace_settings WHERE id='workspace'",
            &[],
        )
        .await?;
        for (key, value) in fields.as_object().unwrap() {
            let d = definition
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["id"] == *key)
                .ok_or_else(|| AppError::BadRequest("Unknown profile field".into()))?;
            if !value.is_null()
                && !match d["type"].as_str() {
                    Some("text") => value.is_string(),
                    Some("number") => value.is_number(),
                    Some("boolean") => value.is_boolean(),
                    _ => false,
                }
            {
                return Err(AppError::BadRequest(
                    "Profile field value has the wrong type".into(),
                ));
            }
        }
    }
    let expires = b.status_expires_at.as_ref().map(timestamp).transpose()?;
    let body = json!({"display_name":b.display_name,"status_text":b.status_text,"status_emoji":b.status_emoji,"status_expires_at":expires,"fields":b.fields});
    let v=one(&mut tx,"UPDATE users SET display_name=COALESCE($2::jsonb->>'display_name',display_name),status_text=COALESCE($2::jsonb->>'status_text',status_text),status_emoji=COALESCE($2::jsonb->>'status_emoji',status_emoji),status_expires_at=CASE WHEN $2::jsonb->>'status_expires_at' IS NOT NULL THEN NULLIF($2::jsonb->>'status_expires_at','')::timestamptz ELSE status_expires_at END,profile_fields=CASE WHEN $2::jsonb->'fields'<>'null'::jsonb THEN ($2::jsonb->'fields')::text ELSE profile_fields END,updated_at=CURRENT_TIMESTAMP WHERE id=$1 RETURNING json_build_object('id',id,'status_text',status_text,'status_emoji',status_emoji,'fields',profile_fields::jsonb)::text",&[&a.user.id,&body.to_string()]).await?;
    if b.preferences.is_some() || b.presence.is_some() || b.dnd_until.is_some() {
        let dnd = b.dnd_until.as_ref().map(timestamp).transpose()?;
        let settings = json!({"preferences":b.preferences,"presence":b.presence,"dnd_until":dnd});
        one(&mut tx,"INSERT INTO user_event_settings(user_id) VALUES($1) ON CONFLICT(user_id) DO UPDATE SET user_id=EXCLUDED.user_id RETURNING '{}'::text",&[&a.user.id]).await?;
        toasty::sql::statement("UPDATE user_event_settings SET preferences=CASE WHEN $2::jsonb->'preferences'<>'null'::jsonb THEN ($2::jsonb->'preferences')::text ELSE preferences END,presence_override=COALESCE($2::jsonb->>'presence',presence_override),presence=CASE WHEN $2::jsonb->>'presence'='away' THEN 'away' WHEN $2::jsonb->>'presence'='auto' THEN CASE WHEN active_until>now() THEN 'active' ELSE 'away' END ELSE presence END,dnd_until=CASE WHEN $2::jsonb->>'dnd_until' IS NOT NULL THEN NULLIF($2::jsonb->>'dnd_until','')::timestamptz ELSE dnd_until END WHERE user_id=$1").bind(a.user.id.as_str()).bind(settings.to_string()).exec(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(Json(v))
}
async fn heartbeat(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("users:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let v=one(&mut tx,"INSERT INTO user_event_settings(user_id,presence,active_until) VALUES($1,'active',now()+interval '90 seconds') ON CONFLICT(user_id) DO UPDATE SET active_until=EXCLUDED.active_until,presence=CASE WHEN user_event_settings.presence_override='away' THEN 'away' ELSE 'active' END RETURNING json_build_object('presence',presence)::text",&[&a.user.id]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Guest {
    kind: String,
}
async fn guest_status(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<Guest>,
) -> Result<Json<Value>, AppError> {
    a.require("users:write")?;
    if !["none", "single", "multi", "deactivated"].contains(&b.kind.as_str()) {
        return Err(AppError::BadRequest("Invalid guest kind".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.admin(&mut tx, &s).await?;
    if id == a.user.id && b.kind != "none" {
        return Err(AppError::BadRequest(
            "An administrator cannot restrict their own account".into(),
        ));
    }
    let v=one(&mut tx,"UPDATE users SET guest_kind=$2 WHERE id=$1 AND NOT is_bot RETURNING json_build_object('id',id,'guest_kind',guest_kind)::text",&[&id,&b.kind]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    topic: Option<String>,
    purpose: Option<String>,
    posting_policy: Option<String>,
}
async fn channel_settings(
    State(s): State<AppState>,
    a: Actor,
    Path(channel): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("channels:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.channel(&mut tx, &channel, false).await?;
    Ok(Json(one(&mut tx,"SELECT json_build_object('topic',topic,'purpose',description,'posting_policy',posting_policy)::text FROM channels WHERE id=$1",&[&channel]).await?))
}
async fn settings(
    State(s): State<AppState>,
    a: Actor,
    Path(channel): Path<String>,
    Json(b): Json<Settings>,
) -> Result<Json<Value>, AppError> {
    a.require("channels:manage")?;
    if let Some(t) = &b.topic {
        events::text(t, 250)?;
    }
    if let Some(p) = &b.purpose {
        crate::validation::validate_channel_description(Some(p))?;
    }
    if b.posting_policy
        .as_deref()
        .is_some_and(|p| !matches!(p, "members" | "managers"))
    {
        return Err(AppError::BadRequest("Invalid posting policy".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.channel_manager(&mut tx, &s, &channel).await?;
    events::actor_context(&mut tx, &a.user.id).await?;
    let v=one(&mut tx,"UPDATE channels SET topic=COALESCE($2::jsonb->>'topic',topic),description=COALESCE($2::jsonb->>'purpose',description),posting_policy=COALESCE($2::jsonb->>'posting_policy',posting_policy),updated_at=CURRENT_TIMESTAMP WHERE id=$1 RETURNING json_build_object('id',id,'topic',topic,'purpose',description,'posting_policy',posting_policy)::text",&[&channel,&json!({"topic":b.topic,"purpose":b.purpose,"posting_policy":b.posting_policy}).to_string()]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
struct View {
    open: bool,
}
async fn view(
    State(s): State<AppState>,
    a: Actor,
    Path(channel): Path<String>,
    Json(b): Json<View>,
) -> Result<Json<Value>, AppError> {
    a.require("channels:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.channel(&mut tx, &channel, false).await?;
    let v=one(&mut tx,"INSERT INTO conversation_views VALUES($1,$2,$3::boolean) ON CONFLICT(user_id,channel_id) DO UPDATE SET is_open=EXCLUDED.is_open RETURNING json_build_object('channel_id',channel_id,'open',is_open)::text",&[&a.user.id,&channel,if b.open{"true"}else{"false"}]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn views(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("channels:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(json!(query(&mut tx,"SELECT json_build_object('channel_id',v.channel_id,'open',is_open)::text FROM conversation_views v JOIN channel_user cu ON cu.channel_id=v.channel_id AND cu.user_id=v.user_id WHERE v.user_id=$1",&[&a.user.id]).await?)))
}
async fn clear_history(
    State(s): State<AppState>,
    a: Actor,
    Path(channel): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("channels:manage")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.channel_manager(&mut tx, &s, &channel).await?;
    events::actor_context(&mut tx, &a.user.id).await?;
    let v=one(&mut tx,"WITH changed AS(UPDATE messages SET deleted_at=CURRENT_TIMESTAMP WHERE channel_id=$1 AND deleted_at IS NULL RETURNING id) SELECT json_build_object('deleted',count(*))::text FROM changed",&[&channel]).await?;
    if v["deleted"].as_u64().unwrap_or(0) > 0 {
        events::emit(
            &mut tx,
            "channel.history_changed",
            json!({"channel_id":channel,"deleted":v["deleted"]}),
            Some(&a.user.id),
            Some(&channel),
            None,
            None,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    item_type: String,
    item_id: String,
    channel_id: Option<String>,
}
async fn item_channel(
    tx: &mut toasty::Transaction<'_>,
    a: &Actor,
    item: &Item,
) -> Result<String, AppError> {
    let row=match item.item_type.as_str(){
 "message"=>one(tx,"SELECT json_build_object('channel',channel_id)::text FROM messages WHERE id=$1 AND deleted_at IS NULL",&[&item.item_id]).await?,
 "channel"=>json!({"channel":item.item_id}),
 "file"=>{let v=one(tx,"SELECT json_build_object('channel',COALESCE(NULLIF($2,''),channel_id))::text FROM app_files WHERE id=$1 AND completed AND deleted_at IS NULL",&[&item.item_id,item.channel_id.as_deref().unwrap_or("")]).await?;one(tx,"SELECT '{}'::text FROM app_file_shares WHERE file_id=$1 AND channel_id=$2",&[&item.item_id,v["channel"].as_str().unwrap()]).await?;v},
 _=>return Err(AppError::BadRequest("Invalid item type".into()))};
    let channel = row["channel"].as_str().unwrap().to_owned();
    if item.channel_id.as_ref().is_some_and(|id| id != &channel) {
        return Err(AppError::NotFound("Item not in channel".into()));
    }
    a.channel(tx, &channel, false).await?;
    Ok(channel)
}
async fn pins(
    State(s): State<AppState>,
    a: Actor,
    Path(channel): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("pins:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.channel(&mut tx, &channel, false).await?;
    Ok(Json(json!(query(&mut tx,"SELECT row_to_json(p)::text FROM channel_pins p WHERE channel_id=$1 ORDER BY created_at",&[&channel]).await?)))
}
async fn add_pin(
    State(s): State<AppState>,
    a: Actor,
    Path(channel): Path<String>,
    Json(mut b): Json<Item>,
) -> Result<Json<Value>, AppError> {
    a.require("pins:write")?;
    if b.item_type == "channel" {
        return Err(AppError::BadRequest("Pin a message or file".into()));
    }
    b.channel_id = Some(channel.clone());
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.channel(&mut tx, &channel, true).await?;
    item_channel(&mut tx, &a, &b).await?;
    toasty::sql::statement("INSERT INTO channel_pins(channel_id,item_type,item_id,created_by) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(channel).bind(b.item_type).bind(b.item_id).bind(a.user.id).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn remove_pin(
    State(s): State<AppState>,
    a: Actor,
    Path((channel, kind, id)): Path<(String, String, String)>,
) -> Result<Json<Value>, AppError> {
    a.require("pins:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.channel(&mut tx, &channel, true).await?;
    toasty::sql::statement(
        "DELETE FROM channel_pins WHERE channel_id=$1 AND item_type=$2 AND item_id=$3",
    )
    .bind(channel)
    .bind(kind)
    .bind(id)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn stars(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("stars:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        "SELECT row_to_json(i)::text FROM saved_items i WHERE user_id=$1 ORDER BY created_at",
        &[&a.user.id],
    )
    .await?;
    let mut visible = Vec::new();
    for row in rows {
        if let Some(c) = row["channel_id"].as_str() {
            match a.channel(&mut tx, c, false).await {
                Ok(_) => {}
                Err(AppError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        visible.push(row);
    }
    Ok(Json(json!(visible)))
}
async fn add_star(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Item>,
) -> Result<Json<Value>, AppError> {
    a.require("stars:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let channel = item_channel(&mut tx, &a, &b).await?;
    toasty::sql::statement("INSERT INTO saved_items(user_id,item_type,item_id,channel_id) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(a.user.id).bind(b.item_type).bind(b.item_id).bind(channel).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn remove_star(
    State(s): State<AppState>,
    a: Actor,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    a.require("stars:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "DELETE FROM saved_items WHERE user_id=$1 AND item_type=$2 AND item_id=$3",
    )
    .bind(a.user.id)
    .bind(kind)
    .bind(id)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

async fn emojis(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("emoji:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(json!(
        query(
            &mut tx,
            "SELECT row_to_json(e)::text FROM workspace_emojis e ORDER BY name",
            &[]
        )
        .await?
    )))
}
async fn emoji(
    State(s): State<AppState>,
    a: Actor,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("emoji:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(
        one(
            &mut tx,
            "SELECT row_to_json(e)::text FROM workspace_emojis e WHERE name=$1",
            &[&name],
        )
        .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Emoji {
    name: String,
    url: String,
}
async fn put_emoji(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Emoji>,
) -> Result<Json<Value>, AppError> {
    a.require("emoji:write")?;
    if b.name.is_empty()
        || b.name.len() > 64
        || !b
            .name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    {
        return Err(AppError::BadRequest("Invalid emoji name".into()));
    }
    validate_https(&b.url)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    if events::guest(&mut tx, &a.user.id).await? {
        return Err(AppError::Forbidden(
            "Guests cannot manage workspace emojis".into(),
        ));
    }
    let existing = query(
        &mut tx,
        "SELECT row_to_json(e)::text FROM workspace_emojis e WHERE name=$1 FOR UPDATE",
        &[&b.name],
    )
    .await?;
    if let Some(old) = existing.first()
        && old["created_by"] != a.user.id
    {
        a.admin(&mut tx, &s).await?;
    }
    let v=one(&mut tx,"INSERT INTO workspace_emojis(name,url,created_by) VALUES($1,$2,$3) ON CONFLICT(name) DO UPDATE SET url=EXCLUDED.url,updated_at=now() RETURNING row_to_json(workspace_emojis)::text",&[&b.name,&b.url,&a.user.id]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn remove_emoji(
    State(s): State<AppState>,
    a: Actor,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("emoji:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let e = one(
        &mut tx,
        "SELECT row_to_json(e)::text FROM workspace_emojis e WHERE name=$1 FOR UPDATE",
        &[&name],
    )
    .await?;
    if e["created_by"] != a.user.id {
        a.admin(&mut tx, &s).await?;
    }
    toasty::sql::statement("DELETE FROM workspace_emojis WHERE name=$1")
        .bind(name)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn workspace(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("workspace:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(one(&mut tx,"SELECT json_build_object('name',name,'preferences',preferences::jsonb,'profile_fields',profile_fields::jsonb)::text FROM workspace_settings WHERE id='workspace'",&[]).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    name: Option<String>,
    preferences: Option<Value>,
    profile_fields: Option<Value>,
}
async fn update_workspace(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Workspace>,
) -> Result<Json<Value>, AppError> {
    a.require("workspace:write")?;
    if let Some(n) = &b.name {
        events::text(n, 80)?;
        if n.trim().is_empty() {
            return Err(AppError::BadRequest("Workspace name is empty".into()));
        }
    }
    if let Some(p) = &b.preferences {
        events::object(p)?;
    }
    if let Some(fields) = &b.profile_fields {
        let fields = fields
            .as_array()
            .filter(|f| f.len() <= 50)
            .ok_or_else(|| AppError::BadRequest("Expected at most 50 profile fields".into()))?;
        let mut ids = std::collections::HashSet::new();
        for f in fields {
            let id = f["id"]
                .as_str()
                .ok_or_else(|| AppError::BadRequest("Profile field requires id".into()))?;
            let label = f["label"]
                .as_str()
                .ok_or_else(|| AppError::BadRequest("Profile field requires label".into()))?;
            events::text(id, 64)?;
            events::text(label, 100)?;
            if id.is_empty()
                || !ids.insert(id)
                || !matches!(f["type"].as_str(), Some("text" | "number" | "boolean"))
            {
                return Err(AppError::BadRequest(
                    "Invalid profile field definition".into(),
                ));
            }
        }
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.admin(&mut tx, &s).await?;
    let body = json!({"name":b.name,"preferences":b.preferences,"fields":b.profile_fields});
    let v=one(&mut tx,"UPDATE workspace_settings SET name=COALESCE($1::jsonb->>'name',name),preferences=CASE WHEN $1::jsonb->'preferences'<>'null'::jsonb THEN ($1::jsonb->'preferences')::text ELSE preferences END,profile_fields=CASE WHEN $1::jsonb->'fields'<>'null'::jsonb THEN ($1::jsonb->'fields')::text ELSE profile_fields END WHERE id='workspace' RETURNING json_build_object('name',name,'preferences',preferences::jsonb,'profile_fields',profile_fields::jsonb)::text",&[&body.to_string()]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn groups(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("usergroups:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(json!(
        query(
            &mut tx,
            "SELECT row_to_json(g)::text FROM usergroups g ORDER BY name",
            &[]
        )
        .await?
    )))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    name: String,
    handle: String,
    #[serde(default)]
    description: String,
    #[serde(default = "yes")]
    active: bool,
}
fn yes() -> bool {
    true
}
impl Group {
    fn validate(&self) -> Result<(), AppError> {
        events::text(&self.name, 80)?;
        events::text(&self.description, 1000)?;
        if self.name.trim().is_empty()
            || self.handle.is_empty()
            || self.handle.len() > 80
            || !self
                .handle
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
        {
            return Err(AppError::BadRequest(
                "Invalid usergroup name or handle".into(),
            ));
        }
        Ok(())
    }
}
async fn create_group(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Group>,
) -> Result<Json<Value>, AppError> {
    a.require("usergroups:write")?;
    b.validate()?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.admin(&mut tx, &s).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let v=one(&mut tx,"INSERT INTO usergroups(id,name,handle,description,active,created_by) VALUES($1,$2,$3,$4,$5::boolean,$6) RETURNING row_to_json(usergroups)::text",&[&id,&b.name,&b.handle,&b.description,if b.active{"true"}else{"false"},&a.user.id]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn update_group(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<Group>,
) -> Result<Json<Value>, AppError> {
    a.require("usergroups:write")?;
    b.validate()?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.admin(&mut tx, &s).await?;
    let v=one(&mut tx,"UPDATE usergroups SET name=$2,handle=$3,description=$4,active=$5::boolean WHERE id=$1 RETURNING row_to_json(usergroups)::text",&[&id,&b.name,&b.handle,&b.description,if b.active{"true"}else{"false"}]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn group_members(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("usergroups:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    one(
        &mut tx,
        "SELECT '{}'::text FROM usergroups WHERE id=$1",
        &[&id],
    )
    .await?;
    Ok(Json(json!(query(&mut tx,"SELECT json_build_object('user_id',user_id)::text FROM usergroup_members WHERE group_id=$1 ORDER BY user_id",&[&id]).await?)))
}
async fn set_group_members(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(users): Json<Vec<String>>,
) -> Result<Json<Value>, AppError> {
    a.require("usergroups:write")?;
    if users.len() > 1000
        || users.iter().collect::<std::collections::HashSet<_>>().len() != users.len()
    {
        return Err(AppError::BadRequest("At most 1000 unique members".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.admin(&mut tx, &s).await?;
    one(
        &mut tx,
        "SELECT '{}'::text FROM usergroups WHERE id=$1 FOR UPDATE",
        &[&id],
    )
    .await?;
    for user in &users {
        one(
            &mut tx,
            "SELECT '{}'::text FROM users WHERE id=$1 AND NOT is_bot AND NOT is_deactivated",
            &[user],
        )
        .await?;
    }
    let encoded = json!(users).to_string();
    toasty::sql::statement("DELETE FROM usergroup_members WHERE group_id=$1 AND user_id NOT IN(SELECT jsonb_array_elements_text($2::jsonb))").bind(id.as_str()).bind(encoded).exec(&mut tx).await?;
    for user in users {
        toasty::sql::statement(
            "INSERT INTO usergroup_members VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(id.as_str())
        .bind(user)
        .exec(&mut tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn invites(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("invites:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let admin =
        crate::auth::rbac::is_workspace_admin(&mut tx, &a.user.id, s.config.dev_auth_enabled)
            .await?;
    Ok(Json(json!(query(&mut tx,"SELECT row_to_json(i)::text FROM workspace_invite_requests i WHERE requested_by=$1 OR $2::boolean ORDER BY created_at DESC LIMIT 100",&[&a.user.id,if admin||a.principal.as_ref().is_some_and(|p|!p.delegated){"true"}else{"false"}]).await?)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    email: String,
    channel_id: Option<String>,
}
async fn invite(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Invite>,
) -> Result<Json<Value>, AppError> {
    a.require("invites:write")?;
    if b.email.len() > 254 || !b.email.contains('@') || b.email.chars().any(char::is_whitespace) {
        return Err(AppError::BadRequest("Invalid email".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    if events::guest(&mut tx, &a.user.id).await? {
        return Err(AppError::Forbidden(
            "Guests cannot request invitations".into(),
        ));
    }
    if let Some(c) = &b.channel_id {
        a.channel(&mut tx, c, true).await?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let v=one(&mut tx,"INSERT INTO workspace_invite_requests(id,email,requested_by,channel_id) VALUES($1,$2,$3,NULLIF($4,'')) RETURNING row_to_json(workspace_invite_requests)::text",&[&id,&b.email,&a.user.id,b.channel_id.as_deref().unwrap_or("")]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
struct Resolution {
    status: String,
}
async fn resolve_invite(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<Resolution>,
) -> Result<Json<Value>, AppError> {
    a.require("invites:write")?;
    if !matches!(b.status.as_str(), "approved" | "rejected") {
        return Err(AppError::BadRequest("Invalid resolution".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    a.admin(&mut tx, &s).await?;
    let v=one(&mut tx,"UPDATE workspace_invite_requests SET status=$2,resolved_by=$3 WHERE id=$1 AND status='pending' RETURNING row_to_json(workspace_invite_requests)::text",&[&id,&b.status,&a.user.id]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn reminders(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("reminders:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    Ok(Json(json!(query(&mut tx,"SELECT row_to_json(r)::text FROM user_reminders r WHERE user_id=$1 ORDER BY due_at LIMIT 500",&[&a.user.id]).await?)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reminder {
    content: String,
    due_at: String,
    channel_id: Option<String>,
}
async fn create_reminder(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Reminder>,
) -> Result<Json<Value>, AppError> {
    a.require("reminders:write")?;
    events::text(&b.content, 4000)?;
    if b.content.trim().is_empty() {
        return Err(AppError::BadRequest("Reminder is empty".into()));
    }
    let due = timestamp(&json!(b.due_at))?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    if let Some(c) = &b.channel_id {
        a.channel(&mut tx, c, false).await?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let v=one(&mut tx,"INSERT INTO user_reminders(id,user_id,channel_id,content,due_at) VALUES($1,$2,NULLIF($3,''),$4,$5::timestamptz) RETURNING row_to_json(user_reminders)::text",&[&id,&a.user.id,b.channel_id.as_deref().unwrap_or(""),&b.content,&due]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn complete_reminder(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("reminders:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let v=one(&mut tx,"UPDATE user_reminders SET completed_at=now() WHERE id=$1 AND user_id=$2 RETURNING row_to_json(user_reminders)::text",&[&id,&a.user.id]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn remove_reminder(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("reminders:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("DELETE FROM user_reminders WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(a.user.id)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn metadata(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, AppError> {
    a.require("metadata:write")?;
    events::object(&b)?;
    message_metadata(s, a, id, Some(b)).await
}
async fn remove_metadata(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("metadata:write")?;
    message_metadata(s, a, id, None).await
}
async fn message_metadata(
    s: AppState,
    a: Actor,
    id: String,
    data: Option<Value>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let m = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
    a.channel(&mut tx, &m.channel_id, true).await?;
    if m.user_id != a.user.id || m.deleted_at.is_some() {
        return Err(AppError::Forbidden(
            "Only the message author may change metadata".into(),
        ));
    }
    let v=one(&mut tx,"UPDATE messages SET metadata=NULLIF($2,''),updated_at=CURRENT_TIMESTAMP WHERE id=$1 RETURNING json_build_object('id',id,'metadata',metadata::jsonb)::text",&[&id,&data.map(|v|v.to_string()).unwrap_or_default()]).await?;
    tx.commit().await?;
    Ok(Json(v))
}

async fn target_app(
    tx: &mut toasty::Transaction<'_>,
    a: &Actor,
    id: &str,
    channel: Option<&str>,
) -> Result<Value, AppError> {
    let v=one(tx,"SELECT json_build_object('id',i.id,'app_id',a.id,'bot',a.bot_user_id,'manifest',i.manifest::jsonb,'scopes',i.scopes::jsonb)::text FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE a.id=$1 AND i.active AND a.status='installed' AND a.deleted_at IS NULL",&[id]).await?;
    if let Some(channel) = channel {
        a.channel(tx, channel, false).await?;
        one(
            tx,
            "SELECT '{}'::text FROM channel_user WHERE channel_id=$1 AND user_id=$2",
            &[channel, v["bot"].as_str().unwrap()],
        )
        .await?;
    }
    Ok(v)
}
async fn callback(
    tx: &mut toasty::Transaction<'_>,
    a: &Actor,
    installation: &str,
    kind: &str,
    channel: Option<&str>,
    mut data: Value,
) -> Result<(), AppError> {
    let trigger = secret("zoo_trigger_");
    let response = secret("zoo_response_");
    toasty::sql::statement("INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,expires_at) VALUES($1,$2,$3,NULLIF($4,''),'trigger',now()+interval '3 seconds')").bind(trigger.as_str()).bind(installation).bind(a.user.id.as_str()).bind(channel.unwrap_or("")).exec(tx).await?;
    toasty::sql::statement("INSERT INTO app_responses(token_hash,installation_id,user_id,channel_id,expires_at) VALUES($1,$2,$3,NULLIF($4,''),now()+interval '15 minutes')").bind(hash(&response)).bind(installation).bind(a.user.id.as_str()).bind(channel.unwrap_or("")).exec(tx).await?;
    data["trigger_id"] = json!(trigger);
    data["response_path"] = json!(format!("/api/app-responses/{response}"));
    data["user_id"] = json!(a.user.id);
    events::emit(
        tx,
        kind,
        data,
        Some(&a.user.id),
        channel,
        None,
        Some(installation),
    )
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    app_id: Option<String>,
    user_id: String,
    channel_id: String,
    parent_id: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default = "empty_object")]
    context: Value,
}
fn empty_object() -> Value {
    json!({})
}
async fn create_session(
    State(s): State<AppState>,
    a: Actor,
    Json(b): Json<Session>,
) -> Result<Json<Value>, AppError> {
    a.require("assistant:write")?;
    events::text(&b.title, 200)?;
    events::object(&b.context)?;
    let app_id = if let Some(p) = &a.principal {
        if b.app_id.as_ref().is_some_and(|id| id != &p.app_id) {
            return Err(AppError::Forbidden("Use your own App".into()));
        }
        p.app_id.as_str()
    } else {
        b.app_id
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("app_id required".into()))?
    };
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let app = target_app(&mut tx, &a, app_id, Some(&b.channel_id)).await?;
    if !app["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "assistant:write")
    {
        return Err(AppError::Forbidden(
            "App has no assistant capability".into(),
        ));
    }
    if a.principal.as_ref().is_none_or(|p| p.delegated) && b.user_id != a.user.id {
        return Err(AppError::Forbidden("Use your own session".into()));
    }
    one(
        &mut tx,
        "SELECT '{}'::text FROM channel_user WHERE channel_id=$1 AND user_id=$2",
        &[&b.channel_id, &b.user_id],
    )
    .await?;
    if let Some(parent) = &b.parent_id {
        one(&mut tx,"SELECT '{}'::text FROM messages WHERE id=$1 AND channel_id=$2 AND parent_id IS NULL AND deleted_at IS NULL",&[parent,&b.channel_id]).await?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let v=one(&mut tx,"INSERT INTO assistant_sessions(id,installation_id,user_id,actor_id,channel_id,parent_id,title,context) VALUES($1,$2,$3,$4,$5,NULLIF($6,''),$7,$8) RETURNING row_to_json(assistant_sessions)::text",&[&id,app["id"].as_str().unwrap(),&b.user_id,&a.user.id,&b.channel_id,b.parent_id.as_deref().unwrap_or(""),&b.title,&b.context.to_string()]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
async fn sessions(State(s): State<AppState>, a: Actor) -> Result<Json<Value>, AppError> {
    a.require("assistant:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT json_build_object('id',s.id,'app_id',i.app_id,'user_id',s.user_id,'channel_id',s.channel_id,'parent_id',s.parent_id,'title',s.title,'context',s.context::jsonb,'state',s.state)::text FROM assistant_sessions s JOIN app_installations i ON i.id=s.installation_id WHERE (s.user_id=$1 OR i.id=NULLIF($2,'')) AND ($3='' OR i.app_id=$3) ORDER BY s.created_at DESC LIMIT 100",&[&a.user.id,a.principal.as_ref().filter(|p|!p.delegated).map(|p|p.installation_id.as_str()).unwrap_or(""),a.principal.as_ref().map(|p|p.app_id.as_str()).unwrap_or("")]).await?;
    let mut visible = Vec::new();
    for row in rows {
        match a
            .channel(&mut tx, row["channel_id"].as_str().unwrap(), false)
            .await
        {
            Ok(_) => visible.push(row),
            Err(AppError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Json(json!(visible)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionUpdate {
    title: Option<String>,
    context: Option<Value>,
    state: Option<String>,
}
async fn update_session(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<SessionUpdate>,
) -> Result<Json<Value>, AppError> {
    a.require("assistant:write")?;
    if let Some(title) = &b.title {
        events::text(title, 200)?;
    }
    if let Some(ctx) = &b.context {
        events::object(ctx)?;
    }
    if b.state.as_deref().is_some_and(|s| s != "stopped") {
        return Err(AppError::BadRequest("Sessions may only be stopped".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let old = one(
        &mut tx,
        "SELECT row_to_json(s)::text FROM assistant_sessions s WHERE id=$1 FOR UPDATE",
        &[&id],
    )
    .await?;
    if a.principal
        .as_ref()
        .is_some_and(|p| old["installation_id"] != p.installation_id)
    {
        return Err(AppError::NotFound("Session not found".into()));
    }
    if old["user_id"] != a.user.id
        && !a
            .principal
            .as_ref()
            .is_some_and(|p| !p.delegated && old["installation_id"] == p.installation_id)
    {
        return Err(AppError::NotFound("Session not found".into()));
    }
    a.channel(&mut tx, old["channel_id"].as_str().unwrap(), false)
        .await?;
    let body = json!({"title":b.title,"context":b.context,"state":b.state});
    let v=one(&mut tx,"UPDATE assistant_sessions SET title=COALESCE($2::jsonb->>'title',title),context=CASE WHEN $2::jsonb->'context'<>'null'::jsonb THEN ($2::jsonb->'context')::text ELSE context END,state=COALESCE($2::jsonb->>'state',state),actor_id=$3 WHERE id=$1 RETURNING row_to_json(assistant_sessions)::text",&[&id,&body.to_string(),&a.user.id]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Context {
    channel_id: Option<String>,
    context: Value,
}
async fn context(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<Context>,
) -> Result<Json<Value>, AppError> {
    a.require("assistant:write")?;
    events::object(&b.context)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let app = target_app(&mut tx, &a, &id, b.channel_id.as_deref()).await?;
    if !app["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "assistant:write")
    {
        return Err(AppError::Forbidden(
            "App has no assistant capability".into(),
        ));
    }
    toasty::sql::statement("INSERT INTO app_view_contexts(installation_id,user_id,channel_id,context) VALUES($1,$2,NULLIF($3,''),$4) ON CONFLICT(installation_id,user_id) DO UPDATE SET channel_id=EXCLUDED.channel_id,context=EXCLUDED.context").bind(app["id"].as_str().unwrap()).bind(a.user.id).bind(b.channel_id.unwrap_or_default()).bind(b.context.to_string()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn connection(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("users:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let app = target_app(&mut tx, &a, &id, None).await?;
    let rows=query(&mut tx,"SELECT json_build_object('state',state)::text FROM app_user_connections WHERE installation_id=$1 AND user_id=$2",&[app["id"].as_str().unwrap(),&a.user.id]).await?;
    Ok(Json(
        rows.into_iter()
            .next()
            .unwrap_or_else(|| json!({"state":"disconnected"})),
    ))
}
#[derive(Deserialize)]
struct Connection {
    connect: bool,
    channel_id: Option<String>,
}
async fn request_connection(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<Connection>,
) -> Result<Json<Value>, AppError> {
    a.require("users:write")?;
    if a.principal.as_ref().is_some_and(|p| !p.delegated) {
        return Err(AppError::Forbidden(
            "A human user must request their connection".into(),
        ));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let app = target_app(&mut tx, &a, &id, b.channel_id.as_deref()).await?;
    if app["manifest"]["user_connections"] != true {
        return Err(AppError::BadRequest(
            "App does not offer user connections".into(),
        ));
    }
    let requested = if b.connect {
        "requested"
    } else {
        "disconnected"
    };
    let old=query(&mut tx,"SELECT to_json(state)::text FROM app_user_connections WHERE installation_id=$1 AND user_id=$2 FOR UPDATE",&[app["id"].as_str().unwrap(),&a.user.id]).await?;
    let changed = old.first().is_none_or(|v| v.as_str() != Some(requested));
    toasty::sql::statement("INSERT INTO app_user_connections VALUES($1,$2,$3) ON CONFLICT(installation_id,user_id) DO UPDATE SET state=EXCLUDED.state").bind(app["id"].as_str().unwrap()).bind(a.user.id.as_str()).bind(requested).exec(&mut tx).await?;
    if changed {
        callback(
            &mut tx,
            &a,
            app["id"].as_str().unwrap(),
            "user.connection_requested",
            b.channel_id.as_deref(),
            json!({"subtype":if b.connect{"connect"}else{"disconnect"}}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"state":requested})))
}
#[derive(Deserialize)]
struct ConfirmConnection {
    connected: bool,
}
async fn confirm_connection(
    State(s): State<AppState>,
    p: Principal,
    Path(user): Path<String>,
    Json(b): Json<ConfirmConnection>,
) -> Result<Json<Value>, AppError> {
    p.require("users:write")?;
    if p.delegated {
        return Err(AppError::Forbidden(
            "Use a bot token to confirm external connections".into(),
        ));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let v=one(&mut tx,"UPDATE app_user_connections SET state=$3 WHERE installation_id=$1 AND user_id=$2 AND ($3='disconnected' OR state IN('requested','connected')) RETURNING json_build_object('state',state)::text",&[&p.installation_id,&user,if b.connected{"connected"}else{"disconnected"}]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Function {
    channel_id: Option<String>,
    #[serde(default = "empty_object")]
    inputs: Value,
    id: Option<String>,
}
async fn function(
    State(s): State<AppState>,
    a: Actor,
    Path((id, name)): Path<(String, String)>,
    Json(b): Json<Function>,
) -> Result<Json<Value>, AppError> {
    a.require("functions:execute")?;
    events::object(&b.inputs)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let app = target_app(&mut tx, &a, &id, b.channel_id.as_deref()).await?;
    if !app["manifest"]["functions"]
        .as_array()
        .is_some_and(|f| f.iter().any(|f| f == &name))
    {
        return Err(AppError::NotFound("Function not registered".into()));
    }
    let execution = b.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid::Uuid::parse_str(&execution)
        .map_err(|_| AppError::BadRequest("Invalid execution id".into()))?;
    let rows = query(
        &mut tx,
        "SELECT row_to_json(e)::text FROM app_function_executions e WHERE id=$1 FOR UPDATE",
        &[&execution],
    )
    .await?;
    if let Some(old) = rows.first() {
        if old["installation_id"] != app["id"]
            || old["actor_id"] != a.user.id
            || old["name"] != name
            || old["inputs"].as_str() != Some(&b.inputs.to_string())
            || old["channel_id"].as_str() != b.channel_id.as_deref()
        {
            return Err(AppError::Conflict("Execution id already used".into()));
        }
        return Ok(Json(old.clone()));
    }
    let v=one(&mut tx,"INSERT INTO app_function_executions(id,installation_id,actor_id,channel_id,name,inputs) VALUES($1,$2,$3,NULLIF($4,''),$5,$6) RETURNING row_to_json(app_function_executions)::text",&[&execution,app["id"].as_str().unwrap(),&a.user.id,b.channel_id.as_deref().unwrap_or(""),&name,&b.inputs.to_string()]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FunctionResult {
    outputs: Option<Value>,
    error: Option<String>,
}
async fn complete_function(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<FunctionResult>,
) -> Result<Json<Value>, AppError> {
    p.require("functions:execute")?;
    if let Some(v) = &b.outputs {
        events::object(v)?;
    }
    if let Some(e) = &b.error {
        events::text(e, 2000)?;
    }
    if b.outputs.is_some() == b.error.is_some() {
        return Err(AppError::BadRequest("Supply outputs or error".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let old=one(&mut tx,"SELECT row_to_json(e)::text FROM app_function_executions e WHERE id=$1 AND installation_id=$2 FOR UPDATE",&[&id,&p.installation_id]).await?;
    if let Some(c) = old["channel_id"].as_str() {
        p.channel(&mut tx, c, false).await?;
    }
    let outputs = b.outputs.map(|v| v.to_string()).unwrap_or_default();
    let error = b.error.unwrap_or_default();
    if old["state"] != "pending" {
        if old["outputs"].as_str().unwrap_or("") != outputs
            || old["error"].as_str().unwrap_or("") != error
        {
            return Err(AppError::Conflict(
                "Function result already recorded".into(),
            ));
        }
        return Ok(Json(old));
    }
    let v=one(&mut tx,"UPDATE app_function_executions SET outputs=NULLIF($2,''),error=NULLIF($3,''),state=CASE WHEN $3='' THEN 'completed' ELSE 'failed' END WHERE id=$1 RETURNING row_to_json(app_function_executions)::text",&[&id,&outputs,&error]).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entity {
    kind: String,
    url: String,
    message_id: String,
    channel_id: String,
    cursor: Option<String>,
    thread_root_id: Option<String>,
    limit: Option<u32>,
}
async fn entity(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<Entity>,
) -> Result<Json<Value>, AppError> {
    a.require("links:write")?;
    if !matches!(b.kind.as_str(), "details" | "comments")
        || b.limit.is_some_and(|n| n == 0 || n > 100)
    {
        return Err(AppError::BadRequest("Invalid entity request".into()));
    }
    if let Some(c) = &b.cursor {
        events::text(c, 2000)?;
    }
    if let Some(c) = &b.thread_root_id {
        events::text(c, 200)?;
    }
    validate_https(&b.url)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let app = target_app(&mut tx, &a, &id, Some(&b.channel_id)).await?;
    one(&mut tx,"SELECT '{}'::text FROM app_link_unfurls u JOIN messages m ON m.id=u.message_id WHERE u.installation_id=$1 AND u.message_id=$2 AND u.url=$3 AND m.channel_id=$4 AND m.deleted_at IS NULL",&[app["id"].as_str().unwrap(),&b.message_id,&b.url,&b.channel_id]).await?;
    callback(&mut tx,&a,app["id"].as_str().unwrap(),if b.kind=="details"{"entity.details_requested"}else{"entity.comments_requested"},Some(&b.channel_id),json!({"url":b.url,"message_id":b.message_id,"cursor":b.cursor,"limit":b.limit.unwrap_or(100),"thread_root_id":b.thread_root_id})).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

fn file_public_data(row: &Value, channel: &str) -> Value {
    json!({"id":row["id"],"name":row["name"],"description":row["description"],"content_type":row["content_type"],"size_bytes":row["size_bytes"],"channel_id":channel,"public":!row["public_token_hash"].is_null()})
}
async fn readable_file(
    tx: &mut toasty::Transaction<'_>,
    a: &Actor,
    id: &str,
) -> Result<(Value, String), AppError> {
    let f=one(tx,"SELECT row_to_json(f)::text FROM app_files f WHERE id=$1 AND completed AND deleted_at IS NULL",&[id]).await?;
    let shares=query(tx,"SELECT json_build_object('channel',channel_id)::text FROM app_file_shares WHERE file_id=$1 ORDER BY channel_id",&[id]).await?;
    for share in shares {
        let c = share["channel"].as_str().unwrap();
        match a.channel(tx, c, false).await {
            Ok(_) => return Ok((f, c.to_owned())),
            Err(AppError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Err(AppError::NotFound("File not found".into()))
}
async fn writable_file(
    tx: &mut toasty::Transaction<'_>,
    a: &Actor,
    s: &AppState,
    id: &str,
) -> Result<Value, AppError> {
    let (f, c) = readable_file(tx, a, id).await?;
    a.channel(tx, &c, true).await?;
    if f["user_id"] != a.user.id
        && !a
            .principal
            .as_ref()
            .is_some_and(|p| !p.delegated && f["installation_id"] == p.installation_id)
    {
        a.admin(tx, s).await?;
    }
    one(
        tx,
        "SELECT '{}'::text FROM app_files WHERE id=$1 FOR UPDATE",
        &[id],
    )
    .await?;
    Ok(f)
}
async fn file(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("files:read")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let (f, c) = readable_file(&mut tx, &a, &id).await?;
    Ok(Json(file_public_data(&f, &c)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileUpdate {
    name: Option<String>,
    description: Option<String>,
}
async fn update_file(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<FileUpdate>,
) -> Result<Json<Value>, AppError> {
    a.require("files:write")?;
    if let Some(name) = &b.name {
        events::text(name, 255)?;
        if name.trim().is_empty() {
            return Err(AppError::BadRequest("File name is empty".into()));
        }
    }
    if let Some(desc) = &b.description {
        events::text(desc, 4000)?;
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let old = writable_file(&mut tx, &a, &s, &id).await?;
    let v=one(&mut tx,"UPDATE app_files SET name=COALESCE($2::jsonb->>'name',name),description=COALESCE($2::jsonb->>'description',description) WHERE id=$1 RETURNING row_to_json(app_files)::text",&[&id,&json!({"name":b.name,"description":b.description}).to_string()]).await?;
    tx.commit().await?;
    Ok(Json(file_public_data(
        &v,
        old["channel_id"].as_str().unwrap(),
    )))
}
async fn delete_file(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    a.require("files:write")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    writable_file(&mut tx, &a, &s, &id).await?;
    toasty::sql::statement(
        "UPDATE app_files SET deleted_at=now(),public_token_hash=NULL WHERE id=$1",
    )
    .bind(id)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct FileShare {
    channel_id: String,
}
async fn share_file(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<FileShare>,
) -> Result<Json<Value>, AppError> {
    a.require("files:share")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    writable_file(&mut tx, &a, &s, &id).await?;
    a.channel(&mut tx, &b.channel_id, true).await?;
    toasty::sql::statement("INSERT INTO app_file_shares VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(id)
        .bind(b.channel_id)
        .bind(a.user.id)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn unshare_file(
    State(s): State<AppState>,
    a: Actor,
    Path((id, channel)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    a.require("files:share")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    writable_file(&mut tx, &a, &s, &id).await?;
    a.channel(&mut tx, &channel, true).await?;
    if one(
        &mut tx,
        "SELECT json_build_object('count',count(*))::text FROM app_file_shares WHERE file_id=$1",
        &[&id],
    )
    .await?["count"]
        .as_u64()
        .unwrap_or(0)
        <= 1
    {
        return Err(AppError::BadRequest(
            "Delete the file instead of removing its last share".into(),
        ));
    }
    toasty::sql::statement("DELETE FROM app_file_shares WHERE file_id=$1 AND channel_id=$2")
        .bind(id)
        .bind(channel)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct FilePublic {
    public: bool,
}
async fn public_file(
    State(s): State<AppState>,
    a: Actor,
    Path(id): Path<String>,
    Json(b): Json<FilePublic>,
) -> Result<Json<Value>, AppError> {
    a.require("files:share")?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    writable_file(&mut tx, &a, &s, &id).await?;
    let token = if b.public {
        secret("zoo_file_")
    } else {
        String::new()
    };
    let digest = if b.public {
        hash(&token)
    } else {
        String::new()
    };
    toasty::sql::statement("UPDATE app_files SET public_token_hash=NULLIF($2,'') WHERE id=$1")
        .bind(id)
        .bind(digest)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"public":b.public,"path":if b.public{Some(format!("/api/public-files/{token}"))}else{None}}),
    ))
}
async fn public_download(
    State(s): State<AppState>,
    Path(token): Path<String>,
) -> Result<axum::response::Redirect, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let f=one(&mut tx,"SELECT row_to_json(f)::text FROM app_files f WHERE public_token_hash=$1 AND completed AND deleted_at IS NULL AND EXISTS(SELECT 1 FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE i.id=f.installation_id AND i.active AND a.status='installed' AND a.deleted_at IS NULL)",&[&hash(&token)]).await?;
    let value = super::files::signed_download(&f)?.0;
    Ok(axum::response::Redirect::temporary(
        value["url"].as_str().unwrap(),
    ))
}
