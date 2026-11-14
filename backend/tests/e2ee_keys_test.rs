//! E2EE のクロス署名、TOFU のピン、Secret Storage、鍵バックアップの API（#356）の結合テスト。
//! 仕様は docs/e2ee.md。
//!
//! 再認証（`X-Zoo-Reauth`）は、テストの鍵で署名した ID トークンで確かめる。実際の AuthZ で `max_age=0` が
//! 再ログインになり、`authenticated_at` が入ることは AuthZ#102 の E2E（`tests/e2e/reauth.spec.ts`）で確かめた。

use std::collections::HashMap;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{
    auth::oidc::JwksVerifier,
    config::{Config, OidcConfig},
    db::init_db,
    routes::create_router,
    state::AppState,
};
use base64::Engine as _;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use vodozemac::{Curve25519SecretKey, Ed25519SecretKey};
use zoo_crypto::DeviceAccount;

const ISSUER: &str = "https://idp.e2ee.test";
const WEB_CLIENT: &str = "zoo-web";
const TUI_CLIENT: &str = "zoo-tui";
const KID: &str = "reauth-key";
const ED25519_PEM: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIA0bID3qxIoHZTqzIn9bRmUiRLn6RyGupe/S6WE1CL7/
-----END PRIVATE KEY-----
";
const ED25519_X: &str = "cB-qRdJ9-hGN7VUdA71a5iLBDK9JKRIMVHe8_F3Dm3Y";

struct Env {
    app: Router,
    db: toasty::Db,
}

async fn env() -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.oidc = Some(OidcConfig {
        issuer: ISSUER.into(),
        audience: "https://zoo.test/api".into(),
        jwks_url: Some("http://127.0.0.1:9/jwks".into()),
        reauth_audiences: vec![WEB_CLIENT.into(), TUI_CLIENT.into()],
    });
    let db = init_db(&config.database_url).await.unwrap();
    let mut keys = HashMap::new();
    keys.insert(
        KID.to_owned(),
        (
            DecodingKey::from_ed_components(ED25519_X).unwrap(),
            Algorithm::EdDSA,
        ),
    );
    let state = AppState::with_verifier(
        db.clone(),
        config,
        reqwest::Client::new(),
        JwksVerifier::with_keys(keys),
    );
    Env {
        app: create_router(state),
        db,
    }
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// テストの IdP が発行した ID トークン。`claims` で既定の値を上書きする（null の項目は消す）。
fn id_token(sub: &str, claims: Value) -> String {
    id_token_with_typ(sub, claims, "JWT")
}

fn id_token_with_typ(sub: &str, overrides: Value, typ: &str) -> String {
    let mut claims = json!({
        "iss": ISSUER, "aud": WEB_CLIENT, "sub": sub,
        "exp": now() + 300, "iat": now(), "authenticated_at": now() - 10,
    });
    for (k, v) in overrides.as_object().unwrap() {
        if v.is_null() {
            claims.as_object_mut().unwrap().remove(k);
        } else {
            claims[k] = v.clone();
        }
    }
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(KID.into());
    header.typ = Some(typ.into());
    encode(
        &header,
        &claims,
        &EncodingKey::from_ed_pem(ED25519_PEM.as_bytes()).unwrap(),
    )
    .unwrap()
}

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    reauth: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("X-Dev-User", actor);
    if let Some(token) = reauth {
        builder = builder.header("X-Zoo-Reauth", token);
    }
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

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
}

fn random_b64(len: usize) -> String {
    b64(&(0..len).map(|_| rand_byte()).collect::<Vec<_>>())
}

fn rand_byte() -> u8 {
    Uuid::new_v4().as_bytes()[0]
}

fn pk(key: &Ed25519SecretKey) -> String {
    key.public_key().to_base64()
}

/// Signing JSON で `key` の署名を `signatures.<user>.ed25519:<公開鍵>` に足す。
fn sign(value: &mut Value, user: &str, key: &Ed25519SecretKey) {
    let message = zoo_crypto::signable_json(value).unwrap();
    let signature = key.sign(message.as_bytes()).to_base64();
    value["signatures"][user][format!("ed25519:{}", pk(key))] = json!(signature);
}

fn signature_of(value: &Value, key: &Ed25519SecretKey) -> String {
    let message = zoo_crypto::signable_json(value).unwrap();
    key.sign(message.as_bytes()).to_base64()
}

fn cs_key(
    user: &str,
    usage: &str,
    key: &Ed25519SecretKey,
    master: Option<&Ed25519SecretKey>,
) -> Value {
    let mut value = json!({"user_id": user, "usage": [usage], "keys": {format!("ed25519:{}", pk(key)): pk(key)}});
    if let Some(master) = master {
        sign(&mut value, user, master);
    }
    value
}

/// 1 人分のクロス署名鍵。
struct Identity {
    user: String,
    master: Ed25519SecretKey,
    ssk: Ed25519SecretKey,
    usk: Ed25519SecretKey,
}

impl Identity {
    fn new(user: &str) -> Self {
        Self {
            user: user.to_owned(),
            master: Ed25519SecretKey::new(),
            ssk: Ed25519SecretKey::new(),
            usk: Ed25519SecretKey::new(),
        }
    }

    fn upload_body(&self, expected: Value) -> Value {
        let mut body = json!({
            "master_key": cs_key(&self.user, "master", &self.master, None),
            "self_signing_key": cs_key(&self.user, "self_signing", &self.ssk, Some(&self.master)),
            "user_signing_key": cs_key(&self.user, "user_signing", &self.usk, Some(&self.master)),
        });
        if !expected.is_null() {
            body["expected_keys"] = expected;
        }
        body
    }

    fn public(&self) -> Value {
        json!({"master": pk(&self.master), "self_signing": pk(&self.ssk), "user_signing": pk(&self.usk)})
    }

    fn pin(&self, target: &str, master_key: &str) -> Value {
        self.pin_signed_by(target, master_key, &self.usk)
    }

    fn pin_signed_by(&self, target: &str, master_key: &str, usk: &Ed25519SecretKey) -> Value {
        let mut pin = json!({"type": "zoo.tofu_pin", "user_id": self.user, "target_user_id": target, "master_key": master_key});
        sign(&mut pin, &self.user, usk);
        json!({"target_user_id": target, "master_key": master_key, "signatures": pin["signatures"]})
    }

    fn auth_data(&self, public_key: &str) -> Value {
        let mut auth = json!({"public_key": public_key});
        sign(&mut auth, &self.user, &self.master);
        auth
    }
}

fn backup_key() -> String {
    vodozemac::Curve25519PublicKey::from(&Curve25519SecretKey::new()).to_base64()
}

fn secret() -> Value {
    json!({"iv": random_b64(16), "ciphertext": random_b64(40), "mac": random_b64(32)})
}

fn recovery_slot() -> Value {
    json!({"id": Uuid::new_v4().to_string(), "kind": "recovery", "nonce": random_b64(12), "ciphertext": random_b64(48)})
}

fn passkey_slot(credential: &str) -> Value {
    json!({"id": Uuid::new_v4().to_string(), "kind": "passkey", "credential_id": credential,
           "prf_salt": random_b64(32), "nonce": random_b64(12), "ciphertext": random_b64(48), "label": "key"})
}

fn ssss_body(key_id: &str, expected_key: Value, expected_revision: Value, slots: Value) -> Value {
    let body = json!({
        "key_id": key_id,
        "key": {"algorithm": "m.secret_storage.v1.aes-hmac-sha2", "iv": random_b64(16), "mac": random_b64(32)},
        "secrets": {
            "m.cross_signing.master": secret(), "m.cross_signing.self_signing": secret(),
            "m.cross_signing.user_signing": secret(), "m.megolm_backup.v1": secret(),
        },
        "slots": slots,
        "expected_default_key_id": expected_key,
        "expected_secrets_revision": expected_revision,
    });
    body
}

