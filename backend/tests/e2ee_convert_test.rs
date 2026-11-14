//! 既存のチャンネルの暗号化（#367）の結合テスト。仕様は docs/e2ee.md の「既存のチャンネルの暗号化」。

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

    async fn call(
        &self,
        method: &str,
        path: &str,
        actor: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        request(&self.app, method, path, actor, body).await
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/users",
                &id,
                json!({"id": id, "user_name": format!("conv_{}", Uuid::new_v4().simple()), "display_name": "Convert"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        id
    }

    async fn channel(&self, creator: &str, private: bool, members: &[&str]) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/channels",
                creator,
                json!({"id": id, "channel_name": format!("conv-{}", Uuid::new_v4().simple()), "is_private": private}),
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

    async fn post(&self, channel: &str, actor: &str, content: &str) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/messages",
                actor,
                json!({"id": id, "channel_id": channel, "content": content}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        id
    }

    async fn encrypt(&self, channel: &str, actor: &str) -> (StatusCode, Value) {
        self.call(
            "POST",
            &format!("/api/channels/{channel}/encrypt"),
            actor,
            Value::Null,
        )
        .await
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

    async fn message_row(&self, id: &str) -> Value {
        self.query(
            "SELECT json_build_object('content', content, 'epoch', epoch, 'ciphertext', ciphertext,
               'deleted', deleted_at IS NOT NULL)::text FROM messages WHERE id = $1",
            &[id],
        )
        .await
        .remove(0)
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

#[tokio::test]
async fn encrypting_keeps_earlier_messages_and_requires_ciphertext_afterwards() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.channel(&alice, true, &[&bob]).await;
    let before = env.post(&channel, &bob, "before the switch").await;
    let edited = env.post(&channel, &bob, "will be deleted").await;

    let (status, body) = env.encrypt(&channel, &alice).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["encrypted"], true);
    assert_eq!(body["data"]["membership_epoch"], 1);
    assert!(
        body["txid"].is_number() || body["txid"].is_string(),
        "{body}"
    );

    // 切り替える前のメッセージは平文のまま残る。
    let row = env.message_row(&before).await;
    assert_eq!(row["content"], "before the switch");
    assert!(row["ciphertext"].is_null() && row["epoch"].is_null());

    // 切り替えたあとは平文を送れず、暗号文だけを受け付ける。
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &bob,
            json!({"channel_id": channel, "content": "plaintext after"}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN"),
        "{body}"
    );
    let device = env.device(&bob).await;
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &bob,
            json!({"id": Uuid::new_v4().to_string(), "channel_id": channel, "content": "", "epoch": 1,
                "revision": 0, "ciphertext": ciphertext(&device, "after")}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 切り替える前のメッセージは、平文でも暗号文でも編集できない。リアクションと削除はできる。
    for patch in [
        json!({"content": "edited"}),
        json!({"content": "", "epoch": 1, "revision": 1, "ciphertext": ciphertext(&device, "x")}),
    ] {
        let (status, body) = env
            .call("PATCH", &format!("/api/messages/{before}"), &bob, patch)
            .await;
        assert_eq!(
            (status, code(&body)),
            (StatusCode::CONFLICT, "E2EE_LEGACY_IMMUTABLE"),
            "{body}"
        );
    }
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/messages/{before}/reactions"),
            &alice,
            json!({"emoji": ":thumbsup:"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = env
        .call(
            "DELETE",
            &format!("/api/messages/{edited}"),
            &bob,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(env.message_row(&edited).await["deleted"], true);
    assert_eq!(
        env.message_row(&before).await["content"],
        "before the switch"
    );

    // もう一度呼んでも何もしない（epoch も増えない）。元には戻せない。
    let (status, body) = env.encrypt(&channel, &alice).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["membership_epoch"], 1);
    assert!(body["txid"].is_null());
    let (status, body) = env
        .call(
            "PATCH",
            &format!("/api/channels/{channel}"),
            &alice,
            json!({"encrypted": false}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_ENCRYPTED_IMMUTABLE")
    );
    assert!(
        env.exec(
            "UPDATE channels SET encrypted = false WHERE id = $1",
            &[&channel]
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn database_keeps_earlier_rows_from_gaining_plaintext() {
    let env = env().await;
    let alice = env.user().await;
    let channel = env.channel(&alice, true, &[]).await;
    let before = env.post(&channel, &alice, "kept").await;
    let attachment = Uuid::new_v4().to_string();
    let other = Uuid::new_v4().to_string();
    for (id, message) in [(&attachment, before.as_str()), (&other, "")] {
        env.exec(
            "INSERT INTO attachments(id, message_id, uploader_id, channel_id, file_name, file_size, content_type, storage_key)
             VALUES ($1, NULLIF($2, ''), $3, $4, 'photo.png', 1, 'image/png', 'attachments/' || $4 || '/' || $1)",
            &[id, message, &alice, &channel],
        )
        .await
        .unwrap();
    }
    assert_eq!(env.encrypt(&channel, &alice).await.0, StatusCode::OK);

    // 本文、平文の列を足す更新と、暗号文への書き換えは拒否する。
    for sql in [
        "UPDATE messages SET content = 'changed' WHERE id = $1",
        "UPDATE messages SET metadata = '{}' WHERE id = $1",
        "UPDATE messages SET apub = true WHERE id = $1",
        "UPDATE messages SET content = '', epoch = 1, ciphertext = '{}'::jsonb WHERE id = $1",
    ] {
        assert!(env.exec(sql, &[&before]).await.is_err(), "{sql}");
    }
    // 別のチャンネルの平文の行を移してくることもできない（切り替える前からあった行の例外に当たらない）。
    let other_channel = env.channel(&alice, true, &[]).await;
    let moved = env.post(&other_channel, &alice, "from elsewhere").await;
    assert!(
        env.exec(
            "UPDATE messages SET channel_id = $2 WHERE id = $1",
            &[&moved, &channel]
        )
        .await
        .is_err()
    );
    // 本文を変えない更新と、消すだけの更新はできる。
    env.exec(
        "UPDATE messages SET updated_at = now() WHERE id = $1",
        &[&before],
    )
    .await
    .unwrap();
    // 平文の添付は、印の更新だけできる。紐付けを変えたり、新しく置いたりはできない。
    env.exec(
        "UPDATE attachments SET is_sensitive = true WHERE id = $1",
        &[&attachment],
    )
    .await
    .unwrap();
    assert!(
        env.exec(
            "UPDATE attachments SET message_id = $2 WHERE id = $1",
            &[&other, &before]
        )
        .await
        .is_err()
    );
    assert!(
        env.exec(
            "INSERT INTO attachments(id, uploader_id, channel_id, file_name, file_size, content_type, storage_key)
             VALUES (gen_random_uuid()::text, $1, $2, 'new.png', 1, 'image/png', 'attachments/' || $2 || '/x')",
            &[&alice, &channel],
        )
        .await
        .is_err()
    );
    // 新しい行は暗号文だけ。
    assert!(
        env.exec(
            "INSERT INTO messages(id, channel_id, user_id, content) VALUES (gen_random_uuid()::text, $1, $2, 'plain')",
            &[&channel, &alice],
        )
        .await
        .is_err()
    );
    env.exec(
        "UPDATE messages SET deleted_at = now() WHERE id = $1",
        &[&before],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn who_can_encrypt_which_channels() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;

    let public = env.channel(&alice, false, &[]).await;
    let (status, body) = env.encrypt(&public, &alice).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PUBLIC_FORBIDDEN")
    );

    let private = env.channel(&alice, true, &[&bob]).await;
    // メンバーでなければ存在を隠す。メンバーでも作成者か管理者でなければ 403。
    assert_eq!(env.encrypt(&private, &carol).await.0, StatusCode::NOT_FOUND);
    assert_eq!(env.encrypt(&private, &bob).await.0, StatusCode::FORBIDDEN);

    let disabled = env_with(false).await;
    let (status, body) = disabled.encrypt(&private, &alice).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "E2EE_DISABLED")
    );

    let archived = env.channel(&alice, true, &[]).await;
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{archived}/archive"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        env.encrypt(&archived, &alice).await.0,
        StatusCode::BAD_REQUEST
    );

    // DM とグループ DM は、メンバーなら誰でも切り替えられる。
    let (status, body) = env
        .call(
            "POST",
            "/api/dms",
            &alice,
            json!({"target_user_ids": [bob, carol]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let group = body["data"]["id"].as_str().unwrap().to_owned();
    let name = body["data"]["channel_name"].as_str().unwrap().to_owned();
    let (status, body) = env.encrypt(&group, &carol).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["encrypted"], true);
    assert_eq!(body["data"]["channel_name"], name);
}

#[tokio::test]
async fn one_to_one_dms_take_the_encrypted_dm_name() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let dm = |encrypted: bool| json!({"target_user_ids": [bob], "encrypted": encrypted});
    let (status, body) = env.call("POST", "/api/dms", &alice, dm(false)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plain = body["data"]["id"].as_str().unwrap().to_owned();
    let before = env.post(&plain, &alice, "hello").await;

    let (status, body) = env.encrypt(&plain, &bob).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["data"]["channel_name"]
            .as_str()
            .unwrap()
            .starts_with("edm:")
    );
    // 暗号化 DM を求めると切り替えた DM が返り、平文の DM を求めると新しい DM ができる。
    let (status, body) = env.call("POST", "/api/dms", &alice, dm(true)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["id"], plain.as_str());
    let (status, body) = env.call("POST", "/api/dms", &alice, dm(false)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let new_plain = body["data"]["id"].as_str().unwrap().to_owned();
    assert_ne!(new_plain, plain);
    assert_eq!(body["data"]["encrypted"], false);
    assert_eq!(env.message_row(&before).await["content"], "hello");

    // 同じ 2 人の暗号化 DM がすでにあれば、もう 1 つの平文の DM は切り替えられない。
    let (status, body) = env.encrypt(&new_plain, &alice).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_CONVERSION_BLOCKED"),
        "{body}"
    );
    assert_eq!(body["blockers"], json!(["encrypted_dm"]));
}

#[tokio::test]
async fn apps_and_webhooks_block_encryption() {
    let env = env().await;
    let alice = env.user().await;
    let channel = env.channel(&alice, true, &[]).await;
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/webhooks"),
            &alice,
            json!({"name": "CI"}),
        )
        .await;
    assert!(status.is_success(), "{body}");
    let bot = env
        .query(
            "SELECT json_build_object('id', user_id)::text FROM channel_webhooks WHERE channel_id = $1",
            &[&channel],
        )
        .await
        .remove(0)["id"]
        .as_str()
        .unwrap()
        .to_owned();
    env.exec(
        "INSERT INTO channel_user(channel_id, user_id, joined_at) VALUES ($1, $2, CURRENT_TIMESTAMP) ON CONFLICT DO NOTHING",
        &[&channel, &bot],
    )
    .await
    .unwrap();

    let (status, body) = env.encrypt(&channel, &alice).await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::CONFLICT, "E2EE_CONVERSION_BLOCKED"),
        "{body}"
    );
    assert_eq!(body["blockers"], json!(["apps", "webhooks"]));
    // DB のトリガーも、直接の切り替えを拒否する。
    assert!(
        env.exec(
            "UPDATE channels SET encrypted = true WHERE id = $1",
            &[&channel]
        )
        .await
        .is_err()
    );
    env.exec(
        "DELETE FROM channel_webhooks WHERE channel_id = $1",
        &[&channel],
    )
    .await
    .unwrap();
    env.exec(
        "DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2",
        &[&channel, &bot],
    )
    .await
    .unwrap();
    assert_eq!(env.encrypt(&channel, &alice).await.0, StatusCode::OK);
    // 切り替えたあとは、Webhook を作れない（API の 403 と、DB のトリガー）。
    assert!(
        env.exec(
            "INSERT INTO channel_webhooks(id, channel_id, name, secret, user_id) VALUES (gen_random_uuid()::text, $1, 'x', 'x', $2)",
            &[&channel, &alice],
        )
        .await
        .is_err()
    );
}

async fn wait_for_lock(env: &Env, pattern: &str) {
    for _ in 0..100 {
        let waiting = env
            .query(
                "SELECT json_build_object('n', count(*))::text FROM pg_stat_activity
                 WHERE wait_event_type = 'Lock' AND query LIKE '%' || $1 || '%'
                   AND query NOT LIKE '%pg_stat_activity%'",
                &[pattern],
            )
            .await;
        if waiting[0]["n"].as_i64().unwrap_or(0) > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no statement waited on a lock: {pattern}");
}

#[tokio::test]
async fn plaintext_writes_wait_for_the_switch() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let channel = env.channel(&alice, true, &[&bob]).await;

    // 切り替えと同じく、チャンネルの行を FOR UPDATE でロックしたまま、平文の送信とメンバーの追加を待たせる。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    toasty::sql::statement("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(channel.as_str())
        .exec(&mut tx)
        .await
        .unwrap();
    let send = {
        let app = env.app.clone();
        let body = json!({"channel_id": channel, "content": "racing"});
        let bob = bob.clone();
        tokio::spawn(async move { request(&app, "POST", "/api/messages", &bob, body).await })
    };
    wait_for_lock(&env, "FOR KEY SHARE").await;
    let add = {
        let app = env.app.clone();
        let (alice, path) = (alice.clone(), format!("/api/channels/{channel}/users"));
        let body = json!({"user_id": carol});
        tokio::spawn(async move { request(&app, "POST", &path, &alice, body).await })
    };
    wait_for_lock(&env, "FOR NO KEY UPDATE").await;
    toasty::sql::statement(
        "UPDATE channels SET encrypted = true, membership_epoch = membership_epoch + 1 WHERE id = $1",
    )
    .bind(channel.as_str())
    .exec(&mut tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (status, body) = send.await.unwrap();
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_PLAINTEXT_FORBIDDEN"),
        "{body}"
    );
    let (status, body) = add.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let rows = env
        .query(
            "SELECT json_build_object('n', count(*))::text FROM messages WHERE channel_id = $1",
            &[&channel],
        )
        .await;
    assert_eq!(rows[0]["n"], 0);
    // やり直せば、暗号化チャンネルの経路で足され、epoch が増える。
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/users"),
            &alice,
            json!({"user_id": carol}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let epoch = env
        .query(
            "SELECT membership_epoch::text FROM channels WHERE id = $1",
            &[&channel],
        )
        .await;
    assert_eq!(epoch[0], 2);
}
