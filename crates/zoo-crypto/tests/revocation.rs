//! 失効の手順（docs/e2ee.md の「失効の手順」、#362）。
//!
//! 署名し直す相手の絞り込み（手順 4 の【決定】）と、保存の要求、再開、読み直しの確認を、
//! サーバーを模した状態で確かめる。本物のサーバーを通した結合テストは
//! `backend/tests/e2ee_revocation_test.rs`。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use zoo_crypto::{
    BackedUpSessionData, BackupDecryptionKey, BackupEncryptionKey, BackupRow, CrossSigningKeyPair,
    CrossSigningKeys, CryptoError, DeviceAccount, KeySignature, KeySlot, KeyUsage, OwnSecrets,
    RecoveryKey, ResignNote, Revocation, RevocationInput, RevocationScope, RevocationStage,
    RevocationState, SecretStorage, SenderClaimedKeys, ServerKeyState, SsssKey, TrustStore,
    UnlockedSecretStorage,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const BOB: &str = "00000000-0000-0000-0000-00000000000b";
const CAROL: &str = "00000000-0000-0000-0000-00000000000c";
const DAVE: &str = "00000000-0000-0000-0000-00000000000d";
const DEVICE: &str = "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const CHANNEL: &str = "6c2e1a8a-5b7d-4e9f-a1b2-c3d4e5f60718";

fn master(user: &str) -> CrossSigningKeyPair {
    CrossSigningKeyPair::generate(user, KeyUsage::Master).unwrap()
}

/// SSSS を模したサーバー（`GET /api/e2ee/secret_storage` の応答を持つ）。
struct Storage {
    response: Value,
}

impl Storage {
    fn create(request: &Value) -> Self {
        let mut storage = Self {
            response: json!({ "secrets_revision": 0 }),
        };
        storage.apply_keys(request);
        storage
    }

    fn revision(&self) -> u64 {
        self.response["secrets_revision"].as_u64().unwrap()
    }

    fn apply_keys(&mut self, request: &Value) {
        let key_id = request["key_id"].as_str().unwrap();
        let mut secrets = request["secrets"].clone();
        for secret in secrets.as_object_mut().unwrap().values_mut() {
            secret["key_id"] = json!(key_id);
        }
        self.response = json!({
            "default_key_id": key_id,
            "keys": { key_id: request["key"] },
            "secrets": secrets,
            "secrets_revision": self.revision() + 1,
            "slots": request["slots"],
        });
    }

    fn apply_secrets(&mut self, request: &Value) {
        assert_eq!(request["key_id"], self.response["default_key_id"]);
        assert_eq!(
            request["expected_secrets_revision"],
            self.response["secrets_revision"]
        );
        for (name, secret) in request["secrets"].as_object().unwrap() {
            let mut secret = secret.clone();
            secret["key_id"] = request["key_id"].clone();
            self.response["secrets"][name] = secret;
        }
        self.response["secrets_revision"] = json!(self.revision() + 1);
    }

    fn unlock(&self, recovery: &RecoveryKey) -> UnlockedSecretStorage {
        SecretStorage::from_json(ALICE, &self.response)
            .unwrap()
            .unwrap()
            .unlock_with_recovery_key(recovery)
            .unwrap()
    }
}

/// 初回設定を済ませたアリス（SSSS、サーバーの鍵、版 1）。
struct Alice {
    storage: Storage,
    recovery: RecoveryKey,
    keys_query: Value,
    versions: Vec<Value>,
}