fn session(channel: &str, sender: &str, id: &str, verified: bool, index: i64) -> Value {
    json!({"channel_id": channel, "sender_key": sender, "session_id": id,
           "first_message_index": index, "forwarded_count": 0, "is_verified": verified,
           "session_data": {"ephemeral": random_b64(32), "ciphertext": random_b64(64), "mac": random_b64(32)}})
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
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

    /// OIDC の identity を持つユーザー（`sub` はユーザー ID と同じにする）。
    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, _) = send(
            &self.app,
            "POST",
            "/api/users",
            &id,
            None,
            json!({"id": id, "user_name": format!("e2ee_{}", Uuid::new_v4().simple()), "display_name": "E2EE"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        self.exec(
            "INSERT INTO user_identities (id, user_id, iss, sub) VALUES ($1, $2, $3, $2)",
            &[&Uuid::new_v4().to_string(), &id, ISSUER],
        )
        .await;
        id
    }

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

    async fn post(
        &self,
        path: &str,
        actor: &str,
        reauth: bool,
        body: Value,
    ) -> (StatusCode, Value) {
        let token = reauth.then(|| id_token(actor, json!({})));
        send(&self.app, "POST", path, actor, token.as_deref(), body).await
    }

    async fn get(&self, path: &str, actor: &str) -> (StatusCode, Value) {
        send(&self.app, "GET", path, actor, None, Value::Null).await
    }

    async fn delete(&self, path: &str, actor: &str, reauth: bool) -> (StatusCode, Value) {
        let token = reauth.then(|| id_token(actor, json!({})));
        send(
            &self.app,
            "DELETE",
            path,
            actor,
            token.as_deref(),
            Value::Null,
        )
        .await
    }

    /// 初回設定のクロス署名鍵を登録した利用者。
    async fn identity(&self) -> Identity {
        let user = self.user().await;
        let identity = Identity::new(&user);
        let (status, body) = self
            .post(
                "/api/e2ee/cross_signing/keys",
                &user,
                true,
                identity.upload_body(Value::Null),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        identity
    }

    /// SSSS を作る。`key_id` を返す。
    async fn ssss(&self, user: &str, slots: Value) -> String {
        let key_id = Uuid::new_v4().to_string();
        let (status, body) = self
            .post(
                "/api/e2ee/secret_storage/keys",
                user,
                true,
                ssss_body(&key_id, Value::Null, Value::Null, slots),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        key_id
    }

    async fn revision(&self, user: &str) -> i64 {
        self.get("/api/e2ee/secret_storage", user).await.1["secrets_revision"]
            .as_i64()
            .unwrap()
    }
}

// ---------------------------------------------------------------------------
// 再認証

#[tokio::test]
async fn reauth_requires_a_fresh_id_token_for_the_same_user() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let identity = Identity::new(&alice);
    let body = identity.upload_body(Value::Null);
    let path = "/api/e2ee/cross_signing/keys";

    // 初回設定のマスター鍵の登録も、再認証なしでは 403。
    let (status, res) = send(&env.app, "POST", path, &alice, None, body.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&res), "E2EE_REAUTH_REQUIRED");

    let rejected = [
        // claim がない（QR ログインのセッションやリフレッシュで発行したもの）。
        id_token(&alice, json!({"authenticated_at": null})),
        // 5 分より前に認証した。
        id_token(&alice, json!({"authenticated_at": now() - 301})),
        // 別のユーザーの ID トークン。
        id_token(&bob, json!({})),
        // zoo2 のクライアントでない aud。
        id_token(&alice, json!({"aud": "other-client"})),
        // 期限切れ。
        id_token(&alice, json!({"exp": now() - 120})),
        // 別の発行者。
        id_token(&alice, json!({"iss": "https://evil.test"})),
        // アクセストークン（typ: at+jwt）。
        id_token_with_typ(&alice, json!({"aud": WEB_CLIENT}), "at+jwt"),
        "not-a-jwt".to_owned(),
    ];
    for token in &rejected {
        let (status, res) = send(&env.app, "POST", path, &alice, Some(token), body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{token}: {res}");
        assert_eq!(code(&res), "E2EE_REAUTH_REQUIRED");
    }
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_cross_signing_keys WHERE user_id = $1",
            &[&alice]
        )
        .await,
        0
    );

    // 一覧の別のクライアント（TUI）で発行した ID トークンも認める。
    let token = id_token(&alice, json!({"aud": [TUI_CLIENT]}));
    let (status, res) = send(&env.app, "POST", path, &alice, Some(&token), body).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(
        res["data"]["master_key"]["keys"][format!("ed25519:{}", pk(&identity.master))],
        pk(&identity.master)
    );
    assert!(res["txid"].is_number() || res["txid"].is_string(), "{res}");

    // SSSS の最初の作成も、再認証なしでは 403。
    let key_id = Uuid::new_v4().to_string();
    let ssss = ssss_body(&key_id, Value::Null, Value::Null, json!([recovery_slot()]));
    let (status, res) = env
        .post("/api/e2ee/secret_storage/keys", &alice, false, ssss.clone())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code(&res), "E2EE_REAUTH_REQUIRED");
    let (status, res) = env
        .post("/api/e2ee/secret_storage/keys", &alice, true, ssss)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["secrets_revision"], 1);
}

#[tokio::test]
async fn cors_allows_the_reauth_header() {
    let env = env().await;
    let response = env
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/api/e2ee/cross_signing/keys")
                .header("origin", "http://localhost:5173")
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "x-zoo-reauth,x-zoo-backup-proof,content-type",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let allowed = response.headers()["access-control-allow-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(allowed.contains("x-zoo-reauth"), "{allowed}");
    assert!(allowed.contains("x-zoo-backup-proof"), "{allowed}");
}

// ---------------------------------------------------------------------------
// クロス署名鍵

#[tokio::test]
async fn cross_signing_keys_are_idempotent_checked_and_never_reused() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    let path = "/api/e2ee/cross_signing/keys";
    let changes = || async {
        env.count(
            "SELECT 1 FROM e2ee_cross_signing_key_changes WHERE user_id = $1",
            &[&user],
        )
        .await
    };
    assert_eq!(changes().await, 3);

    // 同じ鍵の送り直しは、再認証も expected_keys もなしで 200 になり、何も変えない。
    let (status, res) = env
        .post(path, &user, false, alice.upload_body(Value::Null))
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert!(res["txid"].is_null());
    assert_eq!(changes().await, 3);

    // 差し替えでは expected_keys が必須で、今の鍵と違えば 409。
    let mut next = Identity::new(&user);
    next.master = Ed25519SecretKey::from_slice(&alice.master.to_bytes());
    let (status, _) = env
        .post(path, &user, true, next.upload_body(Value::Null))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let mut stale = alice.public();
    stale["self_signing"] = json!(pk(&Ed25519SecretKey::new()));
    let (status, res) = env.post(path, &user, true, next.upload_body(stale)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code(&res), "E2EE_CROSS_SIGNING_CONFLICT");
    assert_eq!(res["current_keys"], alice.public());

    // マスター鍵の公開鍵が同じでも、差し替え（自己署名鍵とユーザー署名鍵）には再認証が要る。
    let (status, res) = env
        .post(path, &user, false, next.upload_body(alice.public()))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{res}");

    // マスター鍵で署名されていない自己署名鍵は 400。
    let mut forged = next.upload_body(alice.public());
    forged["self_signing_key"] = cs_key(
        &user,
        "self_signing",
        &next.ssk,
        Some(&Ed25519SecretKey::new()),
    );
    let (status, res) = env.post(path, &user, true, forged).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(code(&res), "E2EE_BAD_SIGNATURE");

    // 自己署名鍵とユーザー署名鍵だけを差し替える（端末だけの失効）。
    let (status, res) = env
        .post(path, &user, true, next.upload_body(alice.public()))
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(changes().await, 5);

    // 古い自己署名鍵の送り直しは 400 E2EE_KEY_REUSED（再認証があっても）。
    let mut rollback = next.upload_body(next.public());
    rollback["self_signing_key"] = cs_key(&user, "self_signing", &alice.ssk, Some(&alice.master));
    rollback.as_object_mut().unwrap().remove("user_signing_key");
    rollback.as_object_mut().unwrap().remove("master_key");
    let (status, res) = env.post(path, &user, true, rollback).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert_eq!(code(&res), "E2EE_KEY_REUSED");
    // 今ほかの用途で使っている鍵も使えない。
    let mut cross = next.upload_body(next.public());
    cross["self_signing_key"] = cs_key(&user, "self_signing", &next.usk, Some(&next.master));
    cross.as_object_mut().unwrap().remove("user_signing_key");
    let (status, res) = env.post(path, &user, true, cross).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
}

#[tokio::test]
async fn master_key_rotation_needs_resigned_keys_and_updates_same_public_keys() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    let path = "/api/e2ee/cross_signing/keys";
    let new_master = Ed25519SecretKey::new();

    // マスター鍵だけを差し替える要求は 400。
    let body = json!({"master_key": cs_key(&user, "master", &new_master, None), "expected_keys": alice.public()});
    let (status, res) = env.post(path, &user, true, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");

    // 自己署名鍵とユーザー署名鍵の公開鍵は同じで、新しいマスター鍵で署名し直したもの。
    let body = json!({
        "master_key": cs_key(&user, "master", &new_master, None),
        "self_signing_key": cs_key(&user, "self_signing", &alice.ssk, Some(&new_master)),
        "user_signing_key": cs_key(&user, "user_signing", &alice.usk, Some(&new_master)),
        "expected_keys": alice.public(),
    });
    let (status, res) = env.post(path, &user, true, body).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let stored = env
        .query(
            "SELECT key_json::text FROM e2ee_cross_signing_keys WHERE user_id = $1 AND usage = 'self_signing'",
            &[&user],
        )
        .await;
    assert!(
        stored[0]["signatures"][&user]
            .get(format!("ed25519:{}", pk(&new_master)))
            .is_some()
    );
    // 公開鍵が変わったのはマスター鍵だけなので、変更の記録はマスター鍵の 1 件だけ増える。
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_cross_signing_key_changes WHERE user_id = $1 AND old_public_key IS NOT NULL",
            &[&user]
        )
        .await,
        1
    );
    // 古いマスター鍵には戻せない。
    let mut back = alice.upload_body(json!({"master": pk(&new_master), "self_signing": pk(&alice.ssk), "user_signing": pk(&alice.usk)}));
    back["self_signing_key"] = cs_key(&user, "self_signing", &alice.ssk, Some(&alice.master));
    let (status, res) = env.post(path, &user, true, back).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(code(&res), "E2EE_KEY_REUSED");
}

