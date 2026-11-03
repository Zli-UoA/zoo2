//! Requires the normal migrated test PostgreSQL database.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;

static HOME_IMAGE_GC_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let (_, home) = request(&app, "GET", Some(&bob), "/api/home?timeline=home", None).await?;
    assert!(!home.as_array().unwrap().iter().any(|post| post["id"] == id));
    let (_, social) = request(&app, "GET", Some(&bob), "/api/home?timeline=social", None).await?;
    assert!(
        social
            .as_array()
            .unwrap()
            .iter()
            .any(|post| post["id"] == id)
    );
    let (_, own) = request(&app, "GET", Some(&alice), "/api/home?timeline=home", None).await?;
    assert!(own.as_array().unwrap().iter().any(|post| post["id"] == id));
    let (_, timeline) = request(&app, "GET", Some(&bob), "/api/home?timeline=local", None).await?;
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

#[tokio::test]
async fn user_home_invalidations_require_changed_rows() -> anyhow::Result<()> {
    let config = Config::from_env();
    let (mut client, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(connection);
    let tx = client.transaction().await?;
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'Original')",
        &[&id],
    )
    .await?;
    // Probe actual trigger calls inside a rolled-back transaction. Other sessions
    // continue to see the original function, and no test notification leaks out.
    tx.batch_execute(
        "CREATE TEMP TABLE home_notice_probe(value integer);
        CREATE OR REPLACE FUNCTION notify_home_timeline() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN INSERT INTO pg_temp.home_notice_probe VALUES(1); RETURN NULL; END; $$;",
    )
    .await?;
    tx.execute(
        "UPDATE users SET display_name='No matching row' WHERE id=$1",
        &[&format!("missing-{id}")],
    )
    .await?;
    tx.execute(
        "UPDATE users SET display_name=display_name WHERE id=$1",
        &[&id],
    )
    .await?;
    assert_eq!(
        tx.query_one("SELECT count(*) FROM home_notice_probe", &[])
            .await?
            .get::<_, i64>(0),
        0
    );
    tx.execute(
        "UPDATE users SET display_name='Changed' WHERE id=$1",
        &[&id],
    )
    .await?;
    assert_eq!(
        tx.query_one("SELECT count(*) FROM home_notice_probe", &[])
            .await?
            .get::<_, i64>(0),
        1
    );
    tx.rollback().await?;
    driver.abort();
    Ok(())
}

