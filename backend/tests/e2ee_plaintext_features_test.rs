//! 暗号化チャンネルで平文に依存する機能を止める（#358）の結合テスト。仕様は docs/e2ee.md の「機能ごとの扱い」
//! 「メッセージ」「DB」。各機能が暗号化チャンネルでは動かず、平文チャンネルでは今までどおり動くことを確かめる。
//!
//! Slack 取り込みの拒否は、取り込みのテストの仕組みがある `slack_import_test.rs` で確かめる。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zoo_crypto::{
    DeviceAccount, EncryptedMessage, InboundSessionStore, Mentions, MessageContent,
    OutboundSession, OutgoingMessage,
};

struct Env {
    app: Router,
    db: toasty::Db,
}

async fn env() -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.e2ee_encrypted_channels_enabled = true;
    let db = init_db(&config.database_url).await.unwrap();
    let app = create_router(AppState::new(db.clone(), config));
    Env { app, db }
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
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
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

/// テストで作った App（インストール済み）。
struct TestApp {
    id: String,
    installation: String,
    bot: String,
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

    /// ユーザーを作り、`(id, user_name)` を返す。
    async fn user(&self) -> (String, String) {
        let id = Uuid::new_v4().to_string();
        let name = format!("e2ee_{}", Uuid::new_v4().simple());
        let (status, body) = self
            .call(
                "POST",
                "/api/users",
                &id,
                json!({"id": id, "user_name": name, "display_name": "E2EE"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        (id, name)
    }

    async fn channel(&self, creator: &str, members: &[&str], encrypted: bool) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/channels",
                creator,
                json!({"id": id, "channel_name": format!("c-{}", Uuid::new_v4().simple()),
                    "is_private": true, "encrypted": encrypted}),
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

    async fn epoch(&self, channel: &str) -> i64 {
        self.query(
            "SELECT membership_epoch::text FROM channels WHERE id = $1",
            &[channel],
        )
        .await[0]
            .as_i64()
            .unwrap()
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

    /// 暗号化チャンネルに送り、メッセージの ID を返す。
    async fn send_encrypted(
        &self,
        user: &str,
        account: &DeviceAccount,
        channel: &str,
        mentions: Value,
    ) -> (StatusCode, Value) {
        let epoch = self.epoch(channel).await;
        self.call(
            "POST",
            "/api/messages",
            user,
            json!({"id": Uuid::new_v4().to_string(), "channel_id": channel, "content": "",
            "epoch": epoch, "revision": 0, "mentions": mentions,
            "ciphertext": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "sender_key": account.identity_keys().curve25519.to_base64(),
                "device_id": account.device_id(),
                "session_id": "session",
                "ciphertext": "c",
            }}),
        )
        .await
    }

    /// メッセージの通知を `user_id -> notification_type` で返す。
    async fn notifications(&self, message: &str) -> std::collections::HashMap<String, String> {
        self.query(
            "SELECT json_build_object('user', user_id, 'type', notification_type)::text
             FROM notification_events WHERE message_id = $1",
            &[message],
        )
        .await
        .into_iter()
        .map(|row| {
            (
                row["user"].as_str().unwrap().to_string(),
                row["type"].as_str().unwrap().to_string(),
            )
        })
        .collect()
    }

    /// `messages:read` などを持つ App を作ってインストールする（`owner` が作成者）。
    async fn app(&self, owner: &str) -> TestApp {
        let admin = backend::auth::DEFAULT_DEV_USER_ID;
        self.call("POST", "/api/users", admin, json!({"id": admin, "user_name": format!("admin_{}", Uuid::new_v4().simple()), "display_name": "Admin"}))
            .await;
        let manifest = json!({"scopes":["channels:read","channels:manage","messages:read","messages:write","files:write","dm:write"],
            "events":["message.created"],"transport":"websocket","dm_enabled":true,
            "redirect_uris":["https://example.com/callback"]});
        let (status, created) = self
            .call(
                "POST",
                "/api/apps",
                owner,
                json!({"name":"E2EE","description":"E2EE app","manifest":manifest}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let id = created["id"].as_str().unwrap().to_string();
        let action = |a: &str| format!("/api/apps/{id}/actions/{a}");
        let (status, body) = self
            .call("POST", &action("submit"), owner, Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = self
            .call("POST", &action("install"), admin, json!({"revision": 1}))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let row = &self
            .query(
                "SELECT json_build_object('installation', i.id, 'bot', a.bot_user_id)::text
                 FROM app_installations i JOIN apps a ON a.id = i.app_id WHERE a.id = $1",
                &[&id],
            )
            .await[0];
        TestApp {
            id,
            installation: row["installation"].as_str().unwrap().to_string(),
            bot: row["bot"].as_str().unwrap().to_string(),
        }
    }

    /// `user` として動く委任トークンを発行する。
    async fn delegated_token(&self, app: &TestApp, user: &str) -> String {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        let scopes = json!([
            "channels:read",
            "messages:read",
            "messages:write",
            "files:write"
        ]);
        let token = backend::apps::mint(&mut tx, &app.installation, Some(user), &scopes)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        token["access_token"].as_str().unwrap().to_string()
    }
}

// ---------------------------------------------------------------------------
// メンション通知と Web Push

#[tokio::test]
async fn encrypted_mentions_come_only_from_the_plaintext_mentions() {
    let env = env().await;
    let (alice, _) = env.user().await;
    let (bob, bob_name) = env.user().await;
    let (carol, _) = env.user().await;
    let (outsider, _) = env.user().await;
    let encrypted = env.channel(&alice, &[&bob, &carol], true).await;
    let device = env.device(&alice).await;

    // user_ids のメンバーだけに通知する（本人とメンバーでない人には作らない）
    let (status, body) = env
        .send_encrypted(
            &alice,
            &device,
            &encrypted,
            json!({"user_ids": [bob, outsider, alice], "channel": false, "here": false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let notified = env
        .notifications(body["data"]["id"].as_str().unwrap())
        .await;
    assert_eq!(notified.len(), 1, "{notified:?}");
    assert_eq!(notified[&bob], "mention");

    // @channel はメンバー全員
    let (status, body) = env
        .send_encrypted(&alice, &device, &encrypted, json!({"channel": true}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let notified = env
        .notifications(body["data"]["id"].as_str().unwrap())
        .await;
    assert_eq!(notified.len(), 2, "{notified:?}");
    assert_eq!(notified[&bob], "mention");
    assert_eq!(notified[&carol], "mention");

    // mentions がなければ、メンションの通知はない
    let (status, body) = env
        .send_encrypted(&alice, &device, &encrypted, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        env.notifications(body["data"]["id"].as_str().unwrap())
            .await
            .is_empty()
    );

    // 形の違う mentions は 400
    let (status, body) = env
        .send_encrypted(&alice, &device, &encrypted, json!({"user_ids": ["bob"]}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let many: Vec<String> = (0..201).map(|_| Uuid::new_v4().to_string()).collect();
    let (status, body) = env
        .send_encrypted(&alice, &device, &encrypted, json!({"user_ids": many}))
        .await;
    assert_eq!(
        (status, code(&body), body["limit"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            "E2EE_LIMIT_EXCEEDED",
            Some("mentions")
        ),
        "{body}"
    );

    // 平文チャンネルは今までどおり本文から作り、mentions は使わない
    let plain = env.channel(&alice, &[&bob, &carol], false).await;
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &alice,
            json!({"channel_id": plain, "content": format!("hi @{bob_name}"),
                "mentions": {"user_ids": [carol]}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let notified = env
        .notifications(body["data"]["id"].as_str().unwrap())
        .await;
    assert_eq!(notified.len(), 1, "{notified:?}");
    assert_eq!(notified[&bob], "mention");
}

/// 暗号化チャンネルの Web Push には暗号文だけが入り、サーバーは平文を持たない。鍵を持つ端末は、
/// Push の中身だけで本文を復号できる (#365)。
#[tokio::test]
async fn encrypted_push_carries_the_ciphertext_but_no_plaintext() {
    const SECRET: &str = "合言葉は 🦊 zoo-push-secret";
    let env = env().await;
    let (alice, _) = env.user().await;
    let (bob, bob_name) = env.user().await;
    let encrypted = env.channel(&alice, &[&bob], true).await;
    let device = env.device(&alice).await;

    // 送信端末と同じ鍵を持つ受信用セッション（Bob の端末が to-device で受け取るものと同じ鍵）
    let epoch = env.epoch(&encrypted).await as u64;
    let mut outbound = OutboundSession::new(&encrypted, epoch, 1_700_000_000_000);
    let mut receiver = InboundSessionStore::new();
    receiver.add_outbound(&device, &outbound);
    let message_id = Uuid::new_v4().to_string();
    let ciphertext = outbound
        .encrypt(
            &device,
            &OutgoingMessage {
                message_id: message_id.clone(),
                parent_id: None,
                revision: 0,
                content: MessageContent {
                    body: SECRET.into(),
                    format: "mfm".into(),
                    kind: "message".into(),
                },
                mentions: Mentions {
                    user_ids: vec![bob.clone()],
                    ..Mentions::default()
                },
            },
        )
        .unwrap();
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &alice,
            json!({"id": message_id, "channel_id": encrypted, "content": "", "epoch": epoch,
                "revision": 0, "mentions": {"user_ids": [bob]}, "ciphertext": ciphertext}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut db = env.db.clone();
    let notification = backend::models::Notification::filter(
        backend::models::Notification::fields()
            .message_id()
            .eq(message_id.as_str()),
    )
    .first()
    .exec(&mut db)
    .await
    .unwrap()
    .expect("Bob への通知");
    assert_eq!(notification.user_id, bob);
    let payload = backend::push::payload_json(&mut db, &notification)
        .await
        .unwrap()
        .unwrap();

    // サーバーが作る Push には平文がない。本文は「新しいメッセージ」
    let serialized = payload.to_string();
    assert!(!serialized.contains("zoo-push-secret"), "{serialized}");
    assert!(!serialized.contains("合言葉"), "{serialized}");
    assert_eq!(payload["body"], backend::push::ENCRYPTED_MESSAGE_BODY);
    assert_eq!(payload["e2ee"]["recipient_id"], bob.as_str());
    let row = &payload["e2ee"]["message"];
    assert_eq!(row["ciphertext"], ciphertext);

    // 鍵を持つ端末は、Push の中身だけで、受信時の検証を通して復号できる
    let decrypted = receiver
        .decrypt(&EncryptedMessage {
            channel_id: row["channel_id"].as_str().unwrap(),
            message_id: row["message_id"].as_str().unwrap(),
            parent_id: row["parent_id"].as_str(),
            user_id: row["user_id"].as_str().unwrap(),
            message_kind: row["message_kind"].as_str().unwrap(),
            epoch: row["epoch"].as_u64().unwrap(),
            revision: row["revision"].as_u64().unwrap(),
            ciphertext: &row["ciphertext"],
        })
        .unwrap();
    assert_eq!(decrypted.payload.content.body, SECRET);

    // 削除したメッセージは送らない（暗号文も消える）
    let (status, body) = env
        .call(
            "DELETE",
            &format!("/api/messages/{message_id}"),
            &alice,
            Value::Null,
        )
        .await;
    assert!(status.is_success(), "{status} {body}");
    assert!(
        backend::push::payload_json(&mut db, &notification)
            .await
            .unwrap()
            .is_none()
    );

    // 平文チャンネルは今までどおり本文を送り、e2ee を付けない
    let plain = env.channel(&alice, &[&bob], false).await;
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &alice,
            json!({"channel_id": plain, "content": format!("hi @{bob_name}")}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plain_message = body["data"]["id"].as_str().unwrap().to_string();
    let notification = backend::models::Notification::filter(
        backend::models::Notification::fields()
            .message_id()
            .eq(plain_message.as_str()),
    )
    .first()
    .exec(&mut db)
    .await
    .unwrap()
    .unwrap();
    let payload = backend::push::payload_json(&mut db, &notification)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payload["body"], format!("hi @{bob_name}"));
    assert!(payload.get("e2ee").is_none(), "{payload}");
}

// ---------------------------------------------------------------------------
// 添付、Webhook、App（Bot）の参加

#[tokio::test]
async fn attachments_and_webhooks_are_rejected_in_encrypted_channels() {
    let env = env().await;
    let (alice, _) = env.user().await;
    let encrypted = env.channel(&alice, &[], true).await;
    let plain = env.channel(&alice, &[], false).await;

    let presign = |channel: &str| json!({"channel_id": channel, "file_name": "a.png", "file_size": 10, "content_type": "image/png"});
    let (status, body) = env
        .call(
            "POST",
            "/api/attachments/presign",
            &alice,
            presign(&encrypted),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::BAD_REQUEST, "E2EE_ATTACHMENTS_FORBIDDEN"),
        "{body}"
    );
    let (status, body) = env
        .call("POST", "/api/attachments/presign", &alice, presign(&plain))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{encrypted}/webhooks"),
            &alice,
            json!({"name": "CI"}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "E2EE_WEBHOOKS_FORBIDDEN"),
        "{body}"
    );
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/channels/{plain}/webhooks"),
            &alice,
            json!({"name": "CI"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn apps_cannot_join_or_use_encrypted_channels() {
    let env = env().await;
    let (alice, _) = env.user().await;
    let app = env.app(&alice).await;
    let encrypted = env.channel(&alice, &[], true).await;
    let plain = env.channel(&alice, &[], false).await;

    // 管理画面からの追加（add_channel）
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/apps/{}/channels/{encrypted}", app.id),
            &alice,
            Value::Null,
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
            &format!("/api/apps/{}/channels/{plain}", app.id),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 暗号化 DM の相手に Bot は選べない
    let (status, body) = env
        .call(
            "POST",
            "/api/dms",
            &alice,
            json!({"target_user_ids": [app.bot], "encrypted": true}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "E2EE_APPS_FORBIDDEN"),
        "{body}"
    );

    // DB のトリガー：API を通らない Bot の参加も拒否する
    let inserted = env
        .exec(
            "INSERT INTO channel_user(channel_id, user_id, joined_at) VALUES ($1, $2, CURRENT_TIMESTAMP)",
            &[&encrypted, &app.bot],
        )
        .await;
    assert!(inserted.is_err(), "a bot joined an encrypted channel");
    let bots = env
        .query(
            "SELECT json_build_object('n', count(*))::text FROM channel_user cu JOIN users u ON u.id = cu.user_id
             WHERE cu.channel_id = $1 AND u.is_bot",
            &[&encrypted],
        )
        .await;
    assert_eq!(bots[0]["n"], 0);

    // 委任トークンは、メンバーのユーザーとして動いても暗号化チャンネルを読み書きできない
    let token = env.delegated_token(&app, &alice).await;
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/messages",
            &token,
            json!({"channel_id": encrypted, "content": "plaintext"}),
        )
        .await;
    assert_eq!(
        (status, code(&body)),
        (StatusCode::FORBIDDEN, "E2EE_APPS_FORBIDDEN"),
        "{body}"
    );
    let (status, body) = env
        .call(
            "GET",
            &format!("/api/apps/v1/shapes/messages?channel_id={encrypted}&offset=-1"),
            &token,
            Value::Null,
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
            "/api/apps/v1/messages",
            &token,
            json!({"channel_id": plain, "content": "plaintext"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// ---------------------------------------------------------------------------
// App のイベント

#[tokio::test]
async fn encrypted_messages_are_not_app_events() {
    let env = env().await;
    let (alice, _) = env.user().await;
    let app = env.app(&alice).await;
    let encrypted = env.channel(&alice, &[], true).await;
    let plain = env.channel(&alice, &[], false).await;
    let device = env.device(&alice).await;
    // App のイベントは Bot が参加するチャンネルにだけ届く。暗号化チャンネルには Bot を入れられないので、
    // トリガーで止めるのは、それでも届く経路への備え
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/apps/{}/channels/{plain}", app.id),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = env
        .send_encrypted(&alice, &device, &encrypted, json!({"user_ids": []}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let message = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = env
        .call(
            "DELETE",
            &format!("/api/messages/{message}"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &alice,
            json!({"channel_id": plain, "content": "hello"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let count = |channel: String| {
        let env = &env;
        async move {
            env.query(
                "SELECT json_build_object('n', count(*))::text FROM app_deliveries
                 WHERE channel_id = $1 AND event_type LIKE 'message.%'",
                &[&channel],
            )
            .await[0]["n"]
                .as_i64()
                .unwrap()
        }
    };
    assert_eq!(count(encrypted).await, 0);
    assert!(count(plain).await > 0);
}

// ---------------------------------------------------------------------------
// DB のトリガー

#[tokio::test]
async fn the_trigger_rejects_plaintext_columns_in_encrypted_channels() {
    let env = env().await;
    let (alice, _) = env.user().await;
    let encrypted = env.channel(&alice, &[], true).await;
    let plain = env.channel(&alice, &[], false).await;
    let ciphertext = r#"{"algorithm":"m.megolm.v1.aes-sha2","ciphertext":"c"}"#;

    // 暗号化チャンネルには暗号文を付け、平文チャンネルには付けない（どちらも content は空）
    let insert = async |channel: &str, extra_col: &str, extra_val: &str| {
        let id = Uuid::new_v4().to_string();
        let sql = format!(
            "INSERT INTO messages (id, user_id, channel_id, content, ciphertext, epoch{extra_col})
             VALUES ($1, $2, $3, '', NULLIF($4, '')::jsonb, CASE WHEN $4 = '' THEN NULL ELSE 0 END{extra_val})"
        );
        let value = if channel == encrypted { ciphertext } else { "" };
        env.exec(&sql, &[&id, &alice, channel, value]).await
    };

    // 平文の列は、暗号化チャンネルでは拒否し、平文チャンネルでは今までどおり入る
    for (col, val) in [
        (", card", ", '{\"title\":\"x\"}'"),
        (", home_post_id", ", gen_random_uuid()::text"),
        (", apub", ", true"),
        (", app_blocks", ", '[]'"),
        (", app_attachments", ", '[]'"),
        (", app_unfurls", ", '{}'"),
    ] {
        assert!(
            insert(&encrypted, col, val).await.is_err(),
            "{col} was accepted in an encrypted channel"
        );
    }
    assert!(
        insert(&plain, ", card", ", '{\"title\":\"x\"}'")
            .await
            .is_ok()
    );

    // metadata は INSERT では zoo.message_options から入る（app_message_options）
    let with_metadata = async |channel: &str| {
        let mut db = env.db.clone();
        let mut tx = db.transaction().await.unwrap();
        toasty::sql::statement(
            "SELECT set_config('zoo.message_options', '{\"metadata\":{\"a\":1}}', true)",
        )
        .exec(&mut tx)
        .await
        .unwrap();
        let value = if channel == encrypted { ciphertext } else { "" };
        let result = toasty::sql::statement(
            "INSERT INTO messages (id, user_id, channel_id, content, ciphertext, epoch)
             VALUES ($1, $2, $3, '', NULLIF($4, '')::jsonb, CASE WHEN $4 = '' THEN NULL ELSE 0 END)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(alice.as_str())
        .bind(channel)
        .bind(value)
        .exec(&mut tx)
        .await;
        if result.is_ok() {
            tx.commit().await.unwrap();
        }
        result
    };
    assert!(with_metadata(&encrypted).await.is_err());
    assert!(with_metadata(&plain).await.is_ok());

    // 本文と暗号文
    let id = Uuid::new_v4().to_string();
    assert!(
        env.exec(
            "INSERT INTO messages (id, user_id, channel_id, content) VALUES ($1, $2, $3, 'plain')",
            &[&id, &alice, &encrypted],
        )
        .await
        .is_err()
    );
    assert!(insert(&encrypted, "", "").await.is_ok());

    // UPDATE でも同じ
    let id = Uuid::new_v4().to_string();
    env.exec(
        "INSERT INTO messages (id, user_id, channel_id, content, ciphertext, epoch) VALUES ($1, $2, $3, '', $4::jsonb, 0)",
        &[&id, &alice, &encrypted, ciphertext],
    )
    .await
    .unwrap();
    for set in [
        "card = '{\"title\":\"x\"}'",
        "apub = true",
        "home_post_id = id",
        "metadata = '{}'",
    ] {
        assert!(
            env.exec(&format!("UPDATE messages SET {set} WHERE id = $1"), &[&id])
                .await
                .is_err(),
            "{set} was accepted in an encrypted channel"
        );
    }
}

#[tokio::test]
async fn clear_history_removes_the_ciphertext() {
    let env = env().await;
    let (alice, _) = env.user().await;
    let encrypted = env.channel(&alice, &[], true).await;
    let device = env.device(&alice).await;
    let (status, body) = env
        .send_encrypted(&alice, &device, &encrypted, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let message = body["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = env
        .call(
            "DELETE",
            &format!("/api/channels/{encrypted}/history"),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = &env
        .query(
            "SELECT json_build_object('deleted', deleted_at IS NOT NULL, 'ciphertext', ciphertext)::text
             FROM messages WHERE id = $1",
            &[&message],
        )
        .await[0];
    assert_eq!(row["deleted"], true, "{row}");
    assert_eq!(row["ciphertext"], Value::Null, "{row}");
}
