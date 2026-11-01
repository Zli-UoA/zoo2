pub mod attachments;
pub mod bubble_chat;
pub mod channel_webhooks;
pub mod channels;
pub mod dms;
pub mod health;
pub mod home;
pub mod home_live;
pub mod messages;
pub mod notifications;
pub mod push;
pub mod read_states;
pub mod response;
pub mod shapes;
pub mod typing;
pub mod users;
pub mod webhooks;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header},
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::state::AppState;

pub fn create_router(state: AppState) -> Router {
    // 環境変数 `CORS_ALLOWED_ORIGINS` 由来の許可リストのみ受け付ける。
    // 不正な値は無視し、許可リストが空ならクロスオリジンは一切許可しない。
    let allowed_origins: Vec<HeaderValue> = state
        .config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_bytes(origin.as_bytes()).ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        // REST (GET/POST/PATCH/DELETE) と Shape の SSE (GET) のみ
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        // dev の `X-Dev-User`, Bearer の `Authorization`, JSON 書き込みの `Content-Type`
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("x-dev-user"),
        ])
        .expose_headers([
            HeaderName::from_static("electric-handle"),
            HeaderName::from_static("electric-offset"),
            HeaderName::from_static("electric-schema"),
            HeaderName::from_static("electric-cursor"),
            HeaderName::from_static("electric-up-to-date"),
            HeaderName::from_static("electric-chunk-last-offset"),
        ]);

    Router::new()
        .merge(health::router())
        .merge(home::router())
        .merge(crate::activitypub::router())
        .merge(crate::apps::management::router())
        .merge(crate::apps::api::router())
        .merge(crate::apps::oauth::router())
        .merge(crate::apps::delivery::router())
        .merge(crate::apps::ui::router())
        .merge(crate::apps::files::router())
        .merge(crate::apps::events::router())
        .merge(crate::apps::resources::router())
        .merge(channel_webhooks::router())
        .merge(users::router())
        .merge(channels::router())
        .merge(dms::router())
        .merge(messages::router())
        .merge(attachments::router())
        .merge(typing::router())
        .merge(bubble_chat::router())
        .merge(notifications::router())
        .merge(push::router())
        .merge(read_states::router())
        .merge(shapes::router())
        .layer(TraceLayer::new_for_http().make_span_with(
            |request: &axum::http::Request<axum::body::Body>| {
                let path = if request.uri().path().starts_with("/api/app-responses/") {
                    "/api/app-responses/[redacted]"
                } else if request.uri().path().starts_with("/api/webhooks/") {
                    "/api/webhooks/[redacted]"
                } else {
                    request.uri().path()
                };
                tracing::info_span!("http_request", method = %request.method(), path)
            },
        ))
        .layer(cors)
        .with_state(state)
}
