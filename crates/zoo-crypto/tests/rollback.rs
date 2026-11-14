//! 巻き戻しへの対策（docs/e2ee.md の脅威モデルの「相手のマスター鍵の巻き戻し」と
//! 「バックアップ鍵の巻き戻し」）。
//!
//! 2026-11-15 のセキュリティレビューの再現テスト（`rule3_rolls_back_to_previously_signed_master_key`、
//! `new_device_adopts_replayed_old_pin`、`backup_version_number_is_not_signed`）を元にし、直したあとは
//! 攻撃が通らないことを確かめる形にした。新しい端末（記録がない端末）については設計上の限界なので、
//! 限界がどこにあるかを確かめる形にした。

use serde_json::{Value, json};
use zoo_crypto::{
    BACKUP_ALGORITHM, BackupDecryptionKey, BackupUploadState, BackupVersionInfo, CrossSigningKeys,
    CryptoError, DeviceAccount, KeySnapshot, ReverificationReason, SignatureUpload, TrustBasis,
    TrustStore, UserState, VersionChange, next_backup_version, verify_backup_version,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const BOB: &str = "00000000-0000-0000-0000-00000000000b";
const ALICE_DEVICE: &str = "0a000001-5b7d-4e9f-a1b2-c3d4e5f60718";
const BOB_DEVICE: &str = "0b000001-5b7d-4e9f-a1b2-c3d4e5f60718";

fn row(signer: &str, s: &SignatureUpload) -> Value {
    json!({
        "signer_user_id": signer,
        "signer_key_id": s.signer_key_id,
        "target_user_id": s.target_user_id,
        "target_key_id": s.target_key_id,
        "signature": s.signature,
    })
}

/// Alice から見たスナップショット。Bob のマスター鍵は `bob`、端末は `bob_device`（`bob` の自己署名鍵で署名）。
fn snapshot(
    alice: &CrossSigningKeys,
    bob: &CrossSigningKeys,
    bob_device: &DeviceAccount,
    signatures: Vec<Value>,
    pin: Option<&Value>,
) -> KeySnapshot {
    let mut device_keys = bob_device.device_keys();
    let signed = bob.self_signing.sign_device(&device_keys).unwrap();
    device_keys["signatures"][BOB][bob.self_signing.key_id()] = json!(signed.signature);
    let mut value = json!({
        "user_ids": [ALICE, BOB],
        "device_keys": { BOB: { BOB_DEVICE: device_keys } },
        "master_keys": {
            ALICE: alice.master.public_key_json(),
            BOB: bob.master.public_key_json(),
        },
        "self_signing_keys": { BOB: bob.master.sign_subkey(&bob.self_signing).unwrap() },
        "user_signing_keys": { ALICE: alice.master.sign_subkey(&alice.user_signing).unwrap() },
        "signatures": signatures,
        "tofu_pins": {},
    });
    if let Some(pin) = pin {
        value["tofu_pins"][BOB] = pin.clone();
    }
    KeySnapshot::from_json(&value).unwrap()
}

fn alice_store(alice: &CrossSigningKeys) -> TrustStore {
    let mut store = TrustStore::new(ALICE).unwrap();
    store.trust_own_master_key(&alice.master).unwrap();
    store
        .record_own_user_signing_key(&alice.user_signing)
        .unwrap();
    store
}

fn b64(keys: &CrossSigningKeys) -> String {
    keys.master.public_key().to_base64()
}

/// Alice が Bob の M1 を受け入れ、Bob が失効で M2 に作り直し、Alice が M2 を受け入れた状態。
/// M1 への署名とピン（サーバーが残しておける古いもの）を返す。
struct Rotated {
    alice: CrossSigningKeys,
    bob_old: CrossSigningKeys,
    bob_new: CrossSigningKeys,
    store: TrustStore,
    old_signature: Value,
    old_pin: Value,
}

fn rotated(confirm_new: bool) -> Rotated {
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let bob_old = CrossSigningKeys::generate(BOB).unwrap();
    let bob_new = CrossSigningKeys::generate(BOB).unwrap();
    let mut store = alice_store(&alice);
    let first = store
        .accept_master_key(&alice.user_signing, BOB, &bob_old.master.public_key_json())
        .unwrap();
    store.confirm_accepted_master_key(BOB, &first.master_key);
    let second = store
        .accept_master_key(&alice.user_signing, BOB, &bob_new.master.public_key_json())
        .unwrap();
    if confirm_new {
        store.confirm_accepted_master_key(BOB, &second.master_key);
    }
    let record = store.record(BOB).unwrap();
    assert_eq!(record.master_key, b64(&bob_new));
    assert_eq!(record.replaced_master_keys, vec![b64(&bob_old)]);
    Rotated {
        alice,
        bob_old,
        bob_new,
        store,
        old_signature: row(ALICE, &first.signature),
        old_pin: first.pin,
    }
}

/// レビューの指摘 1（High）：古いマスター鍵と、昔それに付けた署名を見せられても、受け入れた新しい鍵の
/// 記録を巻き戻さず、攻撃者の端末に配らない。
#[test]
fn rule3_does_not_roll_back_to_previously_signed_master_key() {
    let Rotated {
        alice,
        bob_old,
        bob_new,
        mut store,
        old_signature,
        old_pin,
    } = rotated(true);
    let attacker = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();
    let snap = snapshot(
        &alice,
        &bob_old,
        &attacker,
        vec![old_signature],
        Some(&old_pin),
    );
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(
        out.users[BOB].state,
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert!(out.devices.is_empty());
    assert!(out.signed_master_key_changes.is_empty());
    let record = store.record(BOB).unwrap();
    assert_eq!(record.master_key, b64(&bob_new));
    assert_eq!(record.replaced_master_keys, vec![b64(&bob_old)]);
}

/// 受け入れた新しい鍵のピンをまだ置けていない（登録待ち）ときに、古い鍵のピンを見せられても採用しない
/// （以前は、別の端末のピンとして記録を消し、共有ピンとして古い鍵を信頼した）。
#[test]
fn pending_acceptance_does_not_adopt_replayed_old_pin() {
    let Rotated {
        alice,
        bob_old,
        bob_new,
        mut store,
        old_pin,
        ..
    } = rotated(false);
    let attacker = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();
    let snap = snapshot(&alice, &bob_old, &attacker, vec![], Some(&old_pin));
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(
        out.users[BOB].state,
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert!(out.devices.is_empty());
    let record = store.record(BOB).unwrap();
    assert_eq!(record.master_key, b64(&bob_new));
    assert!(record.pin_pending);

    // 409 のあとの discard_tofu_pin でも、履歴のある記録は消さない。
    let pin = zoo_crypto::NewTofuPin {
        target_user_id: BOB.to_owned(),
        master_key: b64(&bob_new),
        pin: Value::Null,
    };
    store.discard_tofu_pin(&pin);
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(
        out.users[BOB].state,
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert_eq!(store.record(BOB).unwrap().master_key, b64(&bob_new));
}

/// 自分の別の端末が受け入れた新しい鍵（履歴にない鍵）は、規則 3 で信頼するが、黙っては置き換えない。
/// 置き換えた鍵は履歴に移り、そのあと古い鍵には戻らない。
#[test]
fn rule3_reports_master_key_accepted_on_another_device() {
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let bob_old = CrossSigningKeys::generate(BOB).unwrap();
    let bob_new = CrossSigningKeys::generate(BOB).unwrap();
    let bob_device = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();

    // この端末（store）は M1 を受け入れた。別の端末（other）が M2 を受け入れた。
    let mut store = alice_store(&alice);
    let first = store
        .accept_master_key(&alice.user_signing, BOB, &bob_old.master.public_key_json())
        .unwrap();
    store.confirm_accepted_master_key(BOB, &first.master_key);
    let mut other = alice_store(&alice);
    let second = other
        .accept_master_key(&alice.user_signing, BOB, &bob_new.master.public_key_json())
        .unwrap();

    let snap = snapshot(
        &alice,
        &bob_new,
        &bob_device,
        vec![row(ALICE, &second.signature)],
        Some(&second.pin),
    );
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(
        out.users[BOB].state,
        UserState::Trusted(TrustBasis::UserSigningSignature)
    );
    assert_eq!(out.devices.len(), 1);
    assert_eq!(out.signed_master_key_changes, vec![BOB.to_owned()]);
    let record = store.record(BOB).unwrap();
    assert_eq!(record.master_key, b64(&bob_new));
    assert_eq!(record.replaced_master_keys, vec![b64(&bob_old)]);

    // 同じ判定をもう一度しても、置き換えは知らせない。
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert!(out.signed_master_key_changes.is_empty());

    // 古い鍵と古い署名には戻らない。
    let attacker = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();
    let old = snapshot(
        &alice,
        &bob_old,
        &attacker,
        vec![row(ALICE, &first.signature)],
        Some(&first.pin),
    );
    let out = store.evaluate(&old, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(
        out.users[BOB].state,
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert!(out.devices.is_empty());
}

/// 受信時の検証（`sender_trust`）が先に記録を置き換えても、置き換えの知らせは失わず、保存し直しても
/// 残り、次の判定で一度だけ返る。
#[test]
fn master_key_change_seen_by_sender_trust_is_reported_by_the_next_evaluation() {
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let bob_old = CrossSigningKeys::generate(BOB).unwrap();
    let bob_new = CrossSigningKeys::generate(BOB).unwrap();
    let bob_device = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();
    let mut store = alice_store(&alice);
    let first = store
        .accept_master_key(&alice.user_signing, BOB, &bob_old.master.public_key_json())
        .unwrap();
    store.confirm_accepted_master_key(BOB, &first.master_key);
    let mut other = alice_store(&alice);
    let second = other
        .accept_master_key(&alice.user_signing, BOB, &bob_new.master.public_key_json())
        .unwrap();
    let snap = snapshot(
        &alice,
        &bob_new,
        &bob_device,
        vec![row(ALICE, &second.signature)],
        Some(&second.pin),
    );

    assert!(store.sender_trust(&snap, BOB, BOB_DEVICE, None).is_some());
    assert_eq!(store.record(BOB).unwrap().master_key, b64(&bob_new));
    // 保存して開き直しても、知らせは残る。
    let mut store = TrustStore::from_json(&store.to_json(), ALICE).unwrap();
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(out.signed_master_key_changes, vec![BOB.to_owned()]);
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert!(out.signed_master_key_changes.is_empty());
    assert!(!store.to_json().contains("pending_master_key_changes"));
}

/// 履歴にある鍵でも、利用者が明示的に受け入れ直せば（#362、SAS）信頼し、履歴から外す。
#[test]
fn explicit_acceptance_removes_key_from_history() {
    let Rotated {
        alice,
        bob_old,
        bob_new,
        mut store,
        ..
    } = rotated(true);
    let again = store
        .accept_master_key(&alice.user_signing, BOB, &bob_old.master.public_key_json())
        .unwrap();
    store.confirm_accepted_master_key(BOB, &again.master_key);
    let record = store.record(BOB).unwrap();
    assert_eq!(record.master_key, b64(&bob_old));
    assert_eq!(record.replaced_master_keys, vec![b64(&bob_new)]);

    let bob_device = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();
    let snap = snapshot(
        &alice,
        &bob_old,
        &bob_device,
        vec![row(ALICE, &again.signature)],
        Some(&again.pin),
    );
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(
        out.users[BOB].state,
        UserState::Trusted(TrustBasis::UserSigningSignature)
    );
    assert_eq!(out.devices.len(), 1);
}

/// 置き換えた鍵の履歴は保存形式に残り、履歴のない以前の形式の記録も読める。
#[test]
fn replaced_master_keys_survive_storage_and_old_records_load() {
    let Rotated { store, bob_old, .. } = rotated(true);
    let json = store.to_json();
    let restored = TrustStore::from_json(&json, ALICE).unwrap();
    assert_eq!(restored, store);
    assert_eq!(
        restored.record(BOB).unwrap().replaced_master_keys,
        vec![b64(&bob_old)]
    );

    // 以前の版で保存した記録（replaced_master_keys がない）。
    let mut old: Value = serde_json::from_str(&json).unwrap();
    old["records"][BOB]
        .as_object_mut()
        .unwrap()
        .remove("replaced_master_keys");
    let restored = TrustStore::from_json(&old.to_string(), ALICE).unwrap();
    assert!(
        restored
            .record(BOB)
            .unwrap()
            .replaced_master_keys
            .is_empty()
    );
    assert_eq!(old["version"], json!(1));

    // 履歴のない記録は、項目を書かない（以前の版のクライアントでも読める形のまま）。
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let bob = CrossSigningKeys::generate(BOB).unwrap();
    let mut fresh = alice_store(&alice);
    fresh
        .accept_master_key(&alice.user_signing, BOB, &bob.master.public_key_json())
        .unwrap();
    assert!(!fresh.to_json().contains("replaced_master_keys"));
}

/// 設計上の限界：記録のない新しい端末は、今のユーザー署名鍵で作った古いピンを見せられると、古い
/// マスター鍵を採用する。ピンにも署名にも順序がなく、新しい端末は前の状態を覚えていないため
/// （docs/e2ee.md の脅威モデルの「相手のマスター鍵の巻き戻し」）。この限界を変えたら、このテストと
/// docs を合わせて直す。
#[test]
fn new_device_without_record_cannot_detect_replayed_old_pin() {
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let bob_old = CrossSigningKeys::generate(BOB).unwrap();
    let device = DeviceAccount::new(BOB, BOB_DEVICE).unwrap();
    let old_pin = alice
        .user_signing
        .sign_tofu_pin(BOB, &bob_old.master.public_key())
        .unwrap();
    let mut store = alice_store(&alice);
    let snap = snapshot(&alice, &bob_old, &device, vec![], Some(&old_pin));
    let out = store.evaluate(&snap, ALICE_DEVICE, Some(&alice.user_signing));
    assert_eq!(
        out.users[BOB].state,
        UserState::Trusted(TrustBasis::SharedPin)
    );
}

fn version(number: &str, auth_data: &Value) -> Value {
    json!({ "version": number, "algorithm": BACKUP_ALGORITHM, "auth_data": auth_data })
}

/// レビューの指摘 2（Medium）：版の番号は署名に入るので、古い版の auth_data を大きい番号の版として
/// 見せても通らない。方式のない版も通らない。
#[test]
fn backup_version_number_is_signed() {
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let old_key = BackupDecryptionKey::generate();
    let new_key = BackupDecryptionKey::generate();
    let old_auth = old_key.auth_data(&alice.master, 1).unwrap();
    let new_auth = new_key.auth_data(&alice.master, 2).unwrap();
    assert_eq!(old_auth["version"], json!("1"));
    assert_eq!(old_auth["algorithm"], json!(BACKUP_ALGORITHM));
    let master = alice.master.public_key();
    let mut state = BackupUploadState::new(ALICE).unwrap();
    // 版 1（古い鍵）→ 版 2（新しい鍵。端末だけの失効のあと）。
    assert_eq!(
        state
            .accept_current_version(&version("1", &old_auth), &master, &old_key)
            .unwrap(),
        VersionChange::Changed
    );
    assert_eq!(
        state
            .accept_current_version(&version("2", &new_auth), &master, &new_key)
            .unwrap(),
        VersionChange::Changed
    );
    // 版 1 をそのまま返すと巻き戻しとして検出する（以前からの検出）。
    assert!(matches!(
        state.accept_current_version(&version("1", &old_auth), &master, &old_key),
        Err(CryptoError::BackupVersionRollback {
            max_seen: 2,
            received: 1
        })
    ));
    // 古い auth_data を「版 3」と名乗らせ、SSSS も古い秘密に巻き戻しても（解錠し直しで古い鍵を得る）通らない。
    let relabeled = version("3", &old_auth);
    assert!(matches!(
        state.accept_current_version(&relabeled, &master, &old_key),
        Err(CryptoError::InvalidBackup(_))
    ));
    assert!(state.current_version().is_none());
    // 版に方式がなければ通らない。ほかの検査（署名した番号、見た番号の最大）では落ちない入力
    // （署名と同じ番号の版 1 を、見た番号の最大がない検証）で、方式の検査だけを確かめる。
    let mut without_algorithm = version("1", &old_auth);
    without_algorithm
        .as_object_mut()
        .unwrap()
        .remove("algorithm");
    assert!(verify_backup_version(&version("1", &old_auth), ALICE, &master, &old_key).is_ok());
    assert!(matches!(
        verify_backup_version(&without_algorithm, ALICE, &master, &old_key),
        Err(CryptoError::InvalidBackup(_))
    ));
    // 新しい端末（見た番号の最大がない）でも、署名と違う番号の版は通らない。
    assert!(matches!(
        verify_backup_version(&relabeled, ALICE, &master, &old_key),
        Err(CryptoError::InvalidBackup(_))
    ));
    // 署名した方式や番号を書き換えると、署名が通らない。
    let mut tampered = old_auth.clone();
    tampered["version"] = json!("3");
    assert!(matches!(
        verify_backup_version(&version("3", &tampered), ALICE, &master, &old_key),
        Err(CryptoError::Signature(_))
    ));
}

/// 次に作る版の番号は、消した版も含めた最大の次。今の版の次でない番号は作らない。
#[test]
fn new_versions_sign_the_number_the_server_assigns() {
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let key = BackupDecryptionKey::generate();
    assert_eq!(next_backup_version(&[]), 1);
    let versions: Vec<BackupVersionInfo> = [("1", false), ("2", true)]
        .iter()
        .map(|(number, deleted)| {
            let mut v = version(number, &key.auth_data(&alice.master, 1).unwrap());
            v["deleted"] = json!(deleted);
            BackupVersionInfo::from_json(&v).unwrap()
        })
        .collect();
    assert_eq!(next_backup_version(&versions), 3);

    let body = key
        .create_version_request(&alice.master, Some("2"), 3)
        .unwrap();
    assert_eq!(body["auth_data"]["version"], json!("3"));
    assert_eq!(body["expected_current_version"], json!("2"));
    assert!(matches!(
        key.create_version_request(&alice.master, Some("2"), 4),
        Err(CryptoError::InvalidBackup(_))
    ));
    assert!(matches!(
        key.auth_data(&alice.master, 0),
        Err(CryptoError::InvalidBackup(_))
    ));
}