impl Alice {
    fn new() -> (Self, OwnSecrets) {
        let secrets = OwnSecrets {
            cross_signing: CrossSigningKeys::generate(ALICE).unwrap(),
            backup_key: BackupDecryptionKey::generate(),
        };
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
                slots: &[slot],
            })
            .unwrap();
        let mut alice = Self {
            storage: Storage::create(&request),
            recovery,
            keys_query: json!({ "signatures": [] }),
            versions: Vec::new(),
        };
        alice.apply_cross_signing(&json!({
            "master_key": secrets.cross_signing.upload_json()["master_key"],
            "self_signing_key": secrets.cross_signing.upload_json()["self_signing_key"],
            "user_signing_key": secrets.cross_signing.upload_json()["user_signing_key"],
        }));
        let version = secrets
            .backup_key
            .create_version_request(&secrets.cross_signing.master, None, 1)
            .unwrap();
        alice.apply_version(&version);
        (alice, secrets)
    }

    fn apply_cross_signing(&mut self, body: &Value) {
        for (field, map) in [
            ("master_key", "master_keys"),
            ("self_signing_key", "self_signing_keys"),
            ("user_signing_key", "user_signing_keys"),
        ] {
            if let Some(key) = body.get(field) {
                self.keys_query[map][ALICE] = key.clone();
            }
        }
    }

    fn apply_version(&mut self, body: &Value) -> String {
        let number = (self.versions.len() + 1).to_string();
        // サーバーは、署名した版の番号が振る番号と違う要求を 409 で拒む。
        assert_eq!(body["auth_data"]["version"], json!(number));
        self.versions.push(json!({
            "version": number,
            "algorithm": body["algorithm"],
            "auth_data": body["auth_data"],
            "deleted": false,
        }));
        number
    }

    fn server(&self) -> ServerKeyState {
        ServerKeyState::from_json(
            ALICE,
            &self.keys_query,
            &json!({ "versions": self.versions }),
        )
        .unwrap()
    }

    fn unlock(&self) -> UnlockedSecretStorage {
        self.storage.unlock(&self.recovery)
    }
}

/// 記録を持つ失効を行う端末の記録。bob は TOFU（confirmed_usk あり）、carol は共有ピンから採用した
/// だけ（confirmed_usk なし）。
fn trust_with_records(secrets: &OwnSecrets, bob_master: &CrossSigningKeyPair) -> TrustStore {
    let mut trust = TrustStore::new(ALICE).unwrap();
    trust
        .trust_own_master_key(&secrets.cross_signing.master)
        .unwrap();
    trust
        .record_own_user_signing_key(&secrets.cross_signing.user_signing)
        .unwrap();
    // bob を受け入れて（再検証）、記録と confirmed_usk を作る。
    let accepted = trust
        .accept_master_key(
            &secrets.cross_signing.user_signing,
            BOB,
            &bob_master.public_key_json(),
        )
        .unwrap();
    // ピンと署名を保存できた。
    trust.confirm_accepted_master_key(&accepted.target_user_id, &accepted.master_key);
    trust
}

fn row(signer: &str, upload: &zoo_crypto::SignatureUpload) -> KeySignature {
    KeySignature {
        signer_user_id: signer.to_owned(),
        signer_key_id: upload.signer_key_id.clone(),
        target_user_id: upload.target_user_id.clone(),
        target_key_id: upload.target_key_id.clone(),
        signature: upload.signature.clone(),
    }
}

