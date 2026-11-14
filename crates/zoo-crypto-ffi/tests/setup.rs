//! 初回設定と解錠（#360）のバインディングを、Kotlin と同じ流れで通しで確かめる。

use serde_json::{Value, json};
use zoo_crypto_ffi::*;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const DEVICE: &str = "00000000-0000-4000-8000-0000000000a1";

/// `POST /api/e2ee/secret_storage/keys` の本文から、`GET /api/e2ee/secret_storage` の応答を作る。
fn storage_response(request: &str, secrets_revision: u64) -> String {
    let request: Value = serde_json::from_str(request).unwrap();
    let key_id = request["key_id"].clone();
    let mut secrets = request["secrets"].clone();
    for secret in secrets.as_object_mut().unwrap().values_mut() {
        secret["key_id"] = key_id.clone();
    }
    json!({
        "default_key_id": key_id,
        "keys": { key_id.as_str().unwrap(): request["key"] },
        "secrets": secrets,
        "secrets_revision": secrets_revision,
        "slots": request["slots"],
    })
    .to_string()
}

/// サーバーを模した状態（自分だけの `keys/query` と、版の一覧）。
#[derive(Default)]
struct Server {
    keys: serde_json::Map<String, Value>,
    signatures: Vec<Value>,
    versions: Vec<Value>,
}

impl Server {
    fn keys_query(&self) -> String {
        let mut value = json!({
            "device_keys": {},
            "master_keys": {},
            "self_signing_keys": {},
            "user_signing_keys": {},
            "signatures": self.signatures,
        });
        for (field, target) in [
            ("master_key", "master_keys"),
            ("self_signing_key", "self_signing_keys"),
            ("user_signing_key", "user_signing_keys"),
        ] {
            if let Some(key) = self.keys.get(field) {
                value[target][ALICE] = key.clone();
            }
        }
        value.to_string()
    }

    fn versions(&self) -> String {
        json!({ "versions": self.versions }).to_string()
    }

    fn apply(&mut self, plan: &ResumePlan) {
        if let Some(body) = &plan.cross_signing {
            let body: Value = serde_json::from_str(body).unwrap();
            for field in ["master_key", "self_signing_key", "user_signing_key"] {
                if let Some(key) = body.get(field) {
                    self.keys.insert(field.into(), key.clone());
                }
            }
        }
        if let Some(signature) = &plan.device_signature {
            let upload: Value = serde_json::from_str(signature).unwrap();
            self.signatures.push(json!({
                "signer_user_id": ALICE,
                "signer_key_id": upload["signer_key_id"],
                "target_user_id": upload["target_user_id"],
                "target_key_id": upload["target_key_id"],
                "signature": upload["signature"],
            }));
        }
        if let Some(body) = &plan.backup_version {
            let body: Value = serde_json::from_str(body).unwrap();
            self.versions.push(json!({
                "version": (self.versions.len() + 1).to_string(),
                "algorithm": body["algorithm"],
                "auth_data": body["auth_data"],
                "deleted": false,
            }));
        }
    }
}

