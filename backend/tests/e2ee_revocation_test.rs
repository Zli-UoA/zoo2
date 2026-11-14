//! E2EE の失効の手順と、他のユーザーの検証状態（#362）の結合テスト。仕様は docs/e2ee.md の
//! 「失効の手順」と「クロス署名」。
//!
//! 各端末は `zoo-crypto`（クライアントと同じ実装）で鍵、記録、配布先の判定を持ち、本物のサーバーの
//! API に要求を送る。再認証（`X-Zoo-Reauth`）は、テストの鍵で署名した ID トークンで通す
//! （`e2ee_keys_test.rs` と同じ）。

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zoo_crypto::{
    BackedUpSessionData, BackupEncryptionKey, BackupRow, BackupVersionInfo, CrossSigningKeyPair,
    CrossSigningKeys, DeviceAccount, Distribution, KeySignature, KeySlot, KeySnapshot, KeyUsage,
    OwnKeys, OwnSecrets, RecoveryKey, ReverificationReason, Revocation, RevocationInput,
    RevocationScope, RevocationStage, SecretStorage, SecretStorageKeysRequest, SenderClaimedKeys,
    ServerKeyState, SsssKey, TrustBasis, TrustStore, UnlockedSecretStorage, UserState, plan_resume,
};

const ISSUER: &str = "https://idp.e2ee.test";
const WEB_CLIENT: &str = "zoo-web";
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
        reauth_audiences: vec![WEB_CLIENT.into()],
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

