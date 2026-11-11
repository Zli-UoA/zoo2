//! E2EE の to-device メッセージ（#355）の結合テスト。仕様は docs/e2ee.md の「to-device メッセージ」。
//!
//! 送信の検証は端末鍵の署名を見ないので、端末の行は SQL で直接作る（登録の回数制限と台数の上限を避けるため）。

use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Map, Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Env {
    app: Router,
    db: toasty::Db,
}

async fn env() -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
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

fn olm_content() -> Value {
    json!({
        "algorithm": "m.olm.v1.curve25519-aes-sha2",
        "sender_key": "sender",
        "ciphertext": {"recipient": {"type": 0, "body": "AAAA"}},
    })
}

fn messages_for<U: AsRef<str>, D: AsRef<str>>(targets: &[(U, D)]) -> Value {
    let mut map = Map::new();
    for (user, device) in targets {
        map.entry(user.as_ref().to_owned())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .insert(device.as_ref().to_owned(), olm_content());
    }
    Value::Object(map)
}

async fn send<U: AsRef<str>, D: AsRef<str>>(
    app: &Router,
    sender: &str,
    sender_device: &str,
    txn_id: &str,
    targets: &[(U, D)],
) -> (StatusCode, Value) {
    request(
        app,
        "POST",
        "/api/e2ee/to_device",
        sender,
        json!({
            "device_id": sender_device,
            "txn_id": txn_id,
            "type": "m.room.encrypted",
            "messages": messages_for(targets),
        }),
    )
    .await
}

fn txn() -> String {
    Uuid::new_v4().to_string()
}

impl Env {
    async fn exec(&self, sql: &str, params: &[&str]) {
        let mut statement = toasty::sql::statement(sql);
        for p in params {
            statement = statement.bind(*p);
        }
        let mut db = self.db.clone();
        statement.exec(&mut db).await.unwrap();
    }

