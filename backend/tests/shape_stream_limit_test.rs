//! 1 アカウントが同時に開ける shape のストリームの数には上限がある。
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use backend::{
    config::Config, db::init_db, routes::create_router, routes::shape_limit::ShapeStreamLimiter,
    state::AppState,
};

const USER: &str = "00000000-0000-0000-0000-000000000001";

async fn open(app: &Router, user: &str) -> anyhow::Result<Response> {
    Ok(app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/shapes/users?offset=-1&live=true&live_sse=true")
                .header("X-Dev-User", user)
                .body(Body::empty())?,
        )
        .await?)
}

#[tokio::test]
async fn shape_streams_are_limited_per_account() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let electric = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/shape"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&electric)
        .await;

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.electric_url = electric.uri();
    let db = init_db(&config.database_url).await?;
    let mut state = AppState::new(db, config);
    state.shape_streams = ShapeStreamLimiter::new(2);
    let app = create_router(state);

    // 開いたままの応答（本文を読み終えていないストリーム）が枠を持つ。
    let first = open(&app, USER).await?;
    let second = open(&app, USER).await?;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        open(&app, USER).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    // 閉じると枠が空く。
    drop(first);
    assert_eq!(open(&app, USER).await?.status(), StatusCode::OK);
    drop(second);
    Ok(())
}
