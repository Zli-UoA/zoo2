//! Bot を端末として暗号化チャンネルに参加させる（#366）の結合テスト。仕様は docs/e2ee.md の「Bot の端末」。
//!
//! メンバーの明示的な許可（`e2ee_allowed`）、`e2ee:device` の scope、Bot の端末の用意を確かめてから入れること、
//! Bot の端末の API（`/api/apps/v1/e2ee/...`）、暗号文だけのイベント、外したときの epoch と許可の削除を確かめる。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zoo_crypto::{CrossSigningKeys, DeviceAccount};

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

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

struct TestApp {
    id: String,
    installation: String,
    bot: String,
}

/// Bot の端末（端末アカウントとクロス署名鍵）。
struct BotDevice {
    token: String,
    account: DeviceAccount,
}

impl Env {
    async fn call(
        &self,
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
        let body = Body::from(body.to_string());
        let response = self
            .app
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

    async fn query(&self, sql: &str, params: &[&str]) -> Vec<Value> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        let rows = backend::apps::query(&mut tx, sql, params).await.unwrap();
        tx.commit().await.unwrap();
        rows
    }

    async fn exec(&self, sql: &str, params: &[&str]) -> Result<u64, toasty::Error> {
        let mut statement = toasty::sql::statement(sql);
        for p in params {
            statement = statement.bind(*p);
        }
        let mut db = self.db.clone();
        statement.exec(&mut db).await
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/users",
                &id,
                json!({"id": id, "user_name": format!("bot366_{}", Uuid::new_v4().simple()), "display_name": "E2EE"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        id
    }

    async fn channel(&self, creator: &str, members: &[&str]) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, body) = self
            .call(
                "POST",
                "/api/channels",
                creator,
                json!({"id": id, "channel_name": format!("c-{}", Uuid::new_v4().simple()),
                    "is_private": true, "encrypted": true}),
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

    async fn app(&self, owner: &str, scopes: &[&str]) -> TestApp {
        let admin = backend::auth::DEFAULT_DEV_USER_ID;
        self.call("POST", "/api/users", admin, json!({"id": admin, "user_name": format!("admin_{}", Uuid::new_v4().simple()), "display_name": "Admin"}))
            .await;
        let manifest = json!({"scopes": scopes, "events": ["message.created", "message.updated"],
            "transport": "websocket", "redirect_uris": ["https://example.com/callback"]});
        let (status, created) = self
            .call(
                "POST",
                "/api/apps",
                owner,
                json!({"name": "E2EE bot", "description": "E2EE bot", "manifest": manifest}),
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

    /// App のトークンを発行する（`user` が `Some` なら委任トークン）。
    async fn token(&self, app: &TestApp, user: Option<&str>, scopes: &[&str]) -> String {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        let token = backend::apps::mint(&mut tx, &app.installation, user, &json!(scopes))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        token["access_token"].as_str().unwrap().to_string()
    }

    /// Bot の端末を用意する（端末の登録、クロス署名鍵、自己署名鍵による端末への署名）。
    async fn bot_device(&self, app: &TestApp) -> BotDevice {
        let token = self
            .token(
                app,
                None,
                &[
                    "messages:read",
                    "messages:write",
                    "channels:manage",
                    "e2ee:device",
                ],
            )
            .await;
        let mut account = DeviceAccount::new(&app.bot, &Uuid::new_v4().to_string()).unwrap();
        account.generate_one_time_keys(5);
        account.generate_fallback_key();
        let (status, body) = self
            .call(
                "POST",
                "/api/apps/v1/e2ee/devices",
                &token,
                json!({"device_keys": account.device_keys(), "display_name": "Bot",
                    "one_time_keys": account.one_time_keys(), "fallback_keys": account.fallback_keys()}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        account.mark_keys_as_published();
        // Bot は直前の再認証なしでクロス署名鍵を登録できる。
        let keys = CrossSigningKeys::generate(&app.bot).unwrap();
        let (status, body) = self
            .call(
                "POST",
                "/api/apps/v1/e2ee/cross_signing/keys",
                &token,
                keys.upload_json(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let signature = keys
            .self_signing
            .sign_device(&account.device_keys())
            .unwrap();
        let (status, body) = self
            .call(
                "POST",
                "/api/apps/v1/e2ee/cross_signing/signatures",
                &token,
                json!({"signatures": [signature]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        BotDevice { token, account }
    }

    async fn add(
        &self,
        app: &TestApp,
        channel: &str,
        actor: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        self.call(
            "POST",
            &format!("/api/apps/{}/channels/{channel}", app.id),
            actor,
            body,
        )
        .await
    }

    async fn granted(&self, channel: &str, bot: &str) -> bool {
        !self
            .query(
                "SELECT '{}'::text FROM e2ee_bot_grants WHERE channel_id = $1 AND bot_user_id = $2",
                &[channel, bot],
            )
            .await
            .is_empty()
    }
}

fn ciphertext(account: &DeviceAccount) -> Value {
    json!({
        "algorithm": "m.megolm.v1.aes-sha2",
        "sender_key": account.identity_keys().curve25519.to_base64(),
        "device_id": account.device_id(),
        "session_id": "session",
        "ciphertext": "c",
    })
}

const E2EE_SCOPES: &[&str] = &[
    "messages:read",
    "messages:write",
    "channels:manage",
    "e2ee:device",
];

// ---------------------------------------------------------------------------
// 参加の許可

#[tokio::test]
async fn an_encrypted_channel_accepts_only_an_explicitly_allowed_bot_device() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.channel(&alice, &[&bob]).await;

    // e2ee:device を持たない App は、今までどおり入れない。
    let plain = env.app(&alice, &["messages:read", "messages:write"]).await;
    let (status, body) = env
        .add(&plain, &channel, &alice, json!({"e2ee_allowed": true}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(code(&body), "E2EE_APPS_FORBIDDEN");

    let app = env.app(&alice, E2EE_SCOPES).await;
    // 明示的な許可がなければ入れない（本文なしの今までの要求も）。
    for request in [Value::Null, json!({}), json!({"e2ee_allowed": false})] {
        let (status, body) = env.add(&app, &channel, &alice, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(code(&body), "E2EE_BOT_CONSENT_REQUIRED");
    }
    // Bot の端末がまだなければ入れない。
    let (status, body) = env
        .add(&app, &channel, &alice, json!({"e2ee_allowed": true}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(code(&body), "E2EE_BOT_NOT_READY");
    assert!(!env.granted(&channel, &app.bot).await);

    env.bot_device(&app).await;
    // メンバーでない人（作成者でない、管理者でないメンバーも、非公開チャンネルの規則どおり）は入れられない。
    let outsider = env.user().await;
    let (status, body) = env
        .add(&app, &channel, &outsider, json!({"e2ee_allowed": true}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let before = env.epoch(&channel).await;
    let (status, body) = env
        .add(&app, &channel, &alice, json!({"e2ee_allowed": true}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(env.granted(&channel, &app.bot).await);
    assert_eq!(
        env.epoch(&channel).await,
        before + 1,
        "the bot's join bumps the epoch"
    );
    // もう一度入れても epoch は増えない。
    let (status, body) = env
        .add(&app, &channel, &alice, json!({"e2ee_allowed": true}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(env.epoch(&channel).await, before + 1);

    // 外すと許可も消え、epoch が増える。
    let (status, body) = env
        .call(
            "DELETE",
            &format!("/api/apps/{}/channels/{channel}", app.id),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!env.granted(&channel, &app.bot).await);
    assert_eq!(env.epoch(&channel).await, before + 2);

    // 許可のない Bot は、API を通さずに channel_user に入れても DB のトリガーで拒否する。
    let rejected = env
        .exec(
            "INSERT INTO channel_user(channel_id,user_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP)",
            &[&channel, &app.bot],
        )
        .await;
    assert!(rejected.is_err(), "a bot without a grant cannot join");
}

#[tokio::test]
async fn a_bot_leaving_an_encrypted_channel_loses_its_grant() {
    let env = env().await;
    let alice = env.user().await;
    let channel = env.channel(&alice, &[]).await;
    let app = env.app(&alice, E2EE_SCOPES).await;
    let device = env.bot_device(&app).await;
    let (status, body) = env
        .add(&app, &channel, &alice, json!({"e2ee_allowed": true}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let before = env.epoch(&channel).await;
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/apps/v1/channels/{channel}/leave"),
            &device.token,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!env.granted(&channel, &app.bot).await);
    assert_eq!(env.epoch(&channel).await, before + 1);
}

// ---------------------------------------------------------------------------
// Bot の端末の API

#[tokio::test]
async fn bot_device_routes_need_the_bots_own_token_with_the_scope() {
    let env = env().await;
    let alice = env.user().await;
    let app = env.app(&alice, E2EE_SCOPES).await;
    let account = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    // 委任トークンはユーザーとして動くので、ユーザーの端末を作らせない。
    let delegated = env.token(&app, Some(&alice), E2EE_SCOPES).await;
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/e2ee/devices",
            &delegated,
            json!({"device_keys": account.device_keys(), "display_name": "x"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        env.query(
            "SELECT '{}'::text FROM e2ee_devices WHERE user_id = $1",
            &[&alice]
        )
        .await
        .is_empty()
    );
    // e2ee:device のないトークン。
    let narrow = env.token(&app, None, &["messages:read"]).await;
    let (status, body) = env
        .call("GET", "/api/apps/v1/e2ee/devices", &narrow, Value::Null)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let device = env.bot_device(&app).await;
    let (status, body) = env
        .call(
            "GET",
            "/api/apps/v1/e2ee/devices",
            &device.token,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body[0]["device_id"], device.account.device_id());

    // 他のユーザーのマスター鍵への署名と、ピンの置き換えは Bot には出せない。
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/e2ee/cross_signing/signatures",
            &device.token,
            json!({"signatures": [{"target_user_id": alice, "target_key_id": "x",
                "signer_key_id": "ed25519:x", "signature": "x"}]}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/e2ee/tofu_pins",
            &device.token,
            json!({"pins": [], "replace": true}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn an_allowed_bot_exchanges_keys_and_ciphertext_but_no_plaintext() {
    let env = env().await;
    let alice = env.user().await;
    let channel = env.channel(&alice, &[]).await;
    let app = env.app(&alice, E2EE_SCOPES).await;
    let bot = env.bot_device(&app).await;

    // 許可の前は、配布先のスナップショットも、暗号文の送信もできない。
    let recipients = format!("/api/apps/v1/e2ee/channels/{channel}/recipients");
    let (status, _) = env.call("GET", &recipients, &bot.token, Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = env
        .add(&app, &channel, &alice, json!({"e2ee_allowed": true}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 人間の端末を用意する。
    let human = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    let (status, body) = env
        .call(
            "POST",
            "/api/e2ee/devices",
            &alice,
            json!({"device_keys": human.device_keys(), "display_name": "Alice"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // スナップショットに、人間と Bot の端末とクロス署名鍵が入る。
    let (status, page) = env.call("GET", &recipients, &bot.token, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page["membership_epoch"].as_i64(),
        Some(env.epoch(&channel).await)
    );
    assert!(page["device_keys"][&alice][human.device_id()].is_object());
    assert!(page["device_keys"][&app.bot][bot.account.device_id()].is_object());
    assert!(page["master_keys"][&app.bot].is_object());
    // 人間から見ても、Bot の端末とクロス署名鍵が見える（keys/query の範囲に入る）。
    let (status, query) = env
        .call(
            "POST",
            "/api/e2ee/keys/query",
            &alice,
            json!({"user_ids": [app.bot]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{query}");
    assert!(query["device_keys"][&app.bot][bot.account.device_id()].is_object());
    assert!(query["self_signing_keys"][&app.bot].is_object());

    // 人間の端末から Bot の端末への to-device を、Bot は GET で受け取り、ack で消す。
    let content =
        json!({"algorithm": "m.olm.v1.curve25519-aes-sha2", "sender_key": "k", "ciphertext": {}});
    let (status, body) = env
        .call(
            "POST",
            "/api/e2ee/to_device",
            &alice,
            json!({"device_id": human.device_id(), "txn_id": Uuid::new_v4().to_string(),
            "type": "m.room.encrypted",
            "messages": Value::Object(
                [(app.bot.clone(), Value::Object(
                    [(bot.account.device_id().to_owned(), content.clone())].into_iter().collect(),
                ))]
                .into_iter()
                .collect(),
            )}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let inbox = format!(
        "/api/apps/v1/e2ee/to_device?device_id={}",
        bot.account.device_id()
    );
    let (status, body) = env.call("GET", &inbox, &bot.token, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1, "{body}");
    assert_eq!(messages[0]["sender_user_id"], alice);
    assert_eq!(messages[0]["sender_device_id"], human.device_id());
    assert_eq!(messages[0]["content"], content);
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/e2ee/to_device/ack",
            &bot.token,
            json!({"device_id": bot.account.device_id(), "ids": [messages[0]["id"]]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = env.call("GET", &inbox, &bot.token, Value::Null).await;
    assert_eq!(body["messages"], json!([]));
    // 他の端末の受信箱は読めない。
    let other = format!(
        "/api/apps/v1/e2ee/to_device?device_id={}",
        human.device_id()
    );
    let (status, _) = env.call("GET", &other, &bot.token, Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 人間のメッセージは、暗号文と照合の列だけのイベントとして Bot に届く。
    let epoch = env.epoch(&channel).await;
    let message = Uuid::new_v4().to_string();
    let (status, body) = env
        .call(
            "POST",
            "/api/messages",
            &alice,
            json!({"id": message, "channel_id": channel, "content": "", "epoch": epoch,
                "revision": 0, "ciphertext": ciphertext(&human)}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let events = env
        .query(
            "SELECT (payload::jsonb)::text FROM app_deliveries
             WHERE installation_id = $1 AND event_type = 'message.created' AND payload::jsonb->'data'->>'id' = $2",
            &[&app.installation, &message],
        )
        .await;
    assert_eq!(events.len(), 1, "{events:?}");
    let data = &events[0]["data"];
    assert_eq!(data["encrypted"], true);
    assert_eq!(data["ciphertext"], ciphertext(&human));
    assert_eq!(data["epoch"].as_i64(), Some(epoch));
    assert_eq!(data["user_id"], alice);
    assert!(data.get("content").is_none(), "{data}");
    // 本文の解析によるイベント（app.mentioned、link.shared）は作らない。
    assert!(
        env.query(
            "SELECT '{}'::text FROM app_deliveries WHERE installation_id = $1 AND event_type IN ('app.mentioned', 'link.shared') AND channel_id = $2",
            &[&app.installation, &channel],
        )
        .await
        .is_empty()
    );

    // Bot は暗号文で送れる。平文は今までどおり拒否する。
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/messages",
            &bot.token,
            json!({"id": Uuid::new_v4().to_string(), "channel_id": channel, "content": "",
                "epoch": epoch, "revision": 0, "ciphertext": ciphertext(&bot.account)}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reply = body["data"]["id"].as_str().unwrap().to_string();
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/messages",
            &bot.token,
            json!({"channel_id": channel, "content": "plaintext"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), "E2EE_PLAINTEXT_FORBIDDEN");
    // 別の端末の識別鍵を名乗る暗号文は拒否する（送信端末の照合）。
    let (status, _) = env
        .call(
            "POST",
            "/api/apps/v1/messages",
            &bot.token,
            json!({"id": Uuid::new_v4().to_string(), "channel_id": channel, "content": "",
                "epoch": epoch, "revision": 0, "ciphertext": ciphertext(&human)}),
        )
        .await;
    assert_ne!(status, StatusCode::OK);
    // 平文を持ち込む経路（blocks）は、許可のある Bot にも開かない。
    let (status, body) = env
        .call(
            "POST",
            &format!("/api/apps/v1/messages/{reply}/blocks"),
            &bot.token,
            json!([]),
        )
        .await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "{status} {body}"
    );

    // 委任トークンは、許可のある Bot の App でも暗号化チャンネルを読み書きできない。
    let delegated = env.token(&app, Some(&alice), E2EE_SCOPES).await;
    let (status, body) = env
        .call(
            "POST",
            "/api/apps/v1/messages",
            &delegated,
            json!({"id": Uuid::new_v4().to_string(), "channel_id": channel, "content": "",
                "epoch": epoch, "revision": 0, "ciphertext": ciphertext(&human)}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(code(&body), "E2EE_APPS_FORBIDDEN");

    // 外したあとは、スナップショットも送信も 404。
    let (status, body) = env
        .call(
            "DELETE",
            &format!("/api/apps/{}/channels/{channel}", app.id),
            &alice,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = env.call("GET", &recipients, &bot.token, Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
