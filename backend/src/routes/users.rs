use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{AuthUser, rbac::is_workspace_admin},
    db::get_current_txid,
    error::AppError,
    models::{User, UserRole},
    routes::response::ApiResponse,
    state::AppState,
    validation::{validate_display_name, validate_user_name},
};

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub id: Option<String>,
    pub user_name: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMeRequest {
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
    pub is_bot: bool,
    pub created_at: String,
    pub updated_at: Option<String>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            user_name: user.user_name,
            display_name: user.display_name,
            is_bot: user.is_bot,
            created_at: user.created_at.to_string(),
            updated_at: user.updated_at.map(|t| t.to_string()),
        }
    }
}

/// `GET /api/users/me` のレスポンス。既存クライアントとの互換性を保つため、
/// `UserResponse` のフィールドはそのままフラットに並べ、`is_admin` だけを足す。
#[derive(Debug, Clone, Serialize)]
pub struct MeResponse {
    #[serde(flatten)]
    pub user: UserResponse,
    pub is_admin: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users", post(create_user).get(list_users))
        .route("/api/users/me", get(get_me).patch(update_me))
        .route(
            "/api/users/me/preferences",
            get(get_preferences).patch(update_preferences),
        )
        .route("/api/users/admins", get(list_admin_users))
        .route("/api/users/{id}/admin", patch(update_admin))
        .route("/api/users/{id}", get(get_user_by_id))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUsersResponse {
    pub user_ids: Vec<String>,
    #[serde(default)]
    pub protected_user_ids: Vec<String>,
}

/// キック可否の UI 判定用に、ワークスペース管理者の user_id を返す。
///
/// 管理者ロールは `users` / `channel_user` の Shape に含めていないため、
/// フロントが「キック不可の相手」を正確に判定できるよう軽量な一覧だけを提供する。
/// dev auth の固定管理者はロール行を持たなくても管理者として扱われるため、
/// `is_workspace_admin` と同じ条件で一覧に含める。
async fn list_admin_users(
    State(state): State<AppState>,
    _auth_user: AuthUser,
) -> Result<Json<AdminUsersResponse>, AppError> {
    let mut db = state.db.clone();
    let roles = UserRole::filter(UserRole::fields().role_id().eq("role_admin"))
        .exec(&mut db)
        .await?;

    let mut user_ids: Vec<String> = roles.into_iter().map(|role| role.user_id).collect();
    if state.config.dev_auth_enabled
        && !user_ids
            .iter()
            .any(|id| id == crate::auth::DEFAULT_DEV_USER_ID)
    {
        user_ids.push(crate::auth::DEFAULT_DEV_USER_ID.to_string());
    }
    user_ids.sort();
    user_ids.dedup();

    Ok(Json(AdminUsersResponse {
        user_ids,
        protected_user_ids: if state.config.dev_auth_enabled {
            vec![crate::auth::DEFAULT_DEV_USER_ID.to_string()]
        } else {
            vec![]
        },
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateAdminRequest {
    pub is_admin: bool,
}

/// Serialize role changes so concurrent demotions cannot remove every administrator.
async fn update_admin(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<UpdateAdminRequest>,
) -> Result<Json<ApiResponse<MeResponse>>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("user id is not a valid UUID: {id}")))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('zoo:workspace-admins', 0)) IS NULL",
    )
    .exec(&mut tx)
    .await?;
    // Check authorization after acquiring the lock: another transaction may have demoted us.
    if !is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await? {
        return Err(AppError::Forbidden(
            "Only administrators can change administrator roles".into(),
        ));
    }
    let user = User::get_by_id(&mut tx, id.as_str()).await.map_err(|err| {
        if err.is_record_not_found() {
            AppError::NotFound(format!("User not found: {id}"))
        } else {
            AppError::from(err)
        }
    })?;
    if user.is_bot {
        return Err(AppError::BadRequest(
            "Apps cannot be workspace administrators".into(),
        ));
    }
    let roles = UserRole::filter(UserRole::fields().role_id().eq("role_admin"))
        .exec(&mut tx)
        .await?;
    let already_admin = roles.iter().any(|role| role.user_id == id);
    let fixed_admin = state.config.dev_auth_enabled && id == crate::auth::DEFAULT_DEV_USER_ID;
    if !payload.is_admin {
        ensure_admin_removal_allowed(
            fixed_admin,
            already_admin,
            roles.len(),
            state.config.dev_auth_enabled,
        )?;
    }
    if payload.is_admin && !already_admin {
        toasty::create!(UserRole {
            user_id: id.as_str(),
            role_id: "role_admin"
        })
        .exec(&mut tx)
        .await?;
    } else if !payload.is_admin && already_admin {
        UserRole::filter(
            UserRole::fields()
                .user_id()
                .eq(id.as_str())
                .and(UserRole::fields().role_id().eq("role_admin")),
        )
        .delete()
        .exec(&mut tx)
        .await?;
    }
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(
        MeResponse {
            user: UserResponse::from(user),
            is_admin: payload.is_admin || fixed_admin,
        },
        txid,
    )))
}

async fn get_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<MeResponse>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let user = match User::get_by_id(&mut tx, auth_user.id.as_str()).await {
        Ok(user) => user,
        Err(err) if err.is_record_not_found() => {
            return Err(AppError::NotFound(format!(
                "User not found with id: {}",
                auth_user.id
            )));
        }
        Err(err) => return Err(err.into()),
    };
    let is_admin =
        is_workspace_admin(&mut tx, &auth_user.id, state.config.dev_auth_enabled).await?;
    tx.commit().await?;

    Ok(Json(MeResponse {
        user: UserResponse::from(user),
        is_admin,
    }))
}

