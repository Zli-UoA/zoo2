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

/// Electric の応答を待っている間にアカウントが無効になったら、最初のチャンクも転送しない。
#[tokio::test]
async fn first_chunk_is_not_sent_after_access_is_lost() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let electric = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/shape"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(1500))
                .set_body_string("[{\"value\":\"secret\"}]"),
        )
        .mount(&electric)
        .await;

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.electric_url = electric.uri();
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let user = uuid::Uuid::new_v4().to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/users")
                .header("content-type", "application/json")
                .header("X-Dev-User", &user)
                .body(Body::from(
                    serde_json::json!({"id": user, "user_name": format!("first_{}", uuid::Uuid::new_v4().simple()), "display_name": "First"})
                        .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let pending = tokio::spawn({
        let app = app.clone();
        let user = user.clone();
        async move { open(&app, &user).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let mut tx = db.transaction().await?;
    backend::apps::query(
        &mut tx,
        "UPDATE users SET is_deactivated = true WHERE id = $1 RETURNING json_build_object('ok', true)::text",
        &[&user],
    )
    .await?;
    tx.commit().await?;

    let response = pending.await??;
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await?;
    assert!(body.is_empty(), "{}", String::from_utf8_lossy(&body));
    Ok(())
}

/// 枠のキーを指定した要求（App の資格情報）は、利用者本人の枠から引かない。
#[tokio::test]
async fn keyed_streams_do_not_use_the_users_own_slots() -> anyhow::Result<()> {
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
    state.shape_streams = ShapeStreamLimiter::new(1);
    let app = create_router(state);

    let own = open(&app, USER).await?;
    assert_eq!(own.status(), StatusCode::OK);
    let delegated = backend::routes::shape_limit::SHAPE_LIMIT_KEY
        .scope("app:credential".to_owned(), open(&app, USER))
        .await?;
    assert_eq!(delegated.status(), StatusCode::OK);
    // 本人の枠も、App の枠も、それぞれ 1 本で埋まる。
    assert_eq!(
        open(&app, USER).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let second = backend::routes::shape_limit::SHAPE_LIMIT_KEY
        .scope("app:credential".to_owned(), open(&app, USER))
        .await?;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    drop((own, delegated));
    Ok(())
}
