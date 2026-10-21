use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use backend::{
    auth::oidc::JwksVerifier, config::Config, db::init_db, routes::create_router, state::AppState,
};

#[tokio::test]
async fn test_shape_proxy_and_authorization() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    // 環境変数ではなく Config へ直接注入する (テスト間の env 競合を避ける)
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state);

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    // 1. Alice と Bob を作成
    for (id, name) in [(&alice_id, "alice"), (&bob_id, "bob")] {
        let user_payload = json!({
            "id": id,
            "user_name": format!("{}_{}", name, uuid::Uuid::new_v4().simple()),
            "display_name": name,
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/users")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&user_payload)?))?;

        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // 2. Alice がプライベートチャンネルを作成 (Alice のみ所属)
    let private_channel_id = uuid::Uuid::new_v4().to_string();
    let channel_payload = json!({
        "id": private_channel_id,
        "channel_name": format!("secret_{}", uuid::Uuid::new_v4().simple()),
        "description": "Secret Channel",
        "is_private": true,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&channel_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. Shape プロキシのテスト: Users Shape (Electric への疎通)
    let req = Request::builder()
        .method("GET")
        .uri("/api/shapes/users?offset=-1")
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    println!("Users shape response status: {:?}", resp.status());
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers().contains_key("electric-handle"),
        "electric-handle header must be present"
    );

    // 4. Shape プロキシの認可テスト: Bob (未参加) が Alice のプライベートチャンネルの messages を取得 -> 404 Not Found (存在隠蔽)
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/shapes/messages?channel_id={private_channel_id}&offset=-1"
        ))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 5. Bob がプライベートチャンネルに直接 join しようとすると 403 Forbidden
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{private_channel_id}/join"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 6. Alice (オーナー) が Bob をプライベートチャンネルに追加
    let add_user_payload = json!({
        "user_id": bob_id,
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{private_channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&add_user_payload)?))?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 7. 追加後、Bob が messages を取得 -> 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/shapes/messages?channel_id={private_channel_id}&offset=-1"
        ))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 8. Channels Shape の確認 (Alice)
    let req = Request::builder()
        .method("GET")
        .uri("/api/shapes/channels?offset=-1")
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("electric-handle"));

    // 9. Channels Shape の確認 (Bob: 参加チャンネルがサブクエリ経由で反映されること)
    let req = Request::builder()
        .method("GET")
        .uri("/api/shapes/channels?offset=-1")
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("electric-handle"));

    // 10. Channel Users Shape の確認 (全体取得: サブクエリ経由で公開+所属チャンネルメンバーが取得できること)
    let req = Request::builder()
        .method("GET")
        .uri("/api/shapes/channel_users?offset=-1")
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("electric-handle"));

    // 11. Message Reactions Shape の確認 (非メンバー Charlie は 404)
    let charlie_id = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/shapes/message_reactions?channel_id={private_channel_id}&offset=-1"
        ))
        .header("X-Dev-User", &charlie_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 12. Message Reactions Shape の確認 (メンバー Bob は 200 OK)
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/shapes/message_reactions?channel_id={private_channel_id}&offset=-1"
        ))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().contains_key("electric-handle"));

    Ok(())
}

