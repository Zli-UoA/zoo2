use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::AppError, models::User, state::AppState};

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub id: Option<String>,
    pub user_name: String,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users", post(create_user).get(list_users))
        .route("/api/users/{id}", get(get_user_by_id))
}

async fn create_user(
    State(state): State<AppState>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<UserResponse>, AppError> {
    let mut db = state.db.clone();
    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = crate::models::now_utc_millis();

    let user = toasty::create!(User {
        id: id.as_str(),
        user_name: payload.user_name.as_str(),
        display_name: payload.display_name.as_str(),
        created_at: now,
        updated_at: None,
    })
    .exec(&mut db)
    .await?;

    Ok(Json(UserResponse::from(user)))
}

async fn list_users(State(state): State<AppState>) -> Result<Json<Vec<UserResponse>>, AppError> {
    let mut db = state.db.clone();
    let users = User::all()
        .order_by(User::fields().created_at().asc())
        .exec(&mut db)
        .await?;
    let res = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(res))
}

async fn get_user_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<UserResponse>, AppError> {
    let mut db = state.db.clone();
    match User::get_by_id(&mut db, id.as_str()).await {
        Ok(user) => Ok(Json(UserResponse::from(user))),
        Err(err) if err.is_record_not_found() => {
            Err(AppError::NotFound(format!("User not found with id: {id}")))
        }
        Err(err) => Err(AppError::Database(err)),
    }
}
