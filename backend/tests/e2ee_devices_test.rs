//! E2EE の端末と鍵の API（#354）の結合テスト。仕様は docs/e2ee.md。
//!
//! to-device の送信との競合（削除・無効化と to-device の送信）は #355（`e2ee_to_device_test.rs`）で、
//! `membership_epoch` の増加は #357 で確かめる。

use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    config::Config, db::init_db, routes::create_router, routes::e2ee::sweep_deleted_devices,
    state::AppState,
};
use serde_json::{Map, Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zoo_crypto::DeviceAccount;

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

    async fn register(&self, user: &str, account: &DeviceAccount) -> (StatusCode, Value) {
        request(
            &self.app,
            "POST",
            "/api/e2ee/devices",
            user,
            register_body(account),
        )
        .await
    }

    async fn device(&self, user: &str, otks: usize) -> DeviceAccount {
        let mut account = DeviceAccount::new(user, &Uuid::new_v4().to_string()).unwrap();
        account.generate_one_time_keys(otks);
        account.generate_fallback_key();
        let (status, body) = self.register(user, &account).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        account.mark_keys_as_published();
        account
    }

    async fn claim(&self, claimer: &str, targets: &[(&str, &str)]) -> Value {
        let mut map = Map::new();
        for (user, device) in targets {
            map.entry(user.to_string())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap()
                .insert(device.to_string(), json!("signed_curve25519"));
        }
        let (status, body) = request(
            &self.app,
            "POST",
            "/api/e2ee/keys/claim",
            claimer,
            json!({"one_time_keys": map}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"]["one_time_keys"].clone()
    }

    async fn query_keys(&self, caller: &str, users: &[&str]) -> Value {
        let (status, body) = request(
            &self.app,
            "POST",
            "/api/e2ee/keys/query",
            caller,
            json!({"user_ids": users}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }
}

fn register_body(account: &DeviceAccount) -> Value {
    json!({
        "device_keys": account.device_keys(),
        "display_name": "Test",
        "one_time_keys": account.one_time_keys(),
        "fallback_keys": account.fallback_keys(),
    })
}

fn claimed_key(result: &Value, user: &str, device: &str) -> Option<(String, Value)> {
    result[user][device]
        .as_object()
        .and_then(|keys| keys.iter().next())
        .map(|(id, key)| (id.clone(), key.clone()))
}

#[tokio::test]
async fn registration_is_idempotent_and_rejects_bad_signatures_and_reuse() {
    let env = env().await;
    let alice = env.user().await;
    let mut account = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    account.generate_one_time_keys(5);
    account.generate_fallback_key();
    let (status, body) = env.register(&alice, &account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["one_time_key_counts"]["signed_curve25519"], 5);
    assert_eq!(
        body["data"]["unused_fallback_key_types"],
        json!(["signed_curve25519"])
    );
    assert!(body["data"]["last_seen_at"].is_string());
    assert!(body["txid"].is_string());

    // 削除されていない同じ内容の再送は 200。鍵も増えない。端末の行は変わらないので txid は null。
    let (status, body) = env.register(&alice, &account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["one_time_key_counts"]["signed_curve25519"], 5);
    assert!(body["txid"].is_null(), "{body}");

    // 同じ device_id で内容が違えば 409。
    let other = DeviceAccount::new(&alice, account.device_id()).unwrap();
    let (status, body) = env.register(&alice, &other).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "E2EE_DEVICE_EXISTS");

    // 署名が不正な端末鍵は 400。
    let fresh = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    let mut forged = fresh.device_keys();
    forged["algorithms"] = json!(["m.megolm.v1.aes-sha2"]);
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/devices",
        &alice,
        json!({"device_keys": forged}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "E2EE_BAD_SIGNATURE");

    // 他のユーザーの user_id の端末鍵は登録できない。
    let bob = env.user().await;
    let (status, _) = env.register(&bob, &fresh).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 別の端末の署名鍵で署名したワンタイム鍵は、アップロードでも登録でも 400。
    let mut signer = DeviceAccount::new(&alice, account.device_id()).unwrap();
    signer.generate_one_time_keys(1);
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": account.device_id(), "one_time_keys": signer.one_time_keys()}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "E2EE_BAD_SIGNATURE");

    // 削除済みの device_id の再登録は、同じ内容でも 409。
    let path = format!("/api/e2ee/devices/{}", account.device_id());
    assert_eq!(
        request(&env.app, "DELETE", &path, &alice, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let (status, body) = env.register(&alice, &account).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "E2EE_DEVICE_EXISTS");
}

#[tokio::test]
async fn upload_enforces_limits_and_conflicts() {
    let env = env().await;
    let alice = env.user().await;
    let mut account = env.device(&alice, 50).await;

    assert!(
        account.one_time_keys().is_empty(),
        "keys were marked as published"
    );
    account.generate_one_time_keys(50);
    let copy = account.one_time_keys();
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": account.device_id(), "one_time_keys": copy}),
    )
    .await;
    // 50 + 50 + フォールバック鍵 1 = 101 個は上限（100 個）を超える。
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "E2EE_LIMIT_EXCEEDED");
    assert_eq!(body["limit"], "one_time_keys");

    let mut fewer = copy.clone();
    let extra: Vec<String> = fewer.keys().skip(49).cloned().collect();
    for k in extra {
        fewer.remove(&k);
    }
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": account.device_id(), "one_time_keys": fewer}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["one_time_key_counts"]["signed_curve25519"], 99);
    assert!(body["txid"].is_null());

    // 署名を書き換えた鍵は 400 E2EE_BAD_SIGNATURE。
    let (name, mut value) = fewer
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap();
    let original = value.clone();
    value["signatures"][&alice][format!("ed25519:{}", account.device_id())] = json!("AAAA");
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": account.device_id(), "one_time_keys": {name.clone(): value}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "E2EE_BAD_SIGNATURE");
    // 同じ key_id で内容が違う（署名の対象外の unsigned だけ違う）鍵は 409 E2EE_KEY_EXISTS。
    let mut different = original.clone();
    different["unsigned"] = json!({"note": "different"});
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": account.device_id(), "one_time_keys": {name.clone(): different}}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "E2EE_KEY_EXISTS");
    // 同じ内容の再送は数えず 200。
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": account.device_id(), "one_time_keys": {name: original}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["one_time_key_counts"]["signed_curve25519"], 99);

    // 鍵の JSON が 8 KiB を超えると 400。
    let fresh = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    let mut big_fresh = fresh.device_keys();
    big_fresh["unsigned"] = json!({"padding": "a".repeat(9000)});
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/devices",
        &alice,
        json!({"device_keys": big_fresh}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["limit"], "key_size");

    // 1 回のアップロードの署名が 1000 件を超えると 400（1 つの鍵は 8 KiB までなので、50 個の鍵に分けて載せる）。
    let mut signer = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    signer.generate_one_time_keys(50);
    let mut keys = signer.one_time_keys();
    for value in keys.values_mut() {
        let mut extra = Map::new();
        for i in 0..21 {
            extra.insert(format!("ed25519:k{i}"), json!("AAAA"));
        }
        value["signatures"][Uuid::new_v4().to_string()] = Value::Object(extra);
    }
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/devices",
        &alice,
        json!({"device_keys": signer.device_keys(), "one_time_keys": keys}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["limit"], "signatures");

    // 自分のものでない、存在しない端末へのアップロードは 404。
    let bob = env.user().await;
    let (status, _) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &bob,
        json!({"device_id": account.device_id(), "one_time_keys": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn device_limit_returns_stale_devices_and_does_not_delete_old_ones() {
    let env = env().await;
    let alice = env.user().await;
    let mut devices = Vec::new();
    for _ in 0..20 {
        devices.push(env.device(&alice, 0).await);
    }
    // 1 台を 31 日使っていないことにする。
    env.exec(
        "UPDATE e2ee_devices SET last_seen_at = now() - interval '31 days' WHERE user_id = $1 AND device_id = $2",
        &[&alice, devices[0].device_id()],
    )
    .await;
    let extra = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    let (status, body) = env.register(&alice, &extra).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "E2EE_LIMIT_EXCEEDED");
    assert_eq!(body["limit"], "devices");
    let stale = body["stale_devices"].as_array().unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0]["device_id"], devices[0].device_id());
    assert!(stale[0]["last_seen_at"].is_string());
    let (_, listed) = request(&env.app, "GET", "/api/e2ee/devices", &alice, Value::Null).await;
    assert_eq!(listed.as_array().unwrap().len(), 20);

    // 利用者が選んだ端末を削除すれば登録できる。
    let path = format!("/api/e2ee/devices/{}", devices[0].device_id());
    assert_eq!(
        request(&env.app, "DELETE", &path, &alice, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(env.register(&alice, &extra).await.0, StatusCode::OK);
}

#[tokio::test]
async fn parallel_registrations_never_exceed_twenty_devices() {
    let env = env().await;
    let alice = env.user().await;
    let mut tasks = Vec::new();
    for _ in 0..25 {
        let app = env.app.clone();
        let alice = alice.clone();
        tasks.push(tokio::spawn(async move {
            let account = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
            request(
                &app,
                "POST",
                "/api/e2ee/devices",
                &alice,
                register_body(&account),
            )
            .await
            .0
        }));
    }
    let mut ok = 0;
    for task in tasks {
        match task.await.unwrap() {
            StatusCode::OK => ok += 1,
            StatusCode::BAD_REQUEST => {}
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(ok, 20);
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_devices WHERE user_id = $1 AND deleted_at IS NULL",
            &[&alice]
        )
        .await,
        20
    );
}

#[tokio::test]
async fn registrations_are_rate_limited_per_hour() {
    let env = env().await;
    let alice = env.user().await;
    for _ in 0..30 {
        let account = env.device(&alice, 0).await;
        let path = format!("/api/e2ee/devices/{}", account.device_id());
        assert_eq!(
            request(&env.app, "DELETE", &path, &alice, Value::Null)
                .await
                .0,
            StatusCode::OK
        );
    }
    let account = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    let response = env
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/e2ee/devices")
                .header("content-type", "application/json")
                .header("X-Dev-User", &alice)
                .body(Body::from(register_body(&account).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_header: u64 = response.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["code"], "E2EE_RATE_LIMITED");
    let retry_after = body["retry_after"].as_u64().unwrap();
    assert!((1..=3600).contains(&retry_after));
    assert_eq!(retry_header, retry_after);
}

#[tokio::test]
async fn delete_is_owner_only_idempotent_and_removes_only_own_self_signatures() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let account = env.device(&alice, 3).await;
    let device_id = account.device_id().to_owned();
    let path = format!("/api/e2ee/devices/{device_id}");

    // 他人の端末は削除できない（404）。
    assert_eq!(
        request(&env.app, "DELETE", &path, &bob, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&env.app, "PATCH", &path, &bob, json!({"display_name": "x"}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, renamed) = request(
        &env.app,
        "PATCH",
        &path,
        &alice,
        json!({"display_name": "Laptop"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["data"]["display_name"], "Laptop");

    // Bob が同じ device_id の端末を持ち、両方に自己署名がある。
    env.exec(
        "INSERT INTO e2ee_devices (user_id, device_id, curve25519_key, ed25519_key, device_keys, last_seen_at)
         VALUES ($1, $2, $3, $4, '{}'::jsonb, now())",
        &[&bob, &device_id, &format!("c-{}", Uuid::new_v4()), &format!("e-{}", Uuid::new_v4())],
    )
    .await;
    for (user, key) in [(&alice, "ed25519:alice-ssk"), (&bob, "ed25519:bob-ssk")] {
        env.exec(
            "INSERT INTO e2ee_key_signatures (signer_user_id, signer_key_id, target_user_id, target_key_id, signature)
             VALUES ($1, $2, $1, $3, 'sig')",
            &[user, key, &device_id],
        )
        .await;
    }
    // Alice が Bob のマスター鍵に付けた署名（自己署名ではない）は消さない。
    env.exec(
        "INSERT INTO e2ee_key_signatures (signer_user_id, signer_key_id, target_user_id, target_key_id, signature)
         VALUES ($1, 'ed25519:alice-usk', $2, $3, 'sig')",
        &[&alice, &bob, &device_id],
    )
    .await;

    let (status, body) = request(&env.app, "DELETE", &path, &alice, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["txid"].is_string());
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_one_time_keys WHERE user_id = $1",
            &[&alice]
        )
        .await
            + env
                .count(
                    "SELECT 1 FROM e2ee_fallback_keys WHERE user_id = $1",
                    &[&alice]
                )
                .await,
        0
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_signatures WHERE signer_user_id = $1 AND target_user_id = $1",
            &[&alice]
        )
        .await,
        0
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_signatures WHERE signer_user_id = $1",
            &[&bob]
        )
        .await,
        1
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_signatures WHERE signer_user_id = $1 AND target_user_id = $2",
            &[&alice, &bob]
        )
        .await,
        1
    );

    // 削除済みの端末への再試行は 200（txid なし）。一覧からは消える。
    let (status, body) = request(&env.app, "DELETE", &path, &alice, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["txid"].is_null());
    let (_, listed) = request(&env.app, "GET", "/api/e2ee/devices", &alice, Value::Null).await;
    assert_eq!(listed, json!([]));
    assert_eq!(
        request(
            &env.app,
            "PATCH",
            &path,
            &alice,
            json!({"display_name": "x"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // 削除のあとのアップロードは 404。クライアントが端末の削除を見分けられるよう、コードを付ける
    // （コードのない 404 では、クライアントは暗号ストアを作り直さない）。
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": device_id, "one_time_keys": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "E2EE_DEVICE_NOT_FOUND");
    // ack も同じ。
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/to_device/ack",
        &alice,
        json!({"device_id": device_id, "ids": []}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "E2EE_DEVICE_NOT_FOUND");
}

#[tokio::test]
async fn sweep_moves_old_deleted_devices_to_tombstones() {
    let env = env().await;
    let alice = env.user().await;
    let account = env.device(&alice, 1).await;
    let path = format!("/api/e2ee/devices/{}", account.device_id());
    assert_eq!(
        request(&env.app, "DELETE", &path, &alice, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    env.exec(
        "UPDATE e2ee_devices SET deleted_at = now() - interval '31 days' WHERE user_id = $1",
        &[&alice],
    )
    .await;
    assert!(sweep_deleted_devices(&env.db).await.unwrap() >= 1);
    assert_eq!(
        env.count("SELECT 1 FROM e2ee_devices WHERE user_id = $1", &[&alice])
            .await,
        0
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_deleted_device_ids WHERE user_id = $1",
            &[&alice]
        )
        .await,
        1
    );
    // 墓標の device_id は登録できず、削除は 200。
    let (status, body) = env.register(&alice, &account).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "E2EE_DEVICE_EXISTS");
    assert_eq!(
        request(&env.app, "DELETE", &path, &alice, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn query_and_claim_are_limited_to_shared_encrypted_channels() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let alice_device = env.device(&alice, 2).await;
    let bob_device = env.device(&bob, 2).await;
    let carol_device = env.device(&carol, 2).await;
    for (user, key) in [(&bob, "bob-master"), (&carol, "carol-master")] {
        for usage in ["master", "self_signing", "user_signing"] {
            env.exec(
                "INSERT INTO e2ee_cross_signing_keys (user_id, usage, public_key, key_json) VALUES ($1, $2, $3, $4::jsonb)",
                &[user, usage, &format!("{key}-{usage}"), &json!({"usage": [usage], "k": key}).to_string()],
            )
            .await;
        }
    }

    // まだ何も共有していない：自分の端末だけ。
    let result = env.query_keys(&alice, &[&alice, &bob, &carol]).await;
    assert!(result["device_keys"][&alice][alice_device.device_id()].is_object());
    assert!(result["device_keys"].get(&bob).is_none());
    assert_eq!(result["master_keys"], json!({}));
    assert_eq!(
        env.claim(&alice, &[(&bob, bob_device.device_id())]).await,
        json!({})
    );

    // 平文のプライベートチャンネルでは共有にならない。
    let plain = Uuid::new_v4().to_string();
    env.exec(
        "INSERT INTO channels (id, channel_name, created_user_id, is_private) VALUES ($1, $2, $3, true)",
        &[&plain, &format!("plain-{}", Uuid::new_v4().simple()), &alice],
    )
    .await;
    for member in [&alice, &bob] {
        env.exec(
            "INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, now())",
            &[member, &plain],
        )
        .await;
    }
    assert!(
        env.query_keys(&alice, &[&bob]).await["device_keys"]
            .get(&bob)
            .is_none()
    );

    // 暗号化チャンネルを共有すると取れる。ユーザー署名鍵は本人の分だけ。
    let channel = env.encrypted_channel(&[&alice, &bob]).await;
    let result = env.query_keys(&alice, &[&bob, &carol]).await;
    assert_eq!(
        result["device_keys"][&bob][bob_device.device_id()],
        bob_device.device_keys()
    );
    assert_eq!(result["master_keys"][&bob]["k"], "bob-master");
    assert_eq!(result["self_signing_keys"][&bob]["k"], "bob-master");
    assert_eq!(result["user_signing_keys"], json!({}));
    assert!(result["device_keys"].get(&carol).is_none());
    let claimed = env
        .claim(
            &alice,
            &[
                (&bob, bob_device.device_id()),
                (&carol, carol_device.device_id()),
            ],
        )
        .await;
    assert!(claimed_key(&claimed, &bob, bob_device.device_id()).is_some());
    assert!(claimed.get(&carol).is_none());

    // ピンか署名を持つ相手は、共有していなくてもマスター鍵だけ取れる。
    env.exec(
        "INSERT INTO e2ee_tofu_pins (user_id, target_user_id, master_key, pin_json) VALUES ($1, $2, 'carol-master-master', '{}'::jsonb)",
        &[&alice, &carol],
    )
    .await;
    let result = env.query_keys(&alice, &[&carol]).await;
    assert_eq!(result["master_keys"][&carol]["k"], "carol-master");
    assert!(result["self_signing_keys"].get(&carol).is_none());
    assert!(result["device_keys"].get(&carol).is_none());
    assert_eq!(result["tofu_pins"][&carol], json!({}));
    assert_eq!(
        env.claim(&alice, &[(&carol, carol_device.device_id())])
            .await,
        json!({})
    );

    // チャンネルを削除すると、元メンバーの鍵は問い合わせも払い出しもできない。
    env.exec(
        "UPDATE channels SET deleted_at = now() WHERE id = $1",
        &[&channel],
    )
    .await;
    assert!(
        env.query_keys(&alice, &[&bob]).await["device_keys"]
            .get(&bob)
            .is_none()
    );
    assert_eq!(
        env.claim(&alice, &[(&bob, bob_device.device_id())]).await,
        json!({})
    );
}

#[tokio::test]
async fn claims_never_hand_out_a_key_twice_and_fall_back_after_the_daily_limit() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let bob_device = env.device(&bob, 30).await;
    let device = bob_device.device_id().to_owned();
    env.encrypted_channel(&[&alice, &bob]).await;
    let fallback_name = fallback_key_name(&env, &bob, &device).await;

    // Alice と Carol（別の claimer）から並行に払い出す。
    let carol = env.user().await;
    env.encrypted_channel(&[&carol, &bob]).await;
    let mut tasks = Vec::new();
    for i in 0..30 {
        let app = env.app.clone();
        let claimer = if i % 2 == 0 {
            alice.clone()
        } else {
            carol.clone()
        };
        let (bob, device) = (bob.clone(), device.clone());
        tasks.push(tokio::spawn(async move {
            let (status, body) = request(
                &app,
                "POST",
                "/api/e2ee/keys/claim",
                &claimer,
                json!({"one_time_keys": {bob.clone(): {device.clone(): "signed_curve25519"}}}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let (name, key) = claimed_key(&body["data"]["one_time_keys"], &bob, &device).unwrap();
            (name, key["fallback"] == true)
        }));
    }
    let mut ids = Vec::new();
    for task in tasks {
        ids.push(task.await.unwrap());
    }
    // ワンタイム鍵とフォールバック鍵の key_id は別に採番されるので、名前ではなく fallback で見分ける。
    let one_time: Vec<_> = ids
        .iter()
        .filter(|(_, fallback)| !fallback)
        .map(|(id, _)| id)
        .collect();
    let unique: std::collections::BTreeSet<_> = one_time.iter().collect();
    assert_eq!(
        unique.len(),
        one_time.len(),
        "a one-time key was handed out twice"
    );
    // 1 人 15 回ずつで、どちらも 20 回の上限に達しない。鍵は 30 個あるので全部ワンタイム鍵。
    assert_eq!(one_time.len(), 30);
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_one_time_keys WHERE user_id = $1 AND claimed_at IS NULL",
            &[&bob]
        )
        .await,
        0
    );

    // 鍵がなくなったらフォールバック鍵を返し、used にする。
    let claimed = env.claim(&alice, &[(&bob, &device)]).await;
    let (name, key) = claimed_key(&claimed, &bob, &device).unwrap();
    assert_eq!(name, fallback_name);
    assert_eq!(key["fallback"], true);
    let (_, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &bob,
        json!({"device_id": device, "one_time_keys": {}}),
    )
    .await;
    assert_eq!(body["data"]["unused_fallback_key_types"], json!([]));

    // 1 日の上限（20 回）を超えたら、ワンタイム鍵が残っていてもフォールバック鍵を返す。
    let dave = env.user().await;
    let erin = env.user().await;
    let mut erin_device = env.device(&erin, 50).await;
    erin_device.generate_one_time_keys(40);
    let (status, _) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &erin,
        json!({"device_id": erin_device.device_id(), "one_time_keys": erin_device.one_time_keys()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    env.encrypted_channel(&[&dave, &erin]).await;
    for i in 0..22 {
        let claimed = env.claim(&dave, &[(&erin, erin_device.device_id())]).await;
        let (_, key) = claimed_key(&claimed, &erin, erin_device.device_id()).unwrap();
        assert_eq!(key["fallback"] == true, i >= 20, "claim {i}");
    }
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_one_time_keys WHERE user_id = $1 AND claimed_at IS NULL",
            &[&erin]
        )
        .await,
        70
    );

    // 1 回の払い出しは 1000 台まで。
    let mut many = Map::new();
    for _ in 0..1001 {
        many.insert(Uuid::new_v4().to_string(), json!("signed_curve25519"));
    }
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/claim",
        &dave,
        json!({"one_time_keys": {erin.clone(): many}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["limit"], "claim_devices");
}

async fn fallback_key_name(env: &Env, user: &str, device: &str) -> String {
    let rows = env
        .query(
            "SELECT to_json('signed_curve25519:' || key_id)::text FROM e2ee_fallback_keys WHERE user_id = $1 AND device_id = $2",
            &[user, device],
        )
        .await;
    rows[0].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn concurrent_uploads_and_claims_stay_within_the_key_limit() {
    let env = env().await;
    let alice = env.user().await;
    let mut account = env.device(&alice, 0).await;
    let mut tasks = Vec::new();
    for _ in 0..6 {
        account.generate_one_time_keys(25);
        let keys = account.one_time_keys();
        account.mark_keys_as_published();
        let app = env.app.clone();
        let (owner, device) = (alice.clone(), account.device_id().to_owned());
        tasks.push(tokio::spawn(async move {
            request(
                &app,
                "POST",
                "/api/e2ee/keys/upload",
                &owner,
                json!({"device_id": device, "one_time_keys": keys}),
            )
            .await
            .0
        }));
        let app = env.app.clone();
        let (alice, device) = (alice.clone(), account.device_id().to_owned());
        tasks.push(tokio::spawn(async move {
            request(
                &app,
                "POST",
                "/api/e2ee/keys/claim",
                &alice,
                json!({"one_time_keys": {alice.clone(): {device: "signed_curve25519"}}}),
            )
            .await
            .0
        }));
    }
    for task in tasks {
        let status = task.await.unwrap();
        assert!(
            status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
            "{status}"
        );
    }
    let stored = env
        .count(
            "SELECT 1 FROM e2ee_one_time_keys WHERE user_id = $1 AND claimed_at IS NULL",
            &[&alice],
        )
        .await
        + env
            .count(
                "SELECT 1 FROM e2ee_fallback_keys WHERE user_id = $1",
                &[&alice],
            )
            .await;
    assert!(stored <= 100, "{stored} keys stored");
}

/// 登録の再送が既存の端末を読んだあとに削除がコミットしても、削除済みの端末に鍵を入れない。
#[tokio::test]
async fn registration_retry_racing_a_delete_never_reinserts_keys() {
    let env = env().await;
    let alice = env.user().await;
    let mut account = env.device(&alice, 0).await;
    account.generate_one_time_keys(5);
    let device = account.device_id().to_owned();
    // 削除と同じく、端末単位のロックを持ったまま論理削除する（コミットはあと）。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    for sql in [
        "SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(2, hashtext($1 || ':' || $2))) l",
        "WITH d AS (UPDATE e2ee_devices SET deleted_at = now() WHERE user_id = $1 AND device_id = $2
           RETURNING 1) SELECT '{}'::text FROM d",
    ] {
        backend::apps::query(&mut tx, sql, &[&alice, &device])
            .await
            .unwrap();
    }
    let register = {
        let (app, alice, body) = (env.app.clone(), alice.clone(), register_body(&account));
        tokio::spawn(async move { request(&app, "POST", "/api/e2ee/devices", &alice, body).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !register.is_finished(),
        "registration must wait for the device lock"
    );
    tx.commit().await.unwrap();
    let (status, body) = tokio::time::timeout(Duration::from_secs(10), register)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "E2EE_DEVICE_EXISTS");
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_one_time_keys WHERE user_id = $1 AND device_id = $2",
            &[&alice, &device]
        )
        .await,
        0
    );
}

/// 払い出したワンタイム鍵は、登録やアップロードの再送（応答をなくした場合）でも入り直らない。
#[tokio::test]
async fn resent_uploads_never_resurrect_claimed_one_time_keys() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    env.encrypted_channel(&[&alice, &bob]).await;
    let mut account = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    account.generate_one_time_keys(2);
    account.generate_fallback_key();
    let registration = register_body(&account);
    let (status, body) = env.register(&alice, &account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    account.mark_keys_as_published();
    account.generate_one_time_keys(2);
    let upload =
        json!({"device_id": account.device_id(), "one_time_keys": account.one_time_keys()});
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        upload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let device = account.device_id().to_owned();

    // 4 個すべてを払い出す。
    let mut claimed = std::collections::BTreeSet::new();
    for _ in 0..4 {
        let result = env.claim(&bob, &[(&alice, &device)]).await;
        let (name, key) = claimed_key(&result, &alice, &device).unwrap();
        assert!(key.get("fallback").is_none(), "{key}");
        assert!(claimed.insert(name));
    }
    // 登録とアップロードを同じ内容で送り直しても、払い出した鍵は戻らない。
    let (status, body) = request(&env.app, "POST", "/api/e2ee/devices", &alice, registration).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["one_time_key_counts"]["signed_curve25519"], 0);
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        upload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["one_time_key_counts"]["signed_curve25519"], 0);
    // 次の払い出しはフォールバック鍵になる。
    let result = env.claim(&bob, &[(&alice, &device)]).await;
    let (_, key) = claimed_key(&result, &alice, &device).unwrap();
    assert_eq!(key["fallback"], true, "{key}");

    // 払い出した key_id で内容の違う鍵は 409。
    let mut other = DeviceAccount::new(&alice, &device).unwrap();
    other.generate_one_time_keys(1);
    let name = claimed
        .iter()
        .find(|name| upload["one_time_keys"].get(name.as_str()).is_some())
        .unwrap()
        .clone();
    let mut forged = upload["one_time_keys"][&name].clone();
    forged["key"] = other.one_time_keys().values().next().unwrap()["key"].clone();
    account.sign_json(&mut forged).unwrap();
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": device, "one_time_keys": {name: forged}}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "E2EE_KEY_EXISTS");
}

/// 遅れて届いた古いフォールバック鍵で、今の鍵を巻き戻さない（A → B → C のあとの A は無視する）。
/// 払い出しがコミットする前に同じ鍵の再送が始まっても、コミットのあとで鍵が入り直らない。
#[tokio::test]
async fn resend_racing_an_uncommitted_claim_never_resurrects_the_key() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    env.encrypted_channel(&[&alice, &bob]).await;
    let mut account = env.device(&alice, 0).await;
    account.generate_one_time_keys(1);
    let device = account.device_id().to_owned();
    let upload = json!({"device_id": device, "one_time_keys": account.one_time_keys()});
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        upload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (name, original) = account.one_time_keys().into_iter().next().unwrap();
    let key_id = name.strip_prefix("signed_curve25519:").unwrap().to_owned();

    // keys/claim と同じ文で払い出し、コミットせずに持つ。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    for sql in [
        "SELECT '{}'::text FROM e2ee_one_time_keys
         WHERE user_id = $1 AND device_id = $2 AND algorithm = 'signed_curve25519' AND key_id = $3
           AND claimed_at IS NULL
         ORDER BY created_at, key_id LIMIT 1 FOR UPDATE SKIP LOCKED",
        "WITH c AS (UPDATE e2ee_one_time_keys SET claimed_at = now()
           WHERE user_id = $1 AND device_id = $2 AND algorithm = 'signed_curve25519' AND key_id = $3
           RETURNING 1) SELECT '{}'::text FROM c",
    ] {
        let rows = backend::apps::query(&mut tx, sql, &[&alice, &device, &key_id])
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }
    let resend = {
        let (app, alice) = (env.app.clone(), alice.clone());
        tokio::spawn(
            async move { request(&app, "POST", "/api/e2ee/keys/upload", &alice, upload).await },
        )
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    tx.commit().await.unwrap();
    let (status, body) = tokio::time::timeout(Duration::from_secs(10), resend)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["one_time_key_counts"]["signed_curve25519"], 0);
    // 次の払い出しは同じ鍵ではなく、フォールバック鍵になる。
    let result = env.claim(&bob, &[(&alice, &device)]).await;
    // フォールバック鍵とワンタイム鍵の key_id は別の数え方なので、名前ではなく鍵で比べる。
    let (_, key) = claimed_key(&result, &alice, &device).unwrap();
    assert_ne!(key["key"], original["key"]);
    assert_eq!(key["fallback"], true, "{key}");
}

#[tokio::test]
async fn stale_fallback_keys_never_replace_newer_ones() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    env.encrypted_channel(&[&alice, &bob]).await;
    let mut account = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    account.generate_fallback_key();
    let registration = register_body(&account);
    let a = account.fallback_keys();
    let (status, body) = env.register(&alice, &account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    account.mark_keys_as_published();
    let device = account.device_id().to_owned();
    let upload = |keys: &Map<String, Value>| json!({"device_id": device, "fallback_keys": keys});
    let mut current = a.clone();
    for _ in 0..2 {
        account.generate_fallback_key();
        current = account.fallback_keys();
        let (status, body) = request(
            &env.app,
            "POST",
            "/api/e2ee/keys/upload",
            &alice,
            upload(&current),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        account.mark_keys_as_published();
    }
    let current_name = current.keys().next().unwrap().clone();
    assert_eq!(fallback_key_name(&env, &alice, &device).await, current_name);

    // A の遅れたアップロードと登録の再送は成功するが、C のまま。
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        upload(&a),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = request(&env.app, "POST", "/api/e2ee/devices", &alice, registration).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fallback_key_name(&env, &alice, &device).await, current_name);
    // 払い出されるのも、端末が秘密鍵を持つ C。
    let result = env.claim(&bob, &[(&alice, &device)]).await;
    let (name, key) = claimed_key(&result, &alice, &device).unwrap();
    assert_eq!(name, current_name);
    assert_eq!(key, current[&current_name]);

    // C の再送は used を戻さない。同じ key_id で内容の違う鍵は 409。
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        upload(&current),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["unused_fallback_key_types"], json!([]));
    let mut forged = current[&current_name].clone();
    forged["key"] = a.values().next().unwrap()["key"].clone();
    account.sign_json(&mut forged).unwrap();
    let mut keys = Map::new();
    keys.insert(current_name.clone(), forged);
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        upload(&keys),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "E2EE_KEY_EXISTS");

    // 世代を比べられない key_id のフォールバック鍵は 400。
    let mut keys = Map::new();
    keys.insert(
        "signed_curve25519:not-a-counter".into(),
        current[&current_name].clone(),
    );
    let (status, _) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        upload(&keys),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn concurrent_delete_and_upload_never_reinsert_keys() {
    let env = env().await;
    let alice = env.user().await;
    for _ in 0..5 {
        let mut account = env.device(&alice, 0).await;
        account.generate_one_time_keys(20);
        let keys = account.one_time_keys();
        let device = account.device_id().to_owned();
        let upload = {
            let (app, alice, device) = (env.app.clone(), alice.clone(), device.clone());
            tokio::spawn(async move {
                request(
                    &app,
                    "POST",
                    "/api/e2ee/keys/upload",
                    &alice,
                    json!({"device_id": device, "one_time_keys": keys}),
                )
                .await
                .0
            })
        };
        let delete = {
            let (app, alice, path) = (
                env.app.clone(),
                alice.clone(),
                format!("/api/e2ee/devices/{device}"),
            );
            tokio::spawn(async move { request(&app, "DELETE", &path, &alice, Value::Null).await.0 })
        };
        let upload = upload.await.unwrap();
        assert!(
            upload == StatusCode::OK || upload == StatusCode::NOT_FOUND,
            "{upload}"
        );
        assert_eq!(delete.await.unwrap(), StatusCode::OK);
        assert_eq!(
            env.count(
                "SELECT 1 FROM e2ee_one_time_keys WHERE user_id = $1 AND device_id = $2",
                &[&alice, &device]
            )
            .await,
            0
        );
    }
}

#[tokio::test]
async fn deactivation_deletes_devices_and_blocks_registration() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let first = env.device(&alice, 3).await;
    let second = env.device(&alice, 3).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    env.exec(
        "INSERT INTO e2ee_key_signatures (signer_user_id, signer_key_id, target_user_id, target_key_id, signature)
         VALUES ($1, 'ed25519:ssk', $1, $2, 'sig')",
        &[&alice, first.device_id()],
    )
    .await;

    env.exec(
        "UPDATE users SET guest_kind = 'deactivated' WHERE id = $1",
        &[&alice],
    )
    .await;
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_devices WHERE user_id = $1 AND deleted_at IS NULL",
            &[&alice]
        )
        .await,
        0
    );
    for table in ["e2ee_one_time_keys", "e2ee_fallback_keys"] {
        assert_eq!(
            env.count(
                &format!("SELECT 1 FROM {table} WHERE user_id = $1"),
                &[&alice]
            )
            .await,
            0
        );
    }
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_signatures WHERE signer_user_id = $1",
            &[&alice]
        )
        .await,
        0
    );
    let result = env.query_keys(&bob, &[&alice]).await;
    assert_eq!(result["device_keys"][&alice], json!({}));
    assert_eq!(
        env.claim(&bob, &[(&alice, second.device_id())]).await,
        json!({})
    );

    // 無効化されたユーザーは API を使えない（AuthUser と、書き込みのトランザクションの FOR SHARE）。
    let fresh = DeviceAccount::new(&alice, &Uuid::new_v4().to_string()).unwrap();
    assert_eq!(env.register(&alice, &fresh).await.0, StatusCode::FORBIDDEN);
    let (status, _) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/upload",
        &alice,
        json!({"device_id": second.device_id(), "one_time_keys": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// 登録が `users` の行を `FOR SHARE` で読んだあと、コミットする前に無効化を始めると、
/// 無効化は登録のコミットを待ち、登録した端末も削除済みになる。
#[tokio::test]
async fn deactivation_waits_for_an_in_flight_registration() {
    let env = env().await;
    let alice = env.user().await;
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    backend::apps::query(
        &mut tx,
        "SELECT '{}'::text FROM users WHERE id = $1 AND NOT is_deactivated FOR SHARE",
        &[&alice],
    )
    .await
    .unwrap();
    let deactivate = {
        let db = env.db.clone();
        let alice = alice.clone();
        tokio::spawn(async move {
            let mut db = db;
            toasty::sql::statement("UPDATE users SET guest_kind = 'deactivated' WHERE id = $1")
                .bind(alice.as_str())
                .exec(&mut db)
                .await
                .unwrap();
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !deactivate.is_finished(),
        "deactivation must wait for the registration"
    );
    let device = Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO e2ee_devices (user_id, device_id, curve25519_key, ed25519_key, device_keys, last_seen_at)
         VALUES ($1, $2, $3, $4, '{}'::jsonb, now())",
    )
    .bind(alice.as_str())
    .bind(device.as_str())
    .bind(format!("c-{device}").as_str())
    .bind(format!("e-{device}").as_str())
    .exec(&mut tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), deactivate)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_devices WHERE user_id = $1 AND deleted_at IS NULL",
            &[&alice]
        )
        .await,
        0
    );
}