    async fn query(&self, sql: &str, params: &[&str]) -> Vec<Value> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        let rows = backend::apps::query(&mut tx, sql, params).await.unwrap();
        tx.commit().await.unwrap();
        rows
    }

    async fn count(&self, sql: &str, params: &[&str]) -> i64 {
        self.query(&format!("SELECT count(*)::text FROM ({sql}) q"), params)
            .await[0]
            .as_i64()
            .unwrap()
    }

    async fn queued(&self, user: &str, device: &str) -> i64 {
        self.count(
            "SELECT 1 FROM e2ee_to_device_messages WHERE recipient_user_id = $1 AND recipient_device_id = $2",
            &[user, device],
        )
        .await
    }

    async fn message_ids(&self, user: &str, device: &str) -> Vec<String> {
        self.query(
            "SELECT to_json(id::text)::text FROM e2ee_to_device_messages
             WHERE recipient_user_id = $1 AND recipient_device_id = $2 ORDER BY id",
            &[user, device],
        )
        .await
        .into_iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, _) = request(
            &self.app,
            "POST",
            "/api/users",
            &id,
            json!({"id": id, "user_name": format!("e2ee_{}", Uuid::new_v4().simple()), "display_name": "E2EE"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        id
    }

    /// 端末の行を直接作る（鍵は使わないので、ダミーの一意な値を入れる）。
    async fn devices(&self, user: &str, n: usize) -> Vec<String> {
        self.query(
            "WITH i AS (
               INSERT INTO e2ee_devices (user_id, device_id, curve25519_key, ed25519_key, device_keys, last_seen_at)
               SELECT $1, gen_random_uuid()::text, 'c' || gen_random_uuid(), 'e' || gen_random_uuid(), '{}', now()
               FROM generate_series(1, $2::integer)
               RETURNING device_id)
             SELECT to_json(device_id)::text FROM i",
            &[user, &n.to_string()],
        )
        .await
        .into_iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
    }

    async fn device(&self, user: &str) -> String {
        self.devices(user, 1).await.remove(0)
    }

    /// 暗号化チャンネルを作る（作成時の指定は #357 で足すので、行を直接作る）。
    async fn encrypted_channel(&self, members: &[&str]) -> String {
        let id = Uuid::new_v4().to_string();
        self.exec(
            "INSERT INTO channels (id, channel_name, created_user_id, is_private, encrypted)
             VALUES ($1, $2, $3, true, true)",
            &[
                &id,
                &format!("e2ee-{}", Uuid::new_v4().simple()),
                members[0],
            ],
        )
        .await;
        for member in members {
            self.exec(
                "INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, now())",
                &[member, &id],
            )
            .await;
        }
        id
    }

    /// 宛先の端末のキューに `n` 件を直接入れる。
    async fn fill(&self, user: &str, device: &str, sender: &str, sender_device: &str, n: usize) {
        self.exec(
            "INSERT INTO e2ee_to_device_messages
               (recipient_user_id, recipient_device_id, sender_user_id, sender_device_id, txn_id, type, content)
             SELECT $1, $2, $3, $4, gen_random_uuid()::text, 'm.room.encrypted', '{}'
             FROM generate_series(1, $5::integer)",
            &[user, device, sender, sender_device, &n.to_string()],
        )
        .await;
    }

    async fn ack(&self, user: &str, device: &str, ids: &[String]) -> (StatusCode, Value) {
        request(
            &self.app,
            "POST",
            "/api/e2ee/to_device/ack",
            user,
            json!({"device_id": device, "ids": ids}),
        )
        .await
    }

    /// to-device の Shape の初回の応答（Electric を通す）。
    async fn shape(&self, actor: &str, query: &str) -> (StatusCode, Vec<Value>) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/shapes/e2ee_to_device?offset=-1&{query}"))
                    .header("X-Dev-User", actor)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        if status == StatusCode::OK {
            assert_eq!(response.headers()["cache-control"], "private, no-cache");
        }
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap_or(Value::Null);
        let rows = body
            .as_array()
            .map(|messages| {
                messages
                    .iter()
                    .filter_map(|m| m.get("value").cloned())
                    .collect()
            })
            .unwrap_or_default();
        (status, rows)
    }
}

