//! Requires the normal migrated test PostgreSQL database.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn request(
    app: &Router,
    method: &str,
    actor: Option<&str>,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(actor) = actor {
        request = request.header("X-Dev-User", actor);
    }
    let response = app
        .clone()
        .oneshot(request.body(body.map_or(Body::empty(), |body| Body::from(body.to_string())))?)
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

#[tokio::test]
async fn public_posts_are_separate_and_owner_deleted() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob] {
        toasty::sql::statement(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $1, $1)",
        )
        .bind(id.as_str())
        .exec(&mut db)
        .await?;
    }
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&alice),
            "/api/home",
            Some(json!({"content":" \n"}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&alice),
            "/api/home",
            Some(json!({"content":"あ".repeat(1001)}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, created) = request(
        &app,
        "POST",
        Some(&alice),
        "/api/home",
        Some(json!({"content":"<b>公開の近況</b>","apub":true,"visibility":"public"})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap();
    let (_, timeline) = request(&app, "GET", Some(&bob), "/api/home", None).await?;
    assert!(
        timeline
            .as_array()
            .unwrap()
            .iter()
            .any(|post| post["id"] == id && post["content"] == "<b>公開の近況</b>")
    );
    let note_path = format!("/api/activitypub/notes/{id}");
    let (status, note) = request(&app, "GET", None, &note_path, None).await?;
    assert_eq!(
        status,
        StatusCode::OK,
        "Set ACTIVITYPUB_BASE_URL for this test"
    );
    assert_eq!(note["content"], "&lt;b&gt;公開の近況&lt;&#x2F;b&gt;");
    let (_, outbox) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{alice}/outbox?page=true"),
        None,
    )
    .await?;
    assert_eq!(outbox["orderedItems"][0]["object"]["id"], note["id"]);
    // Another user cannot delete the post, even with a known ID.
    assert_eq!(
        request(&app, "DELETE", Some(&bob), &format!("/api/home/{id}"), None)
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "GET", None, &note_path, None).await?.0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&alice),
            &format!("/api/home/{id}"),
            None
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "GET", None, &note_path, None).await?.0,
        StatusCode::GONE
    );
    // Cursor pagination never returns the cursor row again.
    for index in 0..51 {
        toasty::sql::statement("INSERT INTO home_posts (id, user_id, content, apub, visibility) VALUES ($1, $2, $1, true, 'public')")
            .bind(format!("{alice}-{index:03}"))
            .bind(alice.as_str())
            .exec(&mut db)
            .await?;
    }
    let (_, page) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{alice}/outbox?page=true"),
        None,
    )
    .await?;
    assert_eq!(page["orderedItems"].as_array().unwrap().len(), 50);
    let next = page["next"].as_str().unwrap();
    let path = reqwest::Url::parse(next)?;
    let (_, older) = request(
        &app,
        "GET",
        None,
        &format!("{}?{}", path.path(), path.query().unwrap()),
        None,
    )
    .await?;
    assert_eq!(older["orderedItems"].as_array().unwrap().len(), 1);
    for id in [&alice, &bob] {
        toasty::sql::statement("DELETE FROM users WHERE id = $1")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn live_updates_follow_committed_database_changes_and_require_authentication()
-> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let alice = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO users (id,user_name,display_name) VALUES ($1,$1,'Live profile')",
    )
    .bind(alice.as_str())
    .exec(&mut db)
    .await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let served = app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/home/live")).await?;
    socket
        .send(Message::Text(
            json!({"x-dev-user":alice}).to_string().into(),
        ))
        .await?;
    // First snapshot request, then listener-ready snapshot: closes the LISTEN startup race.
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = frame {
                    assert_eq!(text, "refresh");
                    break;
                }
                socket.flush().await.unwrap();
            }
        })
        .await?;
    }
    let post = uuid::Uuid::new_v4().to_string();
    for sql in [
        "INSERT INTO home_posts(id,user_id,content) VALUES($1,$2,'private contents stay out of the socket')",
        "UPDATE home_posts SET content='edited' WHERE id=$1 AND user_id=$2",
        "DELETE FROM home_posts WHERE id=$1 AND user_id=$2",
    ] {
        // Write through a separate DB connection, as another backend or an inbox worker would.
        let (external, connection) =
            tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
        let driver = tokio::spawn(connection);
        external.execute(sql, &[&post, &alice]).await?;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = frame {
                    assert_eq!(text, "refresh");
                    break;
                }
                socket.flush().await.unwrap();
            }
        })
        .await?;
        driver.abort();
    }
    let (status, users) = request(
        &app,
        "GET",
        Some(&alice),
        &format!("/api/activitypub/search?q={alice}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(users[0]["display_name"], "Live profile");
    assert!(users[0].get("public_key").is_none());
    assert_eq!(users[0]["remote"], false);
    socket.close(None).await?;
    server.abort();

    config.dev_auth_enabled = false;
    config.oidc = None;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let denied_app = create_router(AppState::new(db.clone(), config));
    let server = tokio::spawn(async move { axum::serve(listener, denied_app).await.unwrap() });
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/api/home/live")).await?;
    socket
        .send(Message::Text(
            json!({"x-dev-user":alice}).to_string().into(),
        ))
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(frame)) = socket.next().await {
            assert!(!frame.is_text(), "unauthenticated socket received an event");
        }
    })
    .await?;
    server.abort();
    toasty::sql::statement("DELETE FROM users WHERE id=$1")
        .bind(alice)
        .exec(&mut db)
        .await?;
    Ok(())
}
