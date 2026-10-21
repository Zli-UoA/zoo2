use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{AuthUser, rbac::is_workspace_admin, tenant::TenantAuth},
    db::get_current_txid,
    error::AppError,
    models::{Tenant, TenantUser, User, UserRole},
    routes::response::ApiResponse,
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tenants", get(list_tenants).post(create_tenant))
        .route("/api/tenants/members", post(add_member))
}

#[derive(Deserialize)]
struct CreateTenant {
    name: String,
}

#[derive(Deserialize)]
struct AddMember {
    user_name: String,
}

async fn list_tenants(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<Tenant>>, AppError> {
    let mut db = state.db.clone();
    let memberships = TenantUser::filter(TenantUser::fields().user_id().eq(user.id.as_str()))
        .exec(&mut db)
        .await?;
    let ids: Vec<String> = memberships.into_iter().map(|m| m.tenant_id).collect();
    let tenants = Tenant::filter(Tenant::fields().id().in_list(ids))
        .order_by(Tenant::fields().created_at().asc())
        .exec(&mut db)
        .await?;
    Ok(Json(tenants))
}

async fn create_tenant(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<CreateTenant>,
) -> Result<Json<ApiResponse<Tenant>>, AppError> {
    let name = payload.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::BadRequest(
            "Workspace name must contain 1 to 100 characters".into(),
        ));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let id = Uuid::new_v4().to_string();
    let now = crate::models::now_utc_millis();
    let tenant = toasty::create!(Tenant {
        id: id.as_str(),
        name,
        created_at: now
    })
    .exec(&mut tx)
    .await?;
    toasty::create!(TenantUser {
        tenant_id: id.as_str(),
        user_id: user.id.as_str(),
        joined_at: now
    })
    .exec(&mut tx)
    .await?;
    toasty::create!(UserRole {
        tenant_id: id.as_str(),
        user_id: user.id.as_str(),
        role_id: "role_admin"
    })
    .exec(&mut tx)
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(tenant, txid)))
}

async fn add_member(
    State(state): State<AppState>,
    actor: TenantAuth,
    Json(payload): Json<AddMember>,
) -> Result<Json<ApiResponse<TenantUser>>, AppError> {
    crate::validation::validate_user_name(&payload.user_name)?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if !is_workspace_admin(
        &mut tx,
        &actor.id,
        &actor.tenant_id,
        state.config.dev_auth_enabled,
    )
    .await?
    {
        return Err(AppError::Forbidden(
            "Workspace administrator required".into(),
        ));
    }
    let target = User::filter(User::fields().user_name().eq(payload.user_name.as_str()))
        .first()
        .exec(&mut tx)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    let existing = TenantUser::filter(
        TenantUser::fields()
            .tenant_id()
            .eq(actor.tenant_id.as_str())
            .and(TenantUser::fields().user_id().eq(target.id.as_str())),
    )
    .first()
    .exec(&mut tx)
    .await?;
    if let Some(member) = existing {
        return Ok(Json(ApiResponse::new(member, None)));
    }
    let member = toasty::create!(TenantUser {
        tenant_id: actor.tenant_id.as_str(),
        user_id: target.id.as_str(),
        joined_at: crate::models::now_utc_millis(),
    })
    .exec(&mut tx)
    .await?;
    let txid = get_current_txid(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(ApiResponse::new(member, txid)))
}