#[tokio::test]
async fn concurrent_revocations_never_overwrite_each_other() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    let mut handles = Vec::new();
    for _ in 0..4 {
        let next = Identity::new(&user);
        let body = next.upload_body(alice.public());
        let app = env.app.clone();
        let user = user.clone();
        handles.push(tokio::spawn(async move {
            let token = id_token(&user, json!({}));
            let (status, res) = send(
                &app,
                "POST",
                "/api/e2ee/cross_signing/keys",
                &user,
                Some(&token),
                body,
            )
            .await;
            (status, res, pk(&next.master))
        }));
    }
    let mut winners = Vec::new();
    for handle in handles {
        let (status, res, master) = handle.await.unwrap();
        match status {
            StatusCode::OK => winners.push(master),
            StatusCode::CONFLICT => assert_eq!(code(&res), "E2EE_CROSS_SIGNING_CONFLICT"),
            other => panic!("unexpected {other}: {res}"),
        }
    }
    assert_eq!(winners.len(), 1);
    let stored = env
        .query(
            "SELECT to_json(public_key)::text FROM e2ee_cross_signing_keys WHERE user_id = $1 AND usage = 'master'",
            &[&user],
        )
        .await;
    assert_eq!(stored[0], json!(winners[0]));
}

// ---------------------------------------------------------------------------
// 署名

#[tokio::test]
async fn signatures_are_verified_idempotent_and_need_reauth_for_other_users() {
    let env = env().await;
    let alice = env.identity().await;
    let bob = env.identity().await;
    let user = alice.user.clone();
    env.encrypted_channel(&[&user, &bob.user]).await;
    let device = DeviceAccount::new(&user, &Uuid::new_v4().to_string()).unwrap();
    let (status, res) = env
        .post(
            "/api/e2ee/devices",
            &user,
            false,
            json!({"device_keys": device.device_keys()}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let path = "/api/e2ee/cross_signing/signatures";

    // 自己署名鍵による自分の端末への署名（再認証は要らない）。再送も 200。
    let device_sig = json!({"signatures": [{
        "target_user_id": user, "target_key_id": device.device_id(),
        "signer_key_id": format!("ed25519:{}", pk(&alice.ssk)),
        "signature": signature_of(&device.device_keys(), &alice.ssk),
    }]});
    for attempt in 0..2 {
        let (status, res) = env.post(path, &user, false, device_sig.clone()).await;
        assert_eq!(status, StatusCode::OK, "{res}");
        // 書いたときだけ txid を返し、同じ署名の再送（Shape に流れる変更がない）では null。
        assert_eq!(res["txid"].is_null(), attempt == 1, "{res}");
    }
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_signatures WHERE signer_user_id = $1",
            &[&user]
        )
        .await,
        1
    );

    // 間違った署名と、今の鍵でない署名者は 400。
    let mut bad = device_sig.clone();
    bad["signatures"][0]["signature"] = json!(signature_of(&json!({"x": 1}), &alice.ssk));
    let (status, res) = env.post(path, &user, false, bad).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_BAD_SIGNATURE")
    );

    // ユーザー署名鍵による相手のマスター鍵への署名は、再認証が要る。
    let bob_master = cs_key(&bob.user, "master", &bob.master, None);
    let master_sig = json!({"signatures": [{
        "target_user_id": bob.user, "target_key_id": pk(&bob.master),
        "signer_key_id": format!("ed25519:{}", pk(&alice.usk)),
        "signature": signature_of(&bob_master, &alice.usk),
    }]});
    let (status, res) = env.post(path, &user, false, master_sig.clone()).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    let (status, res) = env.post(path, &user, true, master_sig.clone()).await;
    assert_eq!(status, StatusCode::OK, "{res}");

    // 暗号化チャンネルを共有していない、ピンも署名もない相手のマスター鍵には署名できない（存在を隠して 404）。
    let carol = env.identity().await;
    let carol_master = cs_key(&carol.user, "master", &carol.master, None);
    let carol_sig = json!({"signatures": [{
        "target_user_id": carol.user, "target_key_id": pk(&carol.master),
        "signer_key_id": format!("ed25519:{}", pk(&alice.usk)),
        "signature": signature_of(&carol_master, &alice.usk),
    }]});
    let (status, _) = env.post(path, &user, true, carol_sig).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 自己署名鍵を差し替えると、古い鍵による端末への署名は消える。
    let next = Identity {
        user: user.clone(),
        master: Ed25519SecretKey::from_slice(&alice.master.to_bytes()),
        ssk: Ed25519SecretKey::new(),
        usk: Ed25519SecretKey::from_slice(&alice.usk.to_bytes()),
    };
    let (status, res) = env
        .post(
            "/api/e2ee/cross_signing/keys",
            &user,
            true,
            next.upload_body(alice.public()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_signatures WHERE signer_user_id = $1 AND signer_key_id = $2",
            &[&user, &format!("ed25519:{}", pk(&alice.ssk))]
        )
        .await,
        0
    );
    // マスター鍵への署名（ユーザー署名鍵は同じ）は残る。
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_signatures WHERE signer_user_id = $1",
            &[&user]
        )
        .await,
        1
    );
}