/// The first-party preference endpoint only exposes supported personal settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    #[default]
    Google,
    DuckDuckGo,
    Yandex,
    Baidu,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalPreferences {
    pub search_engine: SearchEngine,
}

async fn get_preferences(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<PersonalPreferences>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let value = crate::apps::one(&mut tx,
        "SELECT jsonb_build_object('search_engine', (SELECT preferences::jsonb->>'search_engine' FROM user_event_settings WHERE user_id=$1))::text",
        &[&auth_user.id]).await?;
    let search_engine = serde_json::from_value(value["search_engine"].clone()).unwrap_or_default();
    tx.commit().await?;
    Ok(Json(PersonalPreferences { search_engine }))
}

async fn update_preferences(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<PersonalPreferences>,
) -> Result<Json<ApiResponse<PersonalPreferences>>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let engine = serde_json::to_value(payload.search_engine)
        .map_err(|err| AppError::Internal(err.into()))?;
    // Merge in SQL so concurrent updates cannot overwrite unrelated App preferences.
    toasty::sql::statement(
        "INSERT INTO user_event_settings(user_id,preferences) VALUES($1,jsonb_build_object('search_engine',$2::text)::text) \
         ON CONFLICT(user_id) DO UPDATE SET preferences=(user_event_settings.preferences::jsonb || jsonb_build_object('search_engine',$2::text))::text",
    ).bind(auth_user.id.as_str()).bind(engine.as_str().unwrap()).exec(&mut tx).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(payload, txid)))
}

async fn update_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<UpdateMeRequest>,
) -> Result<Json<ApiResponse<UserResponse>>, AppError> {
    if let Some(ref display_name) = payload.display_name {
        validate_display_name(display_name)?;
    }

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let now = crate::models::now_utc_millis();

    let mut update_stmt = User::update_by_id(auth_user.id.as_str()).updated_at(Some(now));
    if let Some(ref display_name) = payload.display_name {
        update_stmt = update_stmt.display_name(display_name.as_str());
    }

    update_stmt.exec(&mut tx).await?;
    let updated_user = User::get_by_id(&mut tx, auth_user.id.as_str()).await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(
        UserResponse::from(updated_user),
        txid,
    )))
}

async fn create_user(
    State(state): State<AppState>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserResponse>>, AppError> {
    if !state.config.dev_auth_enabled {
        return Err(AppError::Forbidden(
            "Direct user creation is disabled in production. Users are automatically provisioned via OIDC login."
                .to_string(),
        ));
    }

    validate_user_name(&payload.user_name)?;
    validate_display_name(&payload.display_name)?;

    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("user id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let now = crate::models::now_utc_millis();

    let user = toasty::create!(User {
        is_bot: false,
        id: id.as_str(),
        user_name: payload.user_name.as_str(),
        display_name: payload.display_name.as_str(),
        created_at: now,
        updated_at: None,
    })
    .exec(&mut tx)
    .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(UserResponse::from(user), txid)))
}

async fn list_users(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<Vec<UserResponse>>, AppError> {
    let mut db = state.db.clone();
    let mut users = User::filter(User::fields().is_bot().eq(false))
        .order_by(User::fields().created_at().asc())
        .exec(&mut db)
        .await?;
    if crate::apps::events::guest_in_db(&mut db, &auth_user.id).await? {
        let mut tx = db.transaction().await?;
        let visible=crate::apps::query(&mut tx,"SELECT DISTINCT json_build_object('id',other.user_id)::text FROM channel_user me JOIN channel_user other ON other.channel_id=me.channel_id JOIN channels c ON c.id=me.channel_id WHERE me.user_id=$1 AND c.deleted_at IS NULL",&[&auth_user.id]).await?;
        let ids: std::collections::HashSet<_> =
            visible.iter().filter_map(|v| v["id"].as_str()).collect();
        users.retain(|u| u.id == auth_user.id || ids.contains(u.id.as_str()));
    }
    let res = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(res))
}

async fn get_user_by_id(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<UserResponse>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("user id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();
    let actor = crate::apps::events::Actor {
        user: auth_user,
        principal: None,
    };
    {
        let mut tx = db.transaction().await?;
        actor.visible_user(&mut tx, &id).await?;
    }
    match User::get_by_id(&mut db, id.as_str()).await {
        Ok(user) => Ok(Json(UserResponse::from(user))),
        Err(err) if err.is_record_not_found() => {
            Err(AppError::NotFound(format!("User not found with id: {id}")))
        }
        Err(err) => Err(err.into()),
    }
}

fn ensure_admin_removal_allowed(
    fixed_admin: bool,
    already_admin: bool,
    admin_count: usize,
    dev_auth_enabled: bool,
) -> Result<(), AppError> {
    if fixed_admin {
        return Err(AppError::Conflict(
            "The development administrator cannot be removed".into(),
        ));
    }
    if already_admin && admin_count <= 1 && !dev_auth_enabled {
        return Err(AppError::Conflict(
            "The last administrator cannot be removed".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod admin_role_tests {
    use super::*;
    #[test]
    fn protects_the_last_and_fixed_administrators() {
        assert!(ensure_admin_removal_allowed(false, true, 1, false).is_err());
        assert!(ensure_admin_removal_allowed(true, true, 2, true).is_err());
        assert!(ensure_admin_removal_allowed(false, true, 2, false).is_ok());
        assert!(ensure_admin_removal_allowed(false, false, 1, false).is_ok());
        // The development administrator remains available independently of role rows.
        assert!(ensure_admin_removal_allowed(false, true, 1, true).is_ok());
    }
}
