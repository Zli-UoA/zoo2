use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};

use crate::{auth::AuthUser, error::AppError, models::TenantUser, state::AppState};

pub const DEFAULT_TENANT_ID: &str = "00000000-0000-0000-0000-000000000001";

/// A tenant selection is trusted only after checking the authenticated user's membership.
pub struct TenantAuth {
    pub user: AuthUser,
    pub tenant_id: String,
}

impl std::ops::Deref for TenantAuth {
    type Target = AuthUser;

    fn deref(&self) -> &AuthUser {
        &self.user
    }
}

impl TenantAuth {
    pub fn check_channel(&self, channel: &crate::models::Channel) -> Result<(), AppError> {
        if channel.tenant_id != self.tenant_id {
            return Err(AppError::NotFound("Channel not found".into()));
        }
        Ok(())
    }
}

impl FromRequestParts<AppState> for TenantAuth {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        let tenant_id = tenant_id_from_headers(&parts.headers)?;
        let mut db = state.db.clone();
        require_membership(&mut db, &tenant_id, &user.id).await?;
        Ok(Self { user, tenant_id })
    }
}

pub fn tenant_id_from_headers(headers: &HeaderMap) -> Result<String, AppError> {
    match headers.get("X-Tenant-ID") {
        Some(value) => {
            let value = value
                .to_str()
                .map_err(|_| AppError::BadRequest("Invalid X-Tenant-ID".into()))?;
            uuid::Uuid::parse_str(value)
                .map(|id| id.to_string())
                .map_err(|_| AppError::BadRequest("X-Tenant-ID must be a UUID".into()))
        }
        None => Ok(DEFAULT_TENANT_ID.to_string()),
    }
}

pub async fn require_membership(
    db: &mut toasty::Db,
    tenant_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let member = TenantUser::filter(
        TenantUser::fields()
            .tenant_id()
            .eq(tenant_id)
            .and(TenantUser::fields().user_id().eq(user_id)),
    )
    .first()
    .exec(db)
    .await?;
    if member.is_none() {
        return Err(AppError::NotFound("Workspace or member not found".into()));
    }
    Ok(())
}