#[tokio::test]
async fn deactivated_users_cannot_write_signatures() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    env.exec(
        "UPDATE users SET guest_kind = 'deactivated' WHERE id = $1",
        &[&user],
    )
    .await;
    let body = json!({"signatures": [{
        "target_user_id": user, "target_key_id": Uuid::new_v4().to_string(),
        "signer_key_id": format!("ed25519:{}", pk(&alice.ssk)), "signature": "x",
    }]});
    let (status, _) = env
        .post("/api/e2ee/cross_signing/signatures", &user, true, body)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = env
        .post(
            "/api/e2ee/tofu_pins",
            &user,
            false,
            json!({"pins": [alice.pin(&Uuid::new_v4().to_string(), &pk(&alice.master))]}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// TOFU のピン

#[tokio::test]
async fn pins_conflict_only_against_verifiable_pins() {
    let env = env().await;
    let alice = env.identity().await;
    let bob = env.user().await;
    let user = alice.user.clone();
    let path = "/api/e2ee/tofu_pins";
    let first_master = pk(&Ed25519SecretKey::new());
    let second_master = pk(&Ed25519SecretKey::new());

    let (status, res) = env
        .post(
            path,
            &user,
            false,
            json!({"pins": [alice.pin(&bob, &first_master)]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    // 同じピンの再送は 200。
    let (status, _) = env
        .post(
            path,
            &user,
            false,
            json!({"pins": [alice.pin(&bob, &first_master)]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // 違うマスター鍵は、今のユーザー署名鍵で検証できるピンがあるので 409（今のピンを返す）。
    let (status, res) = env
        .post(
            path,
            &user,
            false,
            json!({"pins": [alice.pin(&bob, &second_master)]}),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_PIN_CONFLICT")
    );
    assert_eq!(res["pin"]["master_key"], first_master);
    // 今のユーザー署名鍵で署名していないピンは 400。
    let forged = alice.pin_signed_by(&bob, &second_master, &Ed25519SecretKey::new());
    let (status, res) = env
        .post(path, &user, false, json!({"pins": [forged]}))
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_BAD_SIGNATURE")
    );
    // replace: true は再認証が要る。
    let replace = json!({"pins": [alice.pin(&bob, &second_master)], "replace": true});
    let (status, res) = env.post(path, &user, false, replace.clone()).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    let (status, res) = env.post(path, &user, true, replace).await;
    assert_eq!(status, StatusCode::OK, "{res}");

    // ユーザー署名鍵を作り直すと、前の鍵で署名したピンは今の鍵で検証できないので、replace: false でも
    // 409 にならずに置き換わる。
    let next = Identity {
        user: user.clone(),
        master: Ed25519SecretKey::from_slice(&alice.master.to_bytes()),
        ssk: Ed25519SecretKey::new(),
        usk: Ed25519SecretKey::new(),
    };
    let (status, res) = env
        .post(
            "/api/e2ee/cross_signing/keys",
            &user,
            true,
            next.upload_body(alice.public()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let (status, res) = env
        .post(
            path,
            &user,
            false,
            json!({"pins": [next.pin(&bob, &first_master)]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let stored = env
        .query("SELECT to_json(master_key)::text FROM e2ee_tofu_pins WHERE user_id = $1 AND target_user_id = $2", &[&user, &bob])
        .await;
    assert_eq!(stored[0], json!(first_master));
}

#[tokio::test]
async fn concurrent_first_pins_never_fail_with_500() {
    let env = env().await;
    let alice = env.identity().await;
    let bob = env.user().await;
    let carol = env.user().await;
    let same_master = pk(&Ed25519SecretKey::new());
    let mut handles = Vec::new();
    for i in 0..6 {
        // bob には全員が同じマスター鍵、carol には端末ごとに違うマスター鍵のピンを作る。
        let (target, master) = if i % 2 == 0 {
            (bob.clone(), same_master.clone())
        } else {
            (carol.clone(), pk(&Ed25519SecretKey::new()))
        };
        let body = json!({"pins": [alice.pin(&target, &master)]});
        let app = env.app.clone();
        let user = alice.user.clone();
        handles.push(tokio::spawn(async move {
            (
                target,
                send(&app, "POST", "/api/e2ee/tofu_pins", &user, None, body).await,
            )
        }));
    }
    let mut carol_ok = 0;
    for handle in handles {
        let (target, (status, res)) = handle.await.unwrap();
        if target == bob {
            assert_eq!(status, StatusCode::OK, "{res}");
        } else if status == StatusCode::OK {
            carol_ok += 1;
        } else {
            assert_eq!(
                (status, code(&res)),
                (StatusCode::CONFLICT, "E2EE_PIN_CONFLICT")
            );
        }
    }
    assert_eq!(carol_ok, 1);
}

#[tokio::test]
async fn pins_are_listed_in_pages_for_the_caller_only() {
    let env = env().await;
    let alice = env.identity().await;
    let mut targets = Vec::new();
    for _ in 0..3 {
        targets.push(env.user().await);
    }
    let pins: Vec<Value> = targets
        .iter()
        .map(|t| alice.pin(t, &pk(&Ed25519SecretKey::new())))
        .collect();
    let (status, res) = env
        .post(
            "/api/e2ee/tofu_pins",
            &alice.user,
            false,
            json!({"pins": pins}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    targets.sort();
    let mut seen = Vec::new();
    let mut after = String::new();
    loop {
        let (status, page) = env
            .get(&format!("/api/e2ee/tofu_pins?limit=2{after}"), &alice.user)
            .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        for pin in page["pins"].as_array().unwrap() {
            assert_eq!(pin["type"], "zoo.tofu_pin");
            seen.push(pin["target_user_id"].as_str().unwrap().to_owned());
        }
        match page["next"].as_str() {
            Some(next) => after = format!("&after={next}"),
            None => break,
        }
    }
    assert_eq!(seen, targets);
    for bad in ["0", "1001", "x"] {
        let (status, _) = env
            .get(&format!("/api/e2ee/tofu_pins?limit={bad}"), &alice.user)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    // 他のユーザーには見えない。
    let other = env.user().await;
    let (_, page) = env.get("/api/e2ee/tofu_pins", &other).await;
    assert_eq!(page, json!({"pins": [], "next": null}));
}

// ---------------------------------------------------------------------------
// Secret Storage

#[tokio::test]
async fn secret_storage_checks_revisions_slots_and_reauth() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let keys = "/api/e2ee/secret_storage/keys";

    // スロットが空の要求は 400 E2EE_SLOTS_REQUIRED。
    let (status, res) = env
        .post(
            keys,
            &alice,
            true,
            ssss_body(
                &Uuid::new_v4().to_string(),
                Value::Null,
                Value::Null,
                json!([]),
            ),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_SLOTS_REQUIRED")
    );
    // 4 つの秘密がそろっていなければ 400。
    let mut partial = ssss_body(
        &Uuid::new_v4().to_string(),
        Value::Null,
        Value::Null,
        json!([recovery_slot()]),
    );
    partial["secrets"]
        .as_object_mut()
        .unwrap()
        .remove("m.megolm_backup.v1");
    let (status, _) = env.post(keys, &alice, true, partial).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let first_key = env
        .ssss(&alice, json!([recovery_slot(), passkey_slot("cred-a")]))
        .await;
    let (_, storage) = env.get("/api/e2ee/secret_storage", &alice).await;
    assert_eq!(storage["default_key_id"], first_key);
    assert_eq!(storage["secrets_revision"], 1);
    assert_eq!(storage["secrets"].as_object().unwrap().len(), 4);
    assert_eq!(storage["slots"].as_array().unwrap().len(), 2);
    // 他人の SSSS は読めない。
    let (_, other) = env.get("/api/e2ee/secret_storage", &bob).await;
    assert_eq!(other["default_key_id"], Value::Null);
    assert_eq!(other["slots"], json!([]));

    // 2 つ目の端末が初回設定を送っても 409（先の SSSS を消さない）。
    let (status, res) = env
        .post(
            keys,
            &alice,
            true,
            ssss_body(
                &Uuid::new_v4().to_string(),
                Value::Null,
                Value::Null,
                json!([recovery_slot()]),
            ),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_SSSS_CONFLICT")
    );
    assert_eq!(res["default_key_id"], first_key);
    assert_eq!(res["secrets_revision"], 1);
    // 作り直しでは expected_* が必須。
    let mut missing = ssss_body(
        &Uuid::new_v4().to_string(),
        Value::Null,
        Value::Null,
        json!([recovery_slot()]),
    );
    missing
        .as_object_mut()
        .unwrap()
        .remove("expected_default_key_id");
    let (status, _) = env.post(keys, &alice, true, missing).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 秘密は、足すにも置き換えるにも再認証が要る（M-3。トークンだけで開けない秘密を足して
    // secrets_revision を進めさせない）。拒否したときは revision も変わらない。
    let secrets = "/api/e2ee/secret_storage/secrets";
    let add = json!({"key_id": first_key, "expected_secrets_revision": 1, "secrets": {"zoo.megolm_backup.previous": secret()}});
    let (status, res) = env.post(secrets, &alice, false, add.clone()).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    assert_eq!(env.revision(&alice).await, 1);
    let (status, res) = env.post(secrets, &alice, true, add).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["secrets_revision"], 2);
    let old_ciphertext = env
        .query("SELECT json_build_object('iv', iv, 'ciphertext', ciphertext, 'mac', mac)::text FROM e2ee_secrets WHERE user_id = $1 AND name = 'm.megolm_backup.v1'", &[&alice])
        .await[0]
        .clone();
    let replace = json!({"key_id": first_key, "expected_secrets_revision": 2, "secrets": {"m.megolm_backup.v1": secret()}});
    let (status, res) = env.post(secrets, &alice, false, replace.clone()).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    let (status, res) = env.post(secrets, &alice, true, replace).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    // 古い暗号文の送り直し：再認証がなければ 403、古い revision なら 409。
    let resend = json!({"key_id": first_key, "expected_secrets_revision": 3, "secrets": {"m.megolm_backup.v1": old_ciphertext}});
    let (status, _) = env.post(secrets, &alice, false, resend.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let mut stale = resend.clone();
    stale["expected_secrets_revision"] = json!(2);
    let (status, res) = env.post(secrets, &alice, true, stale).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_SSSS_CONFLICT")
    );
    // 既定でない鍵への書き込みは 409。
    let wrong_key = json!({"key_id": Uuid::new_v4().to_string(), "expected_secrets_revision": 3, "secrets": {"zoo.megolm_backup.previous": secret()}});
    let (status, res) = env.post(secrets, &alice, true, wrong_key).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_SSSS_CONFLICT")
    );
    // 他人は書けない（自分の SSSS がないので 409）。
    let (status, _) = env
        .post(secrets, &bob, true, json!({"key_id": first_key, "expected_secrets_revision": 3, "secrets": {"zoo.megolm_backup.previous": secret()}}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // 作り直すと古い鍵の秘密とスロットは消え、revision は引き継いで増える。
    let second_key = Uuid::new_v4().to_string();
    let (status, res) = env
        .post(
            keys,
            &alice,
            true,
            ssss_body(
                &second_key,
                json!(first_key),
                json!(3),
                json!([recovery_slot()]),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["secrets_revision"], 4);
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_secrets WHERE user_id = $1 AND key_id = $2",
            &[&alice, &first_key]
        )
        .await,
        0
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_secret_storage_key_slots WHERE user_id = $1",
            &[&alice]
        )
        .await,
        1
    );
}

#[tokio::test]
async fn slots_are_capped_replaced_with_reauth_and_never_drop_to_zero() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let key_id = env.ssss(&alice, json!([recovery_slot()])).await;
    let path = "/api/e2ee/secret_storage/slots";
    let with_key = |slot: Value| {
        let mut slot = slot;
        slot["key_id"] = json!(key_id);
        slot
    };

    // 既定でない鍵へのスロットは 409。
    let mut stale = passkey_slot("cred-x");
    stale["key_id"] = json!(Uuid::new_v4().to_string());
    let (status, res) = env.post(path, &alice, false, stale).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_SSSS_CONFLICT")
    );
    assert_eq!(res["default_key_id"], key_id);

    // スロットを足すにも再認証が要る（M-3。トークンだけで開けないスロットを上限まで足させない）。
    let (status, res) = env
        .post(path, &alice, false, with_key(passkey_slot("cred-0")))
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_secret_storage_key_slots WHERE user_id = $1",
            &[&alice]
        )
        .await,
        1
    );

    // パスキーのスロットは 20 個まで。
    let mut first_slot = Value::Null;
    for i in 0..20 {
        let slot = with_key(passkey_slot(&format!("cred-{i}")));
        let (status, res) = env.post(path, &alice, true, slot.clone()).await;
        assert_eq!(status, StatusCode::OK, "{res}");
        if i == 0 {
            first_slot = slot;
        }
    }
    let (status, res) = env
        .post(path, &alice, true, with_key(passkey_slot("cred-20")))
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_LIMIT_EXCEEDED")
    );
    assert_eq!(res["limit"], "slots");
    // 同じスロットの再送は、再認証なしでも 200（何も変えない）。
    let (status, _) = env.post(path, &alice, false, first_slot.clone()).await;
    assert_eq!(status, StatusCode::OK);
    // 同じ credential ID の置き換えは、20 個に達していても通るが、再認証が要る。
    let replacement = with_key(passkey_slot("cred-0"));
    let (status, res) = env.post(path, &alice, false, replacement.clone()).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    let (status, res) = env.post(path, &alice, true, replacement.clone()).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_secret_storage_key_slots WHERE id = $1",
            &[first_slot["id"].as_str().unwrap()]
        )
        .await,
        0
    );

    // 他人のスロットは消せない（404）。存在しないスロットも 404。削除には再認証が要る。
    let slot_path = format!(
        "/api/e2ee/secret_storage/slots/{}",
        replacement["id"].as_str().unwrap()
    );
    let (status, _) = env.delete(&slot_path, &bob, true).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env
        .delete(
            &format!("/api/e2ee/secret_storage/slots/{}", Uuid::new_v4()),
            &alice,
            true,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, res) = env.delete(&slot_path, &alice, false).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );

    // 最後に使った日時の記録。
    let (status, res) = env
        .post(&format!("{slot_path}/used"), &alice, false, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert!(res["data"]["last_used_at"].is_string());

    // 残り 2 つにしてから、2 つの DELETE を同時に送っても、最後の 1 つが残る。
    let ids: Vec<String> = env
        .query(
            "SELECT to_json(id)::text FROM e2ee_secret_storage_key_slots WHERE user_id = $1",
            &[&alice],
        )
        .await
        .into_iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    for id in &ids[2..] {
        let (status, res) = env
            .delete(
                &format!("/api/e2ee/secret_storage/slots/{id}"),
                &alice,
                true,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{res}");
    }
    let mut handles = Vec::new();
    for id in &ids[..2] {
        let app = env.app.clone();
        let (user, path) = (
            alice.clone(),
            format!("/api/e2ee/secret_storage/slots/{id}"),
        );
        handles.push(tokio::spawn(async move {
            let token = id_token(&user, json!({}));
            send(&app, "DELETE", &path, &user, Some(&token), Value::Null).await
        }));
    }
    let mut statuses = Vec::new();
    for handle in handles {
        let (status, res) = handle.await.unwrap();
        if status == StatusCode::BAD_REQUEST {
            assert_eq!(code(&res), "E2EE_SLOTS_REQUIRED");
        }
        statuses.push(status);
    }
    statuses.sort();
    assert_eq!(statuses, vec![StatusCode::OK, StatusCode::BAD_REQUEST]);
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_secret_storage_key_slots WHERE user_id = $1",
            &[&alice]
        )
        .await,
        1
    );
}

// ---------------------------------------------------------------------------
// 鍵バックアップ

#[tokio::test]
async fn backup_versions_switch_forward_and_reject_old_keys() {
    let env = env().await;
    let alice = env.identity().await;
    let bob = env.identity().await;
    let user = alice.user.clone();
    env.ssss(&user, json!([recovery_slot()])).await;
    let revision = env.revision(&user).await;
    let versions = "/api/e2ee/key_backup/versions";
    let algorithm = "zoo.megolm_backup.v1.curve25519-aes-sha2";

    // マスター鍵の署名がない版は作れない。
    let first_key = backup_key();
    let (status, res) = env
        .post(
            versions,
            &user,
            false,
            json!({"algorithm": algorithm, "auth_data": {"public_key": first_key}}),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_BAD_SIGNATURE")
    );
    // 最初の版は再認証なしで作れる。送り直しは同じ版を返す。
    let first = json!({"algorithm": algorithm, "auth_data": alice.auth_data(&first_key), "expected_current_version": null});
    for _ in 0..2 {
        let (status, res) = env.post(versions, &user, false, first.clone()).await;
        assert_eq!(status, StatusCode::OK, "{res}");
        assert_eq!(res["data"]["version"], "1");
    }
    // 他人の版は読めない。
    let (status, _) = env.get(&format!("{versions}/1"), &bob.user).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env.get(&format!("{versions}/current"), &bob.user).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 2 つ目の版は再認証と expected_current_version が要る。
    let second_key = backup_key();
    let second = json!({"algorithm": algorithm, "auth_data": alice.auth_data(&second_key), "expected_current_version": "1"});
    let (status, res) = env.post(versions, &user, false, second.clone()).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    let mut wrong = second.clone();
    wrong["expected_current_version"] = json!(null);
    let (status, res) = env.post(versions, &user, true, wrong).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_WRONG_BACKUP_VERSION")
    );
    assert_eq!(res["current_version"], "1");
    let (status, res) = env.post(versions, &user, true, second).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["version"], "2");

    // 古い版の auth_data の送り直しは 400 E2EE_KEY_REUSED。
    let mut old = first.clone();
    old["expected_current_version"] = json!("2");
    let (status, res) = env.post(versions, &user, true, old).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_KEY_REUSED")
    );

    // 古い版への書き込みは 409。secrets_revision の省略は 400、違えば 409。
    let sessions = json!({"sessions": [session("c", "k", "s", true, 0)]});
    let upload = |version: &str, rev: &str| {
        format!("/api/e2ee/key_backup/keys?version={version}&secrets_revision={rev}")
    };
    let (status, res) = env
        .post(
            &upload("1", &revision.to_string()),
            &user,
            false,
            sessions.clone(),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_WRONG_BACKUP_VERSION")
    );
    let (status, _) = env
        .post(
            "/api/e2ee/key_backup/keys?version=2",
            &user,
            false,
            sessions.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, res) = env
        .post(
            &upload("2", &(revision + 1).to_string()),
            &user,
            false,
            sessions.clone(),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_SSSS_CONFLICT")
    );
    let (status, res) = env
        .post(
            &upload("2", &revision.to_string()),
            &user,
            false,
            sessions.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    // 他人は自分の版がないので 409。他人の版のセッションも読めない。
    let (status, _) = env
        .post(&upload("2", "1"), &bob.user, false, sessions.clone())
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = env
        .get("/api/e2ee/key_backup/keys?version=2", &bob.user)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 今の版を消すと、今の版はなくなり、古い版（1）には戻らない。
    let (status, res) = env.delete(&format!("{versions}/2"), &user, false).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_REAUTH_REQUIRED")
    );
    let (status, _) = env.delete(&format!("{versions}/2"), &user, true).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = env.get(&format!("{versions}/current"), &user).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env.get(&format!("{versions}/2"), &user).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env.delete(&format!("{versions}/2"), &user, true).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, res) = env
        .post(&upload("1", &revision.to_string()), &user, false, sessions)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{res}");
    // 今の版ではない古い版は、再認証なしで消せる。
    let (status, res) = env.delete(&format!("{versions}/1"), &user, false).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let (_, list) = env.get(versions, &user).await;
    let listed: Vec<(Value, Value)> = list["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["version"].clone(), v["deleted"].clone()))
        .collect();
    assert_eq!(
        listed,
        vec![(json!("1"), json!(true)), (json!("2"), json!(true))]
    );

    // 次の版の番号は消した版を含めた最大 + 1。
    let third = json!({"algorithm": algorithm, "auth_data": alice.auth_data(&backup_key()), "expected_current_version": null});
    let (status, res) = env.post(versions, &user, true, third).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["version"], "3");
}

