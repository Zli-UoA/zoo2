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
use backend::{config::Config, db::init_db_with_pool, routes::create_router, state::AppState};
use futures_util::FutureExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zoo_crypto::DeviceAccount;

struct Env {
    app: Router,
    db: toasty::Db,
    /// 観測用の独立接続を開くための URL。ロック待ちの観測にアプリのプールを
    /// 使うと、競合する API 要求がプールの接続を使い切ったときに観測自体が
    /// 止まってしまう。
    database_url: String,
}

async fn env_with(enabled: bool) -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = enabled;
    // ロック競合のテストは、止めたトランザクション用の接続を保持したまま
    // 競合する API 要求を並行して出す。環境の DB_MAX_POOL_SIZE がテストの
    // 成立を決めないように、必要な接続数（保持 1 + 競合 2 + 余裕 1）を
    // 明示する。観測はこのプールの外の独立接続で行う。
    let database_url = config.database_url.clone();
    let db = init_db_with_pool(&config.database_url, 4).await.unwrap();
    let app = create_router(AppState::new(db.clone(), config));
    Env {
        app,
        db,
        database_url,
    }
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

    /// ロック待ちの観測専用の独立した接続。アプリのプールの接続を競合する
    /// API 要求と取り合わない。接続が止まらないように 5 秒で区切る。
    async fn observer(&self) -> tokio_postgres::Client {
        let (client, connection) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_postgres::connect(&self.database_url, tokio_postgres::NoTls),
        )
        .await
        .expect("observer connection timed out after 5s")
        .expect("observer connection");
        tokio::spawn(connection);
        client
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

    /// 既定チャンネルへ参加しないテストユーザーを作る。既定チャンネルがある環境では
    /// `user()` の自動参加が非 DM 所属数の前提を壊すため、ゲスト系のテストで使う。
    async fn user_without_default_channels(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let user_name = format!("e2ee_{}", Uuid::new_v4().simple());
        self.exec(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $2, 'E2EE')",
            &[id.as_str(), user_name.as_str()],
        )
        .await
        .expect("direct test user insert");
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

    // トリガーと CHECK も直接の更新を拒否する（plain は上で公開にしたので、暗号化は CHECK が拒否する。
    // 非公開チャンネルの false から true への切り替えは #367 で認めた。e2ee_convert_test.rs）。
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
    // 添付は暗号化したものを紐付けられる（#364。e2ee_attachments_test.rs）が、センシティブの印は
    // 暗号化した本文に入れるので、平文の項目では受け付けない。
    let mut sensitive = message(&channel, &Uuid::new_v4().to_string(), &device, epoch, "c");
    sensitive["sensitive"] = json!(true);
    let (status, body) = send(sensitive).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN")
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

// ---------------------------------------------------------------------------
// 競合（#357 のレビュー）