/// 他人の端末宛てのメッセージは、Shape でも ack でも読めない、消せない。Shape のクライアント指定の where は無視される。
/// 他のユーザー宛ての ID の ack は黙って無視され、再送も 200 になる。
#[tokio::test]
async fn others_cannot_read_or_ack_messages_for_a_device() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let alice_device = env.device(&alice).await;
    let bob_device = env.device(&bob).await;
    let carol_device = env.device(&carol).await;
    env.encrypted_channel(&[&alice, &bob, &carol]).await;

    let (status, body) = send(
        &env.app,
        &bob,
        &bob_device,
        &txn(),
        &[(&alice, &alice_device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["undelivered"], json!([]));
    assert!(body["txid"].is_string());
    let (status, _) = send(
        &env.app,
        &alice,
        &alice_device,
        &txn(),
        &[(&carol, &carol_device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let alice_ids = env.message_ids(&alice, &alice_device).await;
    assert_eq!(alice_ids.len(), 1);

    // 他人の端末の Shape は開けない（存在を推測されないよう 404）。
    let (status, _) = env
        .shape(&carol, &format!("device_id={alice_device}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env.shape(&alice, "device_id=").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 自分の端末の Shape は自分の端末宛ての行だけで、クライアントの where と table は使われない。
    let (status, rows) = env
        .shape(
            &alice,
            &format!("device_id={alice_device}&where=true&table=e2ee_devices"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["recipient_device_id"], alice_device.as_str());
    assert_eq!(rows[0]["sender_user_id"], bob.as_str());
    assert_eq!(rows[0]["content"], olm_content());

    // carol は自分の端末を名乗って、alice 宛ての ID を ack しても消せない。再送も 200。
    for _ in 0..2 {
        let (status, body) = env.ack(&carol, &carol_device, &alice_ids).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["deleted"], 0);
        assert!(body["txid"].is_null());
    }
    // 他人の端末の ID を名乗る ack は 404。
    let (status, _) = env.ack(&carol, &alice_device, &alice_ids).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(env.queued(&alice, &alice_device).await, 1);

    // 本人の ack は消える。すでに消えた ID の再送も 200。
    let (status, body) = env.ack(&alice, &alice_device, &alice_ids).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["deleted"], 1);
    assert!(body["txid"].is_string());
    let (status, body) = env.ack(&alice, &alice_device, &alice_ids).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["deleted"], 0);
    assert_eq!(env.queued(&alice, &alice_device).await, 0);
    assert_eq!(env.queued(&carol, &carol_device).await, 1);

    // ack の件数の上限。
    let ids: Vec<String> = (0..1001).map(|i| i.to_string()).collect();
    let (status, body) = env.ack(&alice, &alice_device, &ids).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "E2EE_LIMIT_EXCEEDED");
    assert_eq!(body["limit"], "to_device_ack");
}

/// 範囲外のユーザー、存在しない端末、削除済みの端末への宛先は、知らせずに捨てる。
/// 送信端末は呼び出したユーザーの削除されていない端末でなければならない。
#[tokio::test]
async fn out_of_scope_and_missing_recipients_are_dropped_silently() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let stranger = env.user().await;
    let alice_device = env.device(&alice).await;
    let alice_other = env.device(&alice).await;
    let bob_devices = env.devices(&bob, 2).await;
    let stranger_device = env.device(&stranger).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    // 平文のチャンネルや削除済みの暗号化チャンネルでの共有は数えない。
    let deleted = env.encrypted_channel(&[&alice, &stranger]).await;
    env.exec(
        "UPDATE channels SET deleted_at = now() WHERE id = $1",
        &[&deleted],
    )
    .await;
    let (status, _) = request(
        &env.app,
        "DELETE",
        &format!("/api/e2ee/devices/{}", bob_devices[1]),
        &bob,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let missing = Uuid::new_v4().to_string();
    let (status, body) = send(
        &env.app,
        &alice,
        &alice_device,
        &txn(),
        &[
            (&alice, &alice_other),
            (&bob, &bob_devices[0]),
            (&bob, &bob_devices[1]),
            (&bob, &missing),
            (&stranger, &stranger_device),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["undelivered"], json!([]));
    assert_eq!(env.queued(&alice, &alice_other).await, 1);
    assert_eq!(env.queued(&bob, &bob_devices[0]).await, 1);
    assert_eq!(env.queued(&bob, &bob_devices[1]).await, 0);
    assert_eq!(env.queued(&stranger, &stranger_device).await, 0);

    // 範囲外の宛先だけなら、Shape に流れる変更がないので txid は null。
    let (status, body) = send(
        &env.app,
        &alice,
        &alice_device,
        &txn(),
        &[(&stranger, &stranger_device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["txid"].is_null());

    // 送信端末が他人の端末か削除済みなら 404。
    let (status, _) = send(
        &env.app,
        &alice,
        &bob_devices[0],
        &txn(),
        &[(&bob, &bob_devices[0])],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &env.app,
        &bob,
        &bob_devices[1],
        &txn(),
        &[(&alice, &alice_device)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 入力の検証。
    let (status, _) = request(
        &env.app,
        "POST",
        "/api/e2ee/to_device",
        &alice,
        json!({"device_id": alice_device, "txn_id": txn(), "type": "zoo.room_key", "messages": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = request(
        &env.app,
        "POST",
        "/api/e2ee/to_device",
        &alice,
        json!({"device_id": alice_device, "txn_id": "TXN", "type": "m.room.encrypted", "messages": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let big = "x".repeat(64 * 1024);
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/to_device",
        &alice,
        json!({"device_id": alice_device, "txn_id": txn(), "type": "m.room.encrypted",
               "messages": {bob.clone(): {bob_devices[0].clone(): {"body": big}}}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["limit"], "to_device_content");
}

/// 端末の削除と無効化で、その端末宛ての未受信のメッセージが消える。
#[tokio::test]
async fn deleting_or_deactivating_removes_queued_messages() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let alice_device = env.device(&alice).await;
    let bob_devices = env.devices(&bob, 2).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    let targets: Vec<(&str, &str)> = bob_devices
        .iter()
        .map(|d| (bob.as_str(), d.as_str()))
        .collect();
    let (status, _) = send(&env.app, &alice, &alice_device, &txn(), &targets).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.queued(&bob, &bob_devices[0]).await, 1);

    let (status, _) = request(
        &env.app,
        "DELETE",
        &format!("/api/e2ee/devices/{}", bob_devices[0]),
        &bob,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.queued(&bob, &bob_devices[0]).await, 0);
    assert_eq!(env.queued(&bob, &bob_devices[1]).await, 1);

    env.exec(
        "UPDATE users SET guest_kind = 'deactivated' WHERE id = $1",
        &[&bob],
    )
    .await;
    assert_eq!(env.queued(&bob, &bob_devices[1]).await, 0);
}

/// 自分宛ての to-device の送信と、同じユーザーの無効化を同時に走らせても、行き詰まらない。
#[tokio::test]
async fn self_send_and_deactivation_do_not_deadlock() {
    let env = env().await;
    for _ in 0..5 {
        let alice = env.user().await;
        let devices = env.devices(&alice, 4).await;
        let send = {
            let (app, alice, devices) = (env.app.clone(), alice.clone(), devices.clone());
            tokio::spawn(async move {
                let targets: Vec<(&str, &str)> = devices
                    .iter()
                    .map(|d| (alice.as_str(), d.as_str()))
                    .collect();
                send(&app, &alice, &devices[0], &txn(), &targets).await.0
            })
        };
        let deactivate = {
            let db = env.db.clone();
            let alice = alice.clone();
            tokio::spawn(async move {
                let mut db = db;
                toasty::sql::statement("UPDATE users SET guest_kind = 'deactivated' WHERE id = $1")
                    .bind(alice.as_str())
                    .exec(&mut db)
                    .await
            })
        };
        let send = tokio::time::timeout(Duration::from_secs(10), send)
            .await
            .unwrap()
            .unwrap();
        // 無効化が先にコミットされたら、送信は AuthUser か FOR SHARE で 403 になる。
        assert!(
            send == StatusCode::OK || send == StatusCode::FORBIDDEN,
            "{send}"
        );
        tokio::time::timeout(Duration::from_secs(10), deactivate)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        // どちらが先でも、無効化のあとに未受信のメッセージは残らない。
        assert_eq!(
            env.count(
                "SELECT 1 FROM e2ee_to_device_messages WHERE recipient_user_id = $1",
                &[&alice]
            )
            .await,
            0
        );
    }
}

/// 上限に達した範囲内の宛先は `undelivered` に入り、範囲外の宛先は入らない。
/// `undelivered` を同じ `txn_id` で送り直すと、入れられなかった宛先に入る。
#[tokio::test]
async fn full_queues_are_reported_and_can_be_retried_with_the_same_txn() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let stranger = env.user().await;
    let alice_device = env.device(&alice).await;
    let bob_devices = env.devices(&bob, 2).await;
    let stranger_device = env.device(&stranger).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    env.fill(&bob, &bob_devices[0], &alice, &alice_device, 1000)
        .await;
    env.fill(
        &stranger,
        &stranger_device,
        &stranger,
        &stranger_device,
        1000,
    )
    .await;

    let txn_id = txn();
    let targets = [
        (bob.as_str(), bob_devices[0].as_str()),
        (bob.as_str(), bob_devices[1].as_str()),
        (stranger.as_str(), stranger_device.as_str()),
    ];
    let (status, body) = send(&env.app, &alice, &alice_device, &txn_id, &targets).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["undelivered"],
        json!([{"user_id": bob, "device_id": bob_devices[0]}])
    );
    assert_eq!(env.queued(&bob, &bob_devices[0]).await, 1000);
    assert_eq!(env.queued(&bob, &bob_devices[1]).await, 1);
    assert_eq!(env.queued(&stranger, &stranger_device).await, 1000);
    // 送信の記録は、キューに入れた宛先にだけ書く。
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_to_device_txns WHERE txn_id = $1",
            &[&txn_id]
        )
        .await,
        1
    );

    // 同じ要求の再送は、その時点で入れられない宛先を改めて返す。入った宛先は増えない。
    let (status, body) = send(&env.app, &alice, &alice_device, &txn_id, &targets).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["undelivered"].as_array().unwrap().len(), 1);
    assert_eq!(env.queued(&bob, &bob_devices[1]).await, 1);

    // 受信側が 1 件 ack したあとで、undelivered を同じ txn_id で送り直すと入る。
    let first = env.message_ids(&bob, &bob_devices[0]).await.remove(0);
    let (status, _) = env.ack(&bob, &bob_devices[0], &[first]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        &env.app,
        &alice,
        &alice_device,
        &txn_id,
        &[(&bob, &bob_devices[0])],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["undelivered"], json!([]));
    assert_eq!(env.queued(&bob, &bob_devices[0]).await, 1000);
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_to_device_messages WHERE txn_id = $1",
            &[&txn_id]
        )
        .await,
        2
    );
    // 上限に達していても、すでに入っている宛先への再送は undelivered にしない。
    let (status, body) = send(
        &env.app,
        &alice,
        &alice_device,
        &txn_id,
        &[(&bob, &bob_devices[0])],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["undelivered"], json!([]));
}

/// 101 端末への送信は 400 になり、100 端末ずつ分けた送信は届く。
#[tokio::test]
async fn sends_are_limited_to_one_hundred_devices() {
    let env = env().await;
    let alice = env.user().await;
    let alice_device = env.device(&alice).await;
    let mut members = vec![alice.clone()];
    let mut targets: Vec<(String, String)> = Vec::new();
    for _ in 0..6 {
        let user = env.user().await;
        for device in env.devices(&user, 17).await {
            targets.push((user.clone(), device));
        }
        members.push(user);
    }
    let member_refs: Vec<&str> = members.iter().map(String::as_str).collect();
    env.encrypted_channel(&member_refs).await;
    targets.truncate(101);
    let refs: Vec<(&str, &str)> = targets
        .iter()
        .map(|(u, d)| (u.as_str(), d.as_str()))
        .collect();

    let (status, body) = send(&env.app, &alice, &alice_device, &txn(), &refs).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "E2EE_LIMIT_EXCEEDED");
    assert_eq!(body["limit"], "to_device_recipients");
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_to_device_messages WHERE sender_user_id = $1",
            &[&alice]
        )
        .await,
        0
    );

    for chunk in refs.chunks(100) {
        let (status, body) = send(&env.app, &alice, &alice_device, &txn(), chunk).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["undelivered"], json!([]));
    }
    for (user, device) in &refs {
        assert_eq!(env.queued(user, device).await, 1);
    }
}

/// ack のあとに同じ `txn_id` で再送しても、新しい行は入らず成功する。
#[tokio::test]
async fn resending_after_ack_is_idempotent() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let alice_device = env.device(&alice).await;
    let bob_device = env.device(&bob).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    let txn_id = txn();
    for _ in 0..2 {
        let (status, _) = send(
            &env.app,
            &alice,
            &alice_device,
            &txn_id,
            &[(&bob, &bob_device)],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let ids = env.message_ids(&bob, &bob_device).await;
    assert_eq!(ids.len(), 1);
    let (status, _) = env.ack(&bob, &bob_device, &ids).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        &env.app,
        &alice,
        &alice_device,
        &txn_id,
        &[(&bob, &bob_device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["undelivered"], json!([]));
    assert!(body["txid"].is_null());
    assert_eq!(env.queued(&bob, &bob_device).await, 0);

    // 同じ txn_id でも、別の送信端末からなら別の送信として入る。
    let alice_other = env.device(&alice).await;
    let (status, _) = send(
        &env.app,
        &alice,
        &alice_other,
        &txn_id,
        &[(&bob, &bob_device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.queued(&bob, &bob_device).await, 1);
}

/// 同じ端末宛ての並行した送信でも、1000 件を超えない。
#[tokio::test]
async fn concurrent_sends_never_exceed_the_queue_limit() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let alice_devices = env.devices(&alice, 4).await;
    let bob_device = env.device(&bob).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    env.fill(&bob, &bob_device, &alice, &alice_devices[0], 990)
        .await;

    let mut tasks = Vec::new();
    for i in 0..20 {
        let (app, alice, sender, bob, bob_device) = (
            env.app.clone(),
            alice.clone(),
            alice_devices[i % alice_devices.len()].clone(),
            bob.clone(),
            bob_device.clone(),
        );
        tasks.push(tokio::spawn(async move {
            send(&app, &alice, &sender, &txn(), &[(&bob, &bob_device)]).await
        }));
    }
    let mut undelivered = 0;
    for task in tasks {
        let (status, body) = tokio::time::timeout(Duration::from_secs(20), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        undelivered += body["data"]["undelivered"].as_array().unwrap().len();
    }
    assert_eq!(env.queued(&bob, &bob_device).await, 1000);
    assert_eq!(undelivered, 10);
}

/// 30 日より前の送信の記録は回収され、回収後の再送は新しい送信として入る。
#[tokio::test]
async fn old_txn_records_are_swept() {
    let env = env().await;
    let alice = env.user().await;
    let alice_devices = env.devices(&alice, 2).await;
    let txn_id = txn();
    let (status, _) = send(
        &env.app,
        &alice,
        &alice_devices[0],
        &txn_id,
        &[(&alice, &alice_devices[1])],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    env.exec(
        "UPDATE e2ee_to_device_txns SET created_at = now() - interval '31 days' WHERE txn_id = $1",
        &[&txn_id],
    )
    .await;
    backend::routes::e2ee::to_device::sweep_txns(&env.db)
        .await
        .unwrap();
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_to_device_txns WHERE txn_id = $1",
            &[&txn_id]
        )
        .await,
        0
    );
    // メッセージの行は時間では回収しない。
    assert_eq!(env.queued(&alice, &alice_devices[1]).await, 1);
}

/// 端末の削除と、その端末宛ての送信を同時に走らせても、削除済みの端末宛てのメッセージは残らない。
#[tokio::test]
async fn concurrent_delete_and_send_never_leave_messages_for_deleted_devices() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let alice_device = env.device(&alice).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    for _ in 0..10 {
        let bob_device = env.device(&bob).await;
        let sender = {
            let (app, alice, alice_device, bob, bob_device) = (
                env.app.clone(),
                alice.clone(),
                alice_device.clone(),
                bob.clone(),
                bob_device.clone(),
            );
            tokio::spawn(async move {
                send(&app, &alice, &alice_device, &txn(), &[(&bob, &bob_device)])
                    .await
                    .0
            })
        };
        let delete = {
            let (app, bob, bob_device) = (env.app.clone(), bob.clone(), bob_device.clone());
            tokio::spawn(async move {
                request(
                    &app,
                    "DELETE",
                    &format!("/api/e2ee/devices/{bob_device}"),
                    &bob,
                    json!({}),
                )
                .await
                .0
            })
        };
        let (sent, deleted) = tokio::time::timeout(Duration::from_secs(10), async {
            (sender.await.unwrap(), delete.await.unwrap())
        })
        .await
        .unwrap();
        assert_eq!(sent, StatusCode::OK);
        assert_eq!(deleted, StatusCode::OK);
        assert_eq!(env.queued(&bob, &bob_device).await, 0);
    }
}
