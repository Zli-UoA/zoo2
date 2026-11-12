//! 暗号化チャンネル（#357）の結合テスト。仕様は docs/e2ee.md の「暗号化チャンネル」「配布先のスナップショット」
//! 「メッセージ」「membership_epoch と送信の順序」。
//!
//! クロス署名鍵の変更と鍵バックアップの版の作成での増加は、再認証の仕組みがある `e2ee_keys_test.rs` で確かめる。

use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zoo_crypto::DeviceAccount;

struct Env {
    app: Router,
    db: toasty::Db,
}

async fn env_with(enabled: bool) -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = enabled;
    let db = init_db(&config.database_url).await.unwrap();
    let app = create_router(AppState::new(db.clone(), config));
    Env { app, db }
}

async fn env() -> Env {
    env_with(true).await
}

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    builder = if actor.starts_with("zoo_at_") {
        builder.header("authorization", format!("Bearer {actor}"))
    } else {
        builder.header("X-Dev-User", actor)
    };
    // 既存の結合テストと同じく、本文のない要求も `null` を送る（JSON を読む経路があるため）。
    let body = Body::from(body.to_string());
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

impl Env {
    async fn exec(&self, sql: &str, params: &[&str]) -> Result<u64, toasty::Error> {
        let mut statement = toasty::sql::statement(sql);
        for p in params {
            statement = statement.bind(*p);
        }
        let mut db = self.db.clone();
        statement.exec(&mut db).await
    }