/// 自分のワンタイム鍵の claim と、同じユーザーの無効化を同時に走らせても行き詰まらない。
#[tokio::test]
async fn self_claim_and_deactivation_do_not_deadlock() {
    let env = env().await;
    for _ in 0..5 {
        let alice = env.user().await;
        let mut targets = Vec::new();
        for _ in 0..3 {
            targets.push(env.device(&alice, 5).await.device_id().to_owned());
        }
        let claim = {
            let (app, alice, targets) = (env.app.clone(), alice.clone(), targets.clone());
            tokio::spawn(async move {
                let devices: Map<String, Value> = targets
                    .iter()
                    .map(|d| (d.clone(), json!("signed_curve25519")))
                    .collect();
                request(
                    &app,
                    "POST",
                    "/api/e2ee/keys/claim",
                    &alice,
                    json!({"one_time_keys": {alice.clone(): devices}}),
                )
                .await
                .0
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
        let claim = tokio::time::timeout(Duration::from_secs(10), claim)
            .await
            .unwrap()
            .unwrap();
        // 無効化が先にコミットされたら、claim は AuthUser で 403 になる。
        assert!(
            claim == StatusCode::OK || claim == StatusCode::FORBIDDEN,
            "{claim}"
        );
        tokio::time::timeout(Duration::from_secs(10), deactivate)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            env.count(
                "SELECT 1 FROM e2ee_devices WHERE user_id = $1 AND deleted_at IS NULL",
                &[&alice]
            )
            .await,
            0
        );
    }
}

/// 端末の削除と、同じユーザーのクロス署名鍵の変更（#356。ロックの順に従う書き込みを直接再現する）を
/// 同時に走らせても行き詰まらない。
#[tokio::test]
async fn device_delete_and_cross_signing_change_do_not_deadlock() {
    let env = env().await;
    let alice = env.user().await;
    for _ in 0..5 {
        let account = env.device(&alice, 2).await;
        env.exec(
            "INSERT INTO e2ee_key_signatures (signer_user_id, signer_key_id, target_user_id, target_key_id, signature)
             VALUES ($1, 'ed25519:ssk', $1, $2, 'sig')",
            &[&alice, account.device_id()],
        )
        .await;
        let change = {
            let db = env.db.clone();
            let alice = alice.clone();
            tokio::spawn(async move {
                let mut db = db;
                let mut tx = db.transaction().await.unwrap();
                for sql in [
                    "SELECT '{}'::text FROM users WHERE id = $1 AND NOT is_deactivated FOR SHARE",
                    "SELECT '{}'::text FROM pg_advisory_xact_lock(1, hashtext($1))",
                ] {
                    backend::apps::query(&mut tx, sql, &[&alice]).await.unwrap();
                }
                toasty::sql::statement(
                    "INSERT INTO e2ee_cross_signing_keys (user_id, usage, public_key, key_json)
                     VALUES ($1, 'self_signing', $2, '{}'::jsonb)
                     ON CONFLICT (user_id, usage) DO UPDATE SET public_key = EXCLUDED.public_key",
                )
                .bind(alice.as_str())
                .bind(Uuid::new_v4().to_string().as_str())
                .exec(&mut tx)
                .await
                .unwrap();
                toasty::sql::statement(
                    "DELETE FROM e2ee_key_signatures WHERE signer_user_id = $1 AND target_user_id = $1",
                )
                .bind(alice.as_str())
                .exec(&mut tx)
                .await
                .unwrap();
                tx.commit().await.unwrap();
            })
        };
        let delete = {
            let (app, alice, path) = (
                env.app.clone(),
                alice.clone(),
                format!("/api/e2ee/devices/{}", account.device_id()),
            );
            tokio::spawn(async move { request(&app, "DELETE", &path, &alice, Value::Null).await.0 })
        };
        tokio::time::timeout(Duration::from_secs(10), change)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), delete)
                .await
                .unwrap()
                .unwrap(),
            StatusCode::OK
        );
    }
}

