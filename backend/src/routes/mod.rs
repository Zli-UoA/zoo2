pub mod channels;
pub mod health;
pub mod messages;
pub mod response;
pub mod shapes;
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
        .allow_headers(Any)
        .expose_headers([
            axum::http::HeaderName::from_static("electric-handle"),
            axum::http::HeaderName::from_static("electric-offset"),
            axum::http::HeaderName::from_static("electric-schema"),
            axum::http::HeaderName::from_static("electric-cursor"),
        ]);

    Router::new()
        .merge(health::router())
        .merge(users::router())
        .merge(channels::router())
        .merge(messages::router())
        .merge(shapes::router())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
