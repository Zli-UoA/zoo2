use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use backend::{
    config::Config,
    db::init_db,
    models::{Channel, ChannelUser, Message, Notification, User, now_utc_millis},
    routes::create_router,
    state::AppState,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    user: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value, HeaderMap)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("X-Dev-User", user)
                .header("Content-Type", "application/json")
                .body(
                    body.map(|value| Body::from(value.to_string()))
                        .unwrap_or_else(Body::empty),
                )?,
        )
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?, headers))
}

async fn setup(
    is_dm: bool,
    private: bool,
) -> anyhow::Result<(Router, toasty::Db, String, Vec<String>)> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let now = now_utc_millis();
    let users: Vec<_> = (0..4).map(|_| Uuid::new_v4().to_string()).collect();
    for id in &users {
        User::create()
            .id(id.as_str())
            .user_name(format!("notify_{}", id))
            .display_name(id.as_str())
            .is_bot(false)
            .created_at(now)
            .exec(&mut db)
            .await?;
    }
    let channel = Uuid::new_v4().to_string();
    Channel::create()
        .id(channel.as_str())
        .channel_name(format!("notify-{channel}"))
        .created_user_id(users[0].as_str())
        .is_private(private)
        .is_dm(is_dm)
        .created_at(now)
        .exec(&mut db)
        .await?;
    for user in &users[..3] {
        ChannelUser::create()
            .user_id(user.as_str())
            .channel_id(channel.as_str())
            .joined_at(now)
            .exec(&mut db)
            .await?;
    }
    let app = create_router(AppState::new(db.clone(), config));
    Ok((app, db, channel, users))
}

async fn post(
    app: &Router,
    channel: &str,
    user: &str,
    content: &str,
    parent: Option<&str>,
) -> anyhow::Result<String> {
    let (status, body, _) = request(
        app,
        "POST",
        "/api/messages",
        user,
        Some(json!({
            "channel_id":channel, "content":content, "parent_id":parent
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(body["data"]["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn ordinary_dm_notifies_other_members_once_and_keeps_mentions_and_replies_prioritized()
-> anyhow::Result<()> {
    let (app, mut db, channel, users) = setup(true, true).await?;
    let id = post(&app, &channel, &users[0], "Hello without mentions", None).await?;
    let rows = Notification::filter(Notification::fields().message_id().eq(id.as_str()))
        .exec(&mut db)
        .await?;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.user_id != users[0]
        && row.user_id != users[3]
        && row.notification_type == "dm"));

    let content = format!("Hello @notify_{}", users[1]);
    let id = post(&app, &channel, &users[0], &content, None).await?;
    let rows = Notification::filter(Notification::fields().message_id().eq(id.as_str()))
        .exec(&mut db)
        .await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .find(|row| row.user_id == users[1])
            .unwrap()
            .notification_type,
        "mention"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.user_id == users[2])
            .unwrap()
            .notification_type,
        "dm"
    );

    let reply = post(
        &app,
        &channel,
        &users[1],
        "A reply without mentions",
        Some(&id),
    )
    .await?;
    let rows = Notification::filter(Notification::fields().message_id().eq(reply.as_str()))
        .exec(&mut db)
        .await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .find(|row| row.user_id == users[0])
            .unwrap()
            .notification_type,
        "reply"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.user_id == users[2])
            .unwrap()
            .notification_type,
        "dm"
    );

    ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel.as_str())
            .and(ChannelUser::fields().user_id().eq(users[2].as_str())),
    )
    .delete()
    .exec(&mut db)
    .await?;
    let id = post(&app, &channel, &users[0], "After someone leaves", None).await?;
    let rows = Notification::filter(Notification::fields().message_id().eq(id.as_str()))
        .exec(&mut db)
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_id, users[1]);
    Ok(())
}

#[tokio::test]
async fn ordinary_channel_posts_still_do_not_notify_every_member() -> anyhow::Result<()> {
    let (app, mut db, channel, users) = setup(false, false).await?;
    let id = post(&app, &channel, &users[0], "No mentions here", None).await?;
    assert!(
        Notification::filter(Notification::fields().message_id().eq(id.as_str()))
            .exec(&mut db)
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn context_is_bounded_to_the_target_and_checks_ownership_membership_and_deletion()
-> anyhow::Result<()> {
    let (app, mut db, channel, users) = setup(false, true).await?;
    let id = post(&app, &channel, &users[0], &"🦊".repeat(400), None).await?;
    let unrelated = post(
        &app,
        &channel,
        &users[0],
        "Another message must not be in the preview",
        None,
    )
    .await?;
    for (message, user, emoji) in [(&id, &users[1], "👍"), (&unrelated, &users[2], "❤️")] {
        let (status, body, _) = request(
            &app,
            "POST",
            &format!("/api/messages/{message}/reactions"),
            user,
            Some(json!({"emoji":emoji})),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let notification = Notification::filter(Notification::fields().message_id().eq(id.as_str()))
        .first()
        .exec(&mut db)
        .await?
        .unwrap();
    let path = format!("/api/notifications/{}/context", notification.id);
    let (status, body, headers) = request(&app, "GET", &path, &users[0], None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
    assert_eq!(body["message"]["id"], id);
    assert_eq!(
        body["message"]["content"].as_str().unwrap().chars().count(),
        281
    );
    assert_eq!(body["message"].as_object().unwrap().len(), 3);
    assert_eq!(body["reactions"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["reactions"][0],
        json!({"user_id":users[1],"emoji":"👍"})
    );
    assert!(!body.to_string().contains("Another message"));
    for user in [&users[1], &users[3]] {
        assert_eq!(
            request(&app, "GET", &path, user, None).await?.0,
            StatusCode::NOT_FOUND
        );
    }
    Message::update_by_id(id.as_str())
        .deleted_at(Some(now_utc_millis()))
        .exec(&mut db)
        .await?;
    let (_, body, _) = request(&app, "GET", &path, &users[0], None).await?;
    assert_eq!(body, json!({"message":null,"reactions":[]}));
    ChannelUser::filter(
        ChannelUser::fields()
            .channel_id()
            .eq(channel.as_str())
            .and(ChannelUser::fields().user_id().eq(users[0].as_str())),
    )
    .delete()
    .exec(&mut db)
    .await?;
    assert_eq!(
        request(&app, "GET", &path, &users[0], None).await?.0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}
