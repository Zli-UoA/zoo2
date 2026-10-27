use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::Value;
use tower::ServiceExt;

async fn request(
    app: &Router,
    actor: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("X-Dev-User", actor)
        .header("content-type", "application/json")
        .body(match body {
            Some(body) => Body::from(serde_json::to_vec(&body)?),
            None => Body::empty(),
        })?;
    let response = app.clone().oneshot(req).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let json_val = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Ok((status, json_val))
}

#[tokio::test]
async fn test_calls_routes_authorization_and_lifecycle() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));

    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let outsider = uuid::Uuid::new_v4().to_string();
    let public_ch = uuid::Uuid::new_v4().to_string();
    let private_ch = uuid::Uuid::new_v4().to_string();
    let archived_ch = uuid::Uuid::new_v4().to_string();

    let mut tx = db.transaction().await?;
    for (id, name) in [(&alice, "Alice"), (&bob, "Bob"), (&outsider, "Outsider")] {
        toasty::sql::statement(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $2)",
        )
        .bind(id.as_str())
        .bind(name)
        .exec(&mut tx)
        .await?;
    }

    // Public channel
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, false)",
    )
    .bind(public_ch.as_str())
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;

    // Private channel (alice & bob members)
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $1, $2, true)",
    )
    .bind(private_ch.as_str())
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;
    for member in [&alice, &bob] {
        toasty::sql::statement(
            "INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, CURRENT_TIMESTAMP)",
        )
        .bind(member.as_str())
        .bind(private_ch.as_str())
        .exec(&mut tx)
        .await?;
    }

    // Archived channel
    toasty::sql::statement(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private, archive_at) VALUES ($1, $1, $2, false, CURRENT_TIMESTAMP)",
    )
    .bind(archived_ch.as_str())
    .bind(alice.as_str())
    .exec(&mut tx)
    .await?;

    tx.commit().await?;

    // 1. Initial status: public channel call should be inactive
    let (status, body) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{public_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_active"], false);
    assert_eq!(body["participant_count"], 0);
    assert_eq!(body["max_participants"], 6);
    assert!(body["ice_servers"].is_array());

    // 2. Private channel: outsider should get 404
    let (status, _) = request(
        &app,
        &outsider,
        "GET",
        &format!("/api/channels/{private_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 3. Member of private channel gets 200
    let (status, body) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{private_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_active"], false);

    // 4. Archived channel: returns 403 Forbidden
    let (status, _) = request(
        &app,
        &alice,
        "GET",
        &format!("/api/channels/{archived_ch}/call"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 5. Active calls summary
    let (status, body) = request(&app, &alice, "GET", "/api/calls/active", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());

    Ok(())
}
