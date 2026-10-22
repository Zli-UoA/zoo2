//! Short-lived presence only: drafts are never stored or transmitted.
use crate::{
    auth::AuthUser,
    error::AppError,
    models::{Channel, ChannelUser, Message, User},
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

const TTL: Duration = Duration::from_secs(6);
type Key = (String, Option<String>, String, String);
#[derive(Clone, Default)]
pub struct TypingStore(Arc<RwLock<HashMap<Key, Instant>>>);

impl TypingStore {
    async fn update(&self, key: Key, active: bool) {
        let mut entries = self.0.write().await;
        entries.retain(|_, at| at.elapsed() < TTL);
        if active {
            entries.insert(key, Instant::now());
        } else {
            entries.remove(&key);
        }
    }
    async fn users(&self, channel: &str, parent: &Option<String>, actor: &str) -> Vec<String> {
        let mut entries = self.0.write().await;
        entries.retain(|_, at| at.elapsed() < TTL);
        let mut users: Vec<_> = entries
            .keys()
            .filter(|(c, p, u, _)| c == channel && p == parent && u != actor)
            .map(|(_, _, u, _)| u.clone())
            .collect();
        users.sort();
        users.dedup();
        users
    }
}

#[derive(Deserialize)]
struct Scope {
    parent_id: Option<String>,
}
#[derive(Deserialize)]
struct Update {
    parent_id: Option<String>,
    session_id: uuid::Uuid,
    active: bool,
}
#[derive(Serialize)]
struct TypingUser {
    id: String,
    display_name: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/channels/{id}/typing", get(list).post(update))
}

async fn authorize(
    state: &AppState,
    channel_id: &str,
    parent: &Option<String>,
    user: &AuthUser,
) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let channel = Channel::get_by_id(&mut tx, channel_id).await.map_err(|e| {
        if e.is_record_not_found() {
            AppError::NotFound("Channel not found".into())
        } else {
            e.into()
        }
    })?;
    let member = ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel_id)
            .and(ChannelUser::fields().user_id().eq(user.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?;
    if channel.deleted_at.is_some() || channel.archive_at.is_some() || member.is_none() {
        return Err(AppError::NotFound("Channel not found".into()));
    }
    if let Some(parent) = parent {
        let root = Message::get_by_id(&mut tx, parent.as_str())
            .await
            .map_err(|e| {
                if e.is_record_not_found() {
                    AppError::NotFound("Thread not found".into())
                } else {
                    e.into()
                }
            })?;
        if root.channel_id != channel_id || root.deleted_at.is_some() || root.parent_id.is_some() {
            return Err(AppError::NotFound("Thread not found".into()));
        }
    }
    Ok(())
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<Update>,
) -> Result<Json<serde_json::Value>, AppError> {
    authorize(&state, &id, &body.parent_id, &user).await?;
    state
        .typing
        .update(
            (id, body.parent_id, user.id, body.session_id.to_string()),
            body.active,
        )
        .await;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(scope): Query<Scope>,
) -> Result<Json<Vec<TypingUser>>, AppError> {
    authorize(&state, &id, &scope.parent_id, &user).await?;
    let ids = state.typing.users(&id, &scope.parent_id, &user.id).await;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let mut users = Vec::new();
    for user_id in ids {
        // Membership can change while a presence entry is alive.
        let member = ChannelUser::filter(
            ChannelUser::fields()
                .channel_id()
                .eq(id.as_str())
                .and(ChannelUser::fields().user_id().eq(user_id.as_str())),
        )
        .first()
        .exec(&mut tx)
        .await?;
        if member.is_some() {
            let user = User::get_by_id(&mut tx, user_id.as_str()).await?;
            users.push(TypingUser {
                id: user.id,
                display_name: user.display_name,
            });
        }
    }
    Ok(Json(users))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn presence_is_scoped_deduplicated_and_expires() {
        let store = TypingStore::default();
        let key = ("channel".into(), None, "alice".into(), "tab1".into());
        store.update(key.clone(), true).await;
        store
            .update(
                ("channel".into(), None, "alice".into(), "tab2".into()),
                true,
            )
            .await;
        assert_eq!(store.users("channel", &None, "bob").await, vec!["alice"]);
        assert!(store.users("channel", &None, "alice").await.is_empty());
        assert!(store.users("other", &None, "bob").await.is_empty());
        assert!(
            store
                .users("channel", &Some("thread".into()), "bob")
                .await
                .is_empty()
        );
        store.update(key, false).await;
        assert_eq!(store.users("channel", &None, "bob").await, vec!["alice"]);
        for at in store.0.write().await.values_mut() {
            *at = Instant::now() - TTL;
        }
        assert!(store.users("channel", &None, "bob").await.is_empty());
    }
}