/// 観測が止まったときの診断用に、全セッションの待機状態を集める。DB 側が止まって
/// いてもこの採取自体が報告を止めないように、3 秒で区切る。
async fn lock_observation_dump(observer: &tokio_postgres::Client) -> String {
    match tokio::time::timeout(
        Duration::from_secs(3),
        observer.query(
            "SELECT format('%s|%s|%s|%s', pid, coalesce(wait_event_type, ''), query, pg_blocking_pids(pid)) \
             FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
            &[],
        ),
    )
    .await
    {
        Ok(rows) => rows
            .map(|rows| {
                rows.iter()
                    .map(|row| row.get::<_, String>(0))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_else(|error| format!("session dump failed: {error}")),
        Err(_) => "session dump timed out after 3s".to_string(),
    }
}

/// `pattern` を含む文がロックを待つまで待つ。観測はアプリのプールの外の
/// 独立した接続で行い、タイムアウトで全体の時間を区切る。
async fn wait_for_lock(env: &Env, pattern: &str) {
    let observer = env.observer().await;
    let waiting = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let row = observer
                .query_one(
                    "SELECT count(*)::int8 FROM pg_stat_activity
                     WHERE wait_event_type = 'Lock' AND query LIKE '%' || $1 || '%'
                       AND query NOT LIKE '%pg_stat_activity%'",
                    &[&pattern],
                )
                .await
                .unwrap();
            if row.get::<_, i64>(0) > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    if waiting.is_err() {
        panic!(
            "no statement waited on a lock within 10s: {pattern}; sessions: {}",
            lock_observation_dump(&observer).await
        );
    }
}

async fn wait_for_dm_lock_blocked_by(env: &Env, blocker_pid: i32, member_id: &str) -> i32 {
    let observer = env.observer().await;
    let waiting = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let row = observer
                .query_opt(
                    "SELECT a.pid FROM pg_stat_activity a
                     WHERE a.wait_event_type = 'Lock'
                       AND a.query LIKE '%INSERT INTO%'
                       AND a.query NOT LIKE '%pg_stat_activity%'
                       AND $1 = ANY(pg_blocking_pids(a.pid))
                       AND EXISTS (
                           SELECT 1 FROM pg_locks l
                           WHERE l.pid = a.pid
                             AND l.locktype = 'advisory'
                             AND l.classid = 3
                             AND l.objid = hashtext($2)::oid
                             AND l.granted
                       )
                     LIMIT 1",
                    &[&blocker_pid, &member_id],
                )
                .await
                .unwrap();
            if let Some(row) = row {
                return row.get::<_, i32>(0);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    match waiting {
        Ok(pid) => pid,
        Err(_) => panic!(
            "DM session for member {member_id} did not wait on blocker {blocker_pid} within 10s; sessions: {}",
            lock_observation_dump(&observer).await
        ),
    }
}

async fn wait_for_lock_blocked_by(env: &Env, blocker_pid: i32, pattern: &str) -> i32 {
    let observer = env.observer().await;
    let waiting = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let row = observer
                .query_opt(
                    "SELECT pid FROM pg_stat_activity
                     WHERE wait_event_type = 'Lock'
                       AND query LIKE '%' || $1 || '%'
                       AND query NOT LIKE '%pg_stat_activity%'
                       AND $2 = ANY(pg_blocking_pids(pid))
                     LIMIT 1",
                    &[&pattern, &blocker_pid],
                )
                .await
                .unwrap();
            if let Some(row) = row {
                return row.get::<_, i32>(0);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    match waiting {
        Ok(pid) => pid,
        Err(_) => panic!(
            "no statement waited on blocker {blocker_pid} with lock within 10s: {pattern}; sessions: {}",
            lock_observation_dump(&observer).await
        ),
    }
}

async fn wait_for_pid_blocked_by(env: &Env, waiter_pid: i32, blocker_pid: i32) {
    let observer = env.observer().await;
    let waiting = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let row = observer
                .query_one(
                    "SELECT EXISTS (
                         SELECT 1 FROM pg_stat_activity
                         WHERE pid = $1
                           AND wait_event_type = 'Lock'
                           AND $2 = ANY(pg_blocking_pids(pid))
                     )",
                    &[&waiter_pid, &blocker_pid],
                )
                .await
                .unwrap();
            if row.get::<_, bool>(0) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    if waiting.is_err() {
        panic!(
            "pid {waiter_pid} did not wait on blocker {blocker_pid} within 10s; sessions: {}",
            lock_observation_dump(&observer).await
        );
    }
}

#[tokio::test]
async fn sends_and_edits_recheck_membership_after_the_channel_lock() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.encrypted_channel(&alice, &[&bob]).await;
    let device = env.device(&bob).await;
    let epoch = env.epoch(&channel).await;
    let id = Uuid::new_v4().to_string();
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &bob,
            message(&channel, &id, &device, epoch, "v0"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // チャンネルの行をロックしたまま、次の epoch での送信と編集を待たせる。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    toasty::sql::statement("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel.as_str())
        .exec(&mut tx)
        .await
        .unwrap();
    let send = {
        let app = env.app.clone();
        let body = message(
            &channel,
            &Uuid::new_v4().to_string(),
            &device,
            epoch + 1,
            "after",
        );
        let bob = bob.clone();
        tokio::spawn(async move { request(&app, "POST", "/api/messages", &bob, body).await })
    };
    let edit = {
        let app = env.app.clone();
        let body = json!({"content": "", "epoch": epoch + 1, "revision": 1, "ciphertext": ciphertext(&device, "v1")});
        let (bob, path) = (bob.clone(), format!("/api/messages/{id}"));
        tokio::spawn(async move { request(&app, "PATCH", &path, &bob, body).await })
    };
    wait_for_lock(&env, "FOR SHARE").await;
    // 待っているあいだに、キックと epoch の増加を確定させる。
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2")
        .bind(channel.as_str())
        .bind(bob.as_str())
        .exec(&mut tx)
        .await
        .unwrap();
    toasty::sql::statement(
        "UPDATE channels SET membership_epoch = membership_epoch + 1 WHERE id = $1",
    )
    .bind(channel.as_str())
    .exec(&mut tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (status, body) = send.await.unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = edit.await.unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let rows = env
        .query(
            "SELECT json_build_object('revision', revision)::text FROM messages WHERE channel_id = $1",
            &[&channel],
        )
        .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["revision"], 0);
}

#[tokio::test]
async fn member_additions_wait_for_a_device_deletion_bump() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let joined = env.encrypted_channel(&alice, &[&bob]).await;
    let target = env.encrypted_channel(&alice, &[]).await;
    let device = env.device(&bob).await;
    let before = env.epoch(&joined).await;

    // bob が参加しているチャンネルの行をロックし、端末の削除の一括の増加を待たせる。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    toasty::sql::statement("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(joined.as_str())
        .exec(&mut tx)
        .await
        .unwrap();
    let delete = {
        let app = env.app.clone();
        let (bob, path) = (
            bob.clone(),
            format!("/api/e2ee/devices/{}", device.device_id()),
        );
        tokio::spawn(async move { request(&app, "DELETE", &path, &bob, Value::Null).await })
    };
    wait_for_lock(&env, "e2ee_bump_membership_epochs").await;
    // そのあいだの bob の追加は、メンバー単位のロックで端末の削除のコミットを待つ。
    let add = {
        let app = env.app.clone();
        let (alice, path) = (alice.clone(), format!("/api/channels/{target}/users"));
        let body = json!({"user_id": bob});
        tokio::spawn(async move { request(&app, "POST", &path, &alice, body).await })
    };
    wait_for_lock(&env, "pg_advisory_xact_lock(3").await;
    assert!(!add.is_finished());
    tx.commit().await.unwrap();

    let (status, body) = delete.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = add.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(env.epoch(&joined).await, before + 1);
    // 追加は削除のあとに確定したので、追加先の配布先に削除した端末は入らない。
    let (status, page) = env
        .call(
            "GET",
            &format!("/api/e2ee/channels/{target}/recipients"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        page["user_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u == &json!(bob))
    );
    assert!(page["device_keys"][&bob].get(device.device_id()).is_none());
}

#[tokio::test]
async fn encrypted_dm_and_channel_add_do_not_deadlock_on_member_user_row() -> anyhow::Result<()> {
    let env = env().await;
    let alice = env.user().await;
    // 既定チャンネルへの自動参加があると、暗号化チャンネルへの追加が 2 件目の
    // 非 DM 所属になって 23514 で拒否され、ロック順と無関係に失敗する。
    let bob = env.user_without_default_channels().await;
    env.exec(
        "UPDATE users SET guest_kind = 'single' WHERE id = $1",
        &[&bob],
    )
    .await?;
    let target = env.encrypted_channel(&alice, &[]).await;
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let config = Config::from_env();

    let (mut connection, driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(driver);
    let blocker_pid: i32 = connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = connection.transaction().await?;
    // Hold off the DM's channel INSERT after it has taken the per-user E2EE
    // the member advisory lock. The channel add must wait for the user row
    // before it can acquire the advisory lock, preserving users -> advisory.
    blocker
        .batch_execute("LOCK TABLE channels IN SHARE MODE")
        .await?;

    let dm = {
        let app = env.app.clone();
        let actor = alice.clone();
        let target_user = bob.clone();
        tokio::spawn(async move {
            request(
                &app,
                "POST",
                "/api/dms",
                &actor,
                json!({"target_user_ids":[target_user],"encrypted":true}),
            )
            .await
        })
    };
    let dm_pid = wait_for_dm_lock_blocked_by(&env, blocker_pid, &bob).await;

    let add = {
        let app = env.app.clone();
        let actor = alice.clone();
        let path = format!("/api/channels/{target}/users");
        let body = json!({"user_id": bob});
        tokio::spawn(async move { request(&app, "POST", &path, &actor, body).await })
    };
    wait_for_lock_blocked_by(&env, dm_pid, "WHERE id = $1 FOR NO KEY UPDATE").await;
    blocker.commit().await?;

    let (dm_status, dm_body) = dm.await?;
    let (add_status, add_body) = add.await?;
    assert_eq!(dm_status, StatusCode::OK, "{dm_body}");
    assert_eq!(add_status, StatusCode::OK, "{add_body}");
    Ok::<(), anyhow::Error>(())
}

#[tokio::test]
async fn member_additions_do_not_block_posts_to_the_channel() {
    let env = env().await;
    let alice = env.user().await;
    let channel = env.plain_channel(&alice, &[]).await;
    // 追加と同じロック（FOR NO KEY UPDATE）を持ったままでも、投稿の外部キーの検査（FOR KEY SHARE）は待たない。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    toasty::sql::statement("SELECT 1 FROM channels WHERE id = $1 FOR NO KEY UPDATE")
        .bind(channel.as_str())
        .exec(&mut tx)
        .await
        .unwrap();
    let (status, body) = tokio::time::timeout(
        Duration::from_secs(10),
        env.call(
            "POST",
            "/api/messages",
            &alice,
            json!({"channel_id": channel, "content": "hi"}),
        ),
    )
    .await
    .expect("posting waited on the member addition lock");
    assert_eq!(status, StatusCode::OK, "{body}");
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn plaintext_resends_return_the_stored_revision() {
    let env = env().await;
    let alice = env.user().await;
    let channel = env.plain_channel(&alice, &[]).await;
    let id = Uuid::new_v4().to_string();
    let create = json!({"id": id, "channel_id": channel, "content": "same"});
    let (status, body) = env
        .call("POST", "/api/messages", &alice, create.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 同じ本文での編集でも revision は増える。
    let path = format!("/api/messages/{id}");
    let (status, body) = env
        .call(
            "PATCH",
            &path,
            &alice,
            json!({"content": "same", "revision": 1}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 作成の再送の応答は、保存済みの revision を返し、それを基にした次の編集が通る。
    let (status, body) = env.call("POST", "/api/messages", &alice, create).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["revision"], 1);
    let next = body["data"]["revision"].as_i64().unwrap() + 1;
    let (status, body) = env
        .call(
            "PATCH",
            &path,
            &alice,
            json!({"content": "same", "revision": next}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["revision"], 2);
}

#[tokio::test]
async fn guest_additions_lock_the_user_row_before_the_member_lock() {
    let env = env().await;
    let alice = env.user().await;
    // 既定チャンネルへの自動参加があると 'single' への更新も追加後の所属も
    // 23514 で拒否され、ロック順と無関係に失敗する。
    let guest = env.user_without_default_channels().await;
    env.exec(
        "UPDATE users SET guest_kind = 'single' WHERE id = $1",
        &[&guest],
    )
    .await
    .unwrap();
    let target = env.encrypted_channel(&alice, &[]).await;

    // 無効化と同じく、users の行を更新してから（一括の増加で）メンバー単位のロックを取ったまま止める。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    let blocker_pid = backend::apps::query(
        &mut tx,
        "SELECT json_build_object('pid', pg_backend_pid())::text",
        &[],
    )
    .await
    .unwrap()[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    toasty::sql::statement("UPDATE users SET guest_kind = 'deactivated' WHERE id = $1")
        .bind(guest.as_str())
        .exec(&mut tx)
        .await
        .unwrap();
    // 追加は、メンバー単位のロックより前に、トリガーと同じ users の行のロックで待つ（逆の順だとデッドロックする）。
    let add = {
        let app = env.app.clone();
        let (alice, path) = (alice.clone(), format!("/api/channels/{target}/users"));
        let body = json!({"user_id": guest});
        tokio::spawn(async move { request(&app, "POST", &path, &alice, body).await })
    };
    wait_for_lock_blocked_by(&env, blocker_pid, "WHERE id = $1 FOR NO KEY UPDATE").await;
    assert!(!add.is_finished());
    tx.rollback().await.unwrap();

    let (status, body) = add.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn member_additions_recheck_deactivation_after_waiting_for_users_lock() -> anyhow::Result<()>
{
    let env = env().await;
    let first = env.user().await;
    let second = env.user().await;

    // (1) 対象の users 行のロックで待つ。追加は作成者と対象を ID 順でロックするので、
    // 対象が先になるように役割を選ぶ。待っているあいだに作成者が別接続で無効化されて
    // も、ロック後の再確認で 403 にしなければ追加が通ってしまう。
    let (target, creator) = if first < second {
        (&first, &second)
    } else {
        (&second, &first)
    };
    let channel = env.plain_channel(creator, &[]).await;

    // 対象の users 行を未コミットのまま持つ（対象の行を書く並行処理なら何でもよい）。
    let mut blocker_db = env.db.clone();
    let mut blocker = blocker_db.transaction().await?;
    let blocker_pid = backend::apps::query(
        &mut blocker,
        "SELECT json_build_object('pid', pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    toasty::sql::statement("UPDATE users SET display_name = 'blocked' WHERE id = $1")
        .bind(target.as_str())
        .exec(&mut blocker)
        .await?;

    let add = {
        let app = env.app.clone();
        let (creator, path) = (creator.clone(), format!("/api/channels/{channel}/users"));
        let body = json!({"user_id": target});
        tokio::spawn(async move { request(&app, "POST", &path, &creator, body).await })
    };
    wait_for_lock_blocked_by(&env, blocker_pid, "users WHERE id = $1 FOR NO KEY UPDATE").await;
    assert!(
        !add.is_finished(),
        "the addition must still wait on the target's users row"
    );

    // 待ちのあいだに、別の接続で作成者を無効化して確定させる。
    env.exec(
        "UPDATE users SET guest_kind = 'deactivated' WHERE id = $1",
        &[creator],
    )
    .await?;
    blocker.commit().await?;

    let (status, body) = tokio::time::timeout(Duration::from_secs(15), add)
        .await
        .map_err(|_| {
            anyhow::anyhow!("the addition never finished after the blocker committed")
        })??;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "Account is deactivated", "{body}");
    assert!(
        !env.query(
            "SELECT json_build_object('exists', EXISTS (SELECT 1 FROM channel_user WHERE channel_id = $1 AND user_id = $2))::text",
            &[&channel, target],
        )
        .await[0]["exists"]
            .as_bool()
            .unwrap_or(true),
        "a member addition must not commit after the creator was deactivated while waiting"
    );

    // (2) 作成者の users 行のロックで待つ（ID 順で対象が先でも、作成者の行で必ず待つ）。
    // 未コミットの無効化そのものがブロックになる。dms.rs と同じく、コミット後に 403 にする。
    let creator = env.user().await;
    let target = env.user().await;
    let channel = env.plain_channel(&creator, &[]).await;

    let mut blocker_db = env.db.clone();
    let mut blocker = blocker_db.transaction().await?;
    let blocker_pid = backend::apps::query(
        &mut blocker,
        "SELECT json_build_object('pid', pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    toasty::sql::statement("UPDATE users SET guest_kind = 'deactivated' WHERE id = $1")
        .bind(creator.as_str())
        .exec(&mut blocker)
        .await?;

    let add = {
        let app = env.app.clone();
        let (creator, path) = (creator.clone(), format!("/api/channels/{channel}/users"));
        let body = json!({"user_id": target});
        tokio::spawn(async move { request(&app, "POST", &path, &creator, body).await })
    };
    wait_for_lock_blocked_by(&env, blocker_pid, "users WHERE id = $1 FOR NO KEY UPDATE").await;
    assert!(
        !add.is_finished(),
        "the addition must still wait on the creator's users row"
    );
    blocker.commit().await?;

    let (status, body) = tokio::time::timeout(Duration::from_secs(15), add)
        .await
        .map_err(|_| {
            anyhow::anyhow!("the addition never finished after the deactivation committed")
        })??;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "Account is deactivated", "{body}");
    assert!(
        !env.query(
            "SELECT json_build_object('exists', EXISTS (SELECT 1 FROM channel_user WHERE channel_id = $1 AND user_id = $2))::text",
            &[&channel, &target],
        )
        .await[0]["exists"]
            .as_bool()
            .unwrap_or(true),
        "a member addition must not commit after the creator was deactivated while waiting"
    );
    Ok(())
}

#[tokio::test]
async fn direct_join_rechecks_guest_status_after_waiting_for_user_lock() -> anyhow::Result<()> {
    let env = env().await;
    let owner = env.user().await;
    let guest = env.user().await;
    let channel = Uuid::new_v4().to_string();
    let (status, body) = env
        .call(
            "POST",
            "/api/channels",
            &owner,
            json!({"id":channel,"channel_name":format!("public-{}", Uuid::new_v4().simple()),"is_private":false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/notes"),
            &owner,
            json!({"title":"private to members","content":"note body"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Leave guest_kind's update uncommitted so the join's first ordinary read
    // can see the old non-guest value. The join must take the users row lock
    // before checking guest status and reject after this update commits.
    let mut blocker_db = env.db.clone();
    let mut blocker = blocker_db.transaction().await?;
    let blocker_pid = backend::apps::query(
        &mut blocker,
        "SELECT json_build_object('pid', pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    toasty::sql::statement("UPDATE users SET guest_kind = 'multi' WHERE id = $1")
        .bind(guest.as_str())
        .exec(&mut blocker)
        .await?;

    let join = {
        let app = env.app.clone();
        let path = format!("/api/channels/{channel}/join");
        let actor = guest.clone();
        tokio::spawn(async move { request(&app, "POST", &path, &actor, Value::Null).await })
    };
    wait_for_lock_blocked_by(&env, blocker_pid, "users WHERE id = $1 FOR NO KEY UPDATE").await;
    assert!(
        !join.is_finished(),
        "join must wait for the guest transition"
    );
    blocker.commit().await?;

    let (status, body) = tokio::time::timeout(Duration::from_secs(10), join).await??;
    let notes_status = env
        .call(
            "GET",
            &format!("/api/channels/{channel}/notes"),
            &guest,
            Value::Null,
        )
        .await
        .0;
    let is_member = env
        .query(
            "SELECT json_build_object('exists', EXISTS (SELECT 1 FROM channel_user WHERE channel_id = $1 AND user_id = $2))::text",
            &[&channel, &guest],
        )
        .await[0]
            ["exists"]
            .as_bool()
            .unwrap_or(true);
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a join racing a multi-guest transition must be rejected; response={body}, note read status={notes_status}, membership={is_member}"
    );
    assert_eq!(
        notes_status,
        StatusCode::NOT_FOUND,
        "a guest whose join raced with the transition must not read public-channel notes"
    );
    assert!(!is_member, "the rejected guest must not become a member");

    // E2EE channels are private and direct join remains forbidden through the
    // same handler; invited membership is still handled by the add-member path.
    let encrypted = env.encrypted_channel(&owner, &[]).await;
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{encrypted}/join"),
            &guest,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        !env.query(
            "SELECT json_build_object('exists', EXISTS (SELECT 1 FROM channel_user WHERE channel_id = $1 AND user_id = $2))::text",
            &[&encrypted, &guest],
        )
        .await[0]["exists"]
            .as_bool()
            .unwrap_or(true),
        "a guest cannot directly join an E2EE channel"
    );
    Ok(())
}

#[tokio::test]
async fn encrypted_dm_pins_guest_status_before_waiting_for_member_lock() -> anyhow::Result<()> {
    let env = env().await;
    let config = Config::from_env();
    let alice = env.user().await;
    // 既定チャンネルへの自動参加があると 'single' への更新が 23514 で拒否され、
    // ロック順と無関係に失敗する。
    let bob = env.user_without_default_channels().await;
    let mut blocker_db = env.db.clone();
    let mut blocker = blocker_db.transaction().await?;
    let blocker_pid = backend::apps::query(
        &mut blocker,
        "SELECT json_build_object('pid', pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    toasty::sql::statement("SELECT pg_advisory_xact_lock(3, hashtext($1))")
        .bind(bob.as_str())
        .exec(&mut blocker)
        .await?;

    let dm = {
        let app = env.app.clone();
        let actor = alice.clone();
        let target = bob.clone();
        tokio::spawn(async move {
            request(
                &app,
                "POST",
                "/api/dms",
                &actor,
                json!({"target_user_ids":[target],"encrypted":true}),
            )
            .await
        })
    };
    let dm_pid = wait_for_lock_blocked_by(&env, blocker_pid, "pg_advisory_xact_lock(3").await;

    let (update_connection, update_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(update_driver);
    let update_pid: i32 = update_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let update_bob = bob.clone();
    let update = tokio::spawn(async move {
        update_connection
            .execute(
                "UPDATE users SET guest_kind = 'single' WHERE id = $1",
                &[&update_bob],
            )
            .await
    });
    wait_for_pid_blocked_by(&env, update_pid, dm_pid).await;
    assert!(
        !update.is_finished(),
        "encrypted DM must hold the target users row before waiting on the member advisory lock"
    );

    blocker.commit().await?;
    let (status, body) = tokio::time::timeout(Duration::from_secs(10), dm).await??;
    assert_eq!(status, StatusCode::OK, "{body}");
    update.await??;
    Ok(())
}

#[tokio::test]
async fn membership_insert_racing_single_guest_transition_preserves_single_channel_limit()
-> anyhow::Result<()> {
    let env = env().await;
    // The canonical deploy keeps the old trigger function while old backend
    // instances run. This race is protected by the deferred post-rollout
    // migration, so apply it here to test the final lock order.
    let (deferred_db, deferred_driver) =
        tokio_postgres::connect(&Config::from_env().database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(deferred_driver);
    // Capture the canonical definition so the shared test database is restored
    // even when the race body returns early or panics. The deferred function
    // must not leak into tests that run after this one.
    let canonical = deferred_db
        .query_one(
            "SELECT pg_get_functiondef('app_guest_membership_rules()'::regprocedure)",
            &[],
        )
        .await?
        .get::<_, String>(0);
    deferred_db
        .batch_execute(include_str!(
            "../../infra/db/deferred-migrations/20261129020009_guest_membership_lock_order.sql"
        ))
        .await?;
    let result = std::panic::AssertUnwindSafe(run_membership_race(&env))
        .catch_unwind()
        .await;
    deferred_db.batch_execute(&canonical).await?;
    match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn run_membership_race(env: &Env) -> anyhow::Result<()> {
    let guest = env.user().await;
    let owner = env.user().await;
    let first_channel = Uuid::new_v4().to_string();
    let second_channel = Uuid::new_v4().to_string();

    for (id, name) in [(&first_channel, "first"), (&second_channel, "second")] {
        env.exec(
            "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $2, $3, false)",
            &[id, &format!("guest_race_{name}_{}", Uuid::new_v4().simple()), &owner],
        )
        .await?;
    }
    env.exec(
        "INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, now())",
        &[&guest, &first_channel],
    )
    .await?;

    // Hold the second membership uncommitted while another transaction changes the user to
    // `single`. The status trigger cannot see this row; the insert trigger must serialize both.
    // Both sides run on connections with known PIDs so the observation below can target this
    // transition waiting on this membership insert; a query-shaped match could catch an
    // unrelated test's UPDATE and let the race pass without the needed wait.
    let database_url = Config::from_env().database_url;
    let (mut membership_connection, membership_driver) =
        tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(membership_driver);
    let membership_pid: i32 = membership_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let mut membership_tx = membership_connection.transaction().await?;
    membership_tx
        .execute(
            "INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, now())",
            &[&guest, &second_channel],
        )
        .await?;

    let transition_url = database_url.clone();
    let transitioning_guest = guest.clone();
    let (pid_tx, pid_rx) = tokio::sync::oneshot::channel::<i32>();
    let transition = tokio::spawn(async move {
        let (mut connection, driver) =
            tokio_postgres::connect(&transition_url, tokio_postgres::NoTls).await?;
        tokio::spawn(driver);
        let tx = connection.transaction().await?;
        // Send the PID before the UPDATE: the UPDATE blocks on the membership
        // transaction's users-row lock, so reporting the PID only afterwards
        // would deadlock the test itself.
        let pid: i32 = tx.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
        let _ = pid_tx.send(pid);
        tx.execute(
            "UPDATE users SET guest_kind = 'single' WHERE id = $1",
            &[&transitioning_guest],
        )
        .await?;
        tx.commit().await?;
        Ok::<(), tokio_postgres::Error>(())
    });
    let transition_pid = pid_rx.await?;
    let (observer, observer_driver) =
        tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let transition_was_waiting = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if transition.is_finished() {
                return Ok::<_, tokio_postgres::Error>(false);
            }
            let row = observer
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock' AND $2 = ANY(pg_blocking_pids(pid)))",
                    &[&transition_pid, &membership_pid],
                )
                .await?;
            if row.get::<_, bool>(0) {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    membership_tx.commit().await?;
    let transition_result = tokio::time::timeout(Duration::from_secs(5), transition).await??;

    let state = env
        .query(
            "SELECT json_build_object('guest_kind', u.guest_kind, 'memberships', count(cu.channel_id) FILTER (WHERE NOT c.is_dm AND c.deleted_at IS NULL))::text FROM users u LEFT JOIN channel_user cu ON cu.user_id = u.id LEFT JOIN channels c ON c.id = cu.channel_id WHERE u.id = $1 GROUP BY u.id",
            &[&guest],
        )
        .await;
    let guest_kind = state[0]["guest_kind"].as_str().unwrap();
    let membership_count = state[0]["memberships"].as_i64().unwrap();
    assert!(
        transition_was_waiting,
        "guest_kind transition must wait for the in-flight membership insert"
    );
    let transition_error = transition_result
        .expect_err("single guest transition must reject the second non-DM membership");
    assert_eq!(
        transition_error.code().map(|code| code.code()),
        Some("23514"),
        "the rejection must be the single-channel membership limit, not a lock wait or timeout: {transition_error}"
    );
    assert!(
        guest_kind != "single" || membership_count <= 1,
        "guest_kind={guest_kind}, non-DM memberships={membership_count}; a single-channel guest cannot have two"
    );
    Ok(())
}
