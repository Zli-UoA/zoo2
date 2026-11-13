pub mod attachments;
pub mod bubble_chat;
pub mod calls;
pub mod channel_webhooks;
pub mod channels;
pub mod dms;
pub mod e2ee;
pub mod e2ee_channels;
pub mod e2ee_cross_signing;
pub mod e2ee_key_backup;
pub mod e2ee_reauth;
pub mod e2ee_secret_storage;
pub mod health;
pub mod home;
pub mod home_images;
pub mod home_live;
pub mod messages;
pub mod notifications;
pub mod push;
pub mod read_states;
pub mod response;
pub mod shapes;
pub mod threads;
pub mod typing;
pub mod users;
pub mod webhooks;

use axum::{
    Router,
    http::{HeaderName, HeaderValue, Method, header},
    middleware,
};
use tower_http::cors::CorsLayer;

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
        // dev の `X-Dev-User`, Bearer の `Authorization`, JSON 書き込みの `Content-Type`,
        // E2EE の直前の再認証の `X-Zoo-Reauth`（docs/e2ee.md の「クロス署名」）,
        // OpenTelemetry のトレースコンテキスト伝播 (`traceparent`, `tracestate`)
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("x-dev-user"),
            HeaderName::from_static(e2ee_reauth::REAUTH_HEADER),
            HeaderName::from_static("traceparent"),
            HeaderName::from_static("tracestate"),
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
        .merge(e2ee::router())
        .merge(e2ee_channels::router())
        .merge(e2ee_cross_signing::router())
        .merge(e2ee_secret_storage::router())
        .merge(e2ee_key_backup::router())
        .merge(messages::router())
        .merge(threads::router())
        .merge(attachments::router())
        .merge(crate::slack::import::router())
        .merge(crate::slack::link::router())
        .merge(crate::slack::self_import::router())
        .merge(typing::router())
        .merge(bubble_chat::router())
        .merge(calls::router())
        .merge(notifications::router())
        .merge(push::router())
        .merge(read_states::router())
        .merge(shapes::router())
        .layer(middleware::from_fn(
            crate::telemetry::http_telemetry_middleware,
        ))
        .layer(cors)
        .with_state(state)
}