#[tokio::test]
async fn channel_posts_share_reactions_replies_and_visible_conversation() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let user = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
    let channel = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        &app,
        "POST",
        Some(&user),
        "/api/channels",
        Some(json!({
            "id":channel,"channel_name":format!("conversation-{channel}"),"is_private":false
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (status, root) = request(
        &app,
        "POST",
        Some(&user),
        "/api/home",
        Some(json!({
            "channel_id":channel,"content":"root"
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let root = root["id"].as_str().unwrap();
    let (status, child) = request(
        &app,
        "POST",
        Some(&user),
        "/api/home",
        Some(json!({
            "in_reply_to":root,"content":"child"
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let child = child["id"].as_str().unwrap();
    let (status, grandchild) = request(
        &app,
        "POST",
        Some(&user),
        "/api/messages",
        Some(json!({
            "channel_id":channel,"parent_id":child,"content":"grandchild"
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let grandchild = grandchild["data"]["id"].as_str().unwrap();
    let (status, _) = request(
        &app,
        "POST",
        Some(&user),
        "/api/activitypub/reactions",
        Some(json!({
            "post_id":root,"reaction":"👍"
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (_, post) = request(&app, "GET", Some(&user), &format!("/api/home/{root}"), None).await?;
    assert_eq!(post["reactions"][0]["count"], 1);
    assert_eq!(post["reactions"][0]["selected"], true);
    let (status, _) = request(
        &app,
        "DELETE",
        Some(&user),
        &format!("/api/messages/{root}/reactions?emoji=%F0%9F%91%8D"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (_, post) = request(&app, "GET", Some(&user), &format!("/api/home/{root}"), None).await?;
    assert_eq!(post["likes"], 0);
    let (status, thread) = request(
        &app,
        "GET",
        Some(&user),
        &format!("/api/home/{child}/thread"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<_> = thread
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![root, child, grandchild]);
    assert_eq!(thread[2]["tree_depth"], 2);
    let (_, post) = request(
        &app,
        "GET",
        Some(&user),
        &format!("/api/home/{child}"),
        None,
    )
    .await?;
    assert_eq!(post["parent"]["id"], root);
    let (_, messages) = request(
        &app,
        "GET",
        Some(&user),
        &format!("/api/messages?channel_id={channel}"),
        None,
    )
    .await?;
    assert_eq!(
        messages
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == child)
            .unwrap()["parent_id"],
        root
    );
    // A reply hidden by its audience must not be exposed through the tree or preview.
    let hidden = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(other.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement("INSERT INTO home_posts(id,user_id,content,visibility,parent_id) VALUES($1,$2,'hidden','unlisted',$3)")
        .bind(hidden.as_str()).bind(other.as_str()).bind(root).exec(&mut db).await?;
    let (_, thread) = request(
        &app,
        "GET",
        Some(&user),
        &format!("/api/home/{root}/thread"),
        None,
    )
    .await?;
    assert!(!thread.as_array().unwrap().iter().any(|p| p["id"] == hidden));
    assert_eq!(
        request(
            &app,
            "GET",
            Some(&user),
            &format!("/api/home/{hidden}/thread"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
async fn home_returns_attachment_metadata_for_visible_posts_and_reply_previews()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = uuid::Uuid::new_v4().to_string();
    let viewer = uuid::Uuid::new_v4().to_string();
    for user in [&owner, &viewer] {
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(user.as_str())
            .exec(&mut db)
            .await?;
    }
    let channel = uuid::Uuid::new_v4().to_string();
    let private = uuid::Uuid::new_v4().to_string();
    for (id, is_private) in [(&channel, false), (&private, true)] {
        toasty::sql::statement("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$1,$2,$3::boolean)")
            .bind(id.as_str()).bind(owner.as_str()).bind(is_private.to_string()).exec(&mut db).await?;
    }
    toasty::sql::statement(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
    )
    .bind(owner.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    let image_post = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO home_posts(id,user_id,content,channel_id) VALUES($1,$2,'',$3)",
    )
    .bind(image_post.as_str())
    .bind(owner.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    let private_post = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO messages(id,user_id,content,channel_id) VALUES($1,$2,'',$3)",
    )
    .bind(private_post.as_str())
    .bind(owner.as_str())
    .bind(private.as_str())
    .exec(&mut db)
    .await?;
    let mut attachment_ids = Vec::new();
    for (message, channel_id) in [(&image_post, &channel), (&private_post, &private)] {
        let id = uuid::Uuid::new_v4().to_string();
        toasty::sql::statement("INSERT INTO attachments(id,message_id,uploader_id,channel_id,file_name,file_size,content_type,storage_key) VALUES($1,$2,$3,$4,'photo.png',1024,'image/png',$5)")
            .bind(id.as_str()).bind(message.as_str()).bind(owner.as_str()).bind(channel_id.as_str()).bind(format!("attachments/{id}")).exec(&mut db).await?;
        attachment_ids.push(id);
    }
    let (status, post) = request(
        &app,
        "GET",
        Some(&viewer),
        &format!("/api/home/{image_post}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(post["content"], "");
    assert_eq!(post["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(
        post["attachments"][0],
        json!({"id":attachment_ids[0],"file_name":"photo.png","file_size":1024,"content_type":"image/png"})
    );
    let (status, reply) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"reply to image","in_reply_to":image_post})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let reply_id = reply["id"].as_str().unwrap();
    let (_, reply) = request(
        &app,
        "GET",
        Some(&viewer),
        &format!("/api/home/{reply_id}"),
        None,
    )
    .await?;
    assert_eq!(reply["parent"]["attachments"], post["attachments"]);
    let (_, feed) = request(
        &app,
        "GET",
        Some(&viewer),
        &format!("/api/home?timeline=social&author={owner}"),
        None,
    )
    .await?;
    assert_eq!(
        feed.as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == image_post)
            .unwrap()["attachments"],
        post["attachments"]
    );
    assert!(!feed.to_string().contains(&attachment_ids[1]));
    let (_, thread) = request(
        &app,
        "GET",
        Some(&viewer),
        &format!("/api/home/{reply_id}/thread"),
        None,
    )
    .await?;
    assert_eq!(thread[0]["attachments"], post["attachments"]);
    assert_eq!(
        request(
            &app,
            "GET",
            Some(&viewer),
            &format!("/api/home/{private_post}"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, _) = request(
        &app,
        "DELETE",
        Some(&owner),
        &format!("/api/messages/{image_post}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (_, feed) = request(
        &app,
        "GET",
        Some(&viewer),
        "/api/home?timeline=social",
        None,
    )
    .await?;
    assert!(!feed.to_string().contains(&attachment_ids[0]));
    Ok(())
}

#[tokio::test]
async fn replies_derive_the_channel_from_the_parent_and_nonmembers_can_reply_in_home()
-> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = uuid::Uuid::new_v4().to_string();
    let member = uuid::Uuid::new_v4().to_string();
    let outsider = uuid::Uuid::new_v4().to_string();
    for id in [&owner, &member, &outsider] {
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    let channel = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$1,$2,false)",
    )
    .bind(channel.as_str())
    .bind(owner.as_str())
    .exec(&mut db)
    .await?;
    for user in [&owner, &member] {
        toasty::sql::statement(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
        )
        .bind(user.as_str())
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    }
    let (status, parent) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"Channel parent","channel_id":channel})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let parent_id = parent["id"].as_str().unwrap();
    for (actor, expected_channel) in [(&member, Some(channel.as_str())), (&outsider, None)] {
        let (status, reply)=request(&app,"POST",Some(actor),"/api/home",Some(json!({"content":"reply","in_reply_to":parent_id,"channel_id":"stale-composer-preference"}))).await?;
        assert_eq!(status, StatusCode::CREATED, "{reply}");
        let reply_id = reply["id"].as_str().unwrap();
        let (_, detail) = request(
            &app,
            "GET",
            Some(actor),
            &format!("/api/home/{reply_id}"),
            None,
        )
        .await?;
        assert_eq!(detail["channel_id"], json!(expected_channel));
        assert_eq!(detail["parent_id"], parent_id);
        let mut tx = db.transaction().await?;
        let rows=backend::apps::query(&mut tx,"SELECT json_build_object('source',source)::text FROM notification_events WHERE user_id=$1 AND (message_id=$2 OR post_id=$2)",&[&owner,reply_id]).await?;
        assert_eq!(
            rows.len(),
            1,
            "one logical reply must create one notification"
        );
        assert_eq!(
            rows[0]["source"],
            if expected_channel.is_some() {
                "channel"
            } else {
                "activitypub"
            }
        );
    }
    let (_, plain) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"Home parent"})),
    )
    .await?;
    let (status, reply) = request(
        &app,
        "POST",
        Some(&member),
        "/api/home",
        Some(json!({"content":"Home reply","in_reply_to":plain["id"],"channel_id":channel})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let (_, detail) = request(
        &app,
        "GET",
        Some(&member),
        &format!("/api/home/{}", reply["id"].as_str().unwrap()),
        None,
    )
    .await?;
    assert!(detail["channel_id"].is_null());
    Ok(())
}

#[tokio::test]
async fn reply_link_migration_repairs_legacy_placement_and_missing_descendants()
-> anyhow::Result<()> {
    let config = Config::from_env();
    let (mut client, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(connection);
    let tx = client.transaction().await?;
    let owner = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    let ids: Vec<_> = (0..4).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,'Legacy')",
        &[&owner],
    )
    .await?;
    tx.execute(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1::text,$1::text,$2,false)",
        &[&channel, &owner],
    )
    .await?;
    tx.execute("INSERT INTO home_posts(id,user_id,channel_id,content,visibility) VALUES($1,$2,$3,'parent','unlisted')",&[&ids[0],&owner,&channel]).await?;
    tx.execute("INSERT INTO home_posts(id,user_id,channel_id,content,parent_id,visibility) VALUES($1,$2,$3,'legacy reply',$4,'unlisted')",&[&ids[1],&owner,&channel,&ids[0]]).await?;
    tx.execute("UPDATE messages SET parent_id=NULL WHERE id=$1", &[&ids[1]])
        .await?;
    for index in 2..4 {
        tx.execute("INSERT INTO messages(id,user_id,channel_id,content,parent_id) VALUES($1,$2,$3,'missing descendant',$4)",&[&ids[index],&owner,&channel,&ids[index-1]]).await?;
        tx.execute("DELETE FROM home_posts WHERE id=$1", &[&ids[index]])
            .await?;
        tx.execute(
            "UPDATE messages SET home_post_id=NULL WHERE id=$1",
            &[&ids[index]],
        )
        .await?;
    }
    tx.batch_execute(include_str!(
        "../../infra/db/migrations/20261102150000_repair_reply_links.sql"
    ))
    .await?;
    let parent: String = tx
        .query_one("SELECT parent_id FROM messages WHERE id=$1", &[&ids[1]])
        .await?
        .get(0);
    assert_eq!(parent, ids[0]);
    for index in 2..4 {
        let row = tx
            .query_one(
                "SELECT parent_id,visibility,apub,reply_user_id FROM home_posts WHERE id=$1",
                &[&ids[index]],
            )
            .await?;
        assert_eq!(row.get::<_, String>(0), ids[index - 1]);
        assert_eq!(row.get::<_, String>(1), "unlisted");
        assert!(!row.get::<_, bool>(2));
        assert_eq!(row.get::<_, String>(3), owner);
    }
    // An edit after the migration no longer changes the already-reconciled placement.
    tx.execute(
        "UPDATE messages SET content='edited' WHERE id=$1",
        &[&ids[1]],
    )
    .await?;
    let parent: String = tx
        .query_one("SELECT parent_id FROM messages WHERE id=$1", &[&ids[1]])
        .await?
        .get(0);
    assert_eq!(parent, ids[0]);
    tx.rollback().await?;
    driver.abort();
    Ok(())
}

fn test_home_png(width: u32, height: u32) -> String {
    use base64::Engine;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(width, height)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
}

#[tokio::test]
async fn home_images_are_owned_and_survive_image_only_posts() -> anyhow::Result<()> {
    let storage = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .and(wiremock::matchers::header("content-type", "image/png"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .expect(1)
        .mount(&storage)
        .await;
    let mut config = Config::from_env();
    config.s3.endpoint = storage.uri();
    config.s3.force_path_style = true;
    config.s3.access_key_id = "test-key".into();
    config.s3.secret_access_key = "test-secret".into();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    for id in [&owner, &other] {
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    let (status, uploaded) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home/images",
        Some(json!({"name":"photo.png","content_type":"image/png","data":test_home_png(2,2)})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let image = uploaded["id"].as_str().unwrap().to_owned();
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&owner),
            "/api/home/images",
            Some(json!({"name":"bad.png","content_type":"image/png","data":"bm90IGFuIGltYWdl"}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    for (actor, ids) in [
        (&other, vec![image.clone()]),
        (&owner, vec![image.clone(), image.clone()]),
        (&owner, vec![uuid::Uuid::new_v4().to_string()]),
        (&owner, vec![image.clone(); 11]),
    ] {
        assert_eq!(
            request(
                &app,
                "POST",
                Some(actor),
                "/api/home",
                Some(json!({"content":"","image_ids":ids}))
            )
            .await?
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (status, created) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"","image_ids":[image]})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let path = format!("/api/home/{}", created["id"].as_str().unwrap());
    let (status, post) = request(&app, "GET", Some(&owner), &path, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(post["content"], "");
    assert_eq!(post["media"][0]["name"], "photo.png");
    assert_eq!(post["media"][0]["url"], format!("/api/home/images/{image}"));
    let (status, _) = request(
        &app,
        "PATCH",
        Some(&owner),
        &path,
        Some(json!({"content":"caption"})),
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, post) = request(&app, "GET", Some(&owner), &path, None).await?;
    assert_eq!(post["media"].as_array().unwrap().len(), 1);
    let channel = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$1,$2,false)",
    )
    .bind(channel.as_str())
    .bind(owner.as_str())
    .exec(&mut db)
    .await?;
    toasty::sql::statement(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
    )
    .bind(owner.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    let (status, crosspost) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"","image_ids":[image],"channel_id":channel})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let attachments = backend::models::Attachment::filter(
        backend::models::Attachment::fields()
            .message_id()
            .eq(crosspost["id"].as_str().unwrap()),
    )
    .exec(&mut db)
    .await?;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].storage_key, format!("home-images/{image}"));
    assert!(attachments[0].file_size > 0);
    // Simulate an existing channel image whose Home row predates media metadata.
    toasty::sql::statement("UPDATE home_posts SET media='[]'::jsonb WHERE id=$1")
        .bind(crosspost["id"].as_str().unwrap())
        .exec(&mut db)
        .await?;
    let (_, restored) = request(
        &app,
        "GET",
        Some(&owner),
        &format!("/api/home/{}", crosspost["id"].as_str().unwrap()),
        None,
    )
    .await?;
    assert_eq!(restored["media"][0]["name"], "photo.png");
    let existing_image_url = format!("/api/home/images/{}", attachments[0].id);
    assert_eq!(restored["media"][0]["url"], existing_image_url);
    assert_eq!(
        request(&app, "GET", None, &existing_image_url, None)
            .await?
            .0,
        StatusCode::TEMPORARY_REDIRECT
    );
    toasty::sql::statement("UPDATE channels SET is_private=true WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        request(&app, "GET", None, &existing_image_url, None)
            .await?
            .0,
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        request(
            &app,
            "GET",
            None,
            &format!("/api/home/images/{image}"),
            None
        )
        .await?
        .0,
        StatusCode::TEMPORARY_REDIRECT
    );
    let (status, federated) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"","image_ids":[image],"apub":true,"visibility":"public"})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let (_, note) = request(
        &app,
        "GET",
        None,
        &format!(
            "/api/activitypub/notes/{}",
            federated["id"].as_str().unwrap()
        ),
        None,
    )
    .await?;
    assert_eq!(
        note["attachment"][0]["url"],
        format!(
            "{}/api/home/images/{image}",
            backend::activitypub::base_url()?
        )
    );
    assert_eq!(note["attachment"][0]["mediaType"], "image/png");
    Ok(())
}

#[tokio::test]
async fn home_image_limits_discard_and_gc_preserve_live_references() -> anyhow::Result<()> {
    let _gc_test = HOME_IMAGE_GC_TESTS.lock().await;
    let storage = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&storage)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("DELETE"))
        .respond_with(wiremock::ResponseTemplate::new(204))
        .mount(&storage)
        .await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.s3.endpoint = storage.uri();
    config.s3.force_path_style = true;
    config.s3.access_key_id = "test-key".into();
    config.s3.secret_access_key = "test-secret".into();
    let mut db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    for id in [&owner, &other] {
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    let valid = json!({"name":"photo.png","content_type":"image/png","data":test_home_png(2,2)});
    let mut mismatch = valid.clone();
    mismatch["content_type"] = json!("image/jpeg");
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&owner),
            "/api/home/images",
            Some(mismatch)
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut oversized = valid.clone();
    {
        use base64::Engine;
        oversized["data"] =
            json!(base64::engine::general_purpose::STANDARD.encode(vec![0u8; 8 * 1024 * 1024 + 1]));
    }
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&owner),
            "/api/home/images",
            Some(oversized)
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut dimensions = valid.clone();
    dimensions["data"] = json!(test_home_png(8193, 1));
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&owner),
            "/api/home/images",
            Some(dimensions)
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut pixels = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_luma8(5000, 5000).write_to(&mut pixels, image::ImageFormat::Png)?;
    {
        use base64::Engine;
        let mut body = valid.clone();
        body["data"] = json!(base64::engine::general_purpose::STANDARD.encode(pixels.into_inner()));
        assert_eq!(
            request(&app, "POST", Some(&owner), "/api/home/images", Some(body))
                .await?
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let (_, draft) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home/images",
        Some(valid.clone()),
    )
    .await?;
    let draft_id = draft["id"].as_str().unwrap();
    let draft_url = format!("/api/home/images/{draft_id}");
    assert_eq!(
        request(&app, "GET", None, &draft_url, None).await?.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "DELETE", Some(&other), &draft_url, None)
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "DELETE", Some(&owner), &draft_url, None)
            .await?
            .0,
        StatusCode::NO_CONTENT
    );
    // Limit the failure to one call; scoped mocks with different priorities can
    // deactivate the wrong registration in wiremock 0.6.5 after its internal sort.
    wiremock::Mock::given(wiremock::matchers::method("DELETE"))
        .and(wiremock::matchers::path(format!(
            "/{}/home-images/{draft_id}",
            state.config.s3.bucket
        )))
        .respond_with(wiremock::ResponseTemplate::new(403))
        .with_priority(1)
        .up_to_n_times(1)
        .expect(1)
        .mount(&storage)
        .await;
    assert!(
        backend::routes::home_images::collect_garbage(&state)
            .await
            .is_err()
    );
    let rows = backend::apps::query(
        &mut db.transaction().await?,
        "SELECT json_build_object('id',id)::text FROM home_image_deletions WHERE id=$1",
        &[draft_id],
    )
    .await?;
    assert_eq!(rows.len(), 1, "failed S3 deletion must remain queued");
    backend::routes::home_images::collect_garbage(&state).await?;
    let remaining = backend::apps::query(
        &mut db.transaction().await?,
        "SELECT json_build_object('id',id)::text FROM home_image_deletions WHERE id=$1",
        &[draft_id],
    )
    .await?;
    assert!(
        remaining.is_empty(),
        "successful retry must remove the queued deletion"
    );
    assert!(
        storage
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method == "DELETE" && r.url.path().ends_with(draft_id))
    );
    let (_, abandoned) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home/images",
        Some(valid.clone()),
    )
    .await?;
    let abandoned_id = abandoned["id"].as_str().unwrap();
    toasty::sql::statement(
        "UPDATE home_images SET created_at=now()-interval '25 hours' WHERE id=$1",
    )
    .bind(abandoned_id)
    .exec(&mut db)
    .await?;
    backend::routes::home_images::collect_garbage(&state).await?;
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&owner),
            &format!("/api/home/images/{abandoned_id}"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, live) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home/images",
        Some(valid.clone()),
    )
    .await?;
    let live_id = live["id"].as_str().unwrap();
    let live_url = format!("/api/home/images/{live_id}");
    let (status, post) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"","image_ids":[live_id]})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    toasty::sql::statement(
        "UPDATE home_images SET created_at=now()-interval '25 hours' WHERE id=$1",
    )
    .bind(live_id)
    .exec(&mut db)
    .await?;
    backend::routes::home_images::collect_garbage(&state).await?;
    assert_eq!(
        request(&app, "GET", None, &live_url, None).await?.0,
        StatusCode::TEMPORARY_REDIRECT
    );
    assert_eq!(
        request(&app, "DELETE", Some(&owner), &live_url, None)
            .await?
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&owner),
            &format!("/api/home/{}", post["id"].as_str().unwrap()),
            None
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "GET", None, &live_url, None).await?.0,
        StatusCode::NOT_FOUND
    );
    backend::routes::home_images::collect_garbage(&state).await?;
    assert!(
        storage
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method == "DELETE" && r.url.path().ends_with(live_id))
    );
    let full = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO home_images(id,uploader_id,name,content_type,file_size) VALUES($1,$2,'quota.png','image/png',1073741824)").bind(full.as_str()).bind(owner.as_str()).exec(&mut db).await?;
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&owner),
            "/api/home/images",
            Some(valid.clone())
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&owner),
            &format!("/api/home/images/{full}"),
            None
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    // Pending deletion remains in the quota until object storage confirms removal.
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&owner),
            "/api/home/images",
            Some(valid.clone())
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    backend::routes::home_images::collect_garbage(&state).await?;
    toasty::sql::statement("INSERT INTO home_image_upload_rates(user_id,window_start,requests) SELECT $1,date_trunc('minute',now())+offsets.n*interval '1 minute',30 FROM generate_series(0,1) offsets(n) ON CONFLICT(user_id,window_start) DO UPDATE SET requests=30").bind(owner.as_str()).exec(&mut db).await?;
    assert_eq!(
        request(&app, "POST", Some(&owner), "/api/home/images", Some(valid))
            .await?
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    Ok(())
}

#[tokio::test]
async fn home_images_follow_channel_visibility_and_selection_order() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(owner.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement(
        "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$1,$2,false)",
    )
    .bind(channel.as_str())
    .bind(owner.as_str())
    .exec(&mut db)
    .await?;
    toasty::sql::statement(
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
    )
    .bind(owner.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    let ids = (0..3)
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect::<Vec<_>>();
    for (position, id) in ids.iter().enumerate() {
        toasty::sql::statement("INSERT INTO home_images(id,uploader_id,name,content_type,file_size) VALUES($1,$2,$3,'image/png',80)").bind(id.as_str()).bind(owner.as_str()).bind(format!("{position}.png")).exec(&mut db).await?;
    }
    let selected = vec![ids[2].clone(), ids[0].clone(), ids[1].clone()];
    let (status, post) = request(
        &app,
        "POST",
        Some(&owner),
        "/api/home",
        Some(json!({"content":"","image_ids":selected,"channel_id":channel,"visibility":"public"})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let post_id = post["id"].as_str().unwrap();
    let (_, detail) = request(
        &app,
        "GET",
        Some(&owner),
        &format!("/api/home/{post_id}"),
        None,
    )
    .await?;
    assert_eq!(
        detail["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["file_name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["2.png", "0.png", "1.png"]
    );
    let image_url = format!("/api/home/images/{}", ids[0]);
    assert_eq!(
        request(&app, "GET", None, &image_url, None).await?.0,
        StatusCode::TEMPORARY_REDIRECT
    );
    toasty::sql::statement("UPDATE channels SET is_private=true WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        request(&app, "GET", None, &image_url, None).await?.0,
        StatusCode::NOT_FOUND
    );
    // Defensively cover legacy/inconsistent federated records in a private channel.
    // Imported/replicated legacy rows can predate the current write guards.
    let mut imported = db.transaction().await?;
    toasty::sql::statement("SET LOCAL session_replication_role=replica")
        .exec(&mut imported)
        .await?;
    toasty::sql::statement("UPDATE home_posts SET apub=true WHERE id=$1")
        .bind(post_id)
        .exec(&mut imported)
        .await?;
    imported.commit().await?;
    let (status, profile) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{owner}/profile"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !profile["posts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == post_id)
    );
    let (_, outbox) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{owner}/outbox?page=true"),
        None,
    )
    .await?;
    assert!(outbox["orderedItems"].as_array().unwrap().is_empty());
    assert_eq!(
        request(
            &app,
            "GET",
            None,
            &format!("/api/activitypub/notes/{post_id}"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
async fn home_image_upload_failure_and_interruption_leave_durable_cleanup() -> anyhow::Result<()> {
    let _gc_test = HOME_IMAGE_GC_TESTS.lock().await;
    let storage = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .respond_with(wiremock::ResponseTemplate::new(403))
        .mount(&storage)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("DELETE"))
        .respond_with(wiremock::ResponseTemplate::new(204))
        .mount(&storage)
        .await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.s3.endpoint = storage.uri();
    config.s3.force_path_style = true;
    config.s3.access_key_id = "test-key".into();
    config.s3.secret_access_key = "test-secret".into();
    let mut db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state.clone());
    let owner = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(owner.as_str())
        .exec(&mut db)
        .await?;
    let body = json!({"name":"photo.png","content_type":"image/png","data":test_home_png(2,2)});
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&owner),
            "/api/home/images",
            Some(body.clone())
        )
        .await?
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let queued = backend::apps::query(
        &mut db.transaction().await?,
        "SELECT json_build_object('id',id)::text FROM home_image_deletions WHERE uploader_id=$1",
        &[&owner],
    )
    .await?;
    assert_eq!(queued.len(), 1, "even a failed PUT must be durably queued");
    backend::routes::home_images::collect_garbage(&state).await?;
    let queued = backend::apps::query(
        &mut db.transaction().await?,
        "SELECT json_build_object('id',id)::text FROM home_image_deletions WHERE uploader_id=$1",
        &[&owner],
    )
    .await?;
    assert!(queued.is_empty());
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(2)),
        )
        .with_priority(1)
        .mount(&storage)
        .await;
    let requests_before = storage.received_requests().await.unwrap().len();
    let mut uploading = Vec::new();
    let mut reserved = Vec::new();
    // Three PUTs must be able to start while the earlier PUTs are still delayed:
    // completed raster validation must not retain either of the two decode permits.
    for expected in 1..=3 {
        uploading.push(tokio::spawn({
            let app = app.clone();
            let owner = owner.clone();
            let body = body.clone();
            async move { request(&app, "POST", Some(&owner), "/api/home/images", Some(body)).await }
        }));
        reserved = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let rows = backend::apps::query(
                    &mut db.transaction().await?,
                    "SELECT json_build_object('id',id)::text FROM home_images WHERE uploader_id=$1",
                    &[&owner],
                )
                .await?;
                if rows.len() == expected
                    && storage.received_requests().await.unwrap().len()
                        >= requests_before + expected
                {
                    break Ok::<_, anyhow::Error>(
                        rows.into_iter()
                            .map(|r| r["id"].as_str().unwrap().to_owned())
                            .collect::<Vec<_>>(),
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await??;
    }
    for task in uploading {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    // Model stopping while PUT is in progress: committed reservations survive rollback.
    toasty::sql::statement(
        "UPDATE home_images SET created_at=now()-interval '25 hours' WHERE uploader_id=$1",
    )
    .bind(owner.as_str())
    .exec(&mut db)
    .await?;
    backend::routes::home_images::collect_garbage(&state).await?;
    let requests = storage.received_requests().await.unwrap();
    for id in reserved {
        assert!(
            requests
                .iter()
                .any(|r| r.method == "DELETE" && r.url.path().ends_with(&id))
        );
    }
    Ok(())
}

#[tokio::test]
async fn local_home_posts_include_joined_workspace_emoji_dimensions() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let id = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(id.as_str())
        .exec(&mut db)
        .await?;
    let name = format!("wide-{}", uuid::Uuid::new_v4().simple());
    let alias = format!("alias-{}", uuid::Uuid::new_v4().simple());
    toasty::sql::statement("INSERT INTO workspace_emojis(name,url,aliases,width,height,created_by) VALUES($1,'/api/emoji-images/test',jsonb_build_array($2::text),120,20,$3)")
        .bind(name.as_str()).bind(alias.as_str()).bind(id.as_str()).exec(&mut db).await?;
    let (status, post) = request(
        &app,
        "POST",
        Some(&id),
        "/api/home",
        Some(json!({"content":format!(":{alias}:"),"visibility":"public"})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let (_, rows) = request(&app, "GET", Some(&id), "/api/home?timeline=local", None).await?;
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == post["id"])
        .unwrap();
    let emoji = &row["emojis"][0];
    assert_eq!(emoji["name"], format!(":{alias}:"));
    assert_eq!(emoji["width"], 120);
    assert_eq!(emoji["height"], 20);
    assert_eq!(emoji["local"], true);
    let local_url = emoji["url"].as_str().unwrap();
    let mut tx = db.transaction().await?;
    let count = backend::apps::one(
        &mut tx,
        "SELECT json_build_object('count',count(*))::text FROM apub_emoji_images WHERE url=$1",
        &[local_url],
    )
    .await?;
    assert_eq!(
        count["count"], 0,
        "local descriptors must not enter the remote proxy registry"
    );
    // Model an invalid row left by the prior backend, then successful startup
    // identity backfill. Both list and detail must keep using the direct URL.
    toasty::sql::statement("INSERT INTO apub_emoji_images(id,url) VALUES('', $1)")
        .bind(local_url)
        .exec(&mut tx)
        .await?;
    toasty::sql::statement("UPDATE workspace_emojis SET image_id=$2 WHERE name=$1")
        .bind(name.as_str())
        .bind("b".repeat(64))
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    for path in [
        "/api/home?timeline=local".to_owned(),
        format!("/api/home/{}", post["id"].as_str().unwrap()),
    ] {
        let (status, _) = request(&app, "GET", Some(&id), &path, None).await?;
        assert_eq!(
            status,
            StatusCode::OK,
            "local emoji identity backfill: {path}"
        );
    }
    let mut tx = db.transaction().await?;
    let count = backend::apps::one(
        &mut tx,
        "SELECT json_build_object('count',count(*))::text FROM apub_emoji_images WHERE url=$1",
        &[local_url],
    )
    .await?;
    assert_eq!(
        count["count"], 1,
        "backfill must not attempt another proxy registration"
    );
    toasty::sql::statement("DELETE FROM apub_emoji_images WHERE id='' AND url=$1")
        .bind(local_url)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    toasty::sql::statement("DELETE FROM workspace_emojis WHERE name=$1")
        .bind(name)
        .exec(&mut db)
        .await?;
    Ok(())
}
