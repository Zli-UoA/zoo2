pub mod channels;
pub mod health;
pub mod messages;
pub mod users;

use axum::Router;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::state::AppState;

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .merge(health::router())
        .merge(users::router())
        .merge(channels::router())
        .merge(messages::router())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