#[tokio::test]
async fn backup_auth_data_is_updated_after_master_rotation() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    let versions = "/api/e2ee/key_backup/versions";
    let key = backup_key();
    let body = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&key)});
    let (status, res) = env.post(versions, &user, false, body).await;
    assert_eq!(status, StatusCode::OK, "{res}");

    let next = Identity::new(&user);
    let (status, res) = env
        .post(
            "/api/e2ee/cross_signing/keys",
            &user,
            true,
            next.upload_body(alice.public()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    // 今と同じ auth_data の送り直しは、何もせずに今の版を返す。
    let stale = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&key), "expected_current_version": "1"});
    let (status, _) = env.post(versions, &user, false, stale).await;
    assert_eq!(status, StatusCode::OK, "the same auth_data is a no-op");
    let (status, res) = env
        .post(versions, &user, false, json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": next.auth_data(&key), "expected_current_version": "1"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["version"], "1");
    let (_, current) = env.get(&format!("{versions}/current"), &user).await;
    assert_eq!(current["auth_data"], next.auth_data(&key));
    // 今のマスター鍵の署名がない auth_data の更新は 400。
    let forged = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": {"public_key": key, "signatures": {}}});
    let (status, res) = env.post(versions, &user, false, forged).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_BAD_SIGNATURE")
    );
}