    async fn query(&self, sql: &str, params: &[&str]) -> Vec<Value> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        let rows = backend::apps::query(&mut tx, sql, params).await.unwrap();
        tx.commit().await.unwrap();
        rows
    }

    async fn epoch(&self, channel: &str) -> i64 {
        self.query(
            "SELECT membership_epoch::text FROM channels WHERE id = $1",
            &[channel],
        )
        .await[0]
            .as_i64()
            .unwrap()
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = request(
            &self.app,
            "POST",
            "/api/users",
            &id,
            json!({"id": id, "user_name": format!("e2ee_{}", Uuid::new_v4().simple()), "display_name": "E2EE"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        id
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        actor: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        request(&self.app, method, path, actor, body).await
    }

    /// API で暗号化チャンネルを作り、`members` を足す（作成者は `creator`）。
    async fn encrypted_channel(&self, creator: &str, members: &[&str]) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/channels",
                creator,
                json!({"id": id, "channel_name": format!("enc-{}", Uuid::new_v4().simple()), "is_private": true, "encrypted": true}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for member in members {
            let (status, body) = self
                .call(
                    "POST",
                    &format!("/api/channels/{id}/users"),
                    creator,
                    json!({"user_id": member}),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        id
    }

    async fn plain_channel(&self, creator: &str, members: &[&str]) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/channels",
                creator,
                json!({"id": id, "channel_name": format!("plain-{}", Uuid::new_v4().simple()), "is_private": true}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for member in members {
            self.call(
                "POST",
                &format!("/api/channels/{id}/users"),
                creator,
                json!({"user_id": member}),
            )
            .await;
        }
        id
    }

    async fn device(&self, user: &str) -> DeviceAccount {
        let account = DeviceAccount::new(user, &Uuid::new_v4().to_string()).unwrap();
        let (status, body) = self
            .call(
                "POST",
                "/api/e2ee/devices",
                user,
                json!({"device_keys": account.device_keys(), "display_name": "Test"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        account
    }
}

fn ciphertext(account: &DeviceAccount, text: &str) -> Value {
    json!({
        "algorithm": "m.megolm.v1.aes-sha2",
        "sender_key": account.identity_keys().curve25519.to_base64(),
        "device_id": account.device_id(),
        "session_id": "session",
        "ciphertext": text,
    })
}

fn message(channel: &str, id: &str, account: &DeviceAccount, epoch: i64, text: &str) -> Value {
    json!({"id": id, "channel_id": channel, "content": "", "epoch": epoch, "revision": 0,
        "ciphertext": ciphertext(account, text)})
}

// ---------------------------------------------------------------------------
// 作成と変更

#[tokio::test]
async fn creation_is_gated_by_the_setting_and_private_only() {
    let disabled = env_with(false).await;
    let alice = disabled.user().await;
    let name = format!("enc-{}", Uuid::new_v4().simple());
    let (status, body) = disabled
        .call(
            "POST",
            "/api/channels",
            &alice,
            json!({"channel_name": name, "is_private": true, "encrypted": true}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "E2EE_DISABLED")
    );
    let bob = disabled.user().await;
    let (status, body) = disabled
        .call(
            "POST",
            "/api/dms",
            &alice,
            json!({"target_user_ids": [bob], "encrypted": true}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "E2EE_DISABLED")
    );
    // 平文の作成は今までどおり。
    let (status, body) = disabled
        .call(
            "POST",
            "/api/channels",
            &alice,
            json!({"channel_name": name}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["encrypted"], false);
    assert_eq!(body["data"]["membership_epoch"], 0);

    let env = env().await;
    let (status, body) = env
        .call(
            "POST",
            "/api/channels",
            &alice,
            json!({"channel_name": format!("enc-{}", Uuid::new_v4().simple()), "encrypted": true}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PUBLIC_FORBIDDEN")
    );
    let channel = env.encrypted_channel(&alice, &[]).await;
    let (status, body) = env
        .call(
            "GET",
            &format!("/api/channels/{channel}"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["encrypted"], true);
    assert_eq!(body["membership_epoch"], 0);
}

#[tokio::test]
async fn encrypted_cannot_be_changed_and_encrypted_channels_stay_private() {
    let env = env().await;
    let alice = env.user().await;
    let encrypted = env.encrypted_channel(&alice, &[]).await;
    let plain = env.plain_channel(&alice, &[]).await;
    let patch = |id: &str| format!("/api/channels/{id}");

    for (id, body) in [
        (&plain, json!({"encrypted": true})),
        (&encrypted, json!({"encrypted": false})),
        (&encrypted, json!({"is_private": false})),
    ] {
        let (status, res) = env.call("PATCH", &patch(id), &alice, body).await;
        assert_eq!(
            (status, code(&res)),
            (StatusCode::CONFLICT, "E2EE_ENCRYPTED_IMMUTABLE"),
            "{res}"
        );
    }
    // 同じ値なら何もしない。平文の非公開チャンネルは今までどおり公開にできる。
    let (status, res) = env
        .call(
            "PATCH",
            &patch(&encrypted),
            &alice,
            json!({"encrypted": true, "description": "d"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let (status, res) = env
        .call(
            "PATCH",
            &patch(&plain),
            &alice,
            json!({"is_private": false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");

    // トリガーと CHECK も直接の更新を拒否する。
    assert!(
        env.exec(
            "UPDATE channels SET encrypted = true WHERE id = $1",
            &[&plain]
        )
        .await
        .is_err()
    );
    assert!(
        env.exec(
            "UPDATE channels SET encrypted = false WHERE id = $1",
            &[&encrypted]
        )
        .await
        .is_err()
    );
    assert!(
        env.exec(
            "UPDATE channels SET is_private = false WHERE id = $1",
            &[&encrypted]
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn apps_cannot_create_encrypted_channels_or_dms() {
    // 設定が false でも、App には E2EE_APPS_FORBIDDEN を返す（判定の順は App、設定）。
    for enabled in [true, false] {
        let env = env_with(enabled).await;
        let owner = env.user().await;
        let target = env.user().await;
        let admin = backend::auth::DEFAULT_DEV_USER_ID;
        env.call("POST", "/api/users", admin, json!({"id": admin, "user_name": format!("admin_{}", Uuid::new_v4().simple()), "display_name": "Admin"}))
            .await;
        let manifest = json!({"scopes":["channels:manage","messages:read","messages:write","dm:write"],"events":["message.created"],"transport":"websocket","dm_enabled":true,"redirect_uris":["https://example.com/callback"]});
        let (status, created) = env
            .call(
                "POST",
                "/api/apps",
                &owner,
                json!({"name":"E2EE","description":"E2EE app","manifest":manifest}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let id = created["id"].as_str().unwrap();
        let action = |a: &str| format!("/api/apps/{id}/actions/{a}");
        let (status, body) = env
            .call("POST", &action("submit"), &owner, Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            env.call("POST", &action("install"), admin, json!({"revision": 1}))
                .await
                .0,
            StatusCode::OK
        );
        let (status, token) = env
            .call("POST", &action("token"), &owner, Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK, "{token}");
        let access = token["access_token"].as_str().unwrap();

        let (status, body) = env
            .call(
                "POST",
                "/api/apps/v1/channels",
                access,
                json!({"channel_name": format!("enc-{}", Uuid::new_v4().simple()), "is_private": true, "encrypted": true}),
            )
            .await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::FORBIDDEN, "E2EE_APPS_FORBIDDEN"),
            "{body}"
        );
        let (status, body) = env
            .call(
                "POST",
                "/api/apps/v1/dms",
                access,
                json!({"target_user_ids": [target], "encrypted": true}),
            )
            .await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::FORBIDDEN, "E2EE_APPS_FORBIDDEN"),
            "{body}"
        );
    }
}

// ---------------------------------------------------------------------------
// メンバーと membership_epoch

#[tokio::test]
async fn membership_changes_bump_the_epoch_only_when_rows_change() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let channel = env.encrypted_channel(&alice, &[]).await;
    assert_eq!(env.epoch(&channel).await, 0);
    let users = format!("/api/channels/{channel}/users");

    let (status, body) = env
        .call("POST", &users, &alice, json!({"user_id": bob}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(env.epoch(&channel).await, 1);
    // すでにメンバーなら増やさない。
    let (status, _) = env
        .call("POST", &users, &alice, json!({"user_id": bob}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.epoch(&channel).await, 1);
    env.call("POST", &users, &alice, json!({"user_id": carol}))
        .await;
    assert_eq!(env.epoch(&channel).await, 2);

    let (status, _) = env
        .call("DELETE", &format!("{users}/{carol}"), &alice, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.epoch(&channel).await, 3);
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/leave"),
            &bob,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.epoch(&channel).await, 4);
    // メンバーでない人の退出は 404 で、増やさない。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/leave"),
            &bob,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(env.epoch(&channel).await, 4);

    // epoch だけの変更では、通知の表示を作り直さない。
    env.exec(
        "DELETE FROM display_refresh_queue WHERE kind = 'channel' AND target = $1",
        &[&channel],
    )
    .await
    .unwrap();
    env.exec(
        "UPDATE channels SET membership_epoch = membership_epoch + 1 WHERE id = $1",
        &[&channel],
    )
    .await
    .unwrap();
    assert!(
        env.query(
            "SELECT '{}'::text FROM display_refresh_queue WHERE kind = 'channel' AND target = $1",
            &[&channel]
        )
        .await
        .is_empty()
    );
    // 平文チャンネルの epoch は変わらない。
    let plain = env.plain_channel(&alice, &[&bob]).await;
    env.call(
        "POST",
        &format!("/api/channels/{plain}/leave"),
        &bob,
        Value::Null,
    )
    .await;
    assert_eq!(env.epoch(&plain).await, 0);
}

#[tokio::test]
async fn only_current_members_can_add_to_encrypted_channels() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let admin = backend::auth::DEFAULT_DEV_USER_ID;
    env.call("POST", "/api/users", admin, json!({"id": admin, "user_name": format!("admin_{}", Uuid::new_v4().simple()), "display_name": "Admin"}))
        .await;
    let channel = env.encrypted_channel(&alice, &[]).await;
    let users = format!("/api/channels/{channel}/users");

    // メンバーでない管理者は、自分も他人も足せない。
    for target in [admin, bob.as_str()] {
        let (status, body) = env
            .call("POST", &users, admin, json!({"user_id": target}))
            .await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::FORBIDDEN, "E2EE_ADD_FORBIDDEN"),
            "{body}"
        );
    }
    // 平文の非公開チャンネルでは、今までどおり足せる。
    let plain = env.plain_channel(&alice, &[]).await;
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{plain}/users"),
            admin,
            json!({"user_id": bob}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 退出した作成者（channel_user の行がない）も足せない。
    env.exec(
        "DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2",
        &[&channel, &alice],
    )
    .await
    .unwrap();
    let (status, body) = env
        .call("POST", &users, &alice, json!({"user_id": bob}))
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "E2EE_ADD_FORBIDDEN"),
        "{body}"
    );
    assert_eq!(env.epoch(&channel).await, 0);
}

#[tokio::test]
async fn encrypted_dms_are_separate_rows_and_rejoin_bumps_the_epoch() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let (status, plain) = env
        .call(
            "POST",
            "/api/dms",
            &alice,
            json!({"target_user_ids": [bob]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{plain}");
    let (status, encrypted) = env
        .call(
            "POST",
            "/api/dms",
            &alice,
            json!({"target_user_ids": [bob], "encrypted": true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{encrypted}");
    let encrypted = &encrypted["data"];
    assert_eq!(encrypted["encrypted"], true);
    assert!(
        encrypted["channel_name"]
            .as_str()
            .unwrap()
            .starts_with("edm:")
    );
    assert_ne!(encrypted["id"], plain["data"]["id"]);
    assert_eq!(plain["data"]["encrypted"], false);
    let id = encrypted["id"].as_str().unwrap();

    // 退出して、もう一度 DM を開くと戻り、epoch が増える。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{id}/leave"),
            &bob,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.epoch(id).await, 1);
    let (status, again) = env
        .call(
            "POST",
            "/api/dms",
            &alice,
            json!({"target_user_ids": [bob], "encrypted": true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["data"]["id"], id);
    assert_eq!(again["data"]["membership_epoch"], 2);
    assert_eq!(env.epoch(id).await, 2);

    // edm: の行が encrypted = false という不整合は 409。
    let carol = env.user().await;
    let (a, b) = if alice < carol {
        (&alice, &carol)
    } else {
        (&carol, &alice)
    };
    env.exec(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private, is_dm)
         VALUES ($1, $2, $3, true, true)",
        &[&Uuid::new_v4().to_string(), &format!("edm:{a}:{b}"), &alice],
    )
    .await
    .unwrap();
    let (status, body) = env
        .call(
            "POST",
            "/api/dms",
            &alice,
            json!({"target_user_ids": [carol], "encrypted": true}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_ENCRYPTED_MISMATCH")
    );
}

#[tokio::test]
async fn deactivation_and_device_deletion_bump_every_encrypted_channel() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let first = env.encrypted_channel(&bob, &[&alice]).await;
    let second = env.encrypted_channel(&alice, &[&bob]).await;
    let without_alice = env.encrypted_channel(&bob, &[]).await;
    let plain = env.plain_channel(&alice, &[&bob]).await;
    let device = env.device(&alice).await;
    let before = [env.epoch(&first).await, env.epoch(&second).await];

    // 端末の削除で 1 増え、削除済みの端末への再試行では増えない。
    let path = format!("/api/e2ee/devices/{}", device.device_id());
    for _ in 0..2 {
        let (status, body) = env.call("DELETE", &path, &alice, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(env.epoch(&first).await, before[0] + 1);
        assert_eq!(env.epoch(&second).await, before[1] + 1);
    }

    // アカウントの無効化でも 1 増える（端末がなくても）。平文チャンネルとほかのチャンネルは変わらない。
    env.exec(
        "UPDATE users SET guest_kind = 'deactivated' WHERE id = $1",
        &[&alice],
    )
    .await
    .unwrap();
    assert_eq!(env.epoch(&first).await, before[0] + 2);
    assert_eq!(env.epoch(&second).await, before[1] + 2);
    assert_eq!(env.epoch(&without_alice).await, 0);
    assert_eq!(env.epoch(&plain).await, 0);
}

#[tokio::test]
async fn concurrent_bulk_bumps_do_not_deadlock() {
    let env = env().await;
    let users = [env.user().await, env.user().await, env.user().await];
    // 3 人がそれぞれ同じ 4 つのチャンネルに入る。
    let mut channels = Vec::new();
    for _ in 0..4 {
        channels.push(
            env.encrypted_channel(&users[0], &[&users[1], &users[2]])
                .await,
        );
    }
    let mut devices = Vec::new();
    for user in &users {
        for _ in 0..3 {
            devices.push((user.clone(), env.device(user).await.device_id().to_owned()));
        }
    }
    let before = env.epoch(&channels[0]).await;
    let tasks: Vec<_> = devices
        .iter()
        .map(|(user, device)| {
            let app = env.app.clone();
            let path = format!("/api/e2ee/devices/{device}");
            let user = user.clone();
            tokio::spawn(async move { request(&app, "DELETE", &path, &user, Value::Null).await.0 })
        })
        .collect();
    let results = tokio::time::timeout(
        Duration::from_secs(30),
        futures_util::future::join_all(tasks),
    )
    .await
    .expect("bulk bumps deadlocked");
    for result in results {
        assert_eq!(result.unwrap(), StatusCode::OK);
    }
    for channel in &channels {
        assert_eq!(env.epoch(channel).await, before + devices.len() as i64);
    }
}

// ---------------------------------------------------------------------------
// メッセージ

#[tokio::test]
async fn encrypted_messages_require_ciphertext_and_the_current_epoch() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.encrypted_channel(&alice, &[&bob]).await;
    let epoch = env.epoch(&channel).await;
    let device = env.device(&alice).await;
    let other = env.device(&bob).await;
    let send = |body: Value| env.call("POST", "/api/messages", &alice, body);

    let (status, body) = send(json!({"channel_id": channel, "content": "hello"})).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN")
    );
    let (status, body) = send(json!({"channel_id": channel, "content": "", "epoch": epoch})).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_CIPHERTEXT_REQUIRED")
    );
    let mut with_attachment = message(&channel, &Uuid::new_v4().to_string(), &device, epoch, "c");
    with_attachment["attachment_ids"] = json!([Uuid::new_v4().to_string()]);
    let (status, body) = send(with_attachment).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_ATTACHMENTS_FORBIDDEN")
    );
    let (status, body) = send(message(
        &channel,
        &Uuid::new_v4().to_string(),
        &device,
        epoch - 1,
        "c",
    ))
    .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_STALE_EPOCH")
    );
    assert_eq!(body["current_epoch"], epoch);
    let (status, body) = send(message(
        &channel,
        &Uuid::new_v4().to_string(),
        &device,
        epoch + 1,
        "c",
    ))
    .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_INVALID_EPOCH")
    );
    // 他人の端末を名乗れない。
    let (status, _) = send(message(
        &channel,
        &Uuid::new_v4().to_string(),
        &other,
        epoch,
        "c",
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let id = Uuid::new_v4().to_string();
    let (status, body) = send(message(&channel, &id, &device, epoch, "first")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["content"], "");
    assert_eq!(body["data"]["ciphertext"]["ciphertext"], "first");
    assert_eq!(body["data"]["epoch"], epoch);
    assert!(body["txid"].is_string());

    // 応答をなくした再送は、epoch が古くなっていても 200 で既存の行を返す。
    env.call(
        "POST",
        &format!("/api/channels/{channel}/users"),
        &alice,
        json!({"user_id": env.user().await}),
    )
    .await;
    let (status, body) = send(message(&channel, &id, &device, epoch, "second")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["ciphertext"]["ciphertext"], "first");
    // 別のユーザーの同じ id は衝突（code なし）。
    let bob_device = &other;
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &bob,
            message(&channel, &id, bob_device, epoch + 1, "x"),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code(&body), "");

    // 平文チャンネルに暗号化の項目は送れない。
    let plain = env.plain_channel(&alice, &[]).await;
    let (status, body) = send(json!({"channel_id": plain, "content": "hi", "epoch": 0})).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN")
    );
    let (status, body) = send(json!({"channel_id": plain, "content": "hi"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["data"]["ciphertext"].is_null());

    // 送信で端末の last_seen_at が更新される。
    let seen = env
        .query(
            "SELECT json_build_object('recent', last_seen_at > now() - interval '1 minute')::text
             FROM e2ee_devices WHERE user_id = $1 AND device_id = $2",
            &[&alice, device.device_id()],
        )
        .await;
    assert_eq!(seen[0]["recent"], true);
}

#[tokio::test]
async fn edits_check_revision_and_deletes_clear_ciphertext() {
    let env = env().await;
    let alice = env.user().await;
    let channel = env.encrypted_channel(&alice, &[]).await;
    let device = env.device(&alice).await;
    let id = Uuid::new_v4().to_string();
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &alice,
            message(&channel, &id, &device, 0, "v0"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let path = format!("/api/messages/{id}");
    let edit = |revision: Option<i64>, epoch: i64| {
        let mut body =
            json!({"content": "", "epoch": epoch, "ciphertext": ciphertext(&device, "v1")});
        if let Some(revision) = revision {
            body["revision"] = revision.into();
        }
        body
    };

    let (status, _) = env.call("PATCH", &path, &alice, edit(None, 0)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = env.call("PATCH", &path, &alice, edit(Some(2), 0)).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_STALE_REVISION")
    );
    assert_eq!(body["current_revision"], 0);
    let (status, body) = env
        .call(
            "PATCH",
            &path,
            &alice,
            json!({"content": "plain", "revision": 1}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN")
    );
    let (status, body) = env.call("PATCH", &path, &alice, edit(Some(1), 0)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["revision"], 1);
    assert_eq!(body["data"]["ciphertext"]["ciphertext"], "v1");
    // 同じ revision での 2 回目は 409。
    let (status, body) = env.call("PATCH", &path, &alice, edit(Some(1), 0)).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_STALE_REVISION")
    );
    assert_eq!(body["current_revision"], 1);

    // 削除すると、DB のトリガーが暗号文を消す。
    let (status, body) = env.call("DELETE", &path, &alice, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["data"]["ciphertext"].is_null());
    let row = env
        .query(
            "SELECT json_build_object('ciphertext', ciphertext)::text FROM messages WHERE id = $1",
            &[&id],
        )
        .await;
    assert!(row[0]["ciphertext"].is_null());

    // 平文チャンネル：更新前のクライアントの {"content"} だけの編集は通り、revision を 1 増やす。
    let plain = env.plain_channel(&alice, &[]).await;
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &alice,
            json!({"channel_id": plain, "content": "a"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plain_path = format!("/api/messages/{}", body["data"]["id"].as_str().unwrap());
    let (status, body) = env
        .call("PATCH", &plain_path, &alice, json!({"content": "b"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["revision"], 1);
    let (status, body) = env
        .call(
            "PATCH",
            &plain_path,
            &alice,
            json!({"content": "c", "revision": 5}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_STALE_REVISION")
    );
    let (status, body) = env
        .call(
            "PATCH",
            &plain_path,
            &alice,
            json!({"content": "c", "revision": 2}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = env
        .call(
            "PATCH",
            &plain_path,
            &alice,
            json!({"content": "d", "epoch": 0}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN")
    );
}

// ---------------------------------------------------------------------------
// 配布先のスナップショット

#[tokio::test]
async fn recipients_are_paged_by_user_id_and_tied_to_the_epoch() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let outsider = env.user().await;
    let channel = env.encrypted_channel(&alice, &[&bob, &carol]).await;
    let bob_device = env.device(&bob).await;
    // 削除した端末は入らない。
    let deleted = env.device(&carol).await;
    env.call(
        "DELETE",
        &format!("/api/e2ee/devices/{}", deleted.device_id()),
        &carol,
        Value::Null,
    )
    .await;
    let epoch = env.epoch(&channel).await;
    let path = format!("/api/e2ee/channels/{channel}/recipients");
    let mut sorted = [alice.clone(), bob.clone(), carol.clone()];
    sorted.sort();

    let (status, first) = env
        .call("GET", &format!("{path}?limit=2"), &alice, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["membership_epoch"], epoch);
    assert_eq!(first["user_ids"], json!(sorted[..2]));
    assert_eq!(first["next_after"], sorted[1]);

    // 続きのページは、最初のページの epoch で取る。
    let after = first["next_after"].as_str().unwrap();
    let (status, second) = env
        .call(
            "GET",
            &format!("{path}?limit=2&after={after}&epoch={epoch}"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["user_ids"], json!(sorted[2..]));
    assert!(second["next_after"].is_null());
    let all: Vec<Value> = [first.clone(), second.clone()].into_iter().collect();
    let devices = |user: &str| {
        all.iter()
            .find_map(|page| page["device_keys"].get(user).cloned())
            .unwrap()
    };
    assert!(devices(&bob).get(bob_device.device_id()).is_some());
    assert!(devices(&carol).get(deleted.device_id()).is_none());

    // epoch が変わったら、続きのページは 409。
    env.call(
        "DELETE",
        &format!("/api/channels/{channel}/users/{carol}"),
        &alice,
        Value::Null,
    )
    .await;
    let (status, body) = env
        .call(
            "GET",
            &format!("{path}?limit=2&after={after}&epoch={epoch}"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_STALE_EPOCH")
    );
    assert_eq!(body["current_epoch"], epoch + 1);
    // after だけで epoch がないのは 400。
    let (status, _) = env
        .call("GET", &format!("{path}?after={after}"), &alice, Value::Null)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // メンバーでない人と、平文チャンネルは 404。
    let (status, _) = env.call("GET", &path, &outsider, Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let plain = env.plain_channel(&alice, &[]).await;
    let (status, _) = env
        .call(
            "GET",
            &format!("/api/e2ee/channels/{plain}/recipients"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
