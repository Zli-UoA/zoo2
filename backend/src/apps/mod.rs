//! External applications. Private platform tables never enter Electric shapes.
pub mod api;
pub mod delivery;
pub mod files;
pub mod management;
pub mod oauth;
pub mod ui;

use crate::{auth::AuthUser, error::AppError, state::AppState};
use axum::{extract::FromRequestParts, http::request::Parts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const SCOPES: &[&str] = &[
    "messages:read",
    "messages:write",
    "reactions:write",
    "channels:read",
    "channels:manage",
    "users:read",
    "dm:write",
    "files:read",
    "files:write",
    "ui:write",
    "commands:read",
    "links:write",
];
pub const EVENTS: &[&str] = &[
    "message.created",
    "message.updated",
    "message.deleted",
    "app.mentioned",
    "reaction.added",
    "reaction.removed",
    "member.joined",
    "member.left",
    "channel.created",
    "channel.updated",
    "channel.deleted",
    "file.created",
    "app.installed",
    "app.uninstalled",
    "command.invoked",
    "action.invoked",
    "view.submitted",
    "shortcut.invoked",
    "link.shared",
    "home.opened",
];

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub notification_only: bool,
    #[serde(default)]
    pub include_bot_events: bool,
    #[serde(default)]
    pub dm_enabled: bool,
    #[serde(default)]
    pub transport: Transport,
    pub event_url: Option<String>,
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub shortcuts: Vec<String>,
    #[serde(default)]
    pub message_shortcuts: Vec<String>,
    #[serde(default)]
    pub link_domains: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Http,
    #[default]
    Websocket,
}
impl Manifest {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.scopes.iter().any(|s| !SCOPES.contains(&s.as_str()))
            || self.events.iter().any(|s| !EVENTS.contains(&s.as_str()))
        {
            return Err(AppError::BadRequest("Unknown scope or event".into()));
        }
        if self.transport == Transport::Http {
            validate_https(self.event_url.as_deref().unwrap_or(""))?;
        }
        for uri in &self.redirect_uris {
            validate_https(uri)?;
        }
        if self.commands.len() > 50
            || self.shortcuts.len() > 50
            || self.message_shortcuts.len() > 50
            || self.link_domains.len() > 50
            || self.redirect_uris.len() > 20
        {
            return Err(AppError::BadRequest("Manifest collection too large".into()));
        }
        if self.commands.iter().any(|s| {
            s.is_empty()
                || s.len() > 48
                || !s
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        }) {
            return Err(AppError::BadRequest("Invalid command name".into()));
        }
        if self
            .events
            .iter()
            .any(|s| s.starts_with("message.") || s == "app.mentioned")
            && !self.scopes.iter().any(|s| s == "messages:read")
        {
            return Err(AppError::BadRequest(
                "Message events require messages:read".into(),
            ));
        }
        for domain in &self.link_domains {
            if domain.len() > 253
                || !domain.contains('.')
                || !domain
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
            {
                return Err(AppError::BadRequest("Invalid link domain".into()));
            }
        }
        if !self.message_shortcuts.is_empty() && !self.scopes.iter().any(|s| s == "messages:read") {
            return Err(AppError::BadRequest(
                "Message shortcuts require messages:read".into(),
            ));
        }
        if self.dm_enabled
            && (!self.scopes.iter().any(|s| s == "messages:read")
                || !self.scopes.iter().any(|s| s == "dm:write"))
        {
            return Err(AppError::BadRequest(
                "DMs require messages:read and dm:write".into(),
            ));
        }
        if self.dm_enabled && self.notification_only {
            return Err(AppError::BadRequest(
                "Notification-only apps cannot enable DMs".into(),
            ));
        }
        Ok(())
    }
}
pub fn validate_https(value: &str) -> Result<(), AppError> {
    let url =
        reqwest::Url::parse(value).map_err(|_| AppError::BadRequest("Invalid HTTPS URL".into()))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::BadRequest(
            "An HTTPS URL without credentials or fragment is required".into(),
        ));
    }
    Ok(())
}
pub fn secret(prefix: &str) -> String {
    format!(
        "{prefix}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Return one JSON text column, keeping all SQL parameters bound.
pub async fn query(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<Vec<Value>, AppError> {
    let mut q = toasty::sql::query(sql);
    for value in params {
        q = q.bind(*value);
    }
    let rows = q.exec(tx).await?;
    rows.into_iter()
        .map(|row| {
            if let toasty::stmt::Value::Record(r) = row
                && let Some(toasty::stmt::Value::String(s)) = r.fields.into_iter().next()
            {
                return serde_json::from_str(&s).map_err(|e| AppError::Internal(e.into()));
            }
            Err(AppError::Internal(anyhow::anyhow!(
                "Invalid platform query result"
            )))
        })
        .collect()
}
pub async fn one(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<Value, AppError> {
    query(tx, sql, params)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("App resource not found".into()))
}
pub async fn audit(
    tx: &mut toasty::Transaction<'_>,
    app: &str,
    actor: &str,
    action: &str,
) -> Result<(), AppError> {
    toasty::sql::statement(
        "INSERT INTO app_audit_log(id,app_id,actor_id,action) VALUES($1,$2,$3,$4)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(app)
    .bind(actor)
    .bind(action)
    .exec(tx)
    .await?;
    Ok(())
}
pub async fn human(tx: &mut toasty::Transaction<'_>, actor: &AuthUser) -> Result<(), AppError> {
    let u = crate::models::User::get_by_id(tx, actor.id.as_str()).await?;
    if u.is_bot {
        return Err(AppError::Forbidden("A human account is required".into()));
    }
    Ok(())
}
pub async fn can_manage(
    tx: &mut toasty::Transaction<'_>,
    state: &AppState,
    actor: &AuthUser,
    id: &str,
) -> Result<Value, AppError> {
    human(tx, actor).await?;
    let app = one(
        tx,
        "SELECT row_to_json(a)::text FROM apps a WHERE id=$1 FOR UPDATE",
        &[id],
    )
    .await?;
    if app["owner_id"] != actor.id
        && !crate::auth::rbac::is_workspace_admin(tx, &actor.id, state.config.dev_auth_enabled)
            .await?
    {
        return Err(AppError::Forbidden(
            "Only the developer or an administrator can manage this app".into(),
        ));
    }
    Ok(app)
}

#[derive(Clone, Debug)]
pub struct Principal {
    pub credential_id: String,
    pub installation_id: String,
    pub app_id: String,
    pub user_id: String,
    pub bot_user_id: String,
    pub scopes: Vec<String>,
    pub manifest: Manifest,
    pub delegated: bool,
}
impl Principal {
    pub fn require(&self, scope: &str) -> Result<(), AppError> {
        if self.scopes.iter().any(|s| s == scope) {
            Ok(())
        } else {
            Err(AppError::Forbidden(format!("Missing scope: {scope}")))
        }
    }
    pub fn actor(&self) -> AuthUser {
        AuthUser {
            id: self.user_id.clone(),
            user_name: String::new(),
            display_name: String::new(),
        }
    }
    pub async fn channel(
        &self,
        tx: &mut toasty::Transaction<'_>,
        id: &str,
        write: bool,
    ) -> Result<(), AppError> {
        let value=one(tx,"SELECT json_build_object('allowed',EXISTS(SELECT 1 FROM channels c JOIN channel_user cu ON cu.channel_id=c.id WHERE c.id=$1 AND c.deleted_at IS NULL AND (NOT $3::boolean OR c.archive_at IS NULL) AND cu.user_id=$2))::text",&[id,&self.user_id,if write {"true"} else {"false"}]).await?;
        if value["allowed"] != true {
            return Err(AppError::NotFound("Channel not found".into()));
        }
        Ok(())
    }
}
impl FromRequestParts<AppState> for Principal {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .filter(|s| s.starts_with("zoo_at_"))
            .ok_or_else(|| AppError::Unauthorized("An app access token is required".into()))?;
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        let rows=query(&mut tx,"SELECT json_build_object('credential_id',t.id,'installation_id',i.id,'app_id',a.id,'user_id',COALESCE(t.user_id,a.bot_user_id),'bot_user_id',a.bot_user_id,'scopes',t.scopes::jsonb,'manifest',i.manifest::jsonb,'delegated',t.user_id IS NOT NULL)::text FROM app_credentials t JOIN app_installations i ON i.id=t.installation_id JOIN apps a ON a.id=i.app_id WHERE t.token_hash=$1 AND t.revoked_at IS NULL AND t.expires_at>now() AND i.active AND a.status='installed'",&[&hash(token)]).await?;
        let v = rows
            .first()
            .ok_or_else(|| AppError::Unauthorized("Invalid or expired app token".into()))?;
        let count=one(&mut tx,"INSERT INTO app_rate_limits(installation_id,window_start,requests) VALUES($1,date_trunc('minute',now()),1) ON CONFLICT(installation_id,window_start) DO UPDATE SET requests=app_rate_limits.requests+1 RETURNING json_build_object('requests',requests)::text",&[v["installation_id"].as_str().unwrap()]).await?;
        tx.commit().await?;
        if count["requests"].as_u64().unwrap_or(0) > 120 {
            return Err(AppError::RateLimited);
        }
        Ok(Self {
            credential_id: v["credential_id"].as_str().unwrap().into(),
            installation_id: v["installation_id"].as_str().unwrap().into(),
            app_id: v["app_id"].as_str().unwrap().into(),
            user_id: v["user_id"].as_str().unwrap().into(),
            bot_user_id: v["bot_user_id"].as_str().unwrap().into(),
            scopes: serde_json::from_value(v["scopes"].clone())
                .map_err(|e| AppError::Internal(e.into()))?,
            manifest: serde_json::from_value(v["manifest"].clone())
                .map_err(|e| AppError::Internal(e.into()))?,
            delegated: v["delegated"].as_bool().unwrap_or(false),
        })
    }
}
pub async fn mint(
    tx: &mut toasty::Transaction<'_>,
    installation: &str,
    user: Option<&str>,
    scopes: &Value,
) -> Result<Value, AppError> {
    let access = secret("zoo_at_");
    let refresh = secret("zoo_rt_");
    toasty::sql::statement("INSERT INTO app_credentials(id,installation_id,user_id,token_hash,refresh_hash,scopes,expires_at) VALUES($1,$2,NULLIF($3,''),$4,$5,$6,now()+interval '1 hour')")
        .bind(uuid::Uuid::new_v4().to_string()).bind(installation).bind(user.unwrap_or("")).bind(hash(&access)).bind(hash(&refresh)).bind(scopes.to_string()).exec(tx).await?;
    Ok(
        json!({"access_token":access,"refresh_token":refresh,"expires_in":3600,"token_type":"Bearer","scopes":scopes}),
    )
}

/// Human DM creation checks the approved installation, not merely is_bot.
pub async fn bot_can_chat(db: &mut toasty::Db, user: &str) -> Result<bool, AppError> {
    let mut tx = db.transaction().await?;
    Ok(one(&mut tx,"SELECT json_build_object('ok',EXISTS(SELECT 1 FROM apps a JOIN app_installations i ON i.app_id=a.id WHERE a.bot_user_id=$1 AND a.status='installed' AND i.active AND COALESCE((i.manifest::jsonb->>'dm_enabled')::boolean,false) AND NOT COALESCE((i.manifest::jsonb->>'notification_only')::boolean,false)))::text",&[user]).await?["ok"]==true)
}

pub async fn no_store(mut response: axum::response::Response) -> axum::response::Response {
    response.headers_mut().insert(
        "cache-control",
        axum::http::HeaderValue::from_static("private, no-store"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_privileged_configuration() {
        let m: Manifest = serde_json::from_value(json!({"events":["message.created"]})).unwrap();
        assert!(m.validate().is_err());
        let m: Manifest = serde_json::from_value(
            json!({"scopes":["messages:read"],"events":["message.created"]}),
        )
        .unwrap();
        assert!(m.validate().is_ok());
        assert!(validate_https("https://user:pass@example.com").is_err());
        assert!(validate_https("http://example.com").is_err());
    }
}