#[test]
fn resign_plan_keeps_only_confirmed_records() {
    let (_alice, secrets) = Alice::new();
    let usk = &secrets.cross_signing.user_signing;
    let bob = master(BOB);
    let carol = master(CAROL);
    let attacker = master(CAROL);
    let dave = master(DAVE);
    let trust = trust_with_records(&secrets, &bob);

    // carol: 古い鍵で攻撃者のマスター鍵のピンがある（記録はない）。
    // dave: 古い鍵で署名した。記録はない。
    // bob: 記録あり、confirmed_usk あり。サーバーのピンは別の鍵で署名されている（検証できない）。
    let forged_usk = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    let pins = vec![
        forged_usk.sign_tofu_pin(BOB, &bob.public_key()).unwrap(),
        usk.sign_tofu_pin(CAROL, &attacker.public_key()).unwrap(),
    ];
    let signatures = vec![
        row(
            ALICE,
            &usk.sign_master_key(BOB, &bob.public_key_json()).unwrap(),
        ),
        row(
            ALICE,
            &usk.sign_master_key(DAVE, &dave.public_key_json()).unwrap(),
        ),
        // サーバーが見せた別の鍵による署名は検証できないものとして一覧に出す。
        row(
            ALICE,
            &forged_usk
                .sign_master_key(CAROL, &carol.public_key_json())
                .unwrap(),
        ),
    ];
    let masters: BTreeMap<String, Value> = [
        (BOB.to_owned(), bob.public_key_json()),
        (CAROL.to_owned(), carol.public_key_json()),
        (DAVE.to_owned(), dave.public_key_json()),
    ]
    .into();
    let plan = zoo_crypto::ResignPlan::collect(&trust, usk, &pins, &signatures, &masters).unwrap();

    assert_eq!(plan.pins.len(), 1);
    assert_eq!(plan.pins[0].target_user_id, BOB);
    assert_eq!(plan.pins[0].master_key, bob.public_key().to_base64());
    assert_eq!(plan.master_keys.len(), 1);
    assert_eq!(plan.master_keys[0].target_user_id, BOB);
    assert_eq!(
        plan.notes[BOB].iter().copied().collect::<Vec<_>>(),
        [ResignNote::PinReplaced]
    );
    assert!(plan.notes[CAROL].contains(&ResignNote::NoRecord));
    assert!(plan.notes[CAROL].contains(&ResignNote::SignatureUnverified));
    assert!(plan.notes[DAVE].contains(&ResignNote::NoRecord));

    // 署名し直したピンは新しい鍵で検証できる。
    let new_usk = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    let (pins, signatures) = plan.sign(&new_usk).unwrap();
    assert_eq!(pins[0]["master_key"], bob.public_key().to_base64());
    assert_eq!(signatures[0].signer_key_id, new_usk.key_id());
}

#[test]
fn records_without_confirmed_usk_and_mismatched_signatures_are_not_resigned() {
    let (_alice, secrets) = Alice::new();
    let usk = &secrets.cross_signing.user_signing;
    let bob = master(BOB);
    let other_bob = master(BOB);
    let carol = master(CAROL);
    let mut trust = trust_with_records(&secrets, &bob);
    // carol の共有ピン（今のマスター鍵と違う）を採用し、confirmed_usk のない記録を作る。
    let shared = usk
        .sign_tofu_pin(CAROL, &master(CAROL).public_key())
        .unwrap();
    let snapshot = zoo_crypto::KeySnapshot::from_json(&json!({
        "user_ids": [CAROL],
        "master_keys": { CAROL: carol.public_key_json() },
        "tofu_pins": { CAROL: shared },
    }))
    .unwrap();
    let mut snapshot = snapshot;
    snapshot.master_keys.insert(
        ALICE.to_owned(),
        secrets.cross_signing.master.public_key_json(),
    );
    snapshot.user_signing_keys.insert(
        ALICE.to_owned(),
        secrets
            .cross_signing
            .master
            .sign_subkey(&secrets.cross_signing.user_signing)
            .unwrap(),
    );
    trust.evaluate(&snapshot, DEVICE, Some(usk));
    assert_eq!(trust.record(CAROL).unwrap().confirmed_usk, None);

    // bob への署名が、記録と違うマスター鍵へのもの。
    let signatures = vec![row(
        ALICE,
        &usk.sign_master_key(BOB, &other_bob.public_key_json())
            .unwrap(),
    )];
    let masters: BTreeMap<String, Value> = [(BOB.to_owned(), other_bob.public_key_json())].into();
    let plan = zoo_crypto::ResignPlan::collect(&trust, usk, &[], &signatures, &masters).unwrap();
    assert_eq!(plan.pins.len(), 1, "carol のピンは作り直さない");
    assert!(plan.master_keys.is_empty());
    assert!(plan.notes[BOB].contains(&ResignNote::MasterKeyMismatch));
    assert!(plan.notes[CAROL].contains(&ResignNote::NotConfirmed));
}

