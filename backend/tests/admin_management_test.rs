use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    auth::DEFAULT_DEV_USER_ID, config::Config, db::init_db, routes::create_router, state::AppState,
};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn request(
    app: &axum::Router,
    method: &str,
    path: &str,
    actor: &str,
    payload: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("X-Dev-User", actor)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

#[tokio::test]
async fn administrator_roles_require_authorization_and_take_effect_immediately()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let bot = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob, &bot] {
        assert_eq!(request(&app, "POST", "/api/users", DEFAULT_DEV_USER_ID,
            json!({"id":id,"user_name":format!("admin_test_{}",id.replace('-', "")),"display_name":"Role test"})).await?.0, StatusCode::OK);
    }
    let alice_role = format!("/api/users/{alice}/admin");
    let bob_role = format!("/api/users/{bob}/admin");
    assert_eq!(
        request(&app, "PATCH", &alice_role, &alice, json!({"is_admin":true}))
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    let promoted = request(
        &app,
        "PATCH",
        &alice_role,
        DEFAULT_DEV_USER_ID,
        json!({"is_admin":true}),
    )
    .await?;
    assert_eq!(promoted.0, StatusCode::OK);
    assert_eq!(promoted.1["data"]["is_admin"], true);
    assert!(promoted.1["txid"].is_string());
    assert_eq!(
        request(&app, "GET", "/api/users/me", &alice, json!(null))
            .await?
            .1["is_admin"],
        true
    );
    assert_eq!(
        request(&app, "PATCH", &bob_role, &alice, json!({"is_admin":true}))
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "PATCH", &bob_role, &alice, json!({"is_admin":true}))
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "PATCH", &alice_role, &bob, json!({"is_admin":false}))
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", "/api/users/me", &alice, json!(null))
            .await?
            .1["is_admin"],
        false
    );
    assert_eq!(
        request(&app, "PATCH", &alice_role, &alice, json!({"is_admin":true}))
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    toasty::sql::statement("UPDATE users SET is_bot=true WHERE id=$1")
        .bind(bot.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("/api/users/{bot}/admin"),
            &bob,
            json!({"is_admin":true})
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name,is_bot,created_at) VALUES($1,'fixed_development_admin','Development administrator',false,now()) ON CONFLICT(id) DO NOTHING")
        .bind(DEFAULT_DEV_USER_ID).exec(&mut db).await?;
    assert_eq!(
        request(
            &app,
            "PATCH",
            &format!("/api/users/{DEFAULT_DEV_USER_ID}/admin"),
            &bob,
            json!({"is_admin":false})
        )
        .await?
        .0,
        StatusCode::CONFLICT
    );
    Ok(())
}