#[tokio::test]
async fn backup_sessions_page_past_one_thousand() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    env.ssss(&user, json!([recovery_slot()])).await;
    let revision = env.revision(&user).await;
    let body = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&backup_key())});
    env.post("/api/e2ee/key_backup/versions", &user, false, body)
        .await;
    let channels = ["ch-a", "ch-b"];
    let mut expected = Vec::new();
    for channel in channels {
        let sessions: Vec<Value> = (0..750)
            .map(|i| {
                let id = format!("s{i:04}");
                expected.push(format!("{channel}/{id}"));
                session(channel, "sender", &id, false, 0)
            })
            .collect();
        let (status, res) = env
            .post(
                &format!("/api/e2ee/key_backup/keys?version=1&secrets_revision={revision}"),
                &user,
                false,
                json!({"sessions": sessions}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{res}");
    }
    // 1001 件以上の 1 回のアップロードは 400。
    let too_many: Vec<Value> = (0..1001)
        .map(|i| session("x", "s", &i.to_string(), false, 0))
        .collect();
    let (status, res) = env
        .post(
            &format!("/api/e2ee/key_backup/keys?version=1&secrets_revision={revision}"),
            &user,
            false,
            json!({"sessions": too_many}),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::BAD_REQUEST, "E2EE_LIMIT_EXCEEDED")
    );

    let mut seen = Vec::new();
    let mut pages = 0;
    let mut after = String::new();
    loop {
        let (status, page) = env
            .get(
                &format!("/api/e2ee/key_backup/keys?version=1{after}"),
                &user,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        pages += 1;
        for s in page["sessions"].as_array().unwrap() {
            seen.push(format!(
                "{}/{}",
                s["channel_id"].as_str().unwrap(),
                s["session_id"].as_str().unwrap()
            ));
        }
        match page["next"].as_str() {
            Some(next) => after = format!("&after={next}"),
            None => break,
        }
    }
    assert_eq!(pages, 2);
    expected.sort();
    assert_eq!(seen, expected);
    // channel_id で絞る。
    let (_, page) = env
        .get("/api/e2ee/key_backup/keys?version=1&channel_id=ch-b", &user)
        .await;
    assert_eq!(page["sessions"].as_array().unwrap().len(), 750);
    assert!(page["next"].is_null());
}

#[tokio::test]
async fn backup_totals_are_counted_and_capped_per_version_and_user() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    env.ssss(&user, json!([recovery_slot()])).await;
    let revision = env.revision(&user).await;
    let body = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&backup_key())});
    let (status, res) = env
        .post("/api/e2ee/key_backup/versions", &user, false, body)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let upload = |version: &str| {
        format!("/api/e2ee/key_backup/keys?version={version}&secrets_revision={revision}")
    };
    let totals = |version: &'static str| {
        let (env, user) = (&env, user.clone());
        async move {
            let row = env
                .query(
                    "SELECT json_build_object('count', session_count, 'bytes', session_bytes,
                       'real_count', (SELECT count(*) FROM e2ee_key_backup_sessions s
                                      WHERE s.user_id = v.user_id AND s.version = v.version),
                       'real_bytes', (SELECT coalesce(sum(octet_length(session_data::text)), 0)
                                      FROM e2ee_key_backup_sessions s
                                      WHERE s.user_id = v.user_id AND s.version = v.version))::text
                     FROM e2ee_key_backup_versions v WHERE user_id = $1 AND version = $2::bigint",
                    &[&user, version],
                )
                .await;
            let row = row[0].clone();
            // トリガーが保つ数は、実際の行の数と大きさと一致する。
            assert_eq!(row["count"], row["real_count"], "{row}");
            assert_eq!(row["bytes"], row["real_bytes"], "{row}");
            (
                row["count"].as_i64().unwrap(),
                row["bytes"].as_i64().unwrap(),
            )
        }
    };

    // 足す、よい行で置き換える、同じ要求で重複する、のどれでも数がずれない。
    let sessions: Vec<Value> = (0..5)
        .map(|i| session("c", "k", &format!("s{i}"), false, 3))
        .collect();
    let (status, res) = env
        .post(&upload("1"), &user, false, json!({"sessions": sessions}))
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["count"], 5);
    assert_eq!(totals("1").await.0, 5);
    let mut bigger = session("c", "k", "s0", true, 0);
    bigger["session_data"]["ciphertext"] = json!(random_b64(600));
    let (status, res) = env
        .post(
            &upload("1"),
            &user,
            false,
            json!({"sessions": [bigger, session("c", "k", "s5", false, 0), session("c", "k", "s1", false, 9)]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["data"]["count"], 6);
    let (count, bytes) = totals("1").await;
    assert_eq!(count, 6);
    let (_, current) = env
        .get("/api/e2ee/key_backup/versions/current", &user)
        .await;
    assert_eq!(current["count"], 6);

    // 版の上限：数を増やすアップロードは 400 で、行も数も変わらない。
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_count = session_count + $2::bigint
         WHERE user_id = $1 AND version = 1",
        &[
            &user,
            &(backend::routes::e2ee_key_backup::MAX_BACKUP_VERSION_SESSIONS - count).to_string(),
        ],
    )
    .await;
    let (status, res) = env
        .post(
            &upload("1"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "over", false, 0)]}),
        )
        .await;
    assert_eq!(
        (status, code(&res), res["limit"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            "E2EE_LIMIT_EXCEEDED",
            Some("backup_version_sessions")
        )
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_backup_sessions WHERE user_id = $1 AND session_id = 'over'",
            &[&user]
        )
        .await,
        0
    );
    // 上限に達していても、数を増やさないアップロード（同じ行の再送、よい行での置き換え）は通る。
    let (status, res) = env
        .post(
            &upload("1"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "s2", false, 0)]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_count = $2::bigint
         WHERE user_id = $1 AND version = 1",
        &[&user, &count.to_string()],
    )
    .await;

    // 版の大きさの上限。
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_bytes = $2::bigint
         WHERE user_id = $1 AND version = 1",
        &[
            &user,
            &backend::routes::e2ee_key_backup::MAX_BACKUP_VERSION_BYTES.to_string(),
        ],
    )
    .await;
    let (status, res) = env
        .post(
            &upload("1"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "big", false, 0)]}),
        )
        .await;
    assert_eq!(
        (status, code(&res), res["limit"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            "E2EE_LIMIT_EXCEEDED",
            Some("backup_version_size")
        )
    );
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_bytes = $2::bigint
         WHERE user_id = $1 AND version = 1",
        &[&user, &bytes.to_string()],
    )
    .await;
    assert_eq!(totals("1").await, (count, bytes));

    // ユーザーの上限は、消していないすべての版の合計で数える。版 1 を古い版として残したまま、
    // 版 2 に上げる（失効のあとの移行と同じ形）。
    let create = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&backup_key()), "expected_current_version": "1"});
    let (status, res) = env
        .post("/api/e2ee/key_backup/versions", &user, true, create)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let (status, res) = env
        .post(
            &upload("2"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "s0", false, 0)]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_count = $2::bigint
         WHERE user_id = $1 AND version = 1",
        &[
            &user,
            &backend::routes::e2ee_key_backup::MAX_BACKUP_USER_SESSIONS.to_string(),
        ],
    )
    .await;
    let (status, res) = env
        .post(
            &upload("2"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "s1", false, 0)]}),
        )
        .await;
    assert_eq!(
        (status, code(&res), res["limit"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            "E2EE_LIMIT_EXCEEDED",
            Some("backup_user_sessions")
        )
    );
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_count = $2::bigint, session_bytes = $3::bigint
         WHERE user_id = $1 AND version = 1",
        &[
            &user,
            &count.to_string(),
            &backend::routes::e2ee_key_backup::MAX_BACKUP_USER_BYTES.to_string(),
        ],
    )
    .await;
    let (status, res) = env
        .post(
            &upload("2"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "s1", false, 0)]}),
        )
        .await;
    assert_eq!(
        (status, code(&res), res["limit"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            "E2EE_LIMIT_EXCEEDED",
            Some("backup_user_size")
        )
    );
    // 古い版を消すと、その版は合計に入らない（行も数も消える）。
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_bytes = $2::bigint
         WHERE user_id = $1 AND version = 1",
        &[&user, &bytes.to_string()],
    )
    .await;
    let (status, res) = env
        .delete("/api/e2ee/key_backup/versions/1", &user, false)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(totals("1").await, (0, 0));
    let (status, res) = env
        .post(
            &upload("2"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "s1", false, 0)]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(totals("2").await.0, 2);
}

#[tokio::test]
async fn a_concurrent_shrink_does_not_hide_an_increase_past_the_cap() {
    // レビューの再現の順：上限を超えた版で、A（新しい行）が「書く前」を読むのと、B（縮小）のコミットが
    // 入れ違う。B は版の行をロックしたまま縮小し、A が待っているあいだにコミットする。A は版の行のロックの
    // あとに「書く前」を読むので、縮小のあとの数と比べて上限を超える増加を拒否する。
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    env.ssss(&user, json!([recovery_slot()])).await;
    let revision = env.revision(&user).await;
    let body = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&backup_key())});
    let (status, res) = env
        .post("/api/e2ee/key_backup/versions", &user, false, body)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let max = backend::routes::e2ee_key_backup::MAX_BACKUP_VERSION_BYTES;
    env.exec(
        "UPDATE e2ee_key_backup_versions SET session_bytes = $2::bigint
         WHERE user_id = $1 AND version = 1",
        &[&user, &(max + 1024).to_string()],
    )
    .await;

    // B：版の行をロックして 2048 バイト縮める（コミットはまだ）。
    let mut db = env.db.clone();
    let mut shrink = db.transaction().await.unwrap();
    backend::apps::query(
        &mut shrink,
        "UPDATE e2ee_key_backup_versions SET session_bytes = session_bytes - 2048
         WHERE user_id = $1 AND version = 1 RETURNING '{}'::text",
        &[&user],
    )
    .await
    .unwrap();

    // A：1.5 KiB ほどの新しい行。縮小のあとでも上限を超える。
    let mut big = session("c", "k", "grow", false, 0);
    big["session_data"]["ciphertext"] = json!("a".repeat(1500));
    let app = env.app.clone();
    let (upload_user, path) = (
        user.clone(),
        format!("/api/e2ee/key_backup/keys?version=1&secrets_revision={revision}"),
    );
    let upload = tokio::spawn(async move {
        send(
            &app,
            "POST",
            &path,
            &upload_user,
            None,
            json!({"sessions": [big]}),
        )
        .await
    });
    // A が版の行のロックで待つまで進める（先に B がコミットしても、A は縮小のあとの数を読むので結果は同じ）。
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    shrink.commit().await.unwrap();

    let (status, res) = upload.await.unwrap();
    assert_eq!(
        (status, code(&res), res["limit"].as_str()),
        (
            StatusCode::BAD_REQUEST,
            "E2EE_LIMIT_EXCEEDED",
            Some("backup_version_size")
        ),
        "{res}"
    );
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_backup_sessions WHERE user_id = $1 AND session_id = 'grow'",
            &[&user]
        )
        .await,
        0
    );
}