fn begin(alice: &Alice, secrets: &OwnSecrets, scope: RevocationScope) -> Revocation {
    let trust = trust_with_records(secrets, &master(BOB));
    Revocation::begin(RevocationInput {
        scope,
        unlocked: &alice.unlock(),
        trust: &trust,
        server: &alice.server(),
        pins: &[],
        signatures: &[],
        master_keys: &BTreeMap::new(),
        local_backup_key: Some(&secrets.backup_key),
        old_user_signing: Some(&secrets.cross_signing.user_signing),
        deleted_devices: vec![DEVICE.to_owned()],
    })
    .unwrap()
}

/// 保存の要求を順にサーバーへ反映する。
fn save_all(alice: &mut Alice, revocation: &mut Revocation, slots: &[KeySlot]) {
    let request = revocation.secret_storage_request(slots).unwrap();
    match revocation.state().scope {
        RevocationScope::Full => alice.storage.apply_keys(&request),
        RevocationScope::DeviceOnly => alice.storage.apply_secrets(&request),
    }
    revocation.secret_storage_saved(Some(alice.storage.revision()));
    let body = revocation.cross_signing_request();
    assert_eq!(
        body["expected_keys"]["master"],
        alice.keys_query["master_keys"][ALICE]["keys"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone()
    );
    // 保存した SSSS の既定の鍵を前提にする（別の端末が SSSS だけを作り直したら 409）。
    let default_key_id = json!(revocation.state().key_id);
    assert_eq!(body["expected_default_key_id"], default_key_id);
    assert_eq!(
        body["expected_secrets_revision"],
        json!(alice.storage.revision())
    );
    alice.apply_cross_signing(&body);
    revocation.advance(RevocationStage::CrossSigningSaved);
    let device = DeviceAccount::new(ALICE, DEVICE).unwrap().device_keys();
    let signatures = revocation.signatures(&device).unwrap();
    assert_eq!(signatures.pins.len(), 1);
    revocation.advance(RevocationStage::SignaturesSaved);
    let version_body = revocation.backup_version_request().unwrap();
    assert_eq!(version_body["expected_current_version"], json!("1"));
    assert_eq!(version_body["expected_default_key_id"], default_key_id);
    assert_eq!(
        version_body["expected_secrets_revision"],
        json!(alice.storage.revision())
    );
    let version = alice.apply_version(&version_body);
    revocation.backup_created(version);
}

#[test]
fn device_only_revocation_keeps_master_key_and_moves_previous_backup_key() {
    let (mut alice, secrets) = Alice::new();
    let mut revocation = begin(&alice, &secrets, RevocationScope::DeviceOnly);
    let request = revocation.secret_storage_request(&[]).unwrap();
    let names: Vec<_> = request["secrets"].as_object().unwrap().keys().collect();
    assert_eq!(
        names,
        [
            "m.cross_signing.self_signing",
            "m.cross_signing.user_signing",
            "m.megolm_backup.v1",
            "zoo.megolm_backup.previous",
        ]
    );
    assert!(
        revocation
            .cross_signing_request()
            .get("master_key")
            .is_none()
    );
    save_all(&mut alice, &mut revocation, &[]);

    // SSSS には新しい鍵と、退避した古いバックアップ鍵がある。マスター鍵は変えない。
    let unlocked = alice.unlock();
    let stored = OwnSecrets::from_unlocked(&unlocked).unwrap();
    assert_eq!(
        stored.cross_signing.master.public_key(),
        secrets.cross_signing.master.public_key()
    );
    assert_ne!(
        stored.cross_signing.self_signing.public_key(),
        secrets.cross_signing.self_signing.public_key()
    );
    let previous = unlocked.previous_backup_keys().unwrap();
    assert_eq!(previous.len(), 1);
    assert_eq!(previous[0].public_key(), secrets.backup_key.public_key());

    // 古い版だけを移す。
    let versions = zoo_crypto::BackupVersionInfo::list_from_json(&json!({
        "versions": alice.versions
    }))
    .unwrap();
    let migrate = revocation.versions_to_migrate(&versions);
    assert_eq!(migrate.len(), 1);
    assert_eq!(migrate[0].0, "1");

    // 読み直しで一致を確かめる。
    let current = alice.versions.last().unwrap().clone();
    revocation
        .verify(&alice.storage.response, &alice.keys_query, &current)
        .unwrap();
    // 別の端末が SSSS を書き換えたら一致しない。
    let (other, _) = Alice::new();
    assert!(matches!(
        revocation.verify(&other.storage.response, &alice.keys_query, &current),
        Err(CryptoError::ServerKeyMismatch(_))
    ));
}

#[test]
fn full_revocation_replaces_master_key_and_ssss_key() {
    let (mut alice, secrets) = Alice::new();
    let mut revocation = begin(&alice, &secrets, RevocationScope::Full);
    assert_ne!(
        revocation.ssss_key().key_id(),
        alice.unlock().key().key_id()
    );
    let recovery = RecoveryKey::generate();
    let slot = KeySlot::new_recovery(ALICE, revocation.ssss_key(), &recovery).unwrap();
    assert!(revocation.secret_storage_request(&[]).is_err());
    save_all(&mut alice, &mut revocation, &[slot]);
    assert!(
        revocation
            .cross_signing_request()
            .get("master_key")
            .is_some()
    );

    // 新しいリカバリーキーで開け直すと、作った鍵が出てくる。
    alice.recovery = recovery;
    let unlocked = alice.unlock();
    let stored = OwnSecrets::from_unlocked(&unlocked).unwrap();
    assert_ne!(
        stored.cross_signing.master.public_key(),
        secrets.cross_signing.master.public_key()
    );
    let current = alice.versions.last().unwrap().clone();
    revocation
        .verify(&alice.storage.response, &alice.keys_query, &current)
        .unwrap();

    // 保存した記録から、開け直した SSSS で続けられる。
    let state = revocation.state().clone();
    let resumed = Revocation::resume(state, &unlocked).unwrap();
    assert_eq!(resumed.state().stage, RevocationStage::BackupCreated);
}

#[test]
fn full_revocation_stopped_before_saving_ssss_restarts() {
    let (alice, secrets) = Alice::new();
    let revocation = begin(&alice, &secrets, RevocationScope::Full);
    let state = revocation.state().clone();
    assert!(matches!(
        Revocation::resume(state, &alice.unlock()),
        Err(CryptoError::RevocationRestart)
    ));

    // 端末だけの失効は、同じ SSSS 鍵で開け直して、記録した鍵で続ける。
    let revocation = begin(&alice, &secrets, RevocationScope::DeviceOnly);
    let state = revocation.state().clone();
    let resumed = Revocation::resume(state, &alice.unlock()).unwrap();
    assert_eq!(
        resumed.secrets().cross_signing.self_signing.public_key(),
        revocation.secrets().cross_signing.self_signing.public_key()
    );
}

#[test]
fn resume_rejects_ssss_written_by_another_revocation() {
    let (mut alice, secrets) = Alice::new();
    let mut first = begin(&alice, &secrets, RevocationScope::DeviceOnly);
    let second = begin(&alice, &secrets, RevocationScope::DeviceOnly);
    alice
        .storage
        .apply_secrets(&second.secret_storage_request(&[]).unwrap());
    first.secret_storage_saved(Some(alice.storage.revision()));
    assert!(matches!(
        Revocation::resume(first.state().clone(), &alice.unlock()),
        Err(CryptoError::ServerKeyMismatch(_))
    ));
}

#[test]
fn state_is_sealed_in_the_crypto_store() {
    let (alice, secrets) = Alice::new();
    let revocation = begin(&alice, &secrets, RevocationScope::DeviceOnly);
    let account = DeviceAccount::new(ALICE, DEVICE).unwrap();
    let key = [7u8; 32];
    let sealed = revocation.state().pickle(&account, &key);
    assert!(!sealed.contains(&revocation.state().new_backup_public_key));
    let restored = RevocationState::from_pickle(&sealed, &account, &key).unwrap();
    assert_eq!(&restored, revocation.state());
    assert_eq!(restored.deleted_devices, [DEVICE]);
    assert!(RevocationState::from_pickle(&sealed, &account, &[8u8; 32]).is_err());
}

#[test]
fn previous_backup_rows_are_reencrypted_for_the_new_version() {
    let (alice, secrets) = Alice::new();
    let mut revocation = begin(&alice, &secrets, RevocationScope::DeviceOnly);
    let sender = DeviceAccount::new(BOB, DEVICE).unwrap().identity_keys();
    let sender_key = sender.curve25519.to_base64();
    let data = BackedUpSessionData {
        algorithm: "m.megolm.v1.aes-sha2".into(),
        channel_id: CHANNEL.into(),
        epoch: 3,
        session_id: "session".into(),
        session_key: "key".into(),
        sender_key: sender_key.clone(),
        sender_claimed_keys: SenderClaimedKeys {
            ed25519: sender.ed25519.to_base64(),
        },
        sender_user_id: BOB.into(),
        sender_device_id: DEVICE.into(),
    };
    let old = BackupEncryptionKey::new(secrets.backup_key.public_key());
    let good = BackupRow {
        channel_id: CHANNEL.into(),
        sender_key,
        session_id: "session".into(),
        first_message_index: 4,
        forwarded_count: 0,
        is_verified: true,
        session_data: old.encrypt(&data).unwrap(),
    };
    let mut moved = good.clone();
    moved.session_id = "other".into();
    let (rows, dropped) = revocation.reencrypt_rows(&secrets.backup_key, &[good, moved]);
    assert_eq!(dropped, 1, "行と平文が合わない行は捨てる");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].is_verified);
    assert_eq!(rows[0].first_message_index, 4);
    let (_, _, new_key) = revocation.state().new_subkeys().unwrap();
    let decrypted = new_key.decrypt(&rows[0].session_data).unwrap();
    assert_eq!(decrypted.sender_user_id, BOB);
    assert!(secrets.backup_key.decrypt(&rows[0].session_data).is_err());

    // 捨てた数は版ごとに記録し、同じ版を数え直しても二重に数えない。
    revocation.record_dropped("1", dropped);
    revocation.record_dropped("1", dropped);
    revocation.record_dropped("2", 2);
    assert_eq!(revocation.state().dropped_total(), 3);
}