/// `keys/query` の応答の合計が 8 MiB を超えるときは 400（`limit` は `response_size`）。
#[tokio::test]
async fn query_rejects_responses_over_eight_mebibytes() {
    let env = env().await;
    let alice = env.user().await;
    let mut users = Vec::new();
    for _ in 0..60 {
        let id = Uuid::new_v4().to_string();
        env.exec(
            "INSERT INTO users (id, user_name, display_name) VALUES ($1, $2, 'bulk')",
            &[&id, &format!("bulk_{}", Uuid::new_v4().simple())],
        )
        .await;
        users.push(id);
    }
    let mut members: Vec<&str> = vec![&alice];
    members.extend(users.iter().map(String::as_str));
    env.encrypted_channel(&members).await;
    // 1 人 20 台、1 台 7 KiB の端末鍵（署名の検証を通さず、行を直接作る）。60 × 20 × 7 KiB ≒ 8.2 MiB。
    let padding = "a".repeat(7 * 1024);
    let user_list = Value::from(users.clone()).to_string();
    env.exec(
        "INSERT INTO e2ee_devices (user_id, device_id, curve25519_key, ed25519_key, device_keys, last_seen_at)
         SELECT u, gen_random_uuid()::text, gen_random_uuid()::text, gen_random_uuid()::text,
                jsonb_build_object('padding', $2::text), now()
         FROM jsonb_array_elements_text($1::jsonb) u, generate_series(1, 20)",
        &[&user_list, &padding],
    )
    .await;
    let mut query_users: Vec<&str> = users.iter().map(String::as_str).collect();
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/query",
        &alice,
        json!({"user_ids": query_users}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "E2EE_LIMIT_EXCEEDED");
    assert_eq!(body["limit"], "response_size");

    // クライアントのように半分ずつに分ければ取れる。
    query_users.truncate(30);
    let result = env.query_keys(&alice, &query_users).await;
    assert_eq!(
        result["device_keys"][query_users[0]]
            .as_object()
            .unwrap()
            .len(),
        20
    );

    // 101 人以上は 400。
    let many: Vec<String> = (0..101).map(|_| Uuid::new_v4().to_string()).collect();
    let (status, body) = request(
        &env.app,
        "POST",
        "/api/e2ee/keys/query",
        &alice,
        json!({"user_ids": many}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["limit"], "query_users");
}

/// 端末の Shape（Electric を通す）。自分の端末と、暗号化チャンネルを共有する相手の端末だけを、
/// 決めた列だけで配る。
#[tokio::test]
async fn devices_shape_serves_shared_devices_without_private_columns() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let own = env.device(&alice, 0).await;
    let shared = env.device(&bob, 0).await;
    let other = env.device(&carol, 0).await;
    env.encrypted_channel(&[&alice, &bob]).await;
    let response = env
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/shapes/e2ee_devices?offset=-1")
                .header("X-Dev-User", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-cache");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let rows: Vec<&Value> = body
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m.get("value"))
        .collect();
    let ids: std::collections::BTreeSet<&str> = rows
        .iter()
        .filter_map(|r| r["device_id"].as_str())
        .collect();
    assert!(ids.contains(own.device_id()));
    assert!(ids.contains(shared.device_id()));
    assert!(!ids.contains(other.device_id()));
    for row in rows {
        assert!(row.get("display_name").is_none());
        assert!(row.get("last_seen_at").is_none());
    }
}