/// `X-Zoo-Backup-Proof` を付けたアップロード（`proof` が `None` なら付けない）。
async fn upload_with_proof(
    env: &Env,
    user: &str,
    path: &str,
    proof: Option<&str>,
    sessions: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("X-Dev-User", user);
    if let Some(proof) = proof {
        builder = builder.header("X-Zoo-Backup-Proof", proof);
    }
    let response = env
        .app
        .clone()
        .oneshot(
            builder
                .body(Body::from(json!({"sessions": sessions}).to_string()))
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

#[tokio::test]
async fn backup_upload_proofs_block_preemption_and_overwrites() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    env.ssss(&user, json!([recovery_slot()])).await;
    let revision = env.revision(&user).await;
    let backup = zoo_crypto::BackupDecryptionKey::generate();
    let body = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2",
                      "auth_data": alice.auth_data(&backup.public_key().to_base64())});
    let (status, res) = env
        .post("/api/e2ee/key_backup/versions", &user, false, body)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let path = |version: &str| {
        format!("/api/e2ee/key_backup/keys?version={version}&secrets_revision={revision}")
    };

    // 版の応答は、サーバーの公開鍵（upload_key）を返す。秘密鍵は返さず、版の一覧にも出さない。
    let (_, current) = env
        .get("/api/e2ee/key_backup/versions/current", &user)
        .await;
    let upload_key = current["upload_key"].as_str().unwrap().to_owned();
    let (_, same) = env.get("/api/e2ee/key_backup/versions/1", &user).await;
    assert_eq!(same["upload_key"], upload_key.as_str());
    let (_, list) = env.get("/api/e2ee/key_backup/versions", &user).await;
    assert!(list["versions"][0].get("upload_key").is_none(), "{list}");
    let stored = env
        .query(
            "SELECT to_json(upload_key)::text FROM e2ee_key_backup_versions WHERE user_id = $1",
            &[&user],
        )
        .await;
    assert_ne!(stored[0], json!(upload_key));
    let server_key = zoo_crypto::Curve25519PublicKey::from_base64(&upload_key).unwrap();
    let proof = backup.upload_proof(&server_key, &user, "1").unwrap();
    let row = |id: &'static str| {
        let (env, user) = (&env, user.clone());
        async move {
            env.query(
                "SELECT json_build_object('data', session_data, 'proven', proven, 'v', is_verified,
                   'i', first_message_index)::text
                 FROM e2ee_key_backup_sessions WHERE user_id = $1 AND session_id = $2",
                &[&user, id],
            )
            .await[0]
                .clone()
        }
    };

    // 先取り：アクセストークンだけを持つ者が、いちばんよい順位の偽の行を先に入れても、
    // 証明付きの正規の行（順位が低くても）が置き換える。
    let forged = session("c", "k", "s1", true, 0);
    let (status, res) =
        upload_with_proof(&env, &user, &path("1"), None, json!([forged.clone()])).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let legit = session("c", "k", "s1", false, 4);
    let (status, res) = upload_with_proof(
        &env,
        &user,
        &path("1"),
        Some(&proof),
        json!([legit.clone()]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let saved = row("s1").await;
    assert_eq!(saved["data"], legit["session_data"]);
    assert_eq!(saved["proven"], true);

    // 上書き：証明のない要求は、証明付きの行をどの順位でも置き換えられない（200 だが何も変えない）。
    let (status, res) =
        upload_with_proof(&env, &user, &path("1"), None, json!([forged.clone()])).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(row("s1").await, saved);

    // 証明付きの行どうしは、今までどおりの順位で決める。
    let worse = session("c", "k", "s1", false, 9);
    upload_with_proof(&env, &user, &path("1"), Some(&proof), json!([worse])).await;
    assert_eq!(row("s1").await, saved);
    let better = session("c", "k", "s1", true, 0);
    upload_with_proof(
        &env,
        &user,
        &path("1"),
        Some(&proof),
        json!([better.clone()]),
    )
    .await;
    assert_eq!(row("s1").await["data"], better["session_data"]);

    // 証明を送らない古いクライアントの行は、今までどおりの規則で残す。
    upload_with_proof(
        &env,
        &user,
        &path("1"),
        None,
        json!([session("c", "k", "s2", false, 3)]),
    )
    .await;
    let good = session("c", "k", "s2", false, 0);
    upload_with_proof(&env, &user, &path("1"), None, json!([good.clone()])).await;
    assert_eq!(row("s2").await["data"], good["session_data"]);
    assert_eq!(row("s2").await["proven"], false);

    // 正しくない証明は 403 で、何も書かない。別のバックアップ鍵、別のユーザー、別の版、形の崩れた値。
    let other = zoo_crypto::BackupDecryptionKey::generate();
    let bob = env.user().await;
    for bad in [
        other.upload_proof(&server_key, &user, "1").unwrap(),
        backup.upload_proof(&server_key, &bob, "1").unwrap(),
        backup.upload_proof(&server_key, &user, "2").unwrap(),
        "!!".to_owned(),
        String::new(),
    ] {
        let (status, res) = upload_with_proof(
            &env,
            &user,
            &path("1"),
            Some(&bad),
            json!([session("c", "k", "bad", true, 0)]),
        )
        .await;
        assert_eq!(
            (status, code(&res)),
            (StatusCode::FORBIDDEN, "E2EE_BACKUP_PROOF_INVALID"),
            "{bad}"
        );
    }
    assert_eq!(
        env.count(
            "SELECT 1 FROM e2ee_key_backup_sessions WHERE user_id = $1 AND session_id = 'bad'",
            &[&user]
        )
        .await,
        0
    );

    // 版ごとに鍵が違うので、前の版の証明は新しい版に使えない。
    let next = zoo_crypto::BackupDecryptionKey::generate();
    let create = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2",
                        "auth_data": alice.auth_data(&next.public_key().to_base64()),
                        "expected_current_version": "1"});
    let (status, res) = env
        .post("/api/e2ee/key_backup/versions", &user, true, create)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let (_, current) = env
        .get("/api/e2ee/key_backup/versions/current", &user)
        .await;
    let next_key = current["upload_key"].as_str().unwrap().to_owned();
    assert_ne!(next_key, upload_key);
    let (status, res) = upload_with_proof(
        &env,
        &user,
        &path("2"),
        Some(&proof),
        json!([session("c", "k", "s1", false, 0)]),
    )
    .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::FORBIDDEN, "E2EE_BACKUP_PROOF_INVALID")
    );
    let next_proof = next
        .upload_proof(
            &zoo_crypto::Curve25519PublicKey::from_base64(&next_key).unwrap(),
            &user,
            "2",
        )
        .unwrap();
    let (status, res) = upload_with_proof(
        &env,
        &user,
        &path("2"),
        Some(&next_proof),
        json!([session("c", "k", "s1", false, 0)]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");

    // 鍵のない版（この列より前に作った版）は、初めて要るときに鍵を作り、そのあとは変えない。
    env.exec(
        "UPDATE e2ee_key_backup_versions SET upload_key = NULL WHERE user_id = $1 AND version = 2",
        &[&user],
    )
    .await;
    let (_, first_read) = env
        .get("/api/e2ee/key_backup/versions/current", &user)
        .await;
    let (_, second_read) = env
        .get("/api/e2ee/key_backup/versions/current", &user)
        .await;
    assert!(first_read["upload_key"].is_string());
    assert_eq!(first_read["upload_key"], second_read["upload_key"]);
    assert_ne!(first_read["upload_key"], json!(next_key));
}

#[tokio::test]
async fn concurrent_uploads_keep_the_best_row_and_respect_version_switches() {
    let env = env().await;
    let alice = env.identity().await;
    let user = alice.user.clone();
    env.ssss(&user, json!([recovery_slot()])).await;
    let revision = env.revision(&user).await;
    let body = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&backup_key())});
    env.post("/api/e2ee/key_backup/versions", &user, false, body)
        .await;

    // 同じセッションを、よさの違う 12 通りで同時に上げる。いちばんよい行（検証済み、index 0）が残る。
    let mut handles = Vec::new();
    for i in 0..12 {
        let verified = i == 7;
        let index = if verified { 0 } else { 1 + i % 3 };
        let body = json!({"sessions": [session("c", "k", "same", verified, index)]});
        let app = env.app.clone();
        let (user, path) = (
            user.clone(),
            format!("/api/e2ee/key_backup/keys?version=1&secrets_revision={revision}"),
        );
        handles.push(tokio::spawn(async move {
            send(&app, "POST", &path, &user, None, body).await
        }));
    }
    for handle in handles {
        let (status, res) = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{res}");
    }
    let row = env
        .query(
            "SELECT json_build_object('v', is_verified, 'i', first_message_index)::text FROM e2ee_key_backup_sessions WHERE user_id = $1",
            &[&user],
        )
        .await;
    assert_eq!(row, vec![json!({"v": true, "i": 0})]);

    // 版の作成と同時のアップロード：作成が確定したあとの古い版へのアップロードは 409 で、500 にならない。
    let mut handles = Vec::new();
    for i in 0..8 {
        let app = env.app.clone();
        let (user, path) = (
            user.clone(),
            format!("/api/e2ee/key_backup/keys?version=1&secrets_revision={revision}"),
        );
        let body = json!({"sessions": [session("c", "k", &format!("race{i}"), false, 0)]});
        handles.push(tokio::spawn(async move {
            send(&app, "POST", &path, &user, None, body).await.0
        }));
    }
    let create = json!({"algorithm": "zoo.megolm_backup.v1.curve25519-aes-sha2", "auth_data": alice.auth_data(&backup_key()), "expected_current_version": "1"});
    let (status, res) = env
        .post("/api/e2ee/key_backup/versions", &user, true, create)
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    for handle in handles {
        let status = handle.await.unwrap();
        assert!(
            status == StatusCode::OK || status == StatusCode::CONFLICT,
            "{status}"
        );
    }
    let (status, res) = env
        .post(
            &format!("/api/e2ee/key_backup/keys?version=1&secrets_revision={revision}"),
            &user,
            false,
            json!({"sessions": [session("c", "k", "late", false, 0)]}),
        )
        .await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_WRONG_BACKUP_VERSION")
    );
    assert_eq!(res["current_version"], "2");
}

