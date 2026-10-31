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
async fn search_preferences_are_personal_validated_and_preserve_other_preferences()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob] {
        assert_eq!(request(&app, "POST", "/api/users", DEFAULT_DEV_USER_ID,
            json!({"id":id,"user_name":format!("search_{}",id.replace('-', "")),"display_name":"Search test"})).await?.0, StatusCode::OK);
    }
    let path = "/api/users/me/preferences";
    assert_eq!(
        request(&app, "GET", path, &alice, json!(null)).await?.1["search_engine"],
        "google"
    );
    {
        let mut tx = db.transaction().await?;
        toasty::sql::statement("INSERT INTO user_event_settings(user_id,preferences,presence_override) VALUES($1,'{\"other_app_setting\":{\"enabled\":true}}','away')")
            .bind(alice.as_str()).exec(&mut tx).await?;
        tx.commit().await?;
    }
    for engine in ["google", "duckduckgo", "yandex", "baidu"] {
        let (status, result) =
            request(&app, "PATCH", path, &alice, json!({"search_engine":engine})).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["data"]["search_engine"], engine);
        assert!(result["txid"].is_string());
        assert_eq!(
            request(&app, "GET", path, &alice, json!(null)).await?.1["search_engine"],
            engine
        );
        assert_eq!(
            request(&app, "GET", path, &bob, json!(null)).await?.1["search_engine"],
            "google"
        );
    }
    for invalid in [
        json!({"search_engine":"https://evil.example"}),
        json!({"search_engine":"Google"}),
        json!({"search_engine":"yandex","user_id":bob}),
        json!({}),
    ] {
        // Axum's JSON rejection body is text rather than our JSON error envelope.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("X-Dev-User", &alice)
                    .body(Body::from(serde_json::to_vec(&invalid)?))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let mut tx = db.transaction().await?;
    let settings = backend::apps::one(&mut tx,
        "SELECT jsonb_build_object('preferences',preferences::jsonb,'presence_override',presence_override)::text FROM user_event_settings WHERE user_id=$1", &[&alice]).await?;
    assert_eq!(
        settings["preferences"]["other_app_setting"]["enabled"],
        true
    );
    assert_eq!(settings["preferences"]["search_engine"], "baidu");
    assert_eq!(settings["presence_override"], "away");
    tx.commit().await?;
    // New requests read the durable stored value rather than browser state.
    assert_eq!(
        request(&app, "GET", path, &alice, json!(null)).await?.1["search_engine"],
        "baidu"
    );
    let mut production_config = Config::from_env();
    production_config.dev_auth_enabled = false;
    let authenticated_app = create_router(AppState::new(db, production_config));
    for method in ["GET", "PATCH"] {
        assert_eq!(
            request(
                &authenticated_app,
                method,
                path,
                &alice,
                json!({"search_engine":"google"})
            )
            .await?
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    Ok(())
}