/// 端末の Shape の 1 回分の応答（Electric を通す）。`(handle, offset, メッセージ)` を返す。
async fn shape_page(env: &Env, actor: &str, query: &str) -> (String, String, Vec<Value>) {
    let response = env
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/shapes/e2ee_devices?{query}"))
                .header("X-Dev-User", actor)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let header = |name: &str| response.headers()[name].to_str().unwrap().to_owned();
    let (handle, offset) = (header("electric-handle"), header("electric-offset"));
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    (handle, offset, body.as_array().cloned().unwrap_or_default())
}

/// `up-to-date` か、`until` を満たすメッセージが来るまで Shape を読み進め、読んだメッセージを返す。
async fn shape_until(
    env: &Env,
    actor: &str,
    handle: &mut String,
    offset: &mut String,
    live: bool,
    until: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let mut seen = Vec::new();
    for _ in 0..20 {
        let query = format!(
            "handle={handle}&offset={offset}{}",
            if live { "&live=true" } else { "" }
        );
        let (h, o, messages) = shape_page(env, actor, &query).await;
        (*handle, *offset) = (h, o);
        seen.extend(messages);
        if until(&seen) {
            return seen;
        }
    }
    panic!("shape did not reach the expected state: {seen:?}");
}

fn has_event(messages: &[Value], event: &str) -> bool {
    messages.iter().any(|m| m["headers"]["event"] == event)
}