/// #46: CORS は allowlist のみ許可し、未許可オリジンには ACAO を返さない
#[tokio::test]
async fn test_cors_allowlist() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    // 環境変数ではなく Config へ直接注入する (テスト間の env 競合を避ける)
    config.dev_auth_enabled = true;
    config.cors_allowed_origins = vec!["http://localhost:5173".to_string()];
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state);

    // Use this test's own account: fresh CI databases do not contain the fixed dev user.
    let user_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({ "id": user_id, "user_name": format!("cors_{}", uuid::Uuid::new_v4().simple()), "display_name": "CORS test" });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload)?))?;
    assert_eq!(app.clone().oneshot(req).await?.status(), StatusCode::OK);

    // 許可オリジンの preflight は成功し、メソッド/ヘッダを許可する
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/api/shapes/users")
        .header("Origin", "http://localhost:5173")
        .header("Access-Control-Request-Method", "GET")
        .header(
            "Access-Control-Request-Headers",
            "authorization,x-dev-user,x-tenant-id",
        )
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://localhost:5173")
    );
    let allow_headers = resp
        .headers()
        .get("access-control-allow-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    assert!(allow_headers.contains("authorization"));
    assert!(allow_headers.contains("x-dev-user"));
    assert!(allow_headers.contains("x-tenant-id"));

    // 許可オリジンの実リクエストは electric-* を expose する
    let req = Request::builder()
        .method("GET")
        .uri("/api/shapes/users?offset=-1")
        .header("Origin", "http://localhost:5173")
        .header("X-Dev-User", &user_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://localhost:5173"),
    );
    let expose = resp
        .headers()
        .get("access-control-expose-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    assert!(expose.contains("electric-handle"));
    assert!(expose.contains("electric-offset"));

    // 未許可オリジンには ACAO を返さない (ブラウザ側でブロックされる)
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/api/shapes/users")
        .header("Origin", "http://evil.example.com")
        .header("Access-Control-Request-Method", "GET")
        .body(Body::empty())?;

    let resp = app.clone().oneshot(req).await?;
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "disallowed origin must not receive Access-Control-Allow-Origin"
    );

    Ok(())
}

/// #65 回帰テスト (CI 用): http_client に短いタイムアウトが設定されていても、
/// proxy_shape が electric_client を使用しているためタイムアウトエラー (500) にならず正常応答することを確認
#[tokio::test]
async fn test_shape_proxy_uses_electric_client_not_affected_by_http_client_timeout()
-> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mock_electric = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/shape"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(150))
                .insert_header("electric-handle", "mock-handle")
                .insert_header("electric-offset", "0_0"),
        )
        .mount(&mock_electric)
        .await;

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.electric_url = mock_electric.uri();

    let db = init_db(&config.database_url).await?;

    // http_client に 50ms タイムアウトを設定
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_millis(50))
        .build()?;
    // electric_client はタイムアウトなしのデフォルト
    let electric_client = AppState::default_electric_client();
    let jwks_verifier = JwksVerifier::new(http_client.clone());

    let state = AppState::with_clients(db, config, http_client, electric_client, jwks_verifier);
    let app = create_router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/shapes/users?offset=-1")
        .header("X-Dev-User", "00000000-0000-0000-0000-000000000001")
        .body(Body::empty())?;

    // 150ms かかるが、electric_client を使用しているため 50ms でタイムアウトせず 200 OK になる
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("electric-handle")
            .and_then(|v| v.to_str().ok()),
        Some("mock-handle")
    );

    Ok(())
}

/// #65 回帰テスト (実待機): Electric のロングポーリング待機 (既定20秒) において、
/// 従来の 10秒タイムアウトで 500 にならず 11秒後も正常に応答できることを検証する。
/// ※ 実行に 11秒かかるため、通常の CI では ignore とし `cargo test -- --ignored` で実行可能。
#[tokio::test]
#[ignore = "runs for 11s to verify long polling does not time out at 10s (regression test for #65)"]
async fn test_shape_proxy_long_polling_regression_11s() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mock_electric = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/shape"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(11))
                .insert_header("electric-handle", "mock-handle")
                .insert_header("electric-offset", "0_0"),
        )
        .mount(&mock_electric)
        .await;

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.electric_url = mock_electric.uri();

    let db = init_db(&config.database_url).await?;
    // 本番環境と同じ AppState::new を使用 (http_client は 10s タイムアウト、electric_client はタイムアウトなし)
    let state = AppState::new(db, config);
    let app = create_router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/shapes/users?offset=-1&live=true")
        .header("X-Dev-User", "00000000-0000-0000-0000-000000000001")
        .body(Body::empty())?;

    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("electric-handle")
            .and_then(|v| v.to_str().ok()),
        Some("mock-handle")
    );

    Ok(())
}