// ---------------------------------------------------------------------------
// Shape

async fn shape_rows(env: &Env, table: &str, actor: &str) -> Vec<Value> {
    let response = env
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/shapes/{table}?offset=-1"))
                .header("X-Dev-User", actor)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "private, no-cache");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    body.as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m.get("value").cloned())
        .collect()
}

#[tokio::test]
async fn cross_signing_shapes_hide_user_signing_keys_from_others() {
    let env = env().await;
    let alice = env.identity().await;
    let bob = env.identity().await;
    let carol = env.identity().await;
    env.encrypted_channel(&[&alice.user, &bob.user]).await;
    // alice が bob のマスター鍵に署名する（ユーザー署名鍵の署名は alice にだけ配る）。
    let bob_master = cs_key(&bob.user, "master", &bob.master, None);
    let (status, res) = env
        .post(
            "/api/e2ee/cross_signing/signatures",
            &alice.user,
            true,
            json!({"signatures": [{
                "target_user_id": bob.user, "target_key_id": pk(&bob.master),
                "signer_key_id": format!("ed25519:{}", pk(&alice.usk)),
                "signature": signature_of(&bob_master, &alice.usk),
            }]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");

    let keys: Vec<(String, String)> = shape_rows(&env, "e2ee_cross_signing_keys", &bob.user)
        .await
        .iter()
        .map(|r| {
            (
                r["user_id"].as_str().unwrap().to_owned(),
                r["usage"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(keys.contains(&(alice.user.clone(), "master".into())));
    assert!(keys.contains(&(alice.user.clone(), "self_signing".into())));
    assert!(!keys.contains(&(alice.user.clone(), "user_signing".into())));
    assert!(keys.contains(&(bob.user.clone(), "user_signing".into())));
    assert!(!keys.iter().any(|(u, _)| *u == carol.user));

    let signatures = shape_rows(&env, "e2ee_key_signatures", &bob.user).await;
    assert!(
        signatures
            .iter()
            .all(|s| s["signer_user_id"] != json!(alice.user))
    );
    let own = shape_rows(&env, "e2ee_key_signatures", &alice.user).await;
    assert!(own.iter().any(|s| s["target_user_id"] == json!(bob.user)));
}

// ---------------------------------------------------------------------------
// membership_epoch（#357）

async fn epoch(env: &Env, channel: &str) -> i64 {
    env.query(
        "SELECT membership_epoch::text FROM channels WHERE id = $1",
        &[channel],
    )
    .await[0]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn key_changes_and_backup_versions_bump_membership_epochs() {
    let env = env().await;
    let alice = env.identity().await;
    let bob = env.user().await;
    let user = alice.user.clone();
    let shared = env.encrypted_channel(&[&user, &bob]).await;
    let without_alice = env.encrypted_channel(&[&bob]).await;
    let path = "/api/e2ee/cross_signing/keys";

    // 自己署名鍵を変えると 1 増える。同じ内容の再送では増えない。
    let new_ssk = Ed25519SecretKey::new();
    let body = json!({
        "self_signing_key": cs_key(&user, "self_signing", &new_ssk, Some(&alice.master)),
        "expected_keys": alice.public(),
    });
    for _ in 0..2 {
        let (status, res) = env.post(path, &user, true, body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{res}");
        assert_eq!(epoch(&env, &shared).await, 1);
    }
    // マスター鍵の差し替えでも増える。
    let new_master = Ed25519SecretKey::new();
    let new_usk = Ed25519SecretKey::new();
    let body = json!({
        "master_key": cs_key(&user, "master", &new_master, None),
        "self_signing_key": cs_key(&user, "self_signing", &new_ssk, Some(&new_master)),
        "user_signing_key": cs_key(&user, "user_signing", &new_usk, Some(&new_master)),
        "expected_keys": {"master": pk(&alice.master), "self_signing": pk(&new_ssk), "user_signing": pk(&alice.usk)},
    });
    let (status, res) = env.post(path, &user, true, body).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(epoch(&env, &shared).await, 2);

    // 鍵バックアップ：最初の版では増えず、2 つ目の版で増える。
    let rotated = Identity {
        user: user.clone(),
        master: new_master,
        ssk: new_ssk,
        usk: new_usk,
    };
    env.ssss(&user, json!([recovery_slot()])).await;
    let versions = "/api/e2ee/key_backup/versions";
    let algorithm = "zoo.megolm_backup.v1.curve25519-aes-sha2";
    let first = json!({"algorithm": algorithm, "auth_data": rotated.auth_data(&backup_key()), "expected_current_version": null});
    let (status, res) = env.post(versions, &user, false, first).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(epoch(&env, &shared).await, 2);
    let second = json!({"algorithm": algorithm, "auth_data": rotated.auth_data(&backup_key()), "expected_current_version": "1"});
    let (status, res) = env.post(versions, &user, true, second).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(epoch(&env, &shared).await, 3);
    assert!(res["txid"].is_string());

    assert_eq!(epoch(&env, &without_alice).await, 0);
}

/// 初回設定とリセットの保存（#360）：クロス署名鍵と版の保存は、前提にした既定の SSSS 鍵
/// （`expected_default_key_id`）を同じトランザクションで照合する。別の端末が SSSS を作り直していたら、
/// `expected_keys` が今の鍵と同じでも 409 `E2EE_SSSS_CONFLICT` にし、何も変えない。
#[tokio::test]
async fn cross_signing_and_versions_check_the_expected_default_ssss_key() {
    let env = env().await;
    let user = env.user().await;
    let keys = "/api/e2ee/cross_signing/keys";
    let versions = "/api/e2ee/key_backup/versions";
    let algorithm = "zoo.megolm_backup.v1.curve25519-aes-sha2";

    // 端末 A が SSSS を保存した。
    let key_a = env.ssss(&user, json!([recovery_slot()])).await;
    // その間に端末 B が SSSS を作り直し、クロス署名鍵も保存した。
    let revision = env.revision(&user).await;
    let key_b = Uuid::new_v4().to_string();
    let (status, res) = env
        .post(
            "/api/e2ee/secret_storage/keys",
            &user,
            true,
            ssss_body(
                &key_b,
                json!(key_a),
                json!(revision),
                json!([recovery_slot()]),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    let b = Identity::new(&user);
    let mut body = b.upload_body(Value::Null);
    body["expected_default_key_id"] = json!(key_b);
    let (status, res) = env.post(keys, &user, true, body).await;
    assert_eq!(status, StatusCode::OK, "{res}");

    // 端末 A が、今の B の鍵を expected_keys にして差し替えようとしても、SSSS が A の鍵でないので 409。
    let a = Identity::new(&user);
    let mut body = a.upload_body(b.public());
    body["expected_default_key_id"] = json!(key_a);
    let (status, res) = env.post(keys, &user, true, body).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_SSSS_CONFLICT")
    );
    assert_eq!(res["default_key_id"], json!(key_b));
    let (_, own) = env
        .post(
            "/api/e2ee/keys/query",
            &user,
            false,
            json!({"user_ids": [user]}),
        )
        .await;
    assert_eq!(
        own["master_keys"][&user]["keys"][format!("ed25519:{}", pk(&b.master))],
        json!(pk(&b.master))
    );

    // 版も同じ。A の前提では作れず、B の前提なら作れる。
    let mut first = json!({"algorithm": algorithm, "auth_data": b.auth_data(&backup_key()), "expected_current_version": null});
    first["expected_default_key_id"] = json!(key_a);
    let (status, res) = env.post(versions, &user, false, first.clone()).await;
    assert_eq!(
        (status, code(&res)),
        (StatusCode::CONFLICT, "E2EE_SSSS_CONFLICT")
    );
    first["expected_default_key_id"] = json!(key_b);
    let (status, res) = env.post(versions, &user, false, first).await;
    assert_eq!(status, StatusCode::OK, "{res}");

    // 省略したら照合しない（これまでの要求のまま）。
    let mut body = b.upload_body(Value::Null);
    body.as_object_mut().unwrap().remove("expected_keys");
    let (status, res) = env.post(keys, &user, false, body).await;
    assert_eq!(status, StatusCode::OK, "{res}");
}
