use super::*;
use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Path, State},
    middleware,
    routing::{get, post},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps", get(list).post(create))
        .route("/api/apps/{id}", get(detail).patch(update))
        .route("/api/apps/{id}/actions/{action}", post(action))
        .route(
            "/api/apps/{id}/channels/{channel}",
            post(add_channel).delete(remove_channel),
        )
        .route("/api/apps/{id}/deliveries", get(deliveries))
        .route("/api/apps/{id}/records/{kind}", get(records))
        .route(
            "/api/apps/{id}/credentials/{credential}",
            axum::routing::delete(revoke_credential),
        )
        .route(
            "/api/apps/{id}/authorization",
            axum::routing::delete(revoke_consent),
        )
        .route("/api/apps/metrics", get(metrics))
        .route("/api/apps/{id}/deliveries/{delivery}/retry", post(retry))
        .route_layer(middleware::from_fn(
            |request: axum::extract::Request, next: middleware::Next| async move {
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    "cache-control",
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
}
pub async fn attach_webhook(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    owner: &str,
    name: &str,
) -> Result<(), AppError> {
    toasty::sql::statement("INSERT INTO apps(id,owner_id,bot_user_id,name,status,manifest) VALUES($1,$2,$1,$3,'installed','{\"notification_only\":true,\"scopes\":[],\"events\":[]}')")
 .bind(user).bind(owner).bind(name).exec(tx).await?;
    toasty::sql::statement("UPDATE users SET app_id=$1 WHERE id=$1")
        .bind(user)
        .exec(tx)
        .await?;
    toasty::sql::statement("INSERT INTO app_installations(id,app_id,revision,manifest) SELECT id,id,revision,manifest FROM apps WHERE id=$1").bind(user).exec(tx).await?;
    Ok(())
}
async fn list(State(state): State<AppState>, actor: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    let admin =
        crate::auth::rbac::is_workspace_admin(&mut tx, &actor.id, state.config.dev_auth_enabled)
            .await?;
    let values=query(&mut tx,"SELECT json_build_object('id',id,'name',name,'description',description,'icon_url',icon_url,'bot_user_id',bot_user_id,'owner_id',owner_id,'status',status,'revision',revision,'manifest',CASE WHEN owner_id=$1 OR $2::boolean THEN manifest::jsonb ELSE manifest::jsonb - 'event_url' END,'can_manage',owner_id=$1 OR $2::boolean)::text FROM apps WHERE status='installed' OR owner_id=$1 OR $2::boolean ORDER BY name,id",&[&actor.id,if admin {"true"}else{"false"}]).await?;
    Ok(Json(json!(values)))
}
async fn detail(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let mut app=one(&mut tx,"SELECT json_build_object('id',id,'name',name,'description',description,'icon_url',icon_url,'bot_user_id',bot_user_id,'owner_id',owner_id,'status',status,'manifest',manifest::jsonb,'revision',revision)::text FROM apps WHERE id=$1",&[&id]).await?;
    if app["status"] != "installed" && app["status"] != "disabled" {
        can_manage(&mut tx, &state, &actor, &id).await?;
    }
    let manager = app["owner_id"] == actor.id
        || crate::auth::rbac::is_workspace_admin(&mut tx, &actor.id, state.config.dev_auth_enabled)
            .await?;
    if !manager && let Some(manifest) = app["manifest"].as_object_mut() {
        manifest.remove("event_url");
    }
    Ok(Json(app))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppInput {
    name: String,
    bot_username: Option<String>,
    #[serde(default)]
    description: String,
    icon_url: Option<String>,
    #[serde(default)]
    manifest: Manifest,
}
impl AppInput {
    fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.description.chars().count() > 4000
        {
            return Err(AppError::BadRequest(
                "Invalid app name or description".into(),
            ));
        }
        if let Some(url) = &self.icon_url {
            validate_https(url)?;
        }
        if let Some(username) = &self.bot_username {
            crate::validation::validate_user_name(username)?;
        }
        self.manifest.validate()
    }
}
async fn create(
    State(state): State<AppState>,
    actor: AuthUser,
    Json(input): Json<AppInput>,
) -> Result<Json<Value>, AppError> {
    input.validate()?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let bot = uuid::Uuid::new_v4().to_string();
    let username = input.bot_username.clone().unwrap_or_else(|| {
        let slug = crate::auth::sanitize_for_user_name(&input.name, &bot);
        format!(
            "{}_{}",
            slug.chars().take(60).collect::<String>(),
            &bot[..8]
        )
    });
    toasty::sql::statement(
        "INSERT INTO users(id,user_name,display_name,is_bot) VALUES($1,$2,$3,true)",
    )
    .bind(bot.as_str())
    .bind(username)
    .bind(input.name.trim())
    .exec(&mut tx)
    .await?;
    toasty::sql::statement("INSERT INTO apps(id,owner_id,bot_user_id,name,description,icon_url,manifest) VALUES($1,$2,$3,$4,$5,NULLIF($6,''),$7)")
 .bind(id.as_str()).bind(actor.id.as_str()).bind(bot.as_str()).bind(input.name.trim()).bind(input.description.as_str()).bind(input.icon_url.as_deref().unwrap_or("")).bind(serde_json::to_string(&input.manifest).unwrap()).exec(&mut tx).await?;
    toasty::sql::statement("UPDATE users SET app_id=$1,avatar_url=NULLIF($3,'') WHERE id=$2")
        .bind(id.as_str())
        .bind(bot.as_str())
        .bind(input.icon_url.as_deref().unwrap_or(""))
        .exec(&mut tx)
        .await?;
    audit(&mut tx, &id, &actor.id, "created").await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id,"bot_user_id":bot,"status":"draft"})))
}
async fn update(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<AppInput>,
) -> Result<Json<Value>, AppError> {
    input.validate()?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let app = can_manage(&mut tx, &state, &actor, &id).await?;
    let old: Manifest = serde_json::from_str(app["manifest"].as_str().unwrap())
        .map_err(|e| AppError::Internal(e.into()))?;
    let changed = old != input.manifest;
    toasty::sql::statement("UPDATE apps SET name=$2,description=$3,icon_url=NULLIF($4,''),manifest=$5,revision=revision+CASE WHEN $6::boolean THEN 1 ELSE 0 END,status=CASE WHEN $6::boolean THEN 'draft' ELSE status END,updated_at=CURRENT_TIMESTAMP WHERE id=$1")
 .bind(id.as_str()).bind(input.name.trim()).bind(input.description.as_str()).bind(input.icon_url.as_deref().unwrap_or("")).bind(serde_json::to_string(&input.manifest).unwrap()).bind(if changed {"true"}else{"false"}).exec(&mut tx).await?;
    toasty::sql::statement("UPDATE users SET display_name=$2,avatar_url=NULLIF($3,''),user_name=COALESCE(NULLIF($4,''),user_name),updated_at=CURRENT_TIMESTAMP WHERE id=$1").bind(app["bot_user_id"].as_str().unwrap()).bind(input.name.trim()).bind(input.icon_url.as_deref().unwrap_or("")).bind(input.bot_username.as_deref().unwrap_or("")).exec(&mut tx).await?;
    audit(&mut tx, &id, &actor.id, "updated").await?;
    tx.commit().await?;
    Ok(Json(
        json!({"id":id,"status":if changed {json!("draft")}else{app["status"].clone()}}),
    ))
}
async fn action(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((id, action)): Path<(String, String)>,
    body: Option<Json<Value>>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let app = can_manage(&mut tx, &state, &actor, &id).await?;
    let admin =
        crate::auth::rbac::is_workspace_admin(&mut tx, &actor.id, state.config.dev_auth_enabled)
            .await?;
    if app["deleted_at"].as_str().is_some() {
        return Err(AppError::NotFound("App was deleted".into()));
    }
    let result=match action.as_str(){
 "submit"=>{toasty::sql::statement("UPDATE apps SET status='pending' WHERE id=$1").bind(id.as_str()).exec(&mut tx).await?;json!({"status":"pending"})},
 "install"=>{
  if !admin {return Err(AppError::Forbidden("Administrator approval required".into()));}
  if body.as_ref().and_then(|b|b.0["revision"].as_i64())!=app["revision"].as_i64() {return Err(AppError::Conflict("App configuration changed; review the current revision before approval".into()));}
  if app["status"]!="pending" {return Err(AppError::BadRequest("Submit the app for approval first".into()));}
  let manifest:Manifest=serde_json::from_str(app["manifest"].as_str().unwrap()).map_err(|e|AppError::Internal(e.into()))?;manifest.validate()?;
  let installation=uuid::Uuid::new_v4().to_string();
  toasty::sql::statement("INSERT INTO app_installations(id,app_id,approved_by,scopes,manifest,revision) SELECT $2,id,$3,(manifest::jsonb->'scopes')::text,manifest,revision FROM apps WHERE id=$1 ON CONFLICT(app_id) DO UPDATE SET active=true,approved_by=EXCLUDED.approved_by,scopes=EXCLUDED.scopes,manifest=EXCLUDED.manifest,revision=EXCLUDED.revision")
  .bind(id.as_str()).bind(installation).bind(actor.id.as_str()).exec(&mut tx).await?;
  toasty::sql::statement("UPDATE app_credentials SET revoked_at=now() WHERE installation_id IN(SELECT id FROM app_installations WHERE app_id=$1)").bind(id.as_str()).exec(&mut tx).await?;
  toasty::sql::statement("UPDATE app_oauth_codes SET used_at=now() WHERE installation_id IN(SELECT id FROM app_installations WHERE app_id=$1)").bind(id.as_str()).exec(&mut tx).await?;
  toasty::sql::statement("UPDATE app_deliveries SET expires_at=now(),lease_token=NULL,last_error='App authorization changed' WHERE installation_id IN(SELECT id FROM app_installations WHERE app_id=$1) AND acked_at IS NULL").bind(id.as_str()).exec(&mut tx).await?;
  toasty::sql::statement("UPDATE apps SET status='installed' WHERE id=$1").bind(id.as_str()).exec(&mut tx).await?;
  json!({"status":"installed"})
 },
 "delete" | "disable"=>{
  if action=="delete" {toasty::sql::statement("UPDATE apps SET deleted_at=now() WHERE id=$1").bind(id.as_str()).exec(&mut tx).await?;}
  toasty::sql::statement("UPDATE apps SET status='disabled' WHERE id=$1").bind(id.as_str()).exec(&mut tx).await?;
  toasty::sql::statement("UPDATE app_installations SET active=false WHERE app_id=$1").bind(id.as_str()).exec(&mut tx).await?;
  toasty::sql::statement("UPDATE app_credentials SET revoked_at=now() WHERE installation_id IN(SELECT id FROM app_installations WHERE app_id=$1)").bind(id.as_str()).exec(&mut tx).await?;
  toasty::sql::statement("DELETE FROM channel_user WHERE user_id=$1").bind(app["bot_user_id"].as_str().unwrap()).exec(&mut tx).await?;
  json!({"status":"disabled"})
 },
 "rotate-signing-secret"=>{
  one(&mut tx,"UPDATE app_installations SET signing_secret=$2 WHERE app_id=$1 AND active RETURNING json_build_object('signing_secret',signing_secret)::text",&[&id,&secret("zoo_sign_")]).await?
 },
 "verify-endpoint"=>{
  let connection=one(&mut tx,"SELECT json_build_object('url',manifest::jsonb->>'event_url','signing',signing_secret)::text FROM app_installations WHERE app_id=$1 AND active AND manifest::jsonb->>'transport'='http'",&[&id]).await?;
  super::delivery::verify_endpoint(connection["url"].as_str().unwrap_or(""),connection["signing"].as_str().unwrap()).await?;
  one(&mut tx,"UPDATE app_installations SET endpoint_verified_at=now() WHERE app_id=$1 RETURNING json_build_object('verified_at',endpoint_verified_at)::text",&[&id]).await?
 },
 "connection"=>{
  one(&mut tx,"SELECT json_build_object('signing_secret',signing_secret)::text FROM app_installations WHERE app_id=$1 AND active",&[&id]).await?
 },
 "token"=>{
  let i=one(&mut tx,"SELECT json_build_object('id',i.id,'scopes',i.scopes::jsonb)::text FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE i.app_id=$1 AND i.active AND a.status='installed'",&[&id]).await?;
  mint(&mut tx,i["id"].as_str().unwrap(),None,&i["scopes"]).await?
 },
 _=>return Err(AppError::NotFound("Unknown app action".into()))
 };
    audit(&mut tx, &id, &actor.id, &action).await?;
    tx.commit().await?;
    Ok(Json(result))
}
async fn add_channel(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((id, channel)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    let a=one(&mut tx,"SELECT json_build_object('bot',bot_user_id,'manifest',manifest::jsonb)::text FROM apps WHERE id=$1 AND status='installed'",&[&id]).await?;
    if a["manifest"]["notification_only"] == true {
        return Err(AppError::BadRequest(
            "Manage notification-only apps through webhook settings".into(),
        ));
    }
    let c = crate::models::Channel::get_by_id(&mut tx, channel.as_str()).await?;
    let admin =
        crate::auth::rbac::is_workspace_admin(&mut tx, &actor.id, state.config.dev_auth_enabled)
            .await?;
    if c.is_dm || c.deleted_at.is_some() || c.archive_at.is_some() {
        return Err(AppError::BadRequest("Unavailable channel".into()));
    }
    let member = crate::models::ChannelUser::filter(
        crate::models::ChannelUser::fields()
            .channel_id()
            .eq(channel.as_str())
            .and(
                crate::models::ChannelUser::fields()
                    .user_id()
                    .eq(actor.id.as_str()),
            ),
    )
    .first()
    .exec(&mut tx)
    .await?
    .is_some();
    if (c.is_private && !member)
        || !crate::auth::rbac::can_add_member(
            c.is_private,
            &c.created_user_id,
            &actor.id,
            member,
            admin,
            false,
        )
    {
        return Err(AppError::Forbidden(
            "Cannot add an app to this channel".into(),
        ));
    }
    toasty::sql::statement("INSERT INTO channel_user(channel_id,user_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP) ON CONFLICT DO NOTHING").bind(channel.as_str()).bind(a["bot"].as_str().unwrap()).exec(&mut tx).await?;
    audit(&mut tx, &id, &actor.id, "channel.added").await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn remove_channel(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((id, channel)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    let c = crate::models::Channel::get_by_id(&mut tx, channel.as_str()).await?;
    let admin =
        crate::auth::rbac::is_workspace_admin(&mut tx, &actor.id, state.config.dev_auth_enabled)
            .await?;
    if !crate::auth::rbac::can_manage_channel(&c.created_user_id, &actor.id, admin, c.is_dm) {
        return Err(AppError::Forbidden("Channel manager required".into()));
    }
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1 AND user_id IN(SELECT bot_user_id FROM apps WHERE id=$2)").bind(channel.as_str()).bind(id.as_str()).exec(&mut tx).await?;
    audit(&mut tx, &id, &actor.id, "channel.removed").await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn deliveries(
    State(state): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    can_manage(&mut tx, &state, &actor, &id).await?;
    let rows=query(&mut tx,"SELECT json_build_object('id',d.id,'event_id',event_id,'type',event_type,'attempts',attempts,'acked_at',acked_at,'last_error',last_error,'created_at',d.created_at)::text FROM app_deliveries d JOIN app_installations i ON i.id=d.installation_id WHERE i.app_id=$1 ORDER BY d.created_at DESC LIMIT 100",&[&id]).await?;
    Ok(Json(json!(rows)))
}
async fn retry(
    State(state): State<AppState>,
    actor: AuthUser,
    Path((id, delivery)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    can_manage(&mut tx, &state, &actor, &id).await?;
    let row=one(&mut tx,"UPDATE app_deliveries SET available_at=now(),expires_at=now()+interval '24 hours',lease_until=NULL,lease_token=NULL,last_error=NULL WHERE id=$1 AND installation_id IN(SELECT id FROM app_installations WHERE app_id=$2 AND active) AND acked_at IS NULL RETURNING json_build_object('id',id)::text",&[&delivery,&id]).await?;
    audit(&mut tx, &id, &actor.id, "delivery.retried").await?;
    tx.commit().await?;
    Ok(Json(row))
}

async fn records(
    State(s): State<AppState>,
    actor: AuthUser,
    Path((id, kind)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    can_manage(&mut tx, &s, &actor, &id).await?;
    let sql = match kind.as_str() {
        "versions" => {
            "SELECT json_build_object('revision',revision,'manifest',manifest::jsonb,'created_at',created_at)::text FROM app_versions WHERE app_id=$1 ORDER BY revision DESC LIMIT 100"
        }
        "audit" => {
            "SELECT row_to_json(l)::text FROM app_audit_log l WHERE app_id=$1 ORDER BY created_at DESC LIMIT 100"
        }
        "credentials" => {
            "SELECT json_build_object('id',t.id,'user_id',user_id,'scopes',t.scopes::jsonb,'expires_at',expires_at,'revoked_at',revoked_at)::text FROM app_credentials t JOIN app_installations i ON i.id=t.installation_id WHERE i.app_id=$1 ORDER BY t.created_at DESC LIMIT 100"
        }
        _ => return Err(AppError::NotFound("Unknown app records".into())),
    };
    Ok(Json(json!(query(&mut tx, sql, &[&id]).await?)))
}
async fn revoke_credential(
    State(s): State<AppState>,
    actor: AuthUser,
    Path((id, credential)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    can_manage(&mut tx, &s, &actor, &id).await?;
    one(&mut tx,"UPDATE app_credentials SET revoked_at=now() WHERE id=$1 AND installation_id IN(SELECT id FROM app_installations WHERE app_id=$2) RETURNING '{}'::text",&[&credential,&id]).await?;
    audit(&mut tx, &id, &actor.id, "credential.revoked").await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn revoke_consent(
    State(s): State<AppState>,
    actor: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    human(&mut tx, &actor).await?;
    toasty::sql::statement("UPDATE app_credentials SET revoked_at=now() WHERE user_id=$1 AND installation_id IN(SELECT id FROM app_installations WHERE app_id=$2)").bind(actor.id.as_str()).bind(id.as_str()).exec(&mut tx).await?;
    toasty::sql::statement("UPDATE app_oauth_codes SET used_at=now() WHERE user_id=$1 AND installation_id IN(SELECT id FROM app_installations WHERE app_id=$2)").bind(actor.id.as_str()).bind(id.as_str()).exec(&mut tx).await?;
    audit(&mut tx, &id, &actor.id, "user.consent_revoked").await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn metrics(State(s): State<AppState>, actor: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    if !crate::auth::rbac::is_workspace_admin(&mut tx, &actor.id, s.config.dev_auth_enabled).await?
    {
        return Err(AppError::Forbidden("Administrator required".into()));
    }
    let v=one(&mut tx,"SELECT json_build_object('pending',count(*) FILTER(WHERE acked_at IS NULL AND expires_at>now()),'failed',count(*) FILTER(WHERE acked_at IS NULL AND expires_at<=now()),'delivered',count(*) FILTER(WHERE acked_at IS NOT NULL),'retried',count(*) FILTER(WHERE attempts>1),'oldest_pending_seconds',COALESCE(extract(epoch FROM now()-min(created_at) FILTER(WHERE acked_at IS NULL AND expires_at>now())),0))::text FROM app_deliveries",&[]).await?;
    Ok(Json(v))
}
