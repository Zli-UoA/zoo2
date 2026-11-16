use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{
    auth::DEFAULT_DEV_USER_ID, config::Config, db::init_db, routes::create_router, state::AppState,
};

async fn setup_app() -> anyhow::Result<(axum::Router, AppState)> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let app = create_router(state.clone());
    Ok((app, state))
}

async fn create_test_user(app: &axum::Router, id: &str, name: &str) -> anyhow::Result<()> {
    let payload = json!({
        "id": id,
        "user_name": format!("{}_{}", name, uuid::Uuid::new_v4().simple()),
        "display_name": name,
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/users")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn test_direct_message_1on1_flow() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;

    // 1. Alice が Bob との 1対1 DM を作成
    let dm_payload = json!({
        "target_user_ids": [&bob_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();
    let channel_name = json["data"]["channel_name"].as_str().unwrap().to_string();

    assert_eq!(json["data"]["is_dm"], true);
    assert_eq!(json["data"]["is_private"], true);
    assert!(json["txid"].is_string());

    // チャンネル名が dm:<min_uuid>:<max_uuid> 形式であること
    let (min_u, max_u) = if alice_id < bob_id {
        (&alice_id, &bob_id)
    } else {
        (&bob_id, &alice_id)
    };
    assert_eq!(channel_name, format!("dm:{min_u}:{max_u}"));

    // 2. Bob が Alice との DM を取得 (既存取得・冪等性)
    let dm_payload_bob = json!({
        "target_user_ids": [&alice_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &bob_id)
        .body(Body::from(serde_json::to_vec(&dm_payload_bob)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], channel_id);
    // 既に所属中のため WAL 書込なし -> txid: null
    assert!(json["txid"].is_null());

    // 3. Alice が再度呼んでも同一チャンネルかつ txid: null
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], channel_id);
    assert!(json["txid"].is_null());

    Ok(())
}

#[tokio::test]
async fn test_direct_message_leave_and_reopen() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;

    // 1. Alice が Bob との 1対1 DM を作成 (Alice が created_user_id)
    let dm_payload = json!({
        "target_user_ids": [&bob_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();

    // 2. 作成者 Alice が DM から退出 -> DM では作成者であっても退出可能 (200 OK)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/leave"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. 退出した Alice はチャンネル情報取得で 404 (存在隠蔽)
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Bob は所属したままなので 200 OK
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Alice が再度 Bob との DM をリクエスト (再オープン・Alice 復帰)
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], channel_id);
    // channel_user への INSERT が発生したため有効な txid が返却される
    assert!(json["txid"].is_string());

    // 復帰した Alice は再度チャンネル情報取得可能
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. [P2 指摘事項の検証] 今度は Bob が DM から退出
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/leave"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // Bob は退出したので 404
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 6. Alice（現在所属中）が Bob との DM を開き直す (POST /api/dms)
    // -> Alice 単独ではなく、退出済みの相手 Bob も channel_user に復帰させて有効な txid を返す
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    assert_eq!(json["data"]["id"], channel_id);
    // Bob の再追加が発生したため有効な txid が返却される
    assert!(json["txid"].is_string());

    // 復帰した Bob も再度チャンネル情報取得可能
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    // 7. Alice が送信したメッセージが Bob からも閲覧可能であることを確認
    let msg_req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": channel_id,
            "content": "Hello Bob, welcome back!",
        }))?))?;
    let resp = app.clone().oneshot(msg_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let list_req = Request::builder()
        .method("GET")
        .uri(format!("/api/messages?channel_id={channel_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(list_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let msgs: Value = serde_json::from_slice(&body)?;
    assert_eq!(msgs.as_array().unwrap().len(), 1);

    Ok(())
}

#[tokio::test]
async fn reopening_plain_dm_waits_for_users_before_locking_channel() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    create_test_user(&app, &actor, "lock_order_actor").await?;
    create_test_user(&app, &target, "lock_order_target").await?;

    let create = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(
            json!({"target_user_ids": [&target]}).to_string(),
        ))?;
    let response = app.clone().oneshot(create).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let channel_id = serde_json::from_slice::<Value>(&body)?["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Single-channel guests can reopen a DM only when they share a channel.
    // Keep this API precondition true so this test reaches the DB lock-order path.
    let shared_channel = uuid::Uuid::new_v4().to_string();
    let create_shared = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(
            json!({
                "id": shared_channel,
                "channel_name": format!("dm_lock_shared_{}", uuid::Uuid::new_v4().simple()),
                "is_private": false
            })
            .to_string(),
        ))?;
    assert_eq!(
        app.clone().oneshot(create_shared).await?.status(),
        StatusCode::OK
    );
    let add_target = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{shared_channel}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(json!({"user_id": target}).to_string()))?;
    assert_eq!(
        app.clone().oneshot(add_target).await?.status(),
        StatusCode::OK
    );

    for user in [&actor, &target] {
        let mut user_db = db.clone();
        toasty::sql::statement("UPDATE users SET guest_kind = 'single' WHERE id = $1")
            .bind(user.as_str())
            .exec(&mut user_db)
            .await?;
        let leave = Request::builder()
            .method("POST")
            .uri(format!("/api/channels/{channel_id}/leave"))
            .header("X-Dev-User", user)
            .body(Body::empty())?;
        assert_eq!(app.clone().oneshot(leave).await?.status(), StatusCode::OK);
    }

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE",
            &[&target],
        )
        .await?;

    let reopen_app = app.clone();
    let reopen_actor = actor.clone();
    let reopen_target = target.clone();
    let mut reopen = tokio::spawn(async move {
        let request = Request::builder()
            .method("POST")
            .uri("/api/dms")
            .header("content-type", "application/json")
            .header("X-Dev-User", reopen_actor)
            .body(Body::from(
                json!({"target_user_ids": [reopen_target]}).to_string(),
            ))
            .unwrap();
        reopen_app.oneshot(request).await.unwrap()
    });

    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let waiting_for_user = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting = observer
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND $1 = ANY(pg_blocking_pids(pid)))",
                    &[&blocker_pid],
                )
                .await?
                .get::<_, bool>(0);
            if waiting {
                break Ok::<(), anyhow::Error>(());
            }
            if reopen.is_finished() {
                break Err(anyhow::anyhow!("DM reactivation completed without waiting for the blocked users row"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    if matches!(&waiting_for_user, Err(_) | Ok(Err(_))) {
        reopen.abort();
        blocker.rollback().await?;
        anyhow::bail!(
            "DM reactivation did not wait for the blocked users row: {waiting_for_user:?}"
        );
    }
    waiting_for_user??;

    // The real route must be waiting for users without retaining channels KEY SHARE.
    // A concurrent FOR UPDATE NOWAIT succeeds only when the lock order is correct.
    if let Err(err) = observer
        .query_one(
            "SELECT id FROM channels WHERE id = $1 FOR UPDATE NOWAIT",
            &[&channel_id],
        )
        .await
    {
        reopen.abort();
        blocker.rollback().await?;
        anyhow::bail!("DM reactivation retained a channel lock while waiting for users: {err}");
    }
    blocker.commit().await?;
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), &mut reopen).await??;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn reopening_plain_dm_does_not_deadlock_with_concurrent_e2ee_conversion() -> anyhow::Result<()>
{
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    create_test_user(&app, &actor, "reopen_convert_actor").await?;
    create_test_user(&app, &target, "reopen_convert_target").await?;

    // Single-channel guests may reopen this DM because they share this channel.
    let shared_channel = uuid::Uuid::new_v4().to_string();
    let create_shared = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(
            json!({
                "id": shared_channel,
                "channel_name": format!("reopen_convert_shared_{}", uuid::Uuid::new_v4().simple()),
                "is_private": false
            })
            .to_string(),
        ))?;
    assert_eq!(
        app.clone().oneshot(create_shared).await?.status(),
        StatusCode::OK
    );
    let add_target = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{shared_channel}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(json!({"user_id": target}).to_string()))?;
    assert_eq!(
        app.clone().oneshot(add_target).await?.status(),
        StatusCode::OK
    );

    let create_dm = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(
            json!({"target_user_ids": [&target]}).to_string(),
        ))?;
    let response = app.clone().oneshot(create_dm).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let channel_id = serde_json::from_slice::<Value>(&body)?["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    for user in [&actor, &target] {
        let mut user_db = db.clone();
        toasty::sql::statement("UPDATE users SET guest_kind = 'single' WHERE id = $1")
            .bind(user.as_str())
            .exec(&mut user_db)
            .await?;
    }

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one("SELECT pg_advisory_xact_lock(3, hashtext($1))", &[&actor])
        .await?;

    // Conversion reads both current members and locks their users before it
    // waits for the member advisory lock. The actor then leaves while conversion
    // is paused, making that membership snapshot stale.
    let convert_app = app.clone();
    let convert_target = target.clone();
    let convert_path = format!("/api/channels/{channel_id}/encrypt");
    let mut convert = tokio::spawn(async move {
        let request = Request::builder()
            .method("POST")
            .uri(convert_path)
            .header("X-Dev-User", convert_target)
            .body(Body::empty())
            .unwrap();
        convert_app.oneshot(request).await.unwrap()
    });

    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let conversion_pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let pid: Option<i32> = observer.query_one(
                "SELECT (SELECT pid FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE '%pg_advisory_xact_lock(3%' AND $1 = ANY(pg_blocking_pids(pid)) LIMIT 1)",
                &[&blocker_pid],
            ).await?.get(0);
            if let Some(pid) = pid { break Ok::<i32, anyhow::Error>(pid); }
            if convert.is_finished() {
                break Err(anyhow::anyhow!("conversion did not wait for the held member advisory lock"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await??;

    let leave = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/leave"))
        .header("X-Dev-User", &actor)
        .body(Body::empty())?;
    assert_eq!(app.clone().oneshot(leave).await?.status(), StatusCode::OK);

    let reopen_app = app.clone();
    let reopen_actor = actor.clone();
    let reopen_target = target.clone();
    let mut reopen = tokio::spawn(async move {
        let request = Request::builder()
            .method("POST")
            .uri("/api/dms")
            .header("content-type", "application/json")
            .header("X-Dev-User", reopen_actor)
            .body(Body::from(
                json!({"target_user_ids": [reopen_target]}).to_string(),
            ))
            .unwrap();
        reopen_app.oneshot(request).await.unwrap()
    });

    // Confirm the API is actually contending with conversion before releasing
    // the advisory lock; old ordering makes the wait-for cycle reproducible.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = observer.query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND $1 = ANY(pg_blocking_pids(pid)))",
                &[&conversion_pid],
            ).await?.get(0);
            if waiting { break Ok::<(), anyhow::Error>(()); }
            if reopen.is_finished() {
                break Err(anyhow::anyhow!("DM reactivation did not contend with conversion-held user locks"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await??;

    blocker.commit().await?;
    let results = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::join!(&mut convert, &mut reopen)
    })
    .await;
    if results.is_err() {
        convert.abort();
        reopen.abort();
        anyhow::bail!("DM reactivation and E2EE conversion did not finish");
    }
    let (convert_result, reopen_result) = results?;
    let convert_response = convert_result?;
    let reopen_response = reopen_result?;
    assert_eq!(convert_response.status(), StatusCode::OK);
    assert_eq!(reopen_response.status(), StatusCode::OK);
    let body = to_bytes(reopen_response.into_body(), usize::MAX).await?;
    let reopened = serde_json::from_slice::<Value>(&body)?;
    let reopened_id = reopened["data"]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("DM response lacks channel ID"))?;
    let original_encrypted: bool = observer
        .query_one(
            "SELECT encrypted FROM channels WHERE id = $1",
            &[&channel_id],
        )
        .await?
        .get(0);
    assert!(original_encrypted);
    let row = observer
        .query_one(
            "SELECT c.encrypted, (SELECT count(*) FROM channel_user cu WHERE cu.channel_id = c.id AND cu.user_id IN ($2, $3)) FROM channels c WHERE c.id = $1 AND c.deleted_at IS NULL",
            &[&reopened_id, &actor, &target],
        )
        .await?;
    let reopened_encrypted: bool = row.get(0);
    let members: i64 = row.get(1);
    assert_eq!(members, 2, "both users must belong to the returned DM");
    assert_eq!(reopened_encrypted, reopened_id == channel_id);
    Ok(())
}

#[tokio::test]
async fn reopening_encrypted_dm_serializes_with_encrypted_note_rejection() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    create_test_user(&app, &actor, "reopen_encrypted_actor").await?;
    create_test_user(&app, &target, "reopen_encrypted_target").await?;

    let shared_channel = uuid::Uuid::new_v4().to_string();
    let create_channel = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(
            json!({
                "id": shared_channel,
                "channel_name": format!("reopen_encrypted_notes_{}", uuid::Uuid::new_v4().simple()),
                "is_private": false
            })
            .to_string(),
        ))?;
    assert_eq!(
        app.clone().oneshot(create_channel).await?.status(),
        StatusCode::OK
    );
    let add_target = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{shared_channel}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(json!({"user_id": target}).to_string()))?;
    assert_eq!(
        app.clone().oneshot(add_target).await?.status(),
        StatusCode::OK
    );

    let dm_payload = json!({"target_user_ids": [&target], "encrypted": true});
    let create = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &actor)
        .body(Body::from(dm_payload.to_string()))?;
    let response = app.clone().oneshot(create).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let channel_id = serde_json::from_slice::<Value>(&body)?["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    for user in [&actor, &target] {
        let mut guest_db = db.clone();
        toasty::sql::statement("UPDATE users SET guest_kind = 'single' WHERE id = $1")
            .bind(user.as_str())
            .exec(&mut guest_db)
            .await?;
        let leave = Request::builder()
            .method("POST")
            .uri(format!("/api/channels/{channel_id}/leave"))
            .header("X-Dev-User", user)
            .body(Body::empty())?;
        assert_eq!(app.clone().oneshot(leave).await?.status(), StatusCode::OK);
    }

    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT id FROM channels WHERE id = $1 FOR UPDATE",
            &[&channel_id],
        )
        .await?;

    let note_app = app.clone();
    let note_path = format!("/api/channels/{channel_id}/notes");
    let note_actor = target.clone();
    let note = tokio::spawn(async move {
        let request = Request::builder()
            .method("POST")
            .uri(note_path)
            .header("content-type", "application/json")
            .header("X-Dev-User", note_actor)
            .body(Body::from(
                json!({"title":"encrypted","content":"body"}).to_string(),
            ))
            .unwrap();
        note_app.clone().oneshot(request).await.unwrap()
    });
    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let note_pid: i32 = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(row) = observer
                .query_opt(
                    "SELECT pid FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE '%channels%' AND query LIKE '%FOR SHARE%' AND $1 = ANY(pg_blocking_pids(pid)) LIMIT 1",
                    &[&blocker_pid],
                )
                .await?
            {
                return Ok::<i32, anyhow::Error>(row.get(0));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("encrypted note request did not wait for the channel lock"))??;

    let reopen_app = app.clone();
    let reopen_actor = actor.clone();
    let reopen_payload = dm_payload.clone();
    let mut reopen = tokio::spawn(async move {
        let request = Request::builder()
            .method("POST")
            .uri("/api/dms")
            .header("content-type", "application/json")
            .header("X-Dev-User", reopen_actor)
            .body(Body::from(reopen_payload.to_string()))
            .unwrap();
        reopen_app.clone().oneshot(request).await.unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiters = observer
                .query(
                    "SELECT query, $1 = ANY(pg_blocking_pids(pid)) FROM pg_stat_activity WHERE wait_event_type = 'Lock'",
                    &[&note_pid],
                )
                .await?
                .into_iter()
                .map(|row| (row.get::<_, String>(0), row.get::<_, bool>(1)))
                .collect::<Vec<_>>();
            if waiters
                .iter()
                .any(|(query, blocked_by_note)| *blocked_by_note && query.contains("users") && query.contains("NO KEY UPDATE"))
            {
                break;
            }
            if reopen.is_finished() {
                let response = (&mut reopen).await?;
                let status = response.status();
                let body = to_bytes(response.into_body(), usize::MAX).await?;
                anyhow::bail!("DM reactivation ended early with status {status} and body {} while note authorization held locks: {waiters:?}", String::from_utf8_lossy(&body));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|_| anyhow::anyhow!("DM reactivation did not wait for the users row held by note authorization"))??;

    blocker.commit().await?;
    let (reopen_response, note_response) =
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(reopen, note)
        })
        .await?;
    assert_eq!(reopen_response?.status(), StatusCode::OK);
    assert_eq!(note_response?.status(), StatusCode::NOT_FOUND);
    let note_request = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/notes"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &target)
        .body(Body::from(
            json!({"title":"encrypted","content":"body"}).to_string(),
        ))?;
    assert_eq!(
        app.clone().oneshot(note_request).await?.status(),
        StatusCode::BAD_REQUEST,
        "encrypted notes are rejected after acquiring users then channels locks"
    );
    Ok(())
}