#[test]
fn setup_unlock_and_startup_check() {
    let account = DeviceAccount::new(ALICE.into(), DEVICE.into()).unwrap();
    let storage_key = vec![9u8; 32];
    let start = begin_setup(ALICE.into()).unwrap();
    let created = start.session;
    assert!(created.is_created());
    assert_eq!(created.user_id(), ALICE);
    assert_eq!(created.pending_slots().len(), 1);
    let request = created.secret_storage_request().unwrap();
    let response = storage_response(&request, 1);

    // 保存したあとで止めて再起動し、控えたリカバリーキーで開け直す。
    let unlocked =
        unlock_setup_with_recovery_key(ALICE.into(), response.clone(), start.recovery_key)
            .unwrap()
            .unwrap();
    assert!(!unlocked.is_created());
    assert_eq!(unlocked.key_id(), created.key_id());
    assert_eq!(unlocked.secrets_revision(), Some(1));
    assert!(unlocked.secret_storage_request().is_err());

    let mut server = Server::default();
    let plan = unlocked
        .plan(
            server.keys_query(),
            server.versions(),
            account.device_keys(),
        )
        .unwrap();
    assert!(plan.cross_signing.is_some());
    assert!(plan.device_signature.is_some());
    assert!(plan.backup_version.is_some());
    server.apply(&plan);
    let done = unlocked
        .plan(
            server.keys_query(),
            server.versions(),
            account.device_keys(),
        )
        .unwrap();
    assert_eq!(done.cross_signing, None);
    assert_eq!(done.device_signature, None);
    assert_eq!(done.backup_version, None);
    assert_eq!(done.current_backup_version.as_deref(), Some("1"));

    // 暗号ストアに置く鍵と、起動時の確認。
    let sealed = unlocked
        .own_keys_pickle(&account, storage_key.clone())
        .unwrap();
    let own = OwnKeys::from_pickle(sealed, &account, storage_key.clone()).unwrap();
    assert_eq!(
        own.startup_check(
            server.keys_query(),
            server.versions(),
            account.device_keys()
        )
        .unwrap(),
        StartupCheck::Ready
    );
    // 署名がなければ、署名し直す。
    server.signatures.clear();
    assert!(matches!(
        own.startup_check(
            server.keys_query(),
            server.versions(),
            account.device_keys()
        )
        .unwrap(),
        StartupCheck::SignDevice { .. }
    ));
    // 版がなければ、初回設定の続き。
    server.versions.clear();
    assert_eq!(
        own.startup_check(
            server.keys_query(),
            server.versions(),
            account.device_keys()
        )
        .unwrap(),
        StartupCheck::ResumeSetup
    );

    // 今の版の検証（BackupUploadState）に、暗号ストアの鍵を使える。
    let version = json!({
        "version": "1",
        "algorithm": serde_json::from_str::<Value>(plan.backup_version.as_ref().unwrap()).unwrap()["algorithm"],
        "auth_data": serde_json::from_str::<Value>(plan.backup_version.as_ref().unwrap()).unwrap()["auth_data"],
    });
    let backup = BackupUploadState::new(ALICE.into()).unwrap();
    backup
        .accept_current_version(
            version.to_string(),
            own.master_public_key(),
            &own.backup_key(),
        )
        .unwrap();

    // 配布先の判定の記録に、自分のマスター鍵を入れる。
    let trust = TrustStore::new(ALICE.into()).unwrap();
    assert_eq!(trust.own_master_key(), None);
    unlocked.record_own_keys(&trust).unwrap();
    assert_eq!(trust.own_master_key(), Some(own.master_public_key()));
    assert_eq!(trust.user_signing_key_history().len(), 1);
}

#[test]
fn passkey_slot_and_key_mismatch() {
    let start = begin_setup(ALICE.into()).unwrap();
    let created = start.session;
    created
        .add_passkey_slot("Y3JlZA".into(), vec![1u8; 32], vec![2u8; 32], None)
        .unwrap();
    assert_eq!(created.pending_slots().len(), 2);
    assert!(matches!(
        created.add_passkey_slot("Y3JlZA".into(), vec![1u8; 31], vec![2u8; 32], None),
        Err(ZooCryptoError::InvalidPayload { .. })
    ));
    let response = storage_response(&created.secret_storage_request().unwrap(), 1);
    let storage = parse_secret_storage(ALICE.into(), response)
        .unwrap()
        .unwrap();
    let slots = storage.passkey_slots().unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].credential_id, "Y3JlZA");
    let unlocked = storage
        .unlock_with_passkey("Y3JlZA".into(), vec![2u8; 32])
        .unwrap();
    let session = SetupSession::from_unlocked(&unlocked).unwrap();
    assert_eq!(session.key_id(), created.key_id());

    // ほかの鍵の初回設定が保存したクロス署名鍵は、違う鍵として扱う。
    let account = DeviceAccount::new(ALICE.into(), DEVICE.into()).unwrap();
    let other = begin_setup(ALICE.into()).unwrap().session;
    let mut server = Server::default();
    server.apply(
        &other
            .plan(
                server.keys_query(),
                server.versions(),
                account.device_keys(),
            )
            .unwrap(),
    );
    assert!(matches!(
        session.plan(
            server.keys_query(),
            server.versions(),
            account.device_keys()
        ),
        Err(ZooCryptoError::ServerKeyMismatch { .. })
    ));
    assert!(matches!(
        unlock_setup_with_recovery_key(ALICE.into(), "{}".into(), "abcd".into()),
        Ok(None) | Err(_)
    ));
}
