//! Event coverage and shared authorization for concrete event-producing operations.
use super::*;
use axum::{Json, Router, extract::FromRequestParts, http::request::Parts, routing::get};
use std::sync::OnceLock;

pub fn coverage() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../../docs/slack-event-coverage.json"))
            .expect("checked event catalog")
    })
}
pub fn known(name: &str) -> bool {
    coverage()["native_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["name"] == name)
        || coverage()["aliases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["alias"] == name)
}
pub fn native_names() -> Vec<String> {
    coverage()["native_events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_owned())
        .collect()
}
pub fn required_scopes(name: &str) -> Vec<String> {
    let names: Vec<&str> = coverage()["aliases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["alias"] == name)
        .filter_map(|v| v["event"].as_str())
        .chain(std::iter::once(name))
        .collect();
    coverage()["native_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| names.contains(&v["name"].as_str().unwrap()))
        .filter_map(|v| v["scope"].as_str().map(str::to_owned))
        .collect()
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/apps/events",
            get(|| async { Json(coverage().clone()) }),
        )
        .route(
            "/api/apps/v1/events",
            get(|| async { Json(coverage().clone()) }),
        )
}
pub async fn emit(
    tx: &mut toasty::Transaction<'_>,
    kind: &str,
    data: Value,
    actor: Option<&str>,
    channel: Option<&str>,
    subject: Option<&str>,
    installation: Option<&str>,
) -> Result<(), AppError> {
    one(tx,"SELECT json_build_object('emitted',app_emit($1,$2::jsonb,NULLIF($3,''),NULLIF($4,''),NULLIF($5,''),NULLIF($6,'')))::text",&[kind,&data.to_string(),actor.unwrap_or(""),channel.unwrap_or(""),subject.unwrap_or(""),installation.unwrap_or("")]).await?;
    Ok(())
}
pub async fn actor_context(tx: &mut toasty::Transaction<'_>, actor: &str) -> Result<(), AppError> {
    one(
        tx,
        "SELECT json_build_object('actor',set_config('zoo.actor_id',$1,true))::text",
        &[actor],
    )
    .await?;
    Ok(())
}
pub async fn guest(tx: &mut toasty::Transaction<'_>, user: &str) -> Result<bool, AppError> {
    Ok(one(tx,"SELECT json_build_object('guest',EXISTS(SELECT 1 FROM users WHERE id=$1 AND is_guest))::text",&[user]).await?["guest"]==true)
}
pub async fn guest_in_db(db: &mut toasty::Db, user: &str) -> Result<bool, AppError> {
    let mut tx = db.transaction().await?;
    guest(&mut tx, user).await
}
pub async fn active_user(state: &AppState, user: &str) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if one(&mut tx,"SELECT json_build_object('disabled',EXISTS(SELECT 1 FROM users WHERE id=$1 AND is_deactivated))::text",&[user]).await?["disabled"]==true{return Err(AppError::Forbidden("Account is deactivated".into()));}
    Ok(())
}
pub async fn posting(
    tx: &mut toasty::Transaction<'_>,
    actor: &str,
    channel: &str,
    dev: bool,
) -> Result<(), AppError> {
    let v=one(tx,"SELECT json_build_object('creator',created_user_id,'policy',posting_policy)::text FROM channels WHERE id=$1",&[channel]).await?;
    if v["policy"] == "managers"
        && v["creator"] != actor
        && !crate::auth::rbac::is_workspace_admin(tx, actor, dev).await?
    {
        return Err(AppError::Forbidden(
            "Only channel managers may post here".into(),
        ));
    }
    Ok(())
}
#[derive(Clone)]
pub struct Actor {
    pub user: AuthUser,
    pub principal: Option<Principal>,
}
impl FromRequestParts<AppState> for Actor {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        if parts
            .headers
            .get("authorization")
            .and_then(|s| s.to_str().ok())
            .is_some_and(|s| s.starts_with("Bearer zoo_at_"))
        {
            let p = Principal::from_request_parts(parts, state).await?;
            Ok(Self {
                user: p.actor(),
                principal: Some(p),
            })
        } else {
            Ok(Self {
                user: AuthUser::from_request_parts(parts, state).await?,
                principal: None,
            })
        }
    }
}
impl Actor {
    pub fn require(&self, scope: &str) -> Result<(), AppError> {
        if let Some(p) = &self.principal {
            p.require(scope)?;
        }
        Ok(())
    }
    pub async fn admin(
        &self,
        tx: &mut toasty::Transaction<'_>,
        state: &AppState,
    ) -> Result<(), AppError> {
        if !crate::auth::rbac::is_workspace_admin(tx, &self.user.id, state.config.dev_auth_enabled)
            .await?
        {
            return Err(AppError::Forbidden(
                "Workspace administrator required".into(),
            ));
        }
        Ok(())
    }
    pub async fn channel(
        &self,
        tx: &mut toasty::Transaction<'_>,
        channel: &str,
        write: bool,
    ) -> Result<Value, AppError> {
        if let Some(p) = &self.principal {
            p.channel(tx, channel, write).await?;
        }
        let v=one(tx,"SELECT json_build_object('id',c.id,'creator',c.created_user_id,'is_dm',c.is_dm,'is_private',c.is_private,'member',EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=$2))::text FROM channels c WHERE c.id=$1 AND c.deleted_at IS NULL AND (NOT $3::boolean OR c.archive_at IS NULL) AND (EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=$2) OR NOT $3::boolean AND NOT c.is_private AND NOT c.is_dm AND NOT EXISTS(SELECT 1 FROM users WHERE id=$2 AND is_guest))",&[channel,&self.user.id,if write{"true"}else{"false"}]).await?;
        Ok(v)
    }
    pub async fn channel_manager(
        &self,
        tx: &mut toasty::Transaction<'_>,
        state: &AppState,
        channel: &str,
    ) -> Result<(), AppError> {
        let c = self.channel(tx, channel, true).await?;
        if c["is_dm"] == true {
            return Err(AppError::Forbidden(
                "Cannot manage DM channel settings".into(),
            ));
        }
        if c["creator"] != self.user.id {
            self.admin(tx, state).await?;
        }
        Ok(())
    }
    pub async fn visible_user(
        &self,
        tx: &mut toasty::Transaction<'_>,
        target: &str,
    ) -> Result<(), AppError> {
        if guest(tx, &self.user.id).await? && target != self.user.id {
            one(tx,"SELECT '{}'::text FROM channel_user me JOIN channel_user other ON other.channel_id=me.channel_id JOIN channels c ON c.id=me.channel_id WHERE me.user_id=$1 AND other.user_id=$2 AND c.deleted_at IS NULL LIMIT 1",&[&self.user.id,target]).await?;
        }
        Ok(())
    }
}
pub fn object(value: &Value) -> Result<(), AppError> {
    if !value.is_object() || value.to_string().len() > 32768 {
        return Err(AppError::BadRequest(
            "Expected an object of at most 32 KiB".into(),
        ));
    }
    Ok(())
}
pub fn text(value: &str, max: usize) -> Result<(), AppError> {
    if value.chars().count() > max || value.contains('\0') {
        return Err(AppError::BadRequest(
            "Text is too long or contains a NUL character".into(),
        ));
    }
    Ok(())
}

/// Clock-driven state changes are ordinary transactional mutations with the same outbox guarantees.
pub async fn maintain(state: &AppState) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    for sql in [
        "UPDATE user_event_settings SET presence='away' WHERE presence='active' AND active_until<=now()",
        "UPDATE user_event_settings SET dnd_until=NULL WHERE dnd_until<=now()",
        "UPDATE users SET status_text='',status_emoji='',status_expires_at=NULL WHERE status_expires_at<=now()",
        "UPDATE user_reminders SET fired_at=now() WHERE id IN(SELECT id FROM user_reminders WHERE due_at<=now() AND fired_at IS NULL AND completed_at IS NULL LIMIT 1000 FOR UPDATE SKIP LOCKED)",
        "DELETE FROM app_event_limits WHERE window_start<now()-interval '2 days'",
    ] {
        toasty::sql::statement(sql).exec(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_unclassified_official_events_or_dangling_aliases() {
        let c = coverage();
        assert_eq!(c["official_events"].as_array().unwrap().len(), 120);
        for name in c["official_events"].as_array().unwrap() {
            let rows: Vec<_> = c["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["slack_event"] == *name)
                .collect();
            assert_eq!(rows.len(), 1, "{name}");
            let r = rows[0];
            assert!(
                ["implemented", "excluded", "transport"]
                    .iter()
                    .any(|s| r["status"] == *s)
            );
            if r["status"] == "excluded" {
                assert!(r["reason"].as_str().unwrap().len() > 30);
            }
        }
        for a in c["aliases"].as_array().unwrap() {
            assert!(native_names().iter().any(|n| a["event"] == *n), "{a}");
            assert!(known(a["alias"].as_str().unwrap()));
        }
    }
}
