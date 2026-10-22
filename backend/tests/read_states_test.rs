use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("X-Dev-User", actor)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn server_read_states_are_private_persistent_and_monotonic() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let owner = Uuid::new_v4().to_string();
    let reader = Uuid::new_v4().to_string();
    for actor in [&owner, &reader] {
        assert_eq!(request(&app, "POST", "/api/users", actor, json!({"id":actor,"user_name":format!("read_{}",Uuid::new_v4().simple()),"display_name":"Reader"})).await.0, StatusCode::OK);
    }
    let channel = Uuid::new_v4().to_string();
    let private = Uuid::new_v4().to_string();
    for (id, private) in [(&channel, false), (&private, true)] {
        assert_eq!(request(&app, "POST", "/api/channels", &owner, json!({"id":id,"channel_name":format!("read_{}",Uuid::new_v4().simple()),"is_private":private})).await.0, StatusCode::OK);
    }
    let path = format!("/api/channels/{channel}/read-state");
    let private_path = format!("/api/channels/{private}/read-state");
    assert_eq!(
        request(&app, "GET", &private_path, &reader, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &private_path,
            &reader,
            json!({"message_id":Uuid::new_v4().to_string()})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/channels/{channel}/join"),
            &reader,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let first = Uuid::new_v4().to_string();
    let second = Uuid::new_v4().to_string();
    let own = Uuid::new_v4().to_string();
    for (id, actor, seconds) in [
        (&first, &owner, "01"),
        (&second, &owner, "02"),
        (&own, &reader, "03"),
    ] {
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/messages",
                actor,
                json!({"id":id,"channel_id":channel,"content":"hello"})
            )
            .await
            .0,
            StatusCode::OK
        );
        toasty::sql::statement(
            "UPDATE messages SET created_at = $1::text::timestamp WHERE id = $2",
        )
        .bind(format!("2026-01-01 00:00:{seconds}"))
        .bind(id.as_str())
        .exec(&mut db)
        .await?;
    }
    let (_, initial) = request(&app, "GET", &path, &reader, Value::Null).await;
    assert_eq!(initial["has_unread"], true);
    assert!(initial["last_read_at"].is_null());
    let (_, listed) = request(
        &app,
        "GET",
        "/api/channel-read-states",
        &reader,
        Value::Null,
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["channel_id"], channel);
    assert_eq!(listed[0]["has_unread"], true);
    let (status, saved) = request(&app, "POST", &path, &reader, json!({"message_id":second})).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["has_unread"], false); // own later message is not unread
    let (_, older) = request(&app, "POST", &path, &reader, json!({"message_id":first})).await;
    assert_eq!(older["data"]["last_read_at"], saved["data"]["last_read_at"]);
    let another_app = create_router(AppState::new(init_db(&config.database_url).await?, config));
    let (_, persisted) = request(&another_app, "GET", &path, &reader, Value::Null).await;
    assert_eq!(persisted, saved["data"]);
    let (_, other_user) = request(&app, "GET", &path, &owner, Value::Null).await;
    assert!(other_user["last_read_at"].is_null());
    assert_eq!(other_user["has_unread"], true);
    let foreign = Uuid::new_v4().to_string();
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/messages",
            &owner,
            json!({"id":foreign,"channel_id":private,"content":"private"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "POST", &path, &reader, json!({"message_id":foreign}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            &reader,
            json!({"message_id":"invalid"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let reply = Uuid::new_v4().to_string();
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/messages",
            &owner,
            json!({"id":reply,"channel_id":channel,"content":"reply","parent_id":first})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "GET", &path, &reader, Value::Null).await.1["has_unread"],
        false
    );
    assert_eq!(
        request(&app, "POST", &path, &reader, json!({"message_id":reply}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}