fn inserted(messages: &[Value], device: &str) -> bool {
    messages
        .iter()
        .any(|m| m["headers"]["operation"] == "insert" && m["value"]["device_id"] == device)
}

/// 端末の Shape の move-in と move-out（Electric を通す）。共有する暗号化チャンネルへの参加で相手の端末が入り、
/// 退出とチャンネルの削除で出る。
#[tokio::test]
async fn devices_shape_moves_devices_in_and_out_with_channel_membership() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    env.device(&alice, 0).await;
    let device = env.device(&bob, 0).await.device_id().to_owned();
    let channel = env.encrypted_channel(&[&alice]).await;
    let (mut handle, mut offset, first) = shape_page(&env, &alice, "offset=-1").await;
    assert!(!inserted(&first, &device));
    shape_until(&env, &alice, &mut handle, &mut offset, false, |m| {
        m.iter().any(|m| m["headers"]["control"] == "up-to-date")
    })
    .await;

    let join = "INSERT INTO channel_user (user_id, channel_id, joined_at) VALUES ($1, $2, now())";
    env.exec(join, &[&bob, &channel]).await;
    shape_until(&env, &alice, &mut handle, &mut offset, true, |m| {
        has_event(m, "move-in") && inserted(m, &device)
    })
    .await;

    env.exec(
        "DELETE FROM channel_user WHERE user_id = $1 AND channel_id = $2",
        &[&bob, &channel],
    )
    .await;
    shape_until(&env, &alice, &mut handle, &mut offset, true, |m| {
        has_event(m, "move-out")
    })
    .await;

    env.exec(join, &[&bob, &channel]).await;
    shape_until(&env, &alice, &mut handle, &mut offset, true, |m| {
        has_event(m, "move-in") && inserted(m, &device)
    })
    .await;

    env.exec(
        "UPDATE channels SET deleted_at = now() WHERE id = $1",
        &[&channel],
    )
    .await;
    shape_until(&env, &alice, &mut handle, &mut offset, true, |m| {
        has_event(m, "move-out")
    })
    .await;
}