#[test]
fn revoking_device_updates_its_own_trust_records() {
    let (_alice, secrets) = Alice::new();
    let bob = master(BOB);
    let mut trust = trust_with_records(&secrets, &bob);
    let new_usk = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    let plan = zoo_crypto::ResignPlan::collect(
        &trust,
        &secrets.cross_signing.user_signing,
        &[],
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    trust
        .apply_revocation(&secrets.cross_signing.master, &new_usk, &plan)
        .unwrap();
    assert_eq!(
        trust.record(BOB).unwrap().confirmed_usk.as_deref(),
        Some(new_usk.public_key().to_base64().as_str())
    );
    assert_eq!(trust.user_signing_key_history().len(), 2);
    // 古い鍵には戻せない。
    assert!(matches!(
        trust.record_own_user_signing_key(&secrets.cross_signing.user_signing),
        Err(CryptoError::UserSigningKeyRollback(_))
    ));
}

/// 記録の JSON を書き換えた暗号ストア（登録待ちや、古いユーザー署名鍵で確認した記録を作る）。
fn trust_with_patched_record(
    secrets: &OwnSecrets,
    bob: &CrossSigningKeyPair,
    patch: impl FnOnce(&mut Value),
) -> TrustStore {
    let mut json: Value =
        serde_json::from_str(&trust_with_records(secrets, bob).to_json()).unwrap();
    patch(&mut json["records"][BOB]);
    TrustStore::from_json(&json.to_string(), ALICE).unwrap()
}

#[test]
fn pending_pins_are_not_resigned() {
    let (_alice, secrets) = Alice::new();
    let bob = master(BOB);
    // TOFU のピンの登録に失敗した記録（confirmed_usk があり、登録待ち）。
    let mut trust = trust_with_patched_record(&secrets, &bob, |r| r["pin_pending"] = json!(true));
    let before = trust.record(BOB).unwrap().clone();
    let new_usk = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    let plan = zoo_crypto::ResignPlan::collect(
        &trust,
        &secrets.cross_signing.user_signing,
        &[],
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    // 別の端末のピンを上書きしないよう、署名し直さずに一覧に出す。
    assert!(plan.pins.is_empty());
    assert!(plan.notes[BOB].contains(&ResignNote::PinPending));
    trust
        .apply_revocation(&secrets.cross_signing.master, &new_usk, &plan)
        .unwrap();
    // 記録は変えない（新しいユーザー署名鍵とは違うので、次の判定で再検証になる）。
    assert_eq!(trust.record(BOB).unwrap(), &before);
}

#[test]
fn records_confirmed_with_an_older_user_signing_key_are_not_resigned() {
    let (_alice, secrets) = Alice::new();
    let bob = master(BOB);
    let older = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    // 別の端末の失効の前のユーザー署名鍵で確認した記録（今は再検証が必要）。
    let trust = trust_with_patched_record(&secrets, &bob, |r| {
        r["confirmed_usk"] = json!(older.public_key().to_base64())
    });
    let plan = zoo_crypto::ResignPlan::collect(
        &trust,
        &secrets.cross_signing.user_signing,
        &[],
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(plan.pins.is_empty());
    assert!(plan.notes[BOB].contains(&ResignNote::StaleConfirmation));
}

#[test]
fn records_not_confirmed_with_the_servers_user_signing_key_are_excluded() {
    let (_alice, secrets) = Alice::new();
    let bob = master(BOB);
    let trust = trust_with_records(&secrets, &bob);
    let mut plan = zoo_crypto::ResignPlan::collect(
        &trust,
        &secrets.cross_signing.user_signing,
        &[],
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(plan.pins.len(), 1);
    // 暗号ストアの鍵はサーバーより古い（別の端末の失効のあとで解錠し直していない）。
    let server_usk = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    plan.exclude_stale_confirmations(&trust, Some(&server_usk.public_key().to_base64()));
    assert!(plan.pins.is_empty());
    assert!(plan.notes[BOB].contains(&ResignNote::StaleConfirmation));
}

#[test]
fn master_keys_replaced_before_signing_are_dropped() {
    let bob = master(BOB);
    let carol = master(CAROL);
    let mut plan = zoo_crypto::ResignPlan {
        master_keys: vec![
            zoo_crypto::ResignMasterKey {
                target_user_id: BOB.into(),
                master_key: bob.public_key_json(),
            },
            zoo_crypto::ResignMasterKey {
                target_user_id: CAROL.into(),
                master_key: carol.public_key_json(),
            },
        ],
        ..Default::default()
    };
    // bob はマスター鍵を作り直した。carol は変わらない。
    let current: BTreeMap<String, Value> = [
        (BOB.to_owned(), master(BOB).public_key_json()),
        (CAROL.to_owned(), carol.public_key_json()),
    ]
    .into();
    assert_eq!(plan.drop_stale_master_keys(&current), [BOB.to_owned()]);
    assert_eq!(plan.master_keys.len(), 1);
    assert_eq!(plan.master_keys[0].target_user_id, CAROL);
    assert!(plan.notes[BOB].contains(&ResignNote::MasterKeyMismatch));
    // 問い合わせに出てこない相手（消えた相手）も外す。
    assert_eq!(
        plan.drop_stale_master_keys(&BTreeMap::new()),
        [CAROL.to_owned()]
    );
}