#[tokio::test]
async fn test_group_dm_flow() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();
    let charlie_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;
    create_test_user(&app, &charlie_id, "charlie").await?;

    // 1. Alice が Bob, Charlie とのグループ DM を作成
    let dm_payload = json!({
        "target_user_ids": [&bob_id, &charlie_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();
    let channel_name = json["data"]["channel_name"].as_str().unwrap().to_string();

    assert_eq!(json["data"]["is_dm"], true);
    assert_eq!(json["data"]["is_private"], true);
    assert!(channel_name.starts_with("dm:group:"));
    assert!(json["txid"].is_string());

    // 2. 参加者全員がチャンネル取得可能
    for uid in [&alice_id, &bob_id, &charlie_id] {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/api/channels/{channel_id}"))
            .header("X-Dev-User", uid)
            .body(Body::empty())?;
        let resp = app.clone().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    Ok(())
}

#[tokio::test]
async fn test_dm_validation_errors() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;

    // 1. target_user_ids が空
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": []
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 2. target_user_ids に自身が含まれている
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": [&alice_id]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 3. target_user_ids に重複がある
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": [&bob_id, &bob_id]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 4. target_user_ids が 9 人を超える (自身含め最大 10 人)
    let too_many_users: Vec<String> = (0..10).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": too_many_users
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 5. 存在しないユーザーを指定
    let non_existent_user = uuid::Uuid::new_v4().to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": [non_existent_user]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 6. 不正な UUID
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "target_user_ids": ["invalid-uuid"]
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 7. 既存チャンネルと重複する ID を指定した場合は 409 Conflict が返る (500 に置き換わらないことの検証)
    let duplicate_channel_id = uuid::Uuid::new_v4().to_string();
    let create_ch_req = Request::builder()
        .method("POST")
        .uri("/api/channels")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": &duplicate_channel_id,
            "channel_name": format!("ch-{}", uuid::Uuid::new_v4().simple()),
        }))?))?;
    let resp = app.clone().oneshot(create_ch_req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let charlie_id = uuid::Uuid::new_v4().to_string();
    create_test_user(&app, &charlie_id, "charlie").await?;

    let dm_dup_req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "id": &duplicate_channel_id,
            "target_user_ids": [&charlie_id]
        }))?))?;
    let resp = app.clone().oneshot(dm_dup_req).await?;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    Ok(())
}

