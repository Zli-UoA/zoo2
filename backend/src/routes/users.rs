use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{
        AuthUser,
        rbac::is_workspace_admin,
        tenant::{DEFAULT_TENANT_ID, TenantAuth, require_membership, tenant_id_from_headers},
    },
    db::get_current_txid,
    error::AppError,
    models::{TenantUser, User, UserRole},
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
    pub created_at: String,
    pub updated_at: Option<String>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            user_name: user.user_name,
            display_name: user.display_name,
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
        .route("/api/users/admins", get(list_admin_users))
        .route("/api/users/{id}", get(get_user_by_id))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUsersResponse {
    pub user_ids: Vec<String>,
}

/// キック可否の UI 判定用に、ワークスペース管理者の user_id を返す。
///
/// 管理者ロールは `users` / `channel_user` の Shape に含めていないため、
/// フロントが「キック不可の相手」を正確に判定できるよう軽量な一覧だけを提供する。
/// dev auth の固定管理者はロール行を持たなくても管理者として扱われるため、
/// `is_workspace_admin` と同じ条件で一覧に含める。
async fn list_admin_users(
    State(state): State<AppState>,
    auth_user: TenantAuth,
) -> Result<Json<AdminUsersResponse>, AppError> {
    let mut db = state.db.clone();
    let roles = UserRole::filter(
        UserRole::fields().role_id().eq("role_admin").and(
            UserRole::fields()
                .tenant_id()
                .eq(auth_user.tenant_id.as_str()),
        ),
    )
    .exec(&mut db)
    .await?;

    let mut user_ids: Vec<String> = roles.into_iter().map(|role| role.user_id).collect();
    if state.config.dev_auth_enabled
        && auth_user.tenant_id == DEFAULT_TENANT_ID
        && !user_ids
            .iter()
            .any(|id| id == crate::auth::DEFAULT_DEV_USER_ID)
    {
        user_ids.push(crate::auth::DEFAULT_DEV_USER_ID.to_string());
    }
    user_ids.sort();
    user_ids.dedup();

    Ok(Json(AdminUsersResponse { user_ids }))
}

async fn get_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
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
    // A profile is available before joining a workspace; admin privileges still require membership.
    let tenant_id = tenant_id_from_headers(&headers)?;
    let is_member = TenantUser::filter(
        TenantUser::fields()
            .tenant_id()
            .eq(tenant_id.as_str())
            .and(TenantUser::fields().user_id().eq(auth_user.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?
    .is_some();
    let is_admin = is_member
        && is_workspace_admin(
            &mut tx,
            &auth_user.id,
            &tenant_id,
            state.config.dev_auth_enabled,
        )
        .await?;
    tx.commit().await?;

    Ok(Json(MeResponse {
        user: UserResponse::from(user),
        is_admin,
    }))
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
        id: id.as_str(),
        user_name: payload.user_name.as_str(),
        display_name: payload.display_name.as_str(),
        created_at: now,
        updated_at: None,
    })
    .exec(&mut tx)
    .await?;

    // Development-only fixture creation retains the original single-workspace workflow.
    toasty::create!(TenantUser {
        tenant_id: DEFAULT_TENANT_ID,
        user_id: id.as_str(),
        joined_at: now,
    })
    .exec(&mut tx)
    .await?;

    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;

    Ok(Json(ApiResponse::new(UserResponse::from(user), txid)))
}

async fn list_users(
    State(state): State<AppState>,
    auth_user: TenantAuth,
) -> Result<Json<Vec<UserResponse>>, AppError> {
    let mut db = state.db.clone();
    let members = TenantUser::filter(
        TenantUser::fields()
            .tenant_id()
            .eq(auth_user.tenant_id.as_str()),
    )
    .exec(&mut db)
    .await?;
    let ids: Vec<String> = members.into_iter().map(|member| member.user_id).collect();
    let users = User::filter(User::fields().id().in_list(ids))
        .order_by(User::fields().created_at().asc())
        .exec(&mut db)
        .await?;
    let res = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(res))
}

async fn get_user_by_id(
    State(state): State<AppState>,
    auth_user: TenantAuth,
    Path(id): Path<String>,
) -> Result<Json<UserResponse>, AppError> {
    Uuid::parse_str(&id)
        .map_err(|_| AppError::BadRequest(format!("user id is not a valid UUID: {id}")))?;

    let mut db = state.db.clone();
    require_membership(&mut db, &auth_user.tenant_id, &id).await?;
    match User::get_by_id(&mut db, id.as_str()).await {
        Ok(user) => Ok(Json(UserResponse::from(user))),
        Err(err) if err.is_record_not_found() => {
            Err(AppError::NotFound(format!("User not found with id: {id}")))
        }
        Err(err) => Err(err.into()),
    }
}
