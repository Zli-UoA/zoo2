use super::*;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{id}/invoke", post(invoke))
        .route("/api/app-surfaces", get(private_surfaces))
        .route("/api/app-surfaces/{id}", axum::routing::delete(dismiss))
        .route("/api/app-responses/{token}", post(respond))
        .route("/api/apps/commands", get(commands))
        .route("/api/apps/commands/{name}", post(command))
        .route("/api/apps/v1/messages/{id}/unfurls", post(unfurl))
        .route("/api/apps/{id}/surfaces", get(surfaces))
        .route("/api/apps/v1/surfaces", post(publish))
        .route("/api/apps/v1/messages/{id}/blocks", post(blocks))
        .layer(axum::middleware::map_response(super::no_store))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Block {
    Card {
        card: crate::cards::MessageCard,
    },
    Text {
        text: String,
    },
    Button {
        action_id: String,
        label: String,
        #[serde(default)]
        value: String,
    },
    Input {
        action_id: String,
        label: String,
        #[serde(default)]
        required: bool,
    },
    Select {
        action_id: String,
        label: String,
        options: Vec<String>,
    },
}
pub fn validate_blocks(value: &Value) -> Result<(), AppError> {
    let blocks: Vec<Block> = serde_json::from_value(value.clone())
        .map_err(|_| AppError::BadRequest("Invalid block document".into()))?;
    if blocks.len() > 50 || value.to_string().len() > 32768 {
        return Err(AppError::BadRequest("Block document too large".into()));
    }
    let mut ids = std::collections::HashSet::new();
    for block in blocks {
        let (id, label) = match block {
            Block::Card { card } => {
                card.encode()?;
                continue;
            }
            Block::Text { text } => {
                if text.chars().count() > 4000 {
                    return Err(AppError::BadRequest("Block text too long".into()));
                }
                continue;
            }
            Block::Button {
                action_id, label, ..
            }
            | Block::Input {
                action_id, label, ..
            } => (action_id, label),
            Block::Select {
                action_id,
                label,
                options,
            } => {
                if options.is_empty()
                    || options.len() > 100
                    || options.iter().any(|s| s.chars().count() > 200)
                {
                    return Err(AppError::BadRequest("Invalid select options".into()));
                }
                (action_id, label)
            }
        };
        if id.is_empty()
            || id.len() > 80
            || !ids.insert(id)
            || label.is_empty()
            || label.chars().count() > 200
        {
            return Err(AppError::BadRequest(
                "Invalid action identifier or label".into(),
            ));
        }
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    kind: String,
    user_id: String,
    channel_id: Option<String>,
    #[serde(default)]
    trigger_id: Option<String>,
    blocks: Value,
}
async fn publish(
    State(s): State<AppState>,
    p: Principal,
    Json(b): Json<Publish>,
) -> Result<Json<Value>, AppError> {
    p.require("ui:write")?;
    validate_blocks(&b.blocks)?;
    if !["home", "modal", "ephemeral"].contains(&b.kind.as_str()) {
        return Err(AppError::BadRequest("Invalid surface kind".into()));
    }
    if p.delegated && b.user_id != p.user_id {
        return Err(AppError::Forbidden(
            "Cannot publish private UI for another user".into(),
        ));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    p.authorize_locked(&mut tx, "ui:write").await?;
    if let Some(channel) = &b.channel_id {
        p.channel(&mut tx, channel, true).await?;
        one(&mut tx,"SELECT json_build_object('id',user_id)::text FROM channel_user WHERE channel_id=$1 AND user_id=$2",&[channel,&b.user_id]).await?;
    }
    if b.kind == "ephemeral" && b.channel_id.is_none() {
        return Err(AppError::BadRequest(
            "Ephemeral messages need a channel".into(),
        ));
    }
    if b.kind == "modal" {
        let trigger = b
            .trigger_id
            .as_deref()
            .ok_or_else(|| AppError::BadRequest("A trigger is required".into()))?;
        one(&mut tx,"DELETE FROM app_ui WHERE id=$1 AND installation_id=$2 AND user_id=$3 AND kind='trigger' AND expires_at>now() AND channel_id IS NOT DISTINCT FROM NULLIF($4,'') RETURNING '{}'::text",&[trigger,&p.installation_id,&b.user_id,b.channel_id.as_deref().unwrap_or("")]).await?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let sql = if b.kind == "home" {
        "INSERT INTO app_ui(id,installation_id,user_id,kind,document) VALUES($1,$2,$3,$4,$5) ON CONFLICT(installation_id,user_id) WHERE kind='home' DO UPDATE SET document=EXCLUDED.document,updated_at=now() RETURNING json_build_object('id',id)::text"
    } else {
        "INSERT INTO app_ui(id,installation_id,user_id,kind,document,channel_id,expires_at) VALUES($1,$2,$3,$4,$5,NULLIF($6,''),now()+interval '15 minutes') RETURNING json_build_object('id',id)::text"
    };
    let document = json!({"blocks":b.blocks}).to_string();
    let params = if b.kind == "home" {
        vec![
            id.as_str(),
            p.installation_id.as_str(),
            b.user_id.as_str(),
            b.kind.as_str(),
            document.as_str(),
        ]
    } else {
        vec![
            id.as_str(),
            p.installation_id.as_str(),
            b.user_id.as_str(),
            b.kind.as_str(),
            document.as_str(),
            b.channel_id.as_deref().unwrap_or(""),
        ]
    };
    let v = one(&mut tx, sql, &params).await?;
    tx.commit().await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invoke {
    kind: String,
    channel_id: Option<String>,
    name: Option<String>,
    surface_id: Option<String>,
    message_id: Option<String>,
    action_id: Option<String>,
    #[serde(default)]
    values: Value,
}
async fn invoke(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
    Json(b): Json<Invoke>,
) -> Result<Json<Value>, AppError> {
    if b.values.to_string().len() > 32768 {
        return Err(AppError::BadRequest("Input too large".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    lock_app_installation(&mut tx, &id).await?;
    let app=one(&mut tx,"SELECT json_build_object('installation',i.id,'bot',a.bot_user_id,'manifest',i.manifest::jsonb)::text FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE a.id=$1 AND a.status='installed' AND i.active",&[&id]).await?;
    if let Some(channel) = &b.channel_id {
        one(&mut tx,"SELECT '{}'::text FROM channel_user me JOIN channel_user bot ON bot.channel_id=me.channel_id JOIN channels c ON c.id=me.channel_id WHERE me.channel_id=$1 AND me.user_id=$2 AND bot.user_id=$3 AND c.deleted_at IS NULL AND c.archive_at IS NULL",&[channel,&actor.id,app["bot"].as_str().unwrap()]).await?;
    }
    let manifest: Manifest = serde_json::from_value(app["manifest"].clone())
        .map_err(|e| AppError::Internal(e.into()))?;
    let event_type = match b.kind.as_str() {
        "command" => {
            if b.channel_id.is_none()
                || !manifest
                    .commands
                    .contains(&b.name.clone().unwrap_or_default())
                || !manifest.scopes.iter().any(|s| s == "commands:read")
            {
                return Err(AppError::Forbidden("Command is not available".into()));
            }
            "command.invoked"
        }
        "shortcut" => {
            let names = if b.message_id.is_some() {
                &manifest.message_shortcuts
            } else {
                &manifest.shortcuts
            };
            if !names.contains(&b.name.clone().unwrap_or_default()) {
                return Err(AppError::Forbidden("Shortcut is not registered".into()));
            }
            if let Some(message) = &b.message_id {
                if !manifest.scopes.iter().any(|scope| scope == "messages:read") {
                    return Err(AppError::Forbidden(
                        "Message shortcuts require messages:read".into(),
                    ));
                }
                let channel = b.channel_id.as_deref().ok_or_else(|| {
                    AppError::BadRequest("Message shortcuts need channel_id".into())
                })?;
                one(&mut tx,"SELECT '{}'::text FROM messages WHERE id=$1 AND channel_id=$2 AND deleted_at IS NULL",&[message,channel]).await?;
            }
            "shortcut.invoked"
        }
        "home" => "home.opened",
        "action" | "submit" => {
            let document = if let Some(surface) = &b.surface_id {
                one(&mut tx,"SELECT json_build_object('blocks',document::jsonb->'blocks')::text FROM app_ui WHERE id=$1 AND installation_id=$2 AND user_id=$3 AND (expires_at IS NULL OR expires_at>now()) AND kind IN('home','modal','ephemeral') AND channel_id IS NOT DISTINCT FROM NULLIF($4,'') FOR UPDATE",&[surface,app["installation"].as_str().unwrap(),&actor.id,b.channel_id.as_deref().unwrap_or("")]).await?
            } else if let Some(message) = &b.message_id {
                let channel = b
                    .channel_id
                    .as_deref()
                    .ok_or_else(|| AppError::BadRequest("channel_id required".into()))?;
                one(&mut tx,"SELECT json_build_object('blocks',app_blocks::jsonb)::text FROM messages WHERE id=$1 AND channel_id=$2 AND app_origin_id=$3 AND deleted_at IS NULL AND app_blocks IS NOT NULL",&[message,channel,&id]).await?
            } else {
                return Err(AppError::BadRequest(
                    "A surface or message is required".into(),
                ));
            };
            let parsed: Vec<Block> = serde_json::from_value(document["blocks"].clone())
                .map_err(|e| AppError::Internal(e.into()))?;
            if b.kind == "action"
                && !parsed.iter().any(|block| match block {
                    Block::Button { action_id, .. } | Block::Select { action_id, .. } => {
                        Some(action_id) == b.action_id.as_ref()
                    }
                    _ => false,
                })
            {
                return Err(AppError::BadRequest("Unknown action".into()));
            }
            if b.kind == "submit" {
                for block in &parsed {
                    match block {
                        Block::Input {
                            action_id,
                            required: true,
                            ..
                        } => {
                            if b.values[action_id]
                                .as_str()
                                .is_none_or(|s| s.trim().is_empty())
                            {
                                return Err(AppError::BadRequest(
                                    "Required input is missing".into(),
                                ));
                            }
                        }
                        Block::Select {
                            action_id, options, ..
                        } => {
                            if let Some(value) = b.values[action_id].as_str()
                                && !options.iter().any(|s| s == value)
                            {
                                return Err(AppError::BadRequest("Invalid selection".into()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if b.kind == "submit" {
                "view.submitted"
            } else {
                "action.invoked"
            }
        }
        _ => return Err(AppError::BadRequest("Unknown invocation".into())),
    };
    let allowed = one(
        &mut tx,
        "SELECT json_build_object('ok',app_event_matches($1,$2,NULLIF($3,''),$4,$5))::text",
        &[
            &app["manifest"].to_string(),
            event_type,
            b.channel_id.as_deref().unwrap_or(""),
            &actor.id,
            app["bot"].as_str().unwrap(),
        ],
    )
    .await?;
    if allowed["ok"] != true {
        return Err(AppError::Forbidden(
            "App does not subscribe to this interaction".into(),
        ));
    }
    if matches!(b.kind.as_str(), "home" | "action" | "submit")
        && !manifest.scopes.iter().any(|s| s == "ui:write")
    {
        return Err(AppError::Forbidden("App has no ui:write capability".into()));
    }
    let trigger = secret("zoo_trigger_");
    let installation = app["installation"].as_str().unwrap();
    toasty::sql::statement("INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,expires_at) VALUES($1,$2,$3,NULLIF($4,''),'trigger',now()+interval '3 seconds')").bind(trigger.as_str()).bind(installation).bind(actor.id.as_str()).bind(b.channel_id.as_deref().unwrap_or("")).exec(&mut tx).await?;
    let response_token = secret("zoo_response_");
    toasty::sql::statement("INSERT INTO app_responses(token_hash,installation_id,user_id,channel_id,expires_at) VALUES($1,$2,$3,NULLIF($4,''),now()+interval '15 minutes')").bind(hash(&response_token)).bind(installation).bind(actor.id.as_str()).bind(b.channel_id.as_deref().unwrap_or("")).exec(&mut tx).await?;
    let event_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({"id":event_id,"version":1,"type":event_type,"actor_id":actor.id,"channel_id":b.channel_id,"occurred_at":jiff::Timestamp::now().to_string(),"data":{"name":b.name,"surface_id":b.surface_id,"message_id":b.message_id,"action_id":b.action_id,"values":b.values,"trigger_id":trigger,"response_path":format!("/api/app-responses/{response_token}")}});
    toasty::sql::statement("INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload) VALUES($1,$2,$3,NULLIF($4,''),$5,$6)").bind(uuid::Uuid::new_v4().to_string()).bind(event_id.as_str()).bind(installation).bind(b.channel_id.as_deref().unwrap_or("")).bind(event_type).bind(payload.to_string()).exec(&mut tx).await?;
    if b.kind == "submit"
        && let Some(surface) = &b.surface_id
    {
        toasty::sql::statement("DELETE FROM app_ui WHERE id=$1 AND user_id=$2 AND kind='modal'")
            .bind(surface.as_str())
            .bind(actor.id.as_str())
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"event_id":event_id})))
}
async fn surfaces(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT json_build_object('id',u.id,'kind',kind,'channel_id',u.channel_id,'document',document::jsonb,'updated_at',u.updated_at)::text FROM app_ui u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE a.id=$1 AND a.status='installed' AND i.active AND i.scopes::jsonb ? 'ui:write' AND u.user_id=$2 AND kind<>'trigger' AND (expires_at IS NULL OR expires_at>now()) AND (u.channel_id IS NULL OR EXISTS(SELECT 1 FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.channel_id=u.channel_id AND cu.user_id=$2 AND c.deleted_at IS NULL)) ORDER BY u.updated_at",&[&id,&actor.id]).await?;
    Ok(Json(json!(rows)))
}
async fn blocks(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, AppError> {
    p.require("messages:write")?;
    validate_blocks(&b)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let m = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
    p.channel(&mut tx, &m.channel_id, true).await?;
    if m.user_id != p.user_id || m.deleted_at.is_some() {
        return Err(AppError::Forbidden("Only the author may set blocks".into()));
    }
    toasty::sql::statement(
        "UPDATE messages SET app_blocks=$2,app_origin_id=$3,updated_at=CURRENT_TIMESTAMP WHERE id=$1",
    )
    .bind(id.as_str())
    .bind(b.to_string())
    .bind(p.app_id.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}
#[cfg(test)]
mod tests {
    #[test]
    fn validates_read_only_card_blocks() {
        let card = serde_json::json!([{"type":"card","card":{
            "title":"Deployment", "color":"#2da44e", "fields":[
                {"title":"Commit", "value":"abc1234", "short":true, "code":true,
                 "url":"https://github.com/Org/Repo/commit/abc1234"}
            ]
        }}]);
        assert!(super::validate_blocks(&card).is_ok());
        let mut unsafe_card = card.clone();
        unsafe_card[0]["card"]["fields"][0]["url"] = serde_json::json!("javascript:alert(1)");
        assert!(super::validate_blocks(&unsafe_card).is_err());
        let mut too_many = card;
        too_many[0]["card"]["fields"] =
            serde_json::json!(vec![serde_json::json!({"title":"x","value":"v"}); 11]);
        assert!(super::validate_blocks(&too_many).is_err());
    }
    use super::*;
    #[test]
    fn validates_actions_and_limits() {
        assert!(validate_blocks(&json!([{ "type":"button","action_id":"x","label":"OK"}])).is_ok());
        assert!(validate_blocks(&json!([{ "type":"html","html":"<script>"}])).is_err());
        assert!(validate_blocks(&json!([{ "type":"button","action_id":"x","label":"OK"},{ "type":"input","action_id":"x","label":"duplicate"}])).is_err());
    }
}

#[derive(Deserialize)]
struct CommandsQuery {
    channel_id: String,
}
async fn commands(
    State(s): State<AppState>,
    actor: AuthUser,
    Query(q): Query<CommandsQuery>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    // Use approved installations and the same membership/scope checks as invocation.
    let rows = query(&mut tx, "SELECT json_build_object('app_id',a.id,'app_name',a.name,'name',cmd.name)::text FROM apps a JOIN app_installations i ON i.app_id=a.id JOIN channel_user bot ON bot.user_id=a.bot_user_id JOIN channel_user me ON me.channel_id=bot.channel_id CROSS JOIN LATERAL jsonb_array_elements_text(COALESCE(i.manifest::jsonb->'commands','[]'::jsonb)) cmd(name) WHERE bot.channel_id=$1 AND me.user_id=$2 AND a.status='installed' AND i.active AND i.scopes::jsonb ? 'commands:read' ORDER BY cmd.name,a.name,a.id", &[&q.channel_id, &actor.id]).await?;
    Ok(Json(json!(rows)))
}

#[derive(Deserialize)]
struct CommandInput {
    channel_id: String,
    #[serde(default)]
    text: String,
}
async fn command(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(name): Path<String>,
    Json(b): Json<CommandInput>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    let apps=query(&mut tx,"SELECT json_build_object('id',a.id)::text FROM apps a JOIN app_installations i ON i.app_id=a.id JOIN channel_user bot ON bot.user_id=a.bot_user_id JOIN channel_user me ON me.channel_id=bot.channel_id WHERE bot.channel_id=$1 AND me.user_id=$2 AND a.status='installed' AND i.active AND i.manifest::jsonb->'commands' ? $3 AND i.scopes::jsonb ? 'commands:read'",&[&b.channel_id,&actor.id,&name]).await?;
    if apps.len() != 1 {
        return Err(if apps.is_empty() {
            AppError::NotFound("Command not installed in this channel".into())
        } else {
            AppError::Conflict(
                "Multiple apps provide this command; choose an app from Apps & integrations".into(),
            )
        });
    }
    let id = apps[0]["id"].as_str().unwrap().to_owned();
    drop(tx);
    invoke(
        State(s),
        actor,
        Path(id),
        Json(Invoke {
            kind: "command".into(),
            channel_id: Some(b.channel_id),
            name: Some(name),
            surface_id: None,
            message_id: None,
            action_id: None,
            values: json!({"text":b.text}),
        }),
    )
    .await
}
#[derive(Deserialize)]
struct Unfurl {
    url: String,
    card: crate::cards::MessageCard,
}
async fn unfurl(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<String>,
    Json(b): Json<Unfurl>,
) -> Result<Json<Value>, AppError> {
    p.require("links:write")?;
    let url =
        reqwest::Url::parse(&b.url).map_err(|_| AppError::BadRequest("Invalid link".into()))?;
    if url.scheme() != "https"
        || !p
            .manifest
            .link_domains
            .iter()
            .any(|d| Some(d.as_str()) == url.host_str())
    {
        return Err(AppError::Forbidden("Link domain is not registered".into()));
    }
    let card = b.card.encode()?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let message = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
    p.channel(&mut tx, &message.channel_id, true).await?;
    if message.deleted_at.is_some() || !message.content.split_whitespace().any(|word| word == b.url)
    {
        return Err(AppError::BadRequest(
            "Link is not present in this message".into(),
        ));
    }
    toasty::sql::statement("INSERT INTO app_link_unfurls(message_id,installation_id,url,card) VALUES($1,$2,$3,$4) ON CONFLICT(message_id,installation_id,url) DO UPDATE SET card=EXCLUDED.card").bind(id.as_str()).bind(p.installation_id).bind(b.url).bind(card).exec(&mut tx).await?;
    toasty::sql::statement("UPDATE messages SET app_unfurls=(SELECT COALESCE(jsonb_agg(jsonb_build_object('app_id',i.app_id,'name',a.name,'url',u.url,'card',u.card::jsonb)),'[]'::jsonb)::text FROM app_link_unfurls u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE u.message_id=$1),updated_at=CURRENT_TIMESTAMP WHERE id=$1").bind(id.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}

pub async fn private_surfaces(
    State(s): State<AppState>,
    actor: AuthUser,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT json_build_object('id',u.id,'kind',u.kind,'app_id',a.id,'name',a.name,'channel_id',u.channel_id,'document',u.document::jsonb)::text FROM app_ui u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE a.status='installed' AND i.active AND i.scopes::jsonb ? 'ui:write' AND u.user_id=$1 AND u.kind IN('modal','ephemeral') AND u.expires_at>now() AND (u.channel_id IS NULL OR EXISTS(SELECT 1 FROM channel_user cu JOIN channel_user bot ON bot.channel_id=cu.channel_id JOIN channels c ON c.id=cu.channel_id WHERE cu.channel_id=u.channel_id AND cu.user_id=$1 AND bot.user_id=a.bot_user_id AND c.deleted_at IS NULL)) ORDER BY u.updated_at LIMIT 30",&[&actor.id]).await?;
    Ok(Json(json!(rows)))
}
async fn dismiss(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "DELETE FROM app_ui WHERE id=$1 AND user_id=$2 AND kind IN('modal','ephemeral')",
    )
    .bind(id)
    .bind(actor.id)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn respond(
    State(s): State<AppState>,
    Path(token): Path<String>,
    Json(blocks): Json<Value>,
) -> Result<Json<Value>, AppError> {
    validate_blocks(&blocks)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let installation=one(&mut tx,"SELECT json_build_object('id',installation_id)::text FROM app_responses WHERE token_hash=$1",&[&hash(&token)]).await?;
    lock_installation(&mut tx, installation["id"].as_str().unwrap()).await?;
    let row=one(&mut tx,"UPDATE app_responses r SET used_at=now() FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE r.token_hash=$1 AND r.installation_id=i.id AND r.used_at IS NULL AND r.expires_at>now() AND i.active AND a.status='installed' AND i.scopes::jsonb ? 'ui:write' AND r.channel_id IS NOT NULL AND EXISTS(SELECT 1 FROM channel_user me JOIN channel_user bot ON bot.channel_id=me.channel_id JOIN channels c ON c.id=me.channel_id WHERE me.channel_id=r.channel_id AND me.user_id=r.user_id AND bot.user_id=a.bot_user_id AND c.deleted_at IS NULL AND c.archive_at IS NULL) RETURNING json_build_object('installation',r.installation_id,'user',r.user_id,'channel',r.channel_id)::text",&[&hash(&token)]).await?;
    let id = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,document,expires_at) VALUES($1,$2,$3,$4,'ephemeral',$5,now()+interval '15 minutes')").bind(id.as_str()).bind(row["installation"].as_str().unwrap()).bind(row["user"].as_str().unwrap()).bind(row["channel"].as_str().unwrap()).bind(json!({"blocks":blocks}).to_string()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}