#[tokio::test]
async fn test_dm_access_control_and_privacy() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();
    let charlie_id = uuid::Uuid::new_v4().to_string();
    let admin_id = DEFAULT_DEV_USER_ID; // 開発環境の特権管理者

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;
    create_test_user(&app, &charlie_id, "charlie").await?;

    // Alice と Bob の DM を作成
    let dm_payload = json!({
        "target_user_ids": [&bob_id],
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/dms")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&dm_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let json: Value = serde_json::from_slice(&body)?;
    let channel_id = json["data"]["id"].as_str().unwrap().to_string();

    // Alice がメッセージを投稿
    let msg_payload = json!({
        "channel_id": &channel_id,
        "content": "Secret message between Alice and Bob",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&msg_payload)?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await?;
    let msg_json: Value = serde_json::from_slice(&body)?;
    let message_id = msg_json["data"]["id"].as_str().unwrap().to_string();

    // 1. ワークスペース管理者 Admin (非参加) による存在隠蔽検証 (すべて 404 Not Found)
    // - チャンネル閲覧
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - チャンネル更新試行
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/channels/{channel_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", admin_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_name": "renamed-by-admin"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - チャンネル削除試行
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - チャンネルアーカイブ試行
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メンバー追加試行
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", admin_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "user_id": admin_id
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メッセージ閲覧試行
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/messages?channel_id={channel_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メッセージ投稿試行
    let req = Request::builder()
        .method("POST")
        .uri("/api/messages")
        .header("content-type", "application/json")
        .header("X-Dev-User", admin_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "channel_id": &channel_id,
            "content": "Admin snooping"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - メッセージ削除試行
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", admin_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 2. 参加者 (Alice) であっても DM に対して禁止されている操作 (すべて 403 Forbidden)
    // - 更新
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/channels/{channel_id}"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "description": "forbidden update"
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - 削除
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - アーカイブ
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/archive"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - メンバー追加
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/users"))
        .header("content-type", "application/json")
        .header("X-Dev-User", &alice_id)
        .body(Body::from(serde_json::to_vec(&json!({
            "user_id": &charlie_id
        }))?))?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - メンバーキック
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/channels/{channel_id}/users/{bob_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - 直接 join 試行 (非参加者 Charlie による -> 404 Not Found 存在隠蔽)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/join"))
        .header("X-Dev-User", &charlie_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // - 直接 join 試行 (参加者 Alice による -> 403 Forbidden)
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/channels/{channel_id}/join"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 3. メッセージ削除制御
    // - Bob が Alice のメッセージを削除試行 -> 403 Forbidden (DM では投稿者本人のみ削除可能)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &bob_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // - Alice 本人が削除 -> 200 OK
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/messages/{message_id}"))
        .header("X-Dev-User", &alice_id)
        .body(Body::empty())?;
    let resp = app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn test_concurrent_dm_creation() -> anyhow::Result<()> {
    let (app, _) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();

    create_test_user(&app, &alice_id, "alice").await?;
    create_test_user(&app, &bob_id, "bob").await?;

    // Alice と Bob の双方が同時に POST /api/dms を送信する
    let app1 = app.clone();
    let a_id = alice_id.clone();
    let b_id = bob_id.clone();
    let handle1 = tokio::spawn(async move {
        let req = Request::builder()
            .method("POST")
            .uri("/api/dms")
            .header("content-type", "application/json")
            .header("X-Dev-User", &a_id)
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "target_user_ids": [&b_id],
                }))
                .unwrap(),
            ))
            .unwrap();
        app1.oneshot(req).await.unwrap()
    });

    let app2 = app.clone();
    let a_id2 = alice_id.clone();
    let b_id2 = bob_id.clone();
    let handle2 = tokio::spawn(async move {
        let req = Request::builder()
            .method("POST")
            .uri("/api/dms")
            .header("content-type", "application/json")
            .header("X-Dev-User", &b_id2)
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "target_user_ids": [&a_id2],
                }))
                .unwrap(),
            ))
            .unwrap();
        app2.oneshot(req).await.unwrap()
    });

    let (res1, res2) = tokio::join!(handle1, handle2);
    let resp1 = res1?;
    let resp2 = res2?;

    // どちらのリクエストも 409 や 500 にならず、200 OK で成功すること
    assert_eq!(resp1.status(), StatusCode::OK);
    assert_eq!(resp2.status(), StatusCode::OK);

    let body1 = to_bytes(resp1.into_body(), usize::MAX).await?;
    let json1: Value = serde_json::from_slice(&body1)?;
    let ch_id1 = json1["data"]["id"].as_str().unwrap();

    let body2 = to_bytes(resp2.into_body(), usize::MAX).await?;
    let json2: Value = serde_json::from_slice(&body2)?;
    let ch_id2 = json2["data"]["id"].as_str().unwrap();

    // 両者が取得したチャンネル ID が完全に同一であること
    assert_eq!(ch_id1, ch_id2);

    Ok(())
}

