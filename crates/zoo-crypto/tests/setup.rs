//! 初回設定と解錠のあとに送る要求（docs/e2ee.md の「初回設定と解錠の流れ」、#360）。
//!
//! 初回設定の各保存（SSSS、クロス署名鍵、端末への署名、バックアップの版）の直前と直後で止めた
//! サーバーの状態から、同じ鍵のまま最後まで終わり、失効や改ざんとして扱わないことを確かめる。

use serde_json::{Value, json};
use zoo_crypto::{
    BACKUP_ALGORITHM, BackupDecryptionKey, CrossSigningKeys, CryptoError, DeviceAccount, KeySlot,
    OwnKeys, OwnSecrets, RecoveryKey, SecretStorage, ServerKeyState, SignatureUpload, SsssKey,
    StartupCheck, plan_resume, startup_check,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const DEVICE: &str = "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const OTHER_DEVICE: &str = "4d8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";

fn secrets() -> OwnSecrets {
    OwnSecrets {
        cross_signing: CrossSigningKeys::generate(ALICE).unwrap(),
        backup_key: BackupDecryptionKey::generate(),
    }
}

/// サーバーを模した状態。要求を受け取ったら、その分を反映する。
#[derive(Default)]
struct Server {
    master: Option<Value>,
    self_signing: Option<Value>,
    user_signing: Option<Value>,
    signatures: Vec<Value>,
    versions: Vec<Value>,
}

impl Server {
    fn keys_query(&self) -> Value {
        let mut value = json!({
            "device_keys": {},
            "master_keys": {},
            "self_signing_keys": {},
            "user_signing_keys": {},
            "signatures": self.signatures,
        });
        for (field, key) in [
            ("master_keys", &self.master),
            ("self_signing_keys", &self.self_signing),
            ("user_signing_keys", &self.user_signing),
        ] {
            if let Some(key) = key {
                value[field][ALICE] = key.clone();
            }
        }
        value
    }

    fn state(&self) -> ServerKeyState {
        ServerKeyState::from_json(ALICE, &self.keys_query(), &json!({ "versions": self.versions }))
            .unwrap()
    }

    fn apply_cross_signing(&mut self, body: &Value) {
        for (field, slot) in [
            ("master_key", &mut self.master),
            ("self_signing_key", &mut self.self_signing),
            ("user_signing_key", &mut self.user_signing),
        ] {
            if let Some(key) = body.get(field) {
                *slot = Some(key.clone());
            }
        }
    }

    fn apply_signature(&mut self, upload: &SignatureUpload) {
        self.signatures.push(json!({
            "signer_user_id": ALICE,
            "signer_key_id": upload.signer_key_id,
            "target_user_id": upload.target_user_id,
            "target_key_id": upload.target_key_id,
            "signature": upload.signature,
        }));
    }

    fn apply_version(&mut self, body: &Value) {
        let number = (self.versions.len() + 1).to_string();
        self.versions.push(json!({
            "version": number,
            "algorithm": body["algorithm"],
            "auth_data": body["auth_data"],
            "deleted": false,
        }));
    }
}

/// 計画をすべて送る。
fn run(server: &mut Server, secrets: &OwnSecrets, device_keys: &Value) {
    let plan = plan_resume(secrets, &server.state(), device_keys).unwrap();
    if let Some(body) = &plan.cross_signing {
        server.apply_cross_signing(body);
    }
    if let Some(signature) = &plan.device_signature {
        server.apply_signature(signature);
    }
    if let Some(body) = &plan.backup_version {
        server.apply_version(body);
    }
}

fn device_keys(device_id: &str) -> Value {
    DeviceAccount::new(ALICE, device_id).unwrap().device_keys()
}

#[test]
fn first_setup_uploads_everything_in_order() {
    let secrets = secrets();
    let device = device_keys(DEVICE);
    let plan = plan_resume(&secrets, &ServerKeyState::default(), &device).unwrap();
    assert!(plan.requires_reauth());
    let body = plan.cross_signing.as_ref().unwrap();
    assert_eq!(
        body["expected_keys"],
        json!({"master": null, "self_signing": null, "user_signing": null})
    );
    assert!(body.get("master_key").is_some());
    assert!(body.get("self_signing_key").is_some());
    assert!(body.get("user_signing_key").is_some());
    assert_eq!(plan.device_signature.as_ref().unwrap().target_key_id, DEVICE);
    let version = plan.backup_version.as_ref().unwrap();
    assert_eq!(version["algorithm"], BACKUP_ALGORITHM);
    assert_eq!(version["expected_current_version"], Value::Null);

    let mut server = Server::default();
    run(&mut server, &secrets, &device);
    let after = plan_resume(&secrets, &server.state(), &device).unwrap();
    assert!(after.is_empty(), "{after:?}");
    assert_eq!(after.current_backup_version.as_deref(), Some("1"));
}

/// 各保存の直前と直後で止めても、同じ鍵のまま最後まで終わる。
#[test]
fn resumes_from_every_stop_without_mismatch() {
    let device = device_keys(DEVICE);
    // 0: SSSS の保存の直後（クロス署名鍵の前）、1: クロス署名鍵の後、2: 署名の後、3: 版の後
    for stop in 0..=3 {
        let secrets = secrets();
        let mut server = Server::default();
        let plan = plan_resume(&secrets, &ServerKeyState::default(), &device).unwrap();
        if stop >= 1 {
            server.apply_cross_signing(plan.cross_signing.as_ref().unwrap());
        }
        if stop >= 2 {
            server.apply_signature(plan.device_signature.as_ref().unwrap());
        }
        if stop >= 3 {
            server.apply_version(plan.backup_version.as_ref().unwrap());
        }
        // 再起動して解錠した端末（止まった端末自身）
        run(&mut server, &secrets, &device);
        assert!(plan_resume(&secrets, &server.state(), &device).unwrap().is_empty());
        // 別の端末で解錠しても、署名だけを足して終わる
        let other = device_keys(OTHER_DEVICE);
        let plan = plan_resume(&secrets, &server.state(), &other).unwrap();
        assert!(plan.cross_signing.is_none() && plan.backup_version.is_none());
        assert_eq!(plan.device_signature.unwrap().target_key_id, OTHER_DEVICE);
    }
}

/// クロス署名鍵を一部だけ保存した状態（マスター鍵だけ）からも、ない鍵だけを送って続ける。
#[test]
fn uploads_only_missing_subkeys() {
    let secrets = secrets();
    let device = device_keys(DEVICE);
    let mut server = Server {
        master: Some(secrets.cross_signing.master.public_key_json()),
        ..Server::default()
    };
    let plan = plan_resume(&secrets, &server.state(), &device).unwrap();
    let body = plan.cross_signing.unwrap();
    assert!(body.get("master_key").is_none());
    assert_eq!(
        body["expected_keys"]["master"],
        secrets.cross_signing.master.public_key().to_base64()
    );
    assert_eq!(body["expected_keys"]["self_signing"], Value::Null);
    server.apply_cross_signing(&body);
    run(&mut server, &secrets, &device);
    assert!(plan_resume(&secrets, &server.state(), &device).unwrap().is_empty());
}

#[test]
fn different_server_keys_are_a_mismatch() {
    let secrets = secrets();
    let device = device_keys(DEVICE);
    let other = CrossSigningKeys::generate(ALICE).unwrap();
    let server = Server {
        master: Some(other.master.public_key_json()),
        ..Server::default()
    };
    assert!(matches!(
        plan_resume(&secrets, &server.state(), &device),
        Err(CryptoError::ServerKeyMismatch(_))
    ));

    // 版の公開鍵が違う
    let mut server = Server::default();
    run(&mut server, &secrets, &device);
    let replaced = OwnSecrets {
        cross_signing: CrossSigningKeys::generate(ALICE).unwrap(),
        backup_key: BackupDecryptionKey::generate(),
    };
    assert!(matches!(
        plan_resume(&replaced, &server.state(), &device),
        Err(CryptoError::ServerKeyMismatch(_))
    ));
    let same_keys = OwnSecrets {
        cross_signing: CrossSigningKeys {
            master: zoo_crypto::CrossSigningKeyPair::from_seed(
                ALICE,
                zoo_crypto::KeyUsage::Master,
                &secrets.cross_signing.master.seed(),
            )
            .unwrap(),
            self_signing: zoo_crypto::CrossSigningKeyPair::from_seed(
                ALICE,
                zoo_crypto::KeyUsage::SelfSigning,
                &secrets.cross_signing.self_signing.seed(),
            )
            .unwrap(),
            user_signing: zoo_crypto::CrossSigningKeyPair::from_seed(
                ALICE,
                zoo_crypto::KeyUsage::UserSigning,
                &secrets.cross_signing.user_signing.seed(),
            )
            .unwrap(),
        },
        backup_key: BackupDecryptionKey::generate(),
    };
    assert!(matches!(
        plan_resume(&same_keys, &server.state(), &device),
        Err(CryptoError::ServerKeyMismatch(_))
    ));

    // 今の版を消してある
    server.versions[0]["deleted"] = json!(true);
    assert!(matches!(
        plan_resume(&secrets, &server.state(), &device),
        Err(CryptoError::ServerKeyMismatch(_))
    ));
}

#[test]
fn version_without_cross_signing_keys_is_a_mismatch() {
    let secrets = secrets();
    let device = device_keys(DEVICE);
    let mut server = Server::default();
    let plan = plan_resume(&secrets, &server.state(), &device).unwrap();
    server.apply_version(plan.backup_version.as_ref().unwrap());
    assert!(matches!(
        plan_resume(&secrets, &server.state(), &device),
        Err(CryptoError::ServerKeyMismatch(_))
    ));
}

/// SSSS に入れて解錠し、取り出した鍵で続きを決める（初回設定で作った鍵と同じになる）。
#[test]
fn secrets_round_trip_through_secret_storage() {
    let secrets = secrets();
    let ssss = SsssKey::generate();
    let recovery = RecoveryKey::generate();
    let slot = KeySlot::new_recovery(ALICE, &ssss, &recovery).unwrap();
    let request = ssss
        .keys_request(&zoo_crypto::SecretStorageKeysRequest {
            expected_default_key_id: None,
            expected_secrets_revision: None,
            cross_signing: &secrets.cross_signing,
            backup_key: &secrets.backup_key,
            previous_backup_keys: &[],
            slots: std::slice::from_ref(&slot),
        })
        .unwrap();
    let mut secrets_with_key = request["secrets"].clone();
    for value in secrets_with_key.as_object_mut().unwrap().values_mut() {
        value["key_id"] = json!(ssss.key_id());
    }
    let response = json!({
        "default_key_id": ssss.key_id(),
        "keys": { ssss.key_id(): request["key"] },
        "secrets": secrets_with_key,
        "secrets_revision": 1,
        "slots": request["slots"],
    });
    let storage = SecretStorage::from_json(ALICE, &response).unwrap().unwrap();
    let unlocked = storage.unlock_with_recovery_key(&recovery).unwrap();
    let opened = OwnSecrets::from_unlocked(&unlocked).unwrap();
    assert_eq!(
        opened.cross_signing.master.public_key(),
        secrets.cross_signing.master.public_key()
    );
    assert_eq!(
        opened.backup_key.public_key(),
        secrets.backup_key.public_key()
    );
}

#[test]
fn own_keys_round_trip_and_startup_check() {
    let secrets = secrets();
    let account = DeviceAccount::new(ALICE, DEVICE).unwrap();
    let storage_key = [7u8; 32];
    let pickle = secrets.own_keys().pickle(&account, &storage_key);
    assert!(!pickle.contains(&secrets.backup_key.to_base64().to_string()));
    let own = OwnKeys::from_pickle(&pickle, &account, &storage_key).unwrap();
    assert_eq!(
        own.self_signing.public_key(),
        secrets.cross_signing.self_signing.public_key()
    );
    assert!(OwnKeys::from_pickle(&pickle, &account, &[8u8; 32]).is_err());

    let device = account.device_keys();
    let mut server = Server::default();
    assert!(matches!(
        startup_check(&own, &server.state(), &device).unwrap(),
        StartupCheck::ResumeSetup
    ));
    let plan = plan_resume(&secrets, &server.state(), &device).unwrap();
    server.apply_cross_signing(plan.cross_signing.as_ref().unwrap());
    server.apply_version(plan.backup_version.as_ref().unwrap());
    // 署名だけがない（失効の途中で解錠した端末など）
    let StartupCheck::SignDevice(upload) = startup_check(&own, &server.state(), &device).unwrap()
    else {
        panic!("署名し直すはず");
    };
    server.apply_signature(&upload);
    assert!(matches!(
        startup_check(&own, &server.state(), &device).unwrap(),
        StartupCheck::Ready
    ));
    // サーバーの鍵が変わった（失効のあと）
    server.self_signing = Some(
        CrossSigningKeys::generate(ALICE)
            .unwrap()
            .self_signing
            .public_key_json(),
    );
    assert!(matches!(
        startup_check(&own, &server.state(), &device).unwrap(),
        StartupCheck::Unlock
    ));
}