/// テストの IdP が発行した、直前に認証した ID トークン。
fn reauth_token(sub: &str) -> String {
    let now = jiff::Timestamp::now().as_second();
    let claims = json!({
        "iss": ISSUER, "aud": WEB_CLIENT, "sub": sub,
        "exp": now + 300, "iat": now, "authenticated_at": now - 10,
    });
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(KID.into());
    header.typ = Some("JWT".into());
    encode(
        &header,
        &claims,
        &EncodingKey::from_ed_pem(ED25519_PEM.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn code(body: &Value) -> &str {
    body["code"].as_str().unwrap_or_default()
}

impl Env {
    async fn send(
        &self,
        method: &str,
        path: &str,
        actor: &str,
        reauth: bool,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("X-Dev-User", actor);
        if reauth {
            builder = builder.header("X-Zoo-Reauth", reauth_token(actor));
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
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

    /// 成功を確かめて、応答（`data` があればその中身）を返す。
    async fn ok(&self, method: &str, path: &str, actor: &str, reauth: bool, body: Value) -> Value {
        let (status, res) = self.send(method, path, actor, reauth, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {res}");
        match res.get("data") {
            Some(data) => data.clone(),
            None => res,
        }
    }

    async fn exec(&self, sql: &str, params: &[&str]) {
        let mut statement = toasty::sql::statement(sql);
        for p in params {
            statement = statement.bind(*p);
        }
        let mut db = self.db.clone();
        statement.exec(&mut db).await.unwrap();
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, _) = self
            .send(
                "POST",
                "/api/users",
                &id,
                false,
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

    async fn keys_query(&self, actor: &str, users: &[&str]) -> Value {
        self.ok(
            "POST",
            "/api/e2ee/keys/query",
            actor,
            false,
            json!({ "user_ids": users }),
        )
        .await
    }

    async fn versions(&self, actor: &str) -> Value {
        self.ok(
            "GET",
            "/api/e2ee/key_backup/versions",
            actor,
            false,
            Value::Null,
        )
        .await
    }

    async fn server_state(&self, actor: &str) -> ServerKeyState {
        let keys = self.keys_query(actor, &[actor]).await;
        let versions = self.versions(actor).await;
        ServerKeyState::from_json(actor, &keys, &versions).unwrap()
    }

    async fn secret_storage(&self, actor: &str) -> Value {
        self.ok("GET", "/api/e2ee/secret_storage", actor, false, Value::Null)
            .await
    }

    async fn all_pins(&self, actor: &str) -> Vec<Value> {
        let mut pins = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let path = match &after {
                Some(after) => format!("/api/e2ee/tofu_pins?after={after}"),
                None => "/api/e2ee/tofu_pins".to_owned(),
            };
            let page = self.ok("GET", &path, actor, false, Value::Null).await;
            pins.extend(page["pins"].as_array().unwrap().iter().cloned());
            match page["next"].as_str() {
                Some(next) => after = Some(next.to_owned()),
                None => return pins,
            }
        }
    }

    async fn backup_rows(&self, actor: &str, version: &str) -> Vec<BackupRow> {
        let mut rows = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut path = format!("/api/e2ee/key_backup/keys?version={version}");
            if let Some(after) = &after {
                path.push_str(&format!("&after={after}"));
            }
            let page = self.ok("GET", &path, actor, false, Value::Null).await;
            rows.extend(
                serde_json::from_value::<Vec<BackupRow>>(page["sessions"].clone()).unwrap(),
            );
            match page["next"].as_str() {
                Some(next) => after = Some(next.to_owned()),
                None => return rows,
            }
        }
    }
}

/// 1 人の利用者（リカバリーキーを控えている）。
struct Account {
    id: String,
    recovery: String,
}

/// 1 台の端末。暗号ストアの中身（端末アカウント、配布先の判定の記録、自分の鍵）を持つ。
struct Device {
    user: String,
    account: DeviceAccount,
    trust: TrustStore,
    own: Option<OwnKeys>,
}

impl Device {
    fn id(&self) -> String {
        self.account.device_id().to_owned()
    }

    async fn register(env: &Env, user: &str) -> Self {
        let account = DeviceAccount::new(user, &Uuid::new_v4().to_string()).unwrap();
        env.ok(
            "POST",
            "/api/e2ee/devices",
            user,
            false,
            json!({"device_keys": account.device_keys(), "display_name": "test"}),
        )
        .await;
        Self {
            user: user.to_owned(),
            account,
            trust: TrustStore::new(user).unwrap(),
            own: None,
        }
    }

    fn user_signing(&self) -> Option<&CrossSigningKeyPair> {
        self.own.as_ref().map(|own| &own.user_signing)
    }

    /// SSSS から取り出した鍵を暗号ストアに置く（解錠の最後）。
    fn save_own_keys(&mut self, secrets: &OwnSecrets) {
        self.trust
            .trust_own_master_key(&secrets.cross_signing.master)
            .unwrap();
        self.trust
            .record_own_user_signing_key(&secrets.cross_signing.user_signing)
            .unwrap();
        self.own = Some(secrets.own_keys());
    }

    /// リカバリーキーで解錠する。続きを保存し、今の端末に署名する。失効が途中で止まっていれば
    /// （サーバーの鍵が SSSS と違えば）`Err`。
    async fn unlock(
        &mut self,
        env: &Env,
        account: &Account,
    ) -> Result<(), zoo_crypto::CryptoError> {
        let unlocked = unlock_ssss(env, account).await;
        let secrets = OwnSecrets::from_unlocked(&unlocked).unwrap();
        let plan = plan_resume(
            &secrets,
            &env.server_state(&self.user).await,
            &self.account.device_keys(),
        )?;
        if let Some(body) = plan.cross_signing {
            env.ok(
                "POST",
                "/api/e2ee/cross_signing/keys",
                &self.user,
                true,
                body,
            )
            .await;
        }
        if let Some(signature) = plan.device_signature {
            env.ok(
                "POST",
                "/api/e2ee/cross_signing/signatures",
                &self.user,
                false,
                json!({ "signatures": [signature] }),
            )
            .await;
        }
        if let Some(body) = plan.backup_version {
            env.ok(
                "POST",
                "/api/e2ee/key_backup/versions",
                &self.user,
                false,
                body,
            )
            .await;
        }
        self.save_own_keys(&secrets);
        Ok(())
    }

    /// 配布先を判定する（送信の直前と同じ）。初めて見た相手のピンはサーバーに置く。
    async fn evaluate(&mut self, env: &Env, users: &[&str]) -> Distribution {
        let mut all: Vec<&str> = users.to_vec();
        all.push(&self.user);
        for _ in 0..3 {
            let mut response = env.keys_query(&self.user, &all).await;
            response["user_ids"] = json!(all);
            let snapshot = KeySnapshot::from_json(&response).unwrap();
            let own_usk = self.own.as_ref().map(|own| own.user_signing.seed());
            let usk = own_usk.map(|seed| {
                CrossSigningKeyPair::from_seed(&self.user, KeyUsage::UserSigning, &seed).unwrap()
            });
            let result = self.trust.evaluate(
                &snapshot,
                &self.account.device_id().to_owned(),
                usk.as_ref(),
            );
            if result.new_pins.is_empty() {
                return result;
            }
            for pin in &result.new_pins {
                let mut body = pin.pin.clone();
                for field in ["type", "user_id"] {
                    body.as_object_mut().unwrap().remove(field);
                }
                let (status, res) = env
                    .send(
                        "POST",
                        "/api/e2ee/tofu_pins",
                        &self.user,
                        false,
                        json!({ "pins": [body], "replace": false }),
                    )
                    .await;
                match status {
                    StatusCode::OK => self.trust.confirm_tofu_pin(pin),
                    StatusCode::CONFLICT => {
                        assert_eq!(code(&res), "E2EE_PIN_CONFLICT");
                        self.trust.discard_tofu_pin(pin);
                    }
                    other => panic!("unexpected {other}: {res}"),
                }
            }
        }
        panic!("ピンの登録が終わらない");
    }

    /// 利用者が相手の今のマスター鍵を受け入れる（再検証）。直前の再認証が要る。
    async fn reverify(&mut self, env: &Env, target: &str) {
        let response = env.keys_query(&self.user, &[target]).await;
        let master = response["master_keys"][target].clone();
        let usk = self.user_signing().unwrap();
        let usk =
            CrossSigningKeyPair::from_seed(&self.user, KeyUsage::UserSigning, &usk.seed()).unwrap();
        let accepted = self.trust.accept_master_key(&usk, target, &master).unwrap();
        env.ok(
            "POST",
            "/api/e2ee/tofu_pins",
            &self.user,
            true,
            json!({ "pins": [pin_body(&accepted.pin)], "replace": true }),
        )
        .await;
        env.ok(
            "POST",
            "/api/e2ee/cross_signing/signatures",
            &self.user,
            true,
            json!({ "signatures": [accepted.signature] }),
        )
        .await;
    }
}

/// `POST /api/e2ee/tofu_pins` の 1 件（`type` と `user_id` はサーバーが付ける）。
fn pin_body(pin: &Value) -> Value {
    json!({
        "target_user_id": pin["target_user_id"],
        "master_key": pin["master_key"],
        "signatures": pin["signatures"],
    })
}

async fn unlock_ssss(env: &Env, account: &Account) -> UnlockedSecretStorage {
    let response = env.secret_storage(&account.id).await;
    SecretStorage::from_json(&account.id, &response)
        .unwrap()
        .unwrap()
        .unlock_with_recovery_key(&RecoveryKey::parse(&account.recovery).unwrap())
        .unwrap()
}

/// 初回設定を済ませた利用者と、その最初の端末。
async fn setup(env: &Env) -> (Account, Device) {
    let id = env.user().await;
    let secrets = OwnSecrets {
        cross_signing: CrossSigningKeys::generate(&id).unwrap(),
        backup_key: zoo_crypto::BackupDecryptionKey::generate(),
    };
    let ssss = SsssKey::generate();
    let recovery = RecoveryKey::generate();
    let slot = KeySlot::new_recovery(&id, &ssss, &recovery).unwrap();
    let request = ssss
        .keys_request(&SecretStorageKeysRequest {
            expected_default_key_id: None,
            expected_secrets_revision: None,
            cross_signing: &secrets.cross_signing,
            backup_key: &secrets.backup_key,
            previous_backup_keys: &[],
            slots: &[slot],
        })
        .unwrap();
    env.ok("POST", "/api/e2ee/secret_storage/keys", &id, true, request)
        .await;
    let account = Account {
        id: id.clone(),
        recovery: recovery.to_display().to_string(),
    };
    let mut device = Device::register(env, &id).await;
    device.unlock(env, &account).await.unwrap();
    (account, device)
}

/// 失効を始める（docs/e2ee.md の「失効の手順」の 1 と、4 の一覧の収集、鍵の生成。Web の
/// `revocationController` と同じ順）。
async fn prepare(
    env: &Env,
    account: &Account,
    device: &Device,
    scope: RevocationScope,
    delete: &[String],
) -> Revocation {
    let user = device.user.clone();
    // 1. 選んだ端末を削除する。
    for device_id in delete {
        env.ok(
            "DELETE",
            &format!("/api/e2ee/devices/{device_id}"),
            &user,
            false,
            Value::Null,
        )
        .await;
    }
    // 始める前に SSSS を開け直す。
    let unlocked = unlock_ssss(env, account).await;
    let pins = env.all_pins(&user).await;
    let mut targets: BTreeSet<String> = pins
        .iter()
        .filter_map(|pin| pin["target_user_id"].as_str().map(str::to_owned))
        .collect();
    targets.extend(device.trust.records().map(|(user, _)| user.clone()));
    let target_refs: Vec<&str> = targets.iter().map(String::as_str).collect();
    let targets_query = if target_refs.is_empty() {
        json!({})
    } else {
        env.keys_query(&user, &target_refs).await
    };
    let signatures: Vec<KeySignature> =
        serde_json::from_value(targets_query["signatures"].clone()).unwrap_or_default();
    let master_keys: BTreeMap<String, Value> =
        serde_json::from_value(targets_query["master_keys"].clone()).unwrap_or_default();
    let server = env.server_state(&user).await;
    let own = device.own.as_ref();
    Revocation::begin(RevocationInput {
        scope,
        unlocked: &unlocked,
        trust: &device.trust,
        server: &server,
        pins: &pins,
        signatures: &signatures,
        master_keys: &master_keys,
        local_backup_key: own.map(|own| &own.backup_key),
        old_user_signing: own.map(|own| &own.user_signing),
        deleted_devices: delete.to_vec(),
    })
    .unwrap()
}

/// 記録した段の続きから保存する。`stop` の段まで保存したら返す。409 などで止まったら `Err`。
async fn finish(
    env: &Env,
    account: &mut Account,
    device: &mut Device,
    revocation: &mut Revocation,
    stop: RevocationStage,
) -> Result<(), (StatusCode, Value)> {
    let user = device.user.clone();
    let scope = revocation.state().scope;
    if revocation.state().stage < RevocationStage::SecretStorageSaved {
        // 2〜5：SSSS 鍵を作り直し、新しいリカバリーキーのスロットを作る。
        let (recovery, slots) = match scope {
            RevocationScope::Full => {
                let recovery = RecoveryKey::generate();
                let slot = KeySlot::new_recovery(&user, revocation.ssss_key(), &recovery).unwrap();
                (Some(recovery.to_display().to_string()), vec![slot])
            }
            RevocationScope::DeviceOnly => (None, Vec::new()),
        };
        let request = revocation.secret_storage_request(&slots).unwrap();
        let path = match scope {
            RevocationScope::Full => "/api/e2ee/secret_storage/keys",
            RevocationScope::DeviceOnly => "/api/e2ee/secret_storage/secrets",
        };
        let (status, res) = env.send("POST", path, &user, true, request).await;
        if status != StatusCode::OK {
            return Err((status, res));
        }
        revocation.secret_storage_saved(res["data"]["secrets_revision"].as_u64());
        if let Some(recovery) = recovery {
            account.recovery = recovery;
        }
    }
    if stop <= revocation.state().stage {
        return Ok(());
    }

    if revocation.state().stage < RevocationStage::CrossSigningSaved {
        let (status, res) = env
            .send(
                "POST",
                "/api/e2ee/cross_signing/keys",
                &user,
                true,
                revocation.cross_signing_request(),
            )
            .await;
        if status != StatusCode::OK {
            return Err((status, res));
        }
        revocation.advance(RevocationStage::CrossSigningSaved);
        let keys = &revocation.secrets().cross_signing;
        device
            .trust
            .apply_revocation(&keys.master, &keys.user_signing, &revocation.state().resign)
            .unwrap();
    }
    if stop <= revocation.state().stage {
        return Ok(());
    }

    if revocation.state().stage < RevocationStage::SignaturesSaved {
        let signatures = revocation
            .signatures(&device.account.device_keys())
            .unwrap();
        let mut uploads = vec![signatures.device.clone()];
        uploads.extend(signatures.master_keys.iter().cloned());
        env.ok(
            "POST",
            "/api/e2ee/cross_signing/signatures",
            &user,
            true,
            json!({ "signatures": uploads }),
        )
        .await;
        if !signatures.pins.is_empty() {
            let pins: Vec<Value> = signatures.pins.iter().map(pin_body).collect();
            env.ok(
                "POST",
                "/api/e2ee/tofu_pins",
                &user,
                true,
                json!({ "pins": pins, "replace": true }),
            )
            .await;
        }
        revocation.advance(RevocationStage::SignaturesSaved);
        device.own = Some(revocation.secrets().own_keys());
    }
    if stop <= revocation.state().stage {
        return Ok(());
    }

    if revocation.state().stage < RevocationStage::BackupCreated {
        let (status, res) = env
            .send(
                "POST",
                "/api/e2ee/key_backup/versions",
                &user,
                true,
                revocation.backup_version_request().unwrap(),
            )
            .await;
        if status != StatusCode::OK {
            return Err((status, res));
        }
        revocation.backup_created(res["data"]["version"].as_str().unwrap().to_owned());
    }
    if stop <= revocation.state().stage {
        return Ok(());
    }

    if revocation.state().stage < RevocationStage::Migrated {
        // 古い版のセッションを新しい版に移し、古い版を消す。
        let new_version = revocation.state().new_backup_version.clone().unwrap();
        let versions = BackupVersionInfo::list_from_json(&env.versions(&user).await).unwrap();
        let secrets_revision = env.secret_storage(&user).await["secrets_revision"]
            .as_u64()
            .unwrap();
        for (version, key) in revocation.versions_to_migrate(&versions) {
            let rows = env.backup_rows(&user, &version).await;
            let (rows, dropped) = revocation.reencrypt_rows(&key, &rows);
            assert_eq!(dropped, 0);
            for chunk in rows.chunks(1000) {
                env.ok(
                    "POST",
                    &format!(
                        "/api/e2ee/key_backup/keys?version={new_version}&secrets_revision={secrets_revision}"
                    ),
                    &user,
                    false,
                    json!({ "sessions": chunk }),
                )
                .await;
            }
            let (status, res) = env
                .send(
                    "DELETE",
                    &format!("/api/e2ee/key_backup/versions/{version}"),
                    &user,
                    false,
                    Value::Null,
                )
                .await;
            assert!(
                status == StatusCode::OK || status == StatusCode::NOT_FOUND,
                "{status}: {res}"
            );
        }
        revocation.advance(RevocationStage::Migrated);
    }
    if stop <= revocation.state().stage {
        return Ok(());
    }

    // 読み直して確かめる。
    let current = env
        .ok(
            "GET",
            "/api/e2ee/key_backup/versions/current",
            &user,
            false,
            Value::Null,
        )
        .await;
    revocation
        .verify(
            &env.secret_storage(&user).await,
            &env.keys_query(&user, &[&user]).await,
            &current,
        )
        .map_err(|e| (StatusCode::CONFLICT, json!({ "error": e.to_string() })))?;
    revocation.advance(RevocationStage::Verified);
    Ok(())
}

/// 失効を始めて、`stop` の段まで保存する。
async fn revoke(
    env: &Env,
    account: &mut Account,
    device: &mut Device,
    scope: RevocationScope,
    delete: &[String],
    stop: RevocationStage,
) -> Revocation {
    let mut revocation = prepare(env, account, device, scope, delete).await;
    finish(env, account, device, &mut revocation, stop)
        .await
        .unwrap();
    revocation
}

/// 消していない版の公開鍵に合う秘密鍵が、すべて SSSS にあるか。
async fn assert_live_backup_keys_in_ssss(env: &Env, account: &Account) {
    let unlocked = unlock_ssss(env, account).await;
    let mut keys = vec![
        OwnSecrets::from_unlocked(&unlocked)
            .unwrap()
            .backup_key
            .public_key(),
    ];
    keys.extend(
        unlocked
            .previous_backup_keys()
            .unwrap()
            .iter()
            .map(|k| k.public_key()),
    );
    let versions = BackupVersionInfo::list_from_json(&env.versions(&account.id).await).unwrap();
    for version in versions.iter().filter(|v| !v.deleted) {
        assert!(
            keys.contains(&version.public_key),
            "版 {} の秘密鍵が SSSS にない",
            version.version
        );
    }
}

fn state(result: &Distribution, user: &str) -> UserState {
    result.users[user].state
}

fn distributed(result: &Distribution) -> BTreeSet<(String, String)> {
    result
        .devices
        .iter()
        .map(|d| (d.user_id.clone(), d.device_id.clone()))
        .collect()
}

/// 鍵バックアップの版に、送信端末の識別鍵が本物の行を `count` 件上げる。
async fn upload_sessions(env: &Env, user: &str, count: usize) -> Vec<String> {
    let current = env
        .ok(
            "GET",
            "/api/e2ee/key_backup/versions/current",
            user,
            false,
            Value::Null,
        )
        .await;
    let version = current["version"].as_str().unwrap().to_owned();
    let info = BackupVersionInfo::from_json(&current).unwrap();
    let key = BackupEncryptionKey::new(info.public_key);
    let revision = env.secret_storage(user).await["secrets_revision"]
        .as_u64()
        .unwrap();
    let sender = DeviceAccount::new(user, &Uuid::new_v4().to_string())
        .unwrap()
        .identity_keys();
    let channel = Uuid::new_v4().to_string();
    let mut ids = Vec::new();
    let mut rows = Vec::new();
    for i in 0..count {
        let session_id = format!("session-{i}-{}", Uuid::new_v4().simple());
        let data = BackedUpSessionData {
            algorithm: "m.megolm.v1.aes-sha2".into(),
            channel_id: channel.clone(),
            epoch: 0,
            session_id: session_id.clone(),
            session_key: "exported".into(),
            sender_key: sender.curve25519.to_base64(),
            sender_claimed_keys: SenderClaimedKeys {
                ed25519: sender.ed25519.to_base64(),
            },
            sender_user_id: user.to_owned(),
            sender_device_id: Uuid::new_v4().to_string(),
        };
        rows.push(BackupRow {
            channel_id: channel.clone(),
            sender_key: sender.curve25519.to_base64(),
            session_id: session_id.clone(),
            first_message_index: 0,
            forwarded_count: 0,
            is_verified: true,
            session_data: key.encrypt(&data).unwrap(),
        });
        ids.push(session_id);
    }
    env.ok(
        "POST",
        &format!("/api/e2ee/key_backup/keys?version={version}&secrets_revision={revision}"),
        user,
        false,
        json!({ "sessions": rows }),
    )
    .await;
    ids
}

// ---------------------------------------------------------------------------
// 完了条件

/// 失効した端末は、失効後のメッセージを復号できない（配布先に入らず、新しい版も開けず、
/// 古い自己署名鍵で端末に署名し直すこともできない）。
#[tokio::test]
async fn revoked_device_cannot_read_after_revocation() {
    let env = env().await;
    let (mut alice, mut first) = setup(&env).await;
    let mut lost = Device::register(&env, &alice.id).await;
    lost.unlock(&env, &alice).await.unwrap();
    let (_bob_account, mut bob) = setup(&env).await;
    env.encrypted_channel(&[&alice.id, &bob.user]).await;
    upload_sessions(&env, &alice.id, 3).await;

    // 失効の前は、ボブは失った端末にも配る。
    let before = bob.evaluate(&env, &[&alice.id]).await;
    assert!(distributed(&before).contains(&(alice.id.clone(), lost.id())));

    revoke(
        &env,
        &mut alice,
        &mut first,
        RevocationScope::DeviceOnly,
        &[lost.id()],
        RevocationStage::Verified,
    )
    .await;

    let after = bob.evaluate(&env, &[&alice.id]).await;
    let devices = distributed(&after);
    assert!(!devices.contains(&(alice.id.clone(), lost.id())));
    assert!(devices.contains(&(alice.id.clone(), first.id())));

    // 失った端末のバックアップ鍵では、新しい版を開けない。
    let lost_keys = lost.own.as_ref().unwrap();
    let current = env
        .ok(
            "GET",
            "/api/e2ee/key_backup/versions/current",
            &alice.id,
            false,
            Value::Null,
        )
        .await;
    let version = current["version"].as_str().unwrap();
    let rows = env.backup_rows(&alice.id, version).await;
    assert_eq!(rows.len(), 3, "古い版のセッションは新しい版に移る");
    assert!(
        rows.iter()
            .all(|row| lost_keys.backup_key.decrypt_row(row).is_err())
    );

    // 古い自己署名鍵での署名と、古い鍵の登録し直しは拒否される。
    let attacker = DeviceAccount::new(&alice.id, &Uuid::new_v4().to_string()).unwrap();
    env.ok(
        "POST",
        "/api/e2ee/devices",
        &alice.id,
        false,
        json!({"device_keys": attacker.device_keys()}),
    )
    .await;
    let forged = lost_keys
        .self_signing
        .sign_device(&attacker.device_keys())
        .unwrap();
    let (status, res) = env
        .send(
            "POST",
            "/api/e2ee/cross_signing/signatures",
            &alice.id,
            false,
            json!({ "signatures": [forged] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert_eq!(code(&res), "E2EE_BAD_SIGNATURE");
    let after = bob.evaluate(&env, &[&alice.id]).await;
    assert!(!distributed(&after).contains(&(alice.id.clone(), attacker.device_id().to_owned())));
}

/// マスター鍵が変わったユーザーには、再検証するまで鍵が配られない。
#[tokio::test]
async fn changed_master_key_is_not_distributed_to_until_reverified() {
    let env = env().await;
    let (_alice_account, mut alice) = setup(&env).await;
    let (mut bob_account, mut bob) = setup(&env).await;
    env.encrypted_channel(&[&alice.user, &bob.user]).await;
    let first = alice.evaluate(&env, &[&bob.user]).await;
    assert_eq!(
        state(&first, &bob.user),
        UserState::Trusted(TrustBasis::Record)
    );

    let bob_user = bob.user.clone();
    revoke(
        &env,
        &mut bob_account,
        &mut bob,
        RevocationScope::Full,
        &[],
        RevocationStage::Verified,
    )
    .await;

    let changed = alice.evaluate(&env, &[&bob_user]).await;
    assert_eq!(
        state(&changed, &bob_user),
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert!(
        distributed(&changed)
            .iter()
            .all(|(user, _)| *user != bob_user)
    );

    alice.reverify(&env, &bob_user).await;
    let accepted = alice.evaluate(&env, &[&bob_user]).await;
    assert!(matches!(state(&accepted, &bob_user), UserState::Trusted(_)));
    assert!(distributed(&accepted).contains(&(bob_user.clone(), bob.id())));
}

/// 別のユーザーが同じ `device_id` の端末を登録して削除しても、相手の端末の自己署名が残り、
/// 配布先から外れない。
#[tokio::test]
async fn deleting_another_users_device_with_the_same_id_keeps_signatures() {
    let env = env().await;
    let (_alice_account, alice) = setup(&env).await;
    let (_bob_account, mut bob) = setup(&env).await;
    env.encrypted_channel(&[&alice.user, &bob.user]).await;
    let carol = env.user().await;
    let same = DeviceAccount::new(&carol, alice.account.device_id()).unwrap();
    env.ok(
        "POST",
        "/api/e2ee/devices",
        &carol,
        false,
        json!({"device_keys": same.device_keys()}),
    )
    .await;
    env.ok(
        "DELETE",
        &format!("/api/e2ee/devices/{}", alice.id()),
        &carol,
        false,
        Value::Null,
    )
    .await;
    let result = bob.evaluate(&env, &[&alice.user]).await;
    assert!(distributed(&result).contains(&(alice.user.clone(), alice.id())));
}

/// 1 と 4 のあいだに登録され、古い自己署名鍵で署名された端末は、失効のあとで配布先に入らない。
#[tokio::test]
async fn device_signed_with_the_old_self_signing_key_is_dropped_after_revocation() {
    let env = env().await;
    let (mut alice, mut first) = setup(&env).await;
    let lost = {
        let mut lost = Device::register(&env, &alice.id).await;
        lost.unlock(&env, &alice).await.unwrap();
        lost
    };
    let (_bob_account, mut bob) = setup(&env).await;
    env.encrypted_channel(&[&alice.id, &bob.user]).await;

    // 1 の削除と SSSS の保存のあと、4 の前に、攻撃者が古い自己署名鍵で端末に署名する。
    let partial = revoke(
        &env,
        &mut alice,
        &mut first,
        RevocationScope::DeviceOnly,
        &[lost.id()],
        RevocationStage::SecretStorageSaved,
    )
    .await;
    let attacker = DeviceAccount::new(&alice.id, &Uuid::new_v4().to_string()).unwrap();
    env.ok(
        "POST",
        "/api/e2ee/devices",
        &alice.id,
        false,
        json!({"device_keys": attacker.device_keys()}),
    )
    .await;
    let signature = lost
        .own
        .as_ref()
        .unwrap()
        .self_signing
        .sign_device(&attacker.device_keys())
        .unwrap();
    env.ok(
        "POST",
        "/api/e2ee/cross_signing/signatures",
        &alice.id,
        false,
        json!({ "signatures": [signature] }),
    )
    .await;
    let during = bob.evaluate(&env, &[&alice.id]).await;
    assert!(distributed(&during).contains(&(alice.id.clone(), attacker.device_id().to_owned())));

    // 同じ端末で、暗号ストアに記録した失効を、開け直した SSSS で続ける（4 で自己署名鍵が変わる）。
    let unlocked = unlock_ssss(&env, &alice).await;
    let mut resumed = Revocation::resume(partial.state().clone(), &unlocked).unwrap();
    finish(
        &env,
        &mut alice,
        &mut first,
        &mut resumed,
        RevocationStage::Verified,
    )
    .await
    .unwrap();

    let after = bob.evaluate(&env, &[&alice.id]).await;
    let devices = distributed(&after);
    assert!(!devices.contains(&(alice.id.clone(), attacker.device_id().to_owned())));
    assert!(devices.contains(&(alice.id.clone(), first.id())));
}

/// SSSS を書いたあと版を作る前にログアウトしても、別の端末で失効をやり直すと、古い版の
/// セッションが新しい版に移る。
#[tokio::test]
async fn retried_revocation_on_another_device_moves_old_sessions() {
    let env = env().await;
    let (mut alice, mut first) = setup(&env).await;
    let mut second = Device::register(&env, &alice.id).await;
    second.unlock(&env, &alice).await.unwrap();
    let sessions = upload_sessions(&env, &alice.id, 5).await;

    // 1 台目は、署名まで保存したあと、版を作る前にログアウトする（暗号ストアを消す）。
    revoke(
        &env,
        &mut alice,
        &mut first,
        RevocationScope::DeviceOnly,
        &[],
        RevocationStage::SignaturesSaved,
    )
    .await;
    drop(first);

    // 2 台目は、解錠すると今の版を検証できない（失効が終わっていない）ので、失効をやり直す。
    assert!(second.unlock(&env, &alice).await.is_err());
    revoke(
        &env,
        &mut alice,
        &mut second,
        RevocationScope::DeviceOnly,
        &[],
        RevocationStage::Verified,
    )
    .await;

    let versions = env.versions(&alice.id).await;
    let live: Vec<&Value> = versions["versions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| !v["deleted"].as_bool().unwrap())
        .collect();
    assert_eq!(live.len(), 1, "古い版は消える: {versions}");
    let version = live[0]["version"].as_str().unwrap();
    let rows = env.backup_rows(&alice.id, version).await;
    let moved: BTreeSet<_> = rows.iter().map(|row| row.session_id.clone()).collect();
    assert_eq!(moved, sessions.into_iter().collect());
    let key = &second.own.as_ref().unwrap().backup_key;
    assert!(rows.iter().all(|row| key.decrypt_row(row).is_ok()));
}

/// 2 つの端末で失効を同時に行っても、消していない版の秘密鍵はすべて SSSS に残る。
#[tokio::test]
async fn concurrent_revocations_keep_every_live_backup_key_in_ssss() {
    let env = env().await;
    let (mut alice, mut first) = setup(&env).await;
    let mut second = Device::register(&env, &alice.id).await;
    second.unlock(&env, &alice).await.unwrap();
    upload_sessions(&env, &alice.id, 2).await;

    // 2 台が同じ SSSS を読んで始める。1 台目が先に SSSS を書き、2 台目は 409 で止まる。
    let mut r1 = prepare(&env, &alice, &first, RevocationScope::DeviceOnly, &[]).await;
    let mut r2 = prepare(&env, &alice, &second, RevocationScope::DeviceOnly, &[]).await;
    finish(
        &env,
        &mut alice,
        &mut first,
        &mut r1,
        RevocationStage::SecretStorageSaved,
    )
    .await
    .unwrap();
    let (status, res) = finish(
        &env,
        &mut alice,
        &mut second,
        &mut r2,
        RevocationStage::Verified,
    )
    .await
    .unwrap_err();
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code(&res), "E2EE_SSSS_CONFLICT");
    assert_live_backup_keys_in_ssss(&env, &alice).await;

    // 2 台目は SSSS を開け直してやり直し、最後まで終える。
    let mut r2 = prepare(&env, &alice, &second, RevocationScope::DeviceOnly, &[]).await;
    finish(
        &env,
        &mut alice,
        &mut second,
        &mut r2,
        RevocationStage::Verified,
    )
    .await
    .unwrap();
    assert_live_backup_keys_in_ssss(&env, &alice).await;

    // 1 台目は続きのクロス署名鍵の保存で 409 になり（SSSS の `secrets_revision` が変わった）、開け直して
    // やり直す。
    let (status, res) = finish(
        &env,
        &mut alice,
        &mut first,
        &mut r1,
        RevocationStage::Verified,
    )
    .await
    .unwrap_err();
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(code(&res), "E2EE_SSSS_CONFLICT");
    let mut r1 = prepare(&env, &alice, &first, RevocationScope::DeviceOnly, &[]).await;
    finish(
        &env,
        &mut alice,
        &mut first,
        &mut r1,
        RevocationStage::Verified,
    )
    .await
    .unwrap();
    assert_live_backup_keys_in_ssss(&env, &alice).await;

    // 1〜5 の失効と交互に進めても同じ。
    let mut full = prepare(&env, &alice, &second, RevocationScope::Full, &[]).await;
    let mut device_only = prepare(&env, &alice, &first, RevocationScope::DeviceOnly, &[]).await;
    finish(
        &env,
        &mut alice,
        &mut first,
        &mut device_only,
        RevocationStage::BackupCreated,
    )
    .await
    .unwrap();
    assert_live_backup_keys_in_ssss(&env, &alice).await;
    let (status, _) = finish(
        &env,
        &mut alice,
        &mut second,
        &mut full,
        RevocationStage::Verified,
    )
    .await
    .unwrap_err();
    assert_eq!(status, StatusCode::CONFLICT);
    let mut full = prepare(&env, &alice, &second, RevocationScope::Full, &[]).await;
    finish(
        &env,
        &mut alice,
        &mut second,
        &mut full,
        RevocationStage::Verified,
    )
    .await
    .unwrap();
    assert_live_backup_keys_in_ssss(&env, &alice).await;
    let versions = env.versions(&alice.id).await;
    let live = versions["versions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| !v["deleted"].as_bool().unwrap())
        .count();
    assert_eq!(live, 1, "{versions}");
}

/// 1〜5 の失効が SSSS を保存したあと、別の端末の失効が SSSS だけを作り直して止まったら、続きの
/// クロス署名鍵と鍵バックアップの版を保存しない（`expected_default_key_id` を同じトランザクションで
/// 照合して 409 `E2EE_SSSS_CONFLICT`）。公開鍵の保存の前と、版を作る前の 2 か所で確かめる。
#[tokio::test]
async fn revocation_stops_when_another_device_replaced_secret_storage() {
    let env = env().await;
    let (mut alice, mut first) = setup(&env).await;
    let mut second = Device::register(&env, &alice.id).await;
    second.unlock(&env, &alice).await.unwrap();

    // 端末だけの失効は SSSS の鍵を変えないので、`expected_secrets_revision` で見分ける。
    for (scope, stop) in [
        (RevocationScope::Full, RevocationStage::SecretStorageSaved),
        (RevocationScope::Full, RevocationStage::SignaturesSaved),
        (
            RevocationScope::DeviceOnly,
            RevocationStage::SecretStorageSaved,
        ),
        (
            RevocationScope::DeviceOnly,
            RevocationStage::SignaturesSaved,
        ),
    ] {
        let mut r1 = prepare(&env, &alice, &first, scope, &[]).await;
        finish(&env, &mut alice, &mut first, &mut r1, stop)
            .await
            .unwrap();
        // 2 台目は 1 台目の SSSS を開けて失効を始め、SSSS だけを保存して止まる。
        let mut r2 = prepare(&env, &alice, &second, scope, &[]).await;
        finish(
            &env,
            &mut alice,
            &mut second,
            &mut r2,
            RevocationStage::SecretStorageSaved,
        )
        .await
        .unwrap();
        let ssss = env.secret_storage(&alice.id).await;
        let keys = env.keys_query(&alice.id, &[&alice.id]).await;
        let versions = env.versions(&alice.id).await;

        let (status, res) = finish(
            &env,
            &mut alice,
            &mut first,
            &mut r1,
            RevocationStage::Verified,
        )
        .await
        .unwrap_err();
        assert_eq!(status, StatusCode::CONFLICT, "{scope:?} {stop:?}");
        assert_eq!(code(&res), "E2EE_SSSS_CONFLICT", "{scope:?} {stop:?}");
        assert_eq!(env.secret_storage(&alice.id).await, ssss);
        let after = env.keys_query(&alice.id, &[&alice.id]).await;
        for field in ["master_keys", "self_signing_keys", "user_signing_keys"] {
            assert_eq!(after[field], keys[field], "{scope:?} {stop:?} {field}");
        }
        assert_eq!(
            env.versions(&alice.id).await,
            versions,
            "{scope:?} {stop:?}"
        );

        // 2 台目が最後まで終えれば、SSSS と公開鍵と版がそろう。
        finish(
            &env,
            &mut alice,
            &mut second,
            &mut r2,
            RevocationStage::Verified,
        )
        .await
        .unwrap();
        first.unlock(&env, &alice).await.unwrap();
    }
}

/// 暗号化チャンネルを抜けた相手に受け入れで署名していても、失効のあとで署名が付け直される。
#[tokio::test]
async fn signatures_on_users_who_left_are_resigned() {
    let env = env().await;
    let (mut alice, mut device) = setup(&env).await;
    let (_bob_account, bob) = setup(&env).await;
    let channel = env.encrypted_channel(&[&alice.id, &bob.user]).await;
    device.evaluate(&env, &[&bob.user]).await;
    device.reverify(&env, &bob.user).await;
    env.exec(
        "DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2",
        &[&channel, &bob.user],
    )
    .await;

    let revoked = revoke(
        &env,
        &mut alice,
        &mut device,
        RevocationScope::DeviceOnly,
        &[],
        RevocationStage::Verified,
    )
    .await;
    let new_usk = revoked.secrets().cross_signing.user_signing.key_id();
    let response = env.keys_query(&alice.id, &[&bob.user]).await;
    let signed = response["signatures"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| {
            row["target_user_id"] == bob.user.as_str() && row["signer_key_id"] == new_usk.as_str()
        });
    assert!(signed, "{response}");
    assert!(revoked.state().resign.notes.is_empty());
}

// ---------------------------------------------------------------------------
// 他の端末の判定（結合テスト）

/// 失った端末が失効の前に攻撃者のマスター鍵を記録させても、失効のあと、他の端末はその相手に
/// 再検証を求め、配布しない。
#[tokio::test]
async fn planted_attacker_master_key_requires_reverification_after_revocation() {
    let env = env().await;
    let (mut alice, mut revoker) = setup(&env).await;
    let (_bob_account, bob) = setup(&env).await;
    env.encrypted_channel(&[&alice.id, &bob.user]).await;
    revoker.evaluate(&env, &[&bob.user]).await;

    // 失った端末が、古いユーザー署名鍵と再認証で、攻撃者のマスター鍵のピンを置く。
    let mut lost = Device::register(&env, &alice.id).await;
    lost.unlock(&env, &alice).await.unwrap();
    let attacker = CrossSigningKeyPair::generate(&bob.user, KeyUsage::Master).unwrap();
    let pin = lost
        .user_signing()
        .unwrap()
        .sign_tofu_pin(&bob.user, &attacker.public_key())
        .unwrap();
    env.ok(
        "POST",
        "/api/e2ee/tofu_pins",
        &alice.id,
        true,
        json!({ "pins": [pin_body(&pin)], "replace": true }),
    )
    .await;
    // 記録のない端末が、そのピンを採用する。
    let mut other = Device::register(&env, &alice.id).await;
    other.unlock(&env, &alice).await.unwrap();
    let adopted = other.evaluate(&env, &[&bob.user]).await;
    assert!(matches!(
        state(&adopted, &bob.user),
        UserState::NeedsReverification(_)
    ));

    let revoked = revoke(
        &env,
        &mut alice,
        &mut revoker,
        RevocationScope::DeviceOnly,
        &[lost.id()],
        RevocationStage::Verified,
    )
    .await;
    assert!(revoked.state().resign.notes[&bob.user].contains(&zoo_crypto::ResignNote::PinReplaced));

    // サーバーが攻撃者のマスター鍵を見せても、他の端末は配らない。
    other.unlock(&env, &alice).await.unwrap();
    let key = attacker.public_key().to_base64();
    env.exec(
        "UPDATE e2ee_cross_signing_keys SET public_key = $2, key_json = $3::jsonb
         WHERE user_id = $1 AND usage = 'master'",
        &[&bob.user, &key, &attacker.public_key_json().to_string()],
    )
    .await;
    for device in [&mut other, &mut revoker] {
        let result = device.evaluate(&env, &[&bob.user]).await;
        assert!(
            matches!(state(&result, &bob.user), UserState::NeedsReverification(_)),
            "{:?}",
            state(&result, &bob.user)
        );
        assert!(
            distributed(&result)
                .iter()
                .all(|(user, _)| *user != bob.user)
        );
    }
}

/// 失効のあと、他の端末が解錠し直しても、記録からピンを自動では作らない。再検証のあとだけ作る。
/// 1 人の相手を再検証しても、ほかの相手の変更の検出は止まらない。
#[tokio::test]
async fn reunlocking_never_recreates_pins_and_reverification_is_per_user() {
    let env = env().await;
    let (mut alice, mut revoker) = setup(&env).await;
    let mut other = Device::register(&env, &alice.id).await;
    other.unlock(&env, &alice).await.unwrap();
    let (_b, bob) = setup(&env).await;
    let (_c, carol) = setup(&env).await;
    let (_d, dave) = setup(&env).await;
    env.encrypted_channel(&[&alice.id, &bob.user, &carol.user, &dave.user])
        .await;
    // 失効を行う端末は bob だけを知っている。ほかの端末は 3 人を知っている。
    revoker.evaluate(&env, &[&bob.user]).await;
    other
        .evaluate(&env, &[&bob.user, &carol.user, &dave.user])
        .await;
    revoke(
        &env,
        &mut alice,
        &mut revoker,
        RevocationScope::DeviceOnly,
        &[],
        RevocationStage::Verified,
    )
    .await;

    other.unlock(&env, &alice).await.unwrap();
    let pins_before = env.all_pins(&alice.id).await;
    let result = other
        .evaluate(&env, &[&bob.user, &carol.user, &dave.user])
        .await;
    assert!(matches!(state(&result, &bob.user), UserState::Trusted(_)));
    for user in [&carol.user, &dave.user] {
        assert_eq!(
            state(&result, user),
            UserState::NeedsReverification(ReverificationReason::UserSigningKeyChanged)
        );
    }
    assert!(result.new_pins.is_empty());
    assert_eq!(
        env.all_pins(&alice.id).await,
        pins_before,
        "ピンを自動で作らない"
    );

    // carol だけを再検証する。dave は再検証のまま。
    other.reverify(&env, &carol.user).await;
    let result = other
        .evaluate(&env, &[&bob.user, &carol.user, &dave.user])
        .await;
    assert!(matches!(state(&result, &carol.user), UserState::Trusted(_)));
    assert_eq!(
        state(&result, &dave.user),
        UserState::NeedsReverification(ReverificationReason::UserSigningKeyChanged)
    );
}

/// 初めて見た相手には TOFU で送信でき、`confirmed_usk` が設定される。ユーザー署名鍵の更新後、
/// 署名し直されなかった相手だけが再検証になる。
#[tokio::test]
async fn first_seen_users_are_trusted_and_only_unresigned_users_need_reverification() {
    let env = env().await;
    let (mut alice, mut revoker) = setup(&env).await;
    let mut other = Device::register(&env, &alice.id).await;
    other.unlock(&env, &alice).await.unwrap();
    let (_b, bob) = setup(&env).await;
    let (_e, erin) = setup(&env).await;
    env.encrypted_channel(&[&alice.id, &bob.user, &erin.user])
        .await;

    revoker.evaluate(&env, &[&bob.user]).await;
    let first = other.evaluate(&env, &[&bob.user, &erin.user]).await;
    for user in [&bob.user, &erin.user] {
        assert!(matches!(state(&first, user), UserState::Trusted(_)));
        assert!(distributed(&first).iter().any(|(u, _)| u == user));
    }
    let usk = other.user_signing().unwrap().public_key().to_base64();
    assert_eq!(
        other
            .trust
            .record(&erin.user)
            .unwrap()
            .confirmed_usk
            .as_deref(),
        Some(usk.as_str())
    );

    revoke(
        &env,
        &mut alice,
        &mut revoker,
        RevocationScope::DeviceOnly,
        &[],
        RevocationStage::Verified,
    )
    .await;
    other.unlock(&env, &alice).await.unwrap();
    let result = other.evaluate(&env, &[&bob.user, &erin.user]).await;
    assert!(matches!(state(&result, &bob.user), UserState::Trusted(_)));
    assert_eq!(
        state(&result, &erin.user),
        UserState::NeedsReverification(ReverificationReason::UserSigningKeyChanged)
    );
    let new_usk = other.user_signing().unwrap().public_key().to_base64();
    assert_eq!(
        other
            .trust
            .record(&bob.user)
            .unwrap()
            .confirmed_usk
            .as_deref(),
        Some(new_usk.as_str())
    );
}

/// `confirmed_usk` がない記録は、失効のあとも再検証を経るまで配布されない。
#[tokio::test]
async fn records_without_confirmed_usk_stay_blocked_until_reverified() {
    let env = env().await;
    let (mut alice, mut revoker) = setup(&env).await;
    let (_b, bob) = setup(&env).await;
    env.encrypted_channel(&[&alice.id, &bob.user]).await;
    // 失効を行う端末が、bob の今のマスター鍵と違うピンを置く（記録には confirmed_usk がある）。
    revoker.evaluate(&env, &[&bob.user]).await;
    let stale = CrossSigningKeyPair::generate(&bob.user, KeyUsage::Master).unwrap();
    let pin = revoker
        .user_signing()
        .unwrap()
        .sign_tofu_pin(&bob.user, &stale.public_key())
        .unwrap();
    env.ok(
        "POST",
        "/api/e2ee/tofu_pins",
        &alice.id,
        true,
        json!({ "pins": [pin_body(&pin)], "replace": true }),
    )
    .await;
    // 記録のない別の端末は、そのピンを採用するが、confirmed_usk を設定しない。
    let mut other = Device::register(&env, &alice.id).await;
    other.unlock(&env, &alice).await.unwrap();
    let adopted = other.evaluate(&env, &[&bob.user]).await;
    assert!(matches!(
        state(&adopted, &bob.user),
        UserState::NeedsReverification(_)
    ));
    assert_eq!(other.trust.record(&bob.user).unwrap().confirmed_usk, None);

    revoke(
        &env,
        &mut alice,
        &mut revoker,
        RevocationScope::DeviceOnly,
        &[],
        RevocationStage::Verified,
    )
    .await;
    other.unlock(&env, &alice).await.unwrap();
    let after = other.evaluate(&env, &[&bob.user]).await;
    assert!(matches!(
        state(&after, &bob.user),
        UserState::NeedsReverification(_)
    ));
    assert!(
        distributed(&after)
            .iter()
            .all(|(user, _)| *user != bob.user)
    );

    other.reverify(&env, &bob.user).await;
    let accepted = other.evaluate(&env, &[&bob.user]).await;
    assert!(matches!(state(&accepted, &bob.user), UserState::Trusted(_)));
}

/// 古い `auth_data` と古い自己署名鍵は 400 `E2EE_KEY_REUSED`、古い `key_id` の SSSS の暗号文は
/// 409 `E2EE_SSSS_CONFLICT`、今の `key_id` の古い暗号文は再認証があれば受け付ける。
#[tokio::test]
async fn replaying_old_keys_and_secrets_after_revocation_is_rejected() {
    let env = env().await;
    let (mut alice, mut device) = setup(&env).await;
    let before_storage = env.secret_storage(&alice.id).await;
    let before_keys = env.keys_query(&alice.id, &[&alice.id]).await;
    let before_version = env
        .ok(
            "GET",
            "/api/e2ee/key_backup/versions/current",
            &alice.id,
            false,
            Value::Null,
        )
        .await;

    // 端末だけの失効。SSSS 鍵は同じ。
    revoke(
        &env,
        &mut alice,
        &mut device,
        RevocationScope::DeviceOnly,
        &[],
        RevocationStage::Verified,
    )
    .await;
    let (status, res) = env
        .send(
            "POST",
            "/api/e2ee/cross_signing/keys",
            &alice.id,
            true,
            json!({
                "self_signing_key": before_keys["self_signing_keys"][&alice.id],
                "expected_keys": server_public(&env, &alice.id).await,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert_eq!(code(&res), "E2EE_KEY_REUSED");
    let current = env
        .ok(
            "GET",
            "/api/e2ee/key_backup/versions/current",
            &alice.id,
            false,
            Value::Null,
        )
        .await;
    let (status, res) = env
        .send(
            "POST",
            "/api/e2ee/key_backup/versions",
            &alice.id,
            true,
            json!({
                "algorithm": before_version["algorithm"],
                "auth_data": before_version["auth_data"],
                "expected_current_version": current["version"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{res}");
    assert_eq!(code(&res), "E2EE_KEY_REUSED");

    // 今の key_id で古い暗号文を送ると、再認証がなければ 403、あれば受け付ける。
    let old_secret = before_storage["secrets"]["m.cross_signing.self_signing"].clone();
    let key_id = before_storage["default_key_id"].clone();
    let revision = env.secret_storage(&alice.id).await["secrets_revision"].clone();
    let body = json!({
        "key_id": key_id,
        "expected_secrets_revision": revision,
        "secrets": { "m.cross_signing.self_signing": strip_key_id(&old_secret) },
    });
    let (status, res) = env
        .send(
            "POST",
            "/api/e2ee/secret_storage/secrets",
            &alice.id,
            false,
            body.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{res}");
    assert_eq!(code(&res), "E2EE_REAUTH_REQUIRED");
    env.ok(
        "POST",
        "/api/e2ee/secret_storage/secrets",
        &alice.id,
        true,
        body,
    )
    .await;

    // 1〜5 の失効で SSSS 鍵を作り直すと、古い key_id の暗号文は 409。
    revoke(
        &env,
        &mut alice,
        &mut device,
        RevocationScope::Full,
        &[],
        RevocationStage::Verified,
    )
    .await;
    let revision = env.secret_storage(&alice.id).await["secrets_revision"].clone();
    let (status, res) = env
        .send(
            "POST",
            "/api/e2ee/secret_storage/secrets",
            &alice.id,
            true,
            json!({
                "key_id": key_id,
                "expected_secrets_revision": revision,
                "secrets": { "m.cross_signing.self_signing": strip_key_id(&old_secret) },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{res}");
    assert_eq!(code(&res), "E2EE_SSSS_CONFLICT");
}

fn strip_key_id(secret: &Value) -> Value {
    let mut secret = secret.clone();
    secret.as_object_mut().unwrap().remove("key_id");
    secret
}

async fn server_public(env: &Env, user: &str) -> Value {
    let state = env.server_state(user).await;
    let key = |value: &Option<Value>, usage| {
        value
            .as_ref()
            .map(|v| {
                zoo_crypto::verify_cross_signing_key(v, user, usage)
                    .unwrap()
                    .to_base64()
            })
            .map(Value::String)
            .unwrap_or(Value::Null)
    };
    json!({
        "master": key(&state.master, KeyUsage::Master),
        "self_signing": key(&state.self_signing, KeyUsage::SelfSigning),
        "user_signing": key(&state.user_signing, KeyUsage::UserSigning),
    })
}