#[tokio::test]
async fn test_concurrent_guest_transition_rejects_dm_creation() -> anyhow::Result<()> {
    let (app, state) = setup_app().await?;

    let alice_id = uuid::Uuid::new_v4().to_string();
    let bob_id = uuid::Uuid::new_v4().to_string();
    let carol_id = uuid::Uuid::new_v4().to_string();
    create_test_user(&app, &alice_id, "alice_guest_race").await?;
    create_test_user(&app, &bob_id, "bob_guest_race").await?;
    create_test_user(&app, &carol_id, "carol_guest_race").await?;

    for (label, target_ids) in [
        ("1on1", vec![bob_id.clone()]),
        ("group", vec![bob_id.clone(), carol_id.clone()]),
    ] {
        let mut db = state.db.clone();
        // 各ケースの最初に非ゲストへ戻す（1on1 の後に group を同じユーザーで試すため）
        toasty::sql::statement("UPDATE users SET guest_kind = 'none' WHERE id = $1")
            .bind(alice_id.as_str())
            .exec(&mut db)
            .await?;

        let mut blocker_db = state.db.clone();
        let mut blocker = blocker_db.transaction().await?;
        let pid_rows = backend::apps::query(
            &mut blocker,
            "SELECT json_build_object('pid', pg_backend_pid())::text",
            &[],
        )
        .await?;
        let blocker_pid = pid_rows[0]["pid"].as_i64().unwrap() as i32;

        // users 行をロックしつつ、まだ未コミットのままゲストへ変更する
        toasty::sql::statement("UPDATE users SET guest_kind = 'multi' WHERE id = $1")
            .bind(alice_id.as_str())
            .exec(&mut blocker)
            .await?;

        let dm_id = uuid::Uuid::new_v4().to_string();
        let app_clone = app.clone();
        let actor = alice_id.clone();
        let dm_id_req = dm_id.clone();
        let targets = target_ids.clone();
        let handle = tokio::spawn(async move {
            let req = Request::builder()
                .method("POST")
                .uri("/api/dms")
                .header("content-type", "application/json")
                .header("X-Dev-User", &actor)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "id": dm_id_req,
                        "target_user_ids": targets,
                    }))
                    .unwrap(),
                ))
                .unwrap();
            app_clone.oneshot(req).await.unwrap()
        });

        let mut waited = false;
        let blocker_str = blocker_pid.to_string();
        for _ in 0..100 {
            let mut wait_tx = db.transaction().await?;
            let rows = backend::apps::query(
                &mut wait_tx,
                "SELECT json_build_object('pid', a.pid)::text
                 FROM pg_stat_activity a
                 WHERE a.wait_event_type = 'Lock'
                   AND $1::int = ANY(pg_blocking_pids(a.pid))
                 LIMIT 1",
                &[blocker_str.as_str()],
            )
            .await?;
            wait_tx.commit().await?;
            if !rows.is_empty() {
                waited = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            waited,
            "{label} DM request did not block on users row lock held by {blocker_pid}"
        );

        blocker.commit().await?;
        let resp = handle.await?;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{label} DM must reject user who became guest while waiting on users lock"
        );

        let mut check_tx = db.transaction().await?;
        let created = backend::apps::query(
            &mut check_tx,
            "SELECT json_build_object('id', id)::text FROM channels WHERE id = $1",
            &[dm_id.as_str()],
        )
        .await?;
        check_tx.commit().await?;
        assert!(
            created.is_empty(),
            "{label} DM channel {dm_id} must not be created after concurrent guest transition"
        );
    }

    Ok(())
}
