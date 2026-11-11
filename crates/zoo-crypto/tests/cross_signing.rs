//! クロス署名と鍵の配布先の判定（docs/e2ee.md の「クロス署名」と「鍵の配布先の判定」）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use vodozemac::Ed25519SecretKey;
use zoo_crypto::{
    CrossSigningKeyPair, CrossSigningKeys, CryptoError, DeviceAccount, DeviceExclusion,
    Distribution, KeySignature, KeySnapshot, KeyUsage, ReverificationReason, TOFU_PIN_TYPE,
    TrustBasis, TrustStore, TrustWarning, UserState, cross_signing_key_id, signable_json,
    verify_cross_signing_key, verify_json_signature,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const BOB: &str = "00000000-0000-0000-0000-00000000000b";
const CAROL: &str = "00000000-0000-0000-0000-00000000000c";

/// 端末鍵への自己署名鍵の署名の付け方。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sig {
    /// 端末鍵の JSON の `signatures` に入れる。
    Embedded,
    /// `e2ee_key_signatures` の行で渡す。
    Row,
    /// 署名しない。
    Unsigned,
    /// 別の鍵で作った署名を、自己署名鍵の鍵 ID で入れる。
    Broken,
}

struct User {
    id: &'static str,
    keys: CrossSigningKeys,
    devices: Vec<(DeviceAccount, Sig)>,
}

fn device_id(user: u8, n: u8) -> String {
    format!("{user:02x}{n:06x}-5b7d-4e9f-a1b2-c3d4e5f60718")
}

impl User {
    fn new(id: &'static str, sigs: &[Sig]) -> Self {
        let tag = id.as_bytes()[35];
        let devices = sigs
            .iter()
            .enumerate()
            .map(|(i, sig)| {
                let n = u8::try_from(i).unwrap() + 1;
                (DeviceAccount::new(id, &device_id(tag, n)).unwrap(), *sig)
            })
            .collect();
        Self {
            id,
            keys: CrossSigningKeys::generate(id).unwrap(),
            devices,
        }
    }

    fn device(&self, i: usize) -> String {
        self.devices[i].0.device_id().to_owned()
    }

    /// マスター鍵だけでなく自己署名鍵も作り直す（失効の手順 1〜3 と同じ）。
    fn replace_master_key(&mut self) {
        self.keys = CrossSigningKeys::generate(self.id).unwrap();
    }
}

/// 配布先のスナップショットの JSON を組み立てる。
struct Snapshot {
    value: Value,
}

impl Snapshot {
    fn new(me: &User, users: &[&User]) -> Self {
        let mut value = json!({
            "channel_id": "c0000000-0000-4000-8000-000000000001",
            "membership_epoch": 1,
            "user_ids": [],
            "device_keys": {},
            "master_keys": {},
            "self_signing_keys": {},
            "user_signing_keys": {
                me.id: me.keys.master.sign_subkey(&me.keys.user_signing).unwrap(),
            },
            "signatures": [],
            "tofu_pins": {},
        });
        for user in users {
            let mut snapshot = Self { value };
            snapshot.add_user(user);
            value = snapshot.value;
        }
        Self { value }
    }

    fn add_user(&mut self, user: &User) {
        let v = &mut self.value;
        v["user_ids"].as_array_mut().unwrap().push(json!(user.id));
        v["master_keys"][user.id] = user.keys.master.public_key_json();
        v["self_signing_keys"][user.id] = user
            .keys
            .master
            .sign_subkey(&user.keys.self_signing)
            .unwrap();
        v["device_keys"][user.id] = json!({});
        for (account, sig) in &user.devices {
            let mut keys = account.device_keys();
            let upload = user.keys.self_signing.sign_device(&keys).unwrap();
            let ssk_id = user.keys.self_signing.key_id();
            match sig {
                Sig::Embedded => keys["signatures"][user.id][&ssk_id] = json!(upload.signature),
                Sig::Row => self.value["signatures"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({
                        "signer_user_id": user.id,
                        "signer_key_id": upload.signer_key_id,
                        "target_user_id": user.id,
                        "target_key_id": upload.target_key_id,
                        "signature": upload.signature,
                    })),
                Sig::Unsigned => {}
                Sig::Broken => {
                    let forged = Ed25519SecretKey::new()
                        .sign(signable_json(&keys).unwrap().as_bytes())
                        .to_base64();
                    keys["signatures"][user.id][&ssk_id] = json!(forged);
                }
            }
            self.value["device_keys"][user.id][account.device_id()] = keys;
        }
    }

    fn pin(&mut self, target: &str, pin: Value) {
        self.value["tofu_pins"][target] = pin;
    }

    fn signature(&mut self, row: Value) {
        self.value["signatures"].as_array_mut().unwrap().push(row);
    }

    fn get(&self) -> KeySnapshot {
        KeySnapshot::from_json(&self.value).unwrap()
    }
}

/// 解錠済みの自分の記録（マスター鍵を信頼し、ユーザー署名鍵を履歴に入れたもの）。
fn unlocked_store(me: &User) -> TrustStore {
    let mut store = TrustStore::new(me.id).unwrap();
    store.trust_own_master_key(&me.keys.master).unwrap();
    store
        .record_own_user_signing_key(&me.keys.user_signing)
        .unwrap();
    store
}

fn evaluate(store: &mut TrustStore, me: &User, snapshot: &Snapshot) -> Distribution {
    store.evaluate(&snapshot.get(), &me.device(0), Some(&me.keys.user_signing))
}

/// 判定し、TOFU のピンの登録が成功したものとして登録待ちを外し、判定し直す。
fn evaluate_registered(store: &mut TrustStore, me: &User, snapshot: &Snapshot) -> Distribution {
    let first = evaluate(store, me, snapshot);
    for pin in &first.new_pins {
        store.confirm_tofu_pin(pin);
    }
    let result = evaluate(store, me, snapshot);
    assert!(result.new_pins.is_empty());
    result
}

fn distributed(result: &Distribution) -> Vec<(String, String)> {
    result
        .devices
        .iter()
        .map(|d| (d.user_id.clone(), d.device_id.clone()))
        .collect()
}

fn state(result: &Distribution, user: &str) -> UserState {
    result.users[user].state
}

fn excluded(result: &Distribution, user: &str) -> BTreeMap<String, DeviceExclusion> {
    result.users[user].excluded_devices.clone()
}

/// 自分（今の端末、署名済み、未署名）と Bob（埋め込み、行、未署名、壊れた署名）。
fn alice_and_bob() -> (User, User) {
    let alice = User::new(ALICE, &[Sig::Embedded, Sig::Row, Sig::Unsigned]);
    let bob = User::new(BOB, &[Sig::Embedded, Sig::Row, Sig::Unsigned, Sig::Broken]);
    (alice, bob)
}

/// Bob の端末のうち、配ってよいのは署名済みの 0 と 1 だけ。
fn assert_only_signed_bob_devices(result: &Distribution, alice: &User, bob: &User) {
    assert_eq!(
        distributed(result),
        vec![
            (ALICE.to_owned(), alice.device(1)),
            (BOB.to_owned(), bob.device(0)),
            (BOB.to_owned(), bob.device(1)),
        ]
    );
    assert_eq!(
        excluded(result, ALICE),
        BTreeMap::from([
            (alice.device(0), DeviceExclusion::CurrentDevice),
            (alice.device(2), DeviceExclusion::Unsigned),
        ])
    );
    assert_eq!(
        excluded(result, BOB),
        BTreeMap::from([
            (bob.device(2), DeviceExclusion::Unsigned),
            (bob.device(3), DeviceExclusion::BadSignature),
        ])
    );
}

// ---- 鍵の生成と署名 ----

#[test]
fn generates_and_verifies_cross_signing_keys() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let upload = keys.upload_json();
    let master = verify_cross_signing_key(&upload["master_key"], ALICE, KeyUsage::Master).unwrap();
    assert_eq!(master, keys.master.public_key());
    for (field, usage, pair) in [
        (
            "self_signing_key",
            KeyUsage::SelfSigning,
            &keys.self_signing,
        ),
        (
            "user_signing_key",
            KeyUsage::UserSigning,
            &keys.user_signing,
        ),
    ] {
        let key = verify_cross_signing_key(&upload[field], ALICE, usage).unwrap();
        assert_eq!(key, pair.public_key());
        verify_json_signature(
            &upload[field],
            ALICE,
            &cross_signing_key_id(&master),
            &master,
        )
        .unwrap();
    }
    assert_ne!(keys.master.public_key(), keys.self_signing.public_key());
    assert_eq!(
        KeyUsage::UserSigning.secret_name(),
        "m.cross_signing.user_signing"
    );
}

#[test]
fn restores_keys_from_seed() {
    let key = CrossSigningKeyPair::generate(ALICE, KeyUsage::SelfSigning).unwrap();
    let restored =
        CrossSigningKeyPair::from_seed(ALICE, KeyUsage::SelfSigning, &key.seed()).unwrap();
    assert_eq!(restored.public_key(), key.public_key());
    assert!(!format!("{key:?}").contains(&vodozemac::base64_encode(*key.seed())));
}

#[test]
fn rejects_malformed_cross_signing_keys() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let good = keys.master.public_key_json();
    let other = Ed25519SecretKey::new().public_key().to_base64();
    let pk = keys.master.public_key().to_base64();
    let mut cases = vec![
        (good.clone(), BOB, KeyUsage::Master),
        (good.clone(), ALICE, KeyUsage::SelfSigning),
    ];
    for (field, value) in [
        ("usage", json!(["master", "self_signing"])),
        ("usage", json!("master")),
        ("keys", json!({})),
        (
            "keys",
            json!({ format!("ed25519:{pk}"): pk, format!("ed25519:{other}"): other }),
        ),
        ("keys", json!({ format!("ed25519:{other}"): pk })),
        ("keys", json!({ format!("ed25519:{pk}"): "not base64" })),
    ] {
        let mut broken = good.clone();
        broken[field] = value;
        cases.push((broken, ALICE, KeyUsage::Master));
    }
    for (value, user, usage) in cases {
        assert!(
            verify_cross_signing_key(&value, user, usage).is_err(),
            "{value}"
        );
    }
}

#[test]
fn signs_own_devices_with_the_self_signing_key_only() {
    let alice = User::new(ALICE, &[Sig::Embedded]);
    let keys = alice.devices[0].0.device_keys();
    let upload = alice.keys.self_signing.sign_device(&keys).unwrap();
    assert_eq!(upload.target_user_id, ALICE);
    assert_eq!(upload.target_key_id, alice.device(0));
    assert_eq!(upload.signer_key_id, alice.keys.self_signing.key_id());

    // マスター鍵とユーザー署名鍵では端末に署名しない。
    for key in [&alice.keys.master, &alice.keys.user_signing] {
        assert!(matches!(
            key.sign_device(&keys),
            Err(CryptoError::InvalidCrossSigningKey(_))
        ));
    }
    // 別のユーザーの端末と、自己署名の壊れた端末には署名しない。
    let bob = User::new(BOB, &[Sig::Embedded]);
    assert!(
        alice
            .keys
            .self_signing
            .sign_device(&bob.devices[0].0.device_keys())
            .is_err()
    );
    let mut tampered = keys.clone();
    tampered["algorithms"] = json!([]);
    assert!(alice.keys.self_signing.sign_device(&tampered).is_err());
}

#[test]
fn signs_other_users_master_keys_with_the_user_signing_key_only() {
    let alice = CrossSigningKeys::generate(ALICE).unwrap();
    let bob = CrossSigningKeys::generate(BOB).unwrap();
    let bob_master = bob.master.public_key_json();
    let upload = alice
        .user_signing
        .sign_master_key(BOB, &bob_master)
        .unwrap();
    assert_eq!(upload.target_key_id, bob.master.public_key().to_base64());

    assert!(
        alice
            .self_signing
            .sign_master_key(BOB, &bob_master)
            .is_err()
    );
    assert!(alice.master.sign_master_key(BOB, &bob_master).is_err());
    // 自分のマスター鍵、持ち主の違うマスター鍵には署名しない。
    assert!(
        alice
            .user_signing
            .sign_master_key(ALICE, &alice.master.public_key_json())
            .is_err()
    );
    assert!(
        alice
            .user_signing
            .sign_master_key(CAROL, &bob_master)
            .is_err()
    );
    // マスター鍵は自己署名鍵とユーザー署名鍵にだけ、同じユーザーの鍵にだけ署名する。
    assert!(alice.master.sign_subkey(&alice.master).is_err());
    assert!(alice.master.sign_subkey(&bob.self_signing).is_err());
    assert!(alice.self_signing.sign_subkey(&alice.user_signing).is_err());

    let pin = alice
        .user_signing
        .sign_tofu_pin(BOB, &bob.master.public_key())
        .unwrap();
    assert_eq!(pin["type"], TOFU_PIN_TYPE);
    assert_eq!(pin["user_id"], ALICE);
    let usk = alice.user_signing.public_key();
    verify_json_signature(&pin, ALICE, &cross_signing_key_id(&usk), &usk).unwrap();
    assert!(
        alice
            .self_signing
            .sign_tofu_pin(BOB, &bob.master.public_key())
            .is_err()
    );
}

// ---- 配布先の判定 ----

/// 規則 4：初めて見た相手のピンを作る。ピンを置けるまでは配らず、置けた直後も未署名の端末には配らない。
#[test]
fn tofu_trusts_first_seen_master_key_but_not_unsigned_devices() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let result = evaluate(&mut store, &alice, &snapshot);

    assert_eq!(state(&result, ALICE), UserState::Trusted(TrustBasis::Own));
    assert_eq!(state(&result, BOB), UserState::PinPending);
    assert_eq!(
        distributed(&result),
        vec![(ALICE.to_owned(), alice.device(1))]
    );
    assert!(result.warnings.is_empty());

    assert_eq!(result.new_pins.len(), 1);
    let pin = &result.new_pins[0];
    assert_eq!(pin.target_user_id, BOB);
    assert_eq!(pin.master_key, bob.keys.master.public_key().to_base64());
    let usk = alice.keys.user_signing.public_key();
    verify_json_signature(&pin.pin, ALICE, &cross_signing_key_id(&usk), &usk).unwrap();

    let record = store.record(BOB).unwrap();
    assert_eq!(record.master_key, pin.master_key);
    assert_eq!(record.confirmed_usk, Some(usk.to_base64()));
    assert!(record.pin_pending);

    // 登録が成功したら、署名済みの端末にだけ配る。
    store.confirm_tofu_pin(pin);
    assert!(!store.record(BOB).unwrap().pin_pending);
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(state(&result, BOB), UserState::Trusted(TrustBasis::Record));
    assert!(result.new_pins.is_empty());
    assert_only_signed_bob_devices(&result, &alice, &bob);
}

/// ピンを置く前に保存して終了しても、復元後は登録待ちのままで、同じピンを返し直して配らない。
#[test]
fn pending_tofu_pin_survives_restart_and_is_resent() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let first = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(first.new_pins.len(), 1);

    let mut restored = TrustStore::from_json(&store.to_json(), ALICE).unwrap();
    let result = evaluate(&mut restored, &alice, &snapshot);
    assert_eq!(state(&result, BOB), UserState::PinPending);
    assert!(!distributed(&result).iter().any(|(u, _)| u == BOB));
    assert_eq!(result.new_pins.len(), 1);
    assert_eq!(result.new_pins[0].target_user_id, BOB);
    assert_eq!(result.new_pins[0].master_key, first.new_pins[0].master_key);

    // 秘密鍵がなければピンを作り直せないので、解錠を待つ。
    let result = restored.evaluate(&snapshot.get(), &alice.device(0), None);
    assert_eq!(state(&result, BOB), UserState::NeedsUnlock);
    assert!(result.devices.iter().all(|d| d.user_id != BOB));

    // 登録は成功していたが、confirm を保存する前に終了した：サーバーのピンで登録済みと分かる。
    snapshot.pin(BOB, first.new_pins[0].pin.clone());
    let result = evaluate(&mut restored, &alice, &snapshot);
    assert_eq!(state(&result, BOB), UserState::Trusted(TrustBasis::Record));
    assert!(result.new_pins.is_empty());
    assert!(!restored.record(BOB).unwrap().pin_pending);
    assert_only_signed_bob_devices(&result, &alice, &bob);
}

/// 登録待ちの間に、別の端末が違うマスター鍵のピンを置いていたら、それを採用して再検証にする。
#[test]
fn pending_tofu_pin_yields_to_conflicting_pin_on_server() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    evaluate(&mut store, &alice, &snapshot);
    let other_master = Ed25519SecretKey::new().public_key();
    snapshot.pin(
        BOB,
        alice
            .keys
            .user_signing
            .sign_tofu_pin(BOB, &other_master)
            .unwrap(),
    );
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert!(result.new_pins.is_empty());
    let record = store.record(BOB).unwrap();
    assert_eq!(record.master_key, other_master.to_base64());
    assert_eq!(record.confirmed_usk, None);
    assert!(!record.pin_pending);
}

/// 記録と一致した場合も、未署名、壊れた署名、削除済み（スナップショットにない）の端末には配らない。
#[test]
fn record_match_excludes_unsigned_broken_and_deleted_devices() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let first = evaluate(&mut store, &alice, &snapshot);
    snapshot.pin(BOB, first.new_pins[0].pin.clone());

    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(state(&result, BOB), UserState::Trusted(TrustBasis::Record));
    assert!(result.new_pins.is_empty());
    assert_only_signed_bob_devices(&result, &alice, &bob);

    // 端末を削除すると、サーバーはスナップショットから除く。
    snapshot.value["device_keys"][BOB]
        .as_object_mut()
        .unwrap()
        .remove(&bob.device(0));
    let result = evaluate(&mut store, &alice, &snapshot);
    assert!(!distributed(&result).contains(&(BOB.to_owned(), bob.device(0))));
    assert!(distributed(&result).contains(&(BOB.to_owned(), bob.device(1))));
}

/// 規則 3：自分のユーザー署名鍵で署名されたマスター鍵を信頼する。未署名の端末には配らない。
#[test]
fn user_signing_signature_trusts_master_key_but_not_unsigned_devices() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let upload = alice
        .keys
        .user_signing
        .sign_master_key(BOB, &bob.keys.master.public_key_json())
        .unwrap();
    snapshot.signature(row_json(ALICE, &upload));

    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::Trusted(TrustBasis::UserSigningSignature)
    );
    assert!(result.new_pins.is_empty(), "規則 3 ではピンを書かない");
    assert_only_signed_bob_devices(&result, &alice, &bob);
    assert_eq!(
        store.record(BOB).unwrap().confirmed_usk,
        Some(alice.keys.user_signing.public_key().to_base64())
    );
}

/// 偽の署名：ユーザー署名鍵以外の鍵で作った「マスター鍵への署名」では規則 3 にならない。
#[test]
fn forged_user_signing_signature_is_ignored() {
    let (alice, mut bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let first = evaluate_registered(&mut store, &alice, &snapshot);
    assert_eq!(state(&first, BOB), UserState::Trusted(TrustBasis::Record));

    // Bob のマスター鍵を差し替え、ユーザー署名鍵の鍵 ID で偽の署名を付ける。
    bob.replace_master_key();
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let forged = Ed25519SecretKey::new()
        .sign(
            signable_json(&bob.keys.master.public_key_json())
                .unwrap()
                .as_bytes(),
        )
        .to_base64();
    snapshot.signature(json!({
        "signer_user_id": ALICE,
        "signer_key_id": alice.keys.user_signing.key_id(),
        "target_user_id": BOB,
        "target_key_id": bob.keys.master.public_key().to_base64(),
        "signature": forged,
    }));
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert!(!distributed(&result).iter().any(|(u, _)| u == BOB));
}

/// 規則 5：ユーザー署名鍵を変えずにマスター鍵だけを差し替えたら、`confirmed_usk` が今の鍵と
/// 一致していても配布を止める。受け入れ（#362）たら再開する。
#[test]
fn master_key_replacement_stops_distribution_until_accepted() {
    let (alice, mut bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let first = evaluate(&mut store, &alice, &snapshot);
    let old_pin = first.new_pins[0].pin.clone();
    let usk = alice.keys.user_signing.public_key().to_base64();
    assert_eq!(store.record(BOB).unwrap().confirmed_usk, Some(usk.clone()));

    bob.replace_master_key();
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    snapshot.pin(BOB, old_pin);
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert_eq!(
        distributed(&result),
        vec![(ALICE.to_owned(), alice.device(1))]
    );
    assert!(result.new_pins.is_empty());
    // 記録は古いマスター鍵のまま。
    assert_ne!(
        store.record(BOB).unwrap().master_key,
        bob.keys.master.public_key().to_base64()
    );

    // ピンがなくても（サーバーが消しても）止める。
    snapshot.value["tofu_pins"] = json!({});
    let result = evaluate(&mut store, &alice, &snapshot);
    assert!(matches!(
        state(&result, BOB),
        UserState::NeedsReverification(_)
    ));

    // 受け入れ：replace のピンと署名を送り、記録を更新する。
    let accepted = store
        .accept_master_key(
            &alice.keys.user_signing,
            BOB,
            &bob.keys.master.public_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.signature.target_user_id, BOB);
    snapshot.pin(BOB, accepted.pin);
    snapshot.signature(row_json(ALICE, &accepted.signature));
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::Trusted(TrustBasis::UserSigningSignature)
    );
    assert_only_signed_bob_devices(&result, &alice, &bob);
}

/// 規則 2：クロス署名鍵のない相手には配らない。壊れたマスター鍵も同じ。
#[test]
fn users_without_cross_signing_keys_get_nothing() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    snapshot.value["master_keys"]
        .as_object_mut()
        .unwrap()
        .remove(BOB);
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(state(&result, BOB), UserState::NoCrossSigningKeys);
    assert!(store.record(BOB).is_none());

    snapshot.value["master_keys"][BOB] = json!({ "user_id": BOB, "usage": ["master"], "keys": {} });
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(state(&result, BOB), UserState::InvalidCrossSigningKeys);
    assert!(!distributed(&result).iter().any(|(u, _)| u == BOB));
}

/// マスター鍵で署名されていない自己署名鍵（攻撃者の鍵）で署名した端末には配らない。
#[test]
fn self_signing_key_not_signed_by_master_excludes_all_devices() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    // 攻撃者が自分の鍵で自己署名鍵に署名し、その自己署名鍵で端末に署名した。
    let attacker = CrossSigningKeys::generate(BOB).unwrap();
    let mut bad = attacker.master.sign_subkey(&attacker.self_signing).unwrap();
    snapshot.value["self_signing_keys"][BOB] = bad.clone();
    let device = bob.devices[0].0.device_keys();
    let upload = attacker.self_signing.sign_device(&device).unwrap();
    snapshot.value["device_keys"][BOB][bob.device(0)]["signatures"][BOB][&upload.signer_key_id] =
        json!(upload.signature);

    let result = evaluate_registered(&mut store, &alice, &snapshot);
    assert_eq!(state(&result, BOB), UserState::Trusted(TrustBasis::Record));
    assert!(!distributed(&result).iter().any(|(u, _)| u == BOB));
    assert!(
        excluded(&result, BOB)
            .values()
            .all(|r| *r == DeviceExclusion::InvalidSelfSigningKey)
    );

    // 正しいマスター鍵の鍵 ID で偽の署名を付けても同じ。
    bad["signatures"] = json!({ BOB: { bob.keys.master.key_id(): "AAAA" } });
    snapshot.value["self_signing_keys"][BOB] = bad;
    let result = evaluate(&mut store, &alice, &snapshot);
    assert!(!distributed(&result).iter().any(|(u, _)| u == BOB));
}

/// 自分のマスター鍵がサーバー上で変わったら、自分の他の端末にも、他のユーザーにも配らない。
#[test]
fn own_master_key_change_stops_all_distribution() {
    let (mut alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    alice.replace_master_key();
    let snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(result.warnings, vec![TrustWarning::OwnMasterKeyChanged]);
    assert!(result.devices.is_empty());
    assert_eq!(state(&result, ALICE), UserState::Blocked);
    assert_eq!(state(&result, BOB), UserState::Blocked);
    assert!(result.new_pins.is_empty());

    // 解錠前（自分のマスター鍵を信頼していない）も配らない。
    let mut store = TrustStore::new(ALICE).unwrap();
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(result.warnings, vec![TrustWarning::OwnIdentityUnknown]);
    assert!(result.devices.is_empty());
}

/// 偽のユーザー署名鍵（自分のマスター鍵で署名されていない）は基準にせず、他のユーザーには配らない。
#[test]
fn fake_user_signing_key_blocks_other_users() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let attacker = CrossSigningKeys::generate(ALICE).unwrap();
    snapshot.value["user_signing_keys"][ALICE] =
        attacker.master.sign_subkey(&attacker.user_signing).unwrap();
    let result = store.evaluate(
        &snapshot.get(),
        &alice.device(0),
        Some(&attacker.user_signing),
    );
    assert_eq!(result.warnings, vec![TrustWarning::UntrustedUserSigningKey]);
    assert_eq!(state(&result, BOB), UserState::Blocked);
    assert!(result.new_pins.is_empty());
    // 自分の端末は規則 1 だけで決まる。
    assert_eq!(
        distributed(&result),
        vec![(ALICE.to_owned(), alice.device(1))]
    );
    assert_eq!(store.user_signing_key_history().len(), 1);
}

/// ユーザー署名鍵が変わり、署名し直されたピンがない相手には配らない。ある相手は更新して配る。
#[test]
fn user_signing_key_change_requires_resigned_pin() {
    let (mut alice, bob) = alice_and_bob();
    let carol = User::new(CAROL, &[Sig::Embedded]);
    let mut store = unlocked_store(&alice);
    let snapshot = Snapshot::new(&alice, &[&alice, &bob, &carol]);
    evaluate_registered(&mut store, &alice, &snapshot);

    // 失効の手順 4：ユーザー署名鍵を作り直し、Carol のピンだけを署名し直す。
    alice.keys.user_signing = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob, &carol]);
    snapshot.pin(
        CAROL,
        alice
            .keys
            .user_signing
            .sign_tofu_pin(CAROL, &carol.keys.master.public_key())
            .unwrap(),
    );
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(result.warnings, vec![TrustWarning::UserSigningKeyChanged]);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::UserSigningKeyChanged)
    );
    assert_eq!(
        state(&result, CAROL),
        UserState::Trusted(TrustBasis::Record)
    );
    assert!(!distributed(&result).iter().any(|(u, _)| u == BOB));
    assert!(distributed(&result).contains(&(CAROL.to_owned(), carol.device(0))));
    let new_usk = alice.keys.user_signing.public_key().to_base64();
    assert_eq!(store.record(CAROL).unwrap().confirmed_usk, Some(new_usk));
    assert_eq!(store.user_signing_key_history().len(), 2);
    assert!(
        result.new_pins.is_empty(),
        "記録のある相手のピンは自動で作らない"
    );

    // 古いユーザー署名鍵のピンしかない相手は、何度判定しても再検証のまま。
    let result = evaluate(&mut store, &alice, &snapshot);
    assert!(result.warnings.is_empty());
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::UserSigningKeyChanged)
    );
}

/// `confirmed_usk` のない記録には、マスター鍵が一致しても、署名し直されたピンがあっても配らない。
#[test]
fn record_without_confirmed_usk_is_never_distributed_to() {
    let (mut alice, mut bob) = alice_and_bob();
    let original_master = bob.keys.master.public_key();
    // 記録のない新しい端末が、今と違うマスター鍵の共有ピンを採用した。
    bob.replace_master_key();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    snapshot.pin(
        BOB,
        alice
            .keys
            .user_signing
            .sign_tofu_pin(BOB, &original_master)
            .unwrap(),
    );
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    let record = store.record(BOB).unwrap().clone();
    assert_eq!(record.master_key, original_master.to_base64());
    assert_eq!(record.confirmed_usk, None);
    assert!(result.new_pins.is_empty());

    // サーバーがピンのマスター鍵に戻しても、confirmed_usk がないので配らない。
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    snapshot.value["master_keys"][BOB] = json!({
        "user_id": BOB, "usage": ["master"],
        "keys": { cross_signing_key_id(&original_master): original_master.to_base64() },
    });
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::NotConfirmed)
    );

    // ユーザー署名鍵が変わり、署名し直されたピンがあっても、confirmed_usk は設定しない。
    alice.keys.user_signing = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    let value = snapshot.value.clone();
    let mut snapshot = Snapshot { value };
    snapshot.value["user_signing_keys"][ALICE] = alice
        .keys
        .master
        .sign_subkey(&alice.keys.user_signing)
        .unwrap();
    snapshot.pin(
        BOB,
        alice
            .keys
            .user_signing
            .sign_tofu_pin(BOB, &original_master)
            .unwrap(),
    );
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::NotConfirmed)
    );
    assert_eq!(store.record(BOB).unwrap().confirmed_usk, None);
}

/// 自分の別の端末が作ったピンを採用する（マスター鍵が一致するときだけ confirmed_usk を設定する）。
#[test]
fn adopts_shared_pin_from_another_device() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    snapshot.pin(
        BOB,
        alice
            .keys
            .user_signing
            .sign_tofu_pin(BOB, &bob.keys.master.public_key())
            .unwrap(),
    );
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::Trusted(TrustBasis::SharedPin)
    );
    assert!(result.new_pins.is_empty());
    assert_only_signed_bob_devices(&result, &alice, &bob);
    assert_eq!(
        store.record(BOB).unwrap().confirmed_usk,
        Some(alice.keys.user_signing.public_key().to_base64())
    );
}

/// 記録とマスター鍵が一致しても、今のユーザー署名鍵で検証できる別のマスター鍵のピンがあれば止める
/// （サーバーが端末ごとに違うマスター鍵を見せる攻撃）。
#[test]
fn conflicting_pin_from_another_device_stops_distribution() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    evaluate_registered(&mut store, &alice, &snapshot);
    let other_master = Ed25519SecretKey::new().public_key();
    snapshot.pin(
        BOB,
        alice
            .keys
            .user_signing
            .sign_tofu_pin(BOB, &other_master)
            .unwrap(),
    );
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
}

/// 署名の壊れたピン、別の相手や別のユーザーのピンは無視する（TOFU を行う）。
#[test]
fn ignores_pins_that_do_not_verify() {
    let (alice, bob) = alice_and_bob();
    let attacker_master = Ed25519SecretKey::new().public_key();
    let usk = &alice.keys.user_signing;
    let mut forged = usk.sign_tofu_pin(BOB, &attacker_master).unwrap();
    forged["signatures"][ALICE][usk.key_id()] = json!(
        Ed25519SecretKey::new()
            .sign(signable_json(&forged).unwrap().as_bytes())
            .to_base64()
    );
    let other_user = CrossSigningKeyPair::generate(CAROL, KeyUsage::UserSigning)
        .unwrap()
        .sign_tofu_pin(BOB, &attacker_master)
        .unwrap();
    let other_target = usk.sign_tofu_pin(CAROL, &attacker_master).unwrap();
    let mut wrong_type = usk.sign_tofu_pin(BOB, &attacker_master).unwrap();
    wrong_type["type"] = json!("zoo.other");

    for pin in [forged, other_user, other_target, wrong_type] {
        let mut store = unlocked_store(&alice);
        let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
        snapshot.pin(BOB, pin.clone());
        let result = evaluate(&mut store, &alice, &snapshot);
        assert_eq!(state(&result, BOB), UserState::PinPending, "{pin}");
        assert_eq!(
            store.record(BOB).unwrap().master_key,
            bob.keys.master.public_key().to_base64()
        );
    }
}

/// 一度置き換わったユーザー署名鍵に戻ったら、その鍵を基準にせず、警告して配布を止める。
#[test]
fn rollback_to_replaced_user_signing_key_is_detected() {
    let (mut alice, mut bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let first = evaluate(&mut store, &alice, &snapshot);
    let old_pin = first.new_pins[0].pin.clone();
    let old_usk = std::mem::replace(
        &mut alice.keys.user_signing,
        CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap(),
    );
    store
        .record_own_user_signing_key(&alice.keys.user_signing)
        .unwrap();
    assert!(matches!(
        store.record_own_user_signing_key(&old_usk),
        Err(CryptoError::UserSigningKeyRollback(_))
    ));

    // Bob のマスター鍵も差し替え、サーバーが古いユーザー署名鍵（マスター鍵の署名は有効）と、
    // 古い鍵で作ったピン（攻撃者のマスター鍵）を見せる。
    bob.replace_master_key();
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    snapshot.value["user_signing_keys"][ALICE] = alice.keys.master.sign_subkey(&old_usk).unwrap();
    snapshot.pin(
        BOB,
        old_usk
            .sign_tofu_pin(BOB, &bob.keys.master.public_key())
            .unwrap(),
    );
    let result = store.evaluate(&snapshot.get(), &alice.device(0), Some(&old_usk));
    assert_eq!(result.warnings, vec![TrustWarning::UserSigningKeyRollback]);
    assert_eq!(state(&result, BOB), UserState::Blocked);
    assert!(!distributed(&result).iter().any(|(u, _)| u == BOB));
    assert!(result.new_pins.is_empty());
    // confirmed_usk も記録も更新しない。
    let record = store.record(BOB).unwrap();
    assert_eq!(record.confirmed_usk, Some(old_usk.public_key().to_base64()));
    assert_eq!(
        record.master_key,
        old_pin["master_key"].as_str().unwrap().to_owned()
    );
    assert_eq!(store.user_signing_key_history().len(), 2);
}

/// ユーザー署名鍵の秘密鍵がない（解錠前）と、初めて見た相手のピンを作れないので配らない。
#[test]
fn first_contact_without_user_signing_secret_waits_for_unlock() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let result = store.evaluate(&snapshot.get(), &alice.device(0), None);
    assert_eq!(state(&result, BOB), UserState::NeedsUnlock);
    assert!(store.record(BOB).is_none());

    // 今の基準と違う（古い）秘密鍵でもピンを作らない。
    let stale = CrossSigningKeyPair::generate(ALICE, KeyUsage::UserSigning).unwrap();
    let result = store.evaluate(&snapshot.get(), &alice.device(0), Some(&stale));
    assert_eq!(state(&result, BOB), UserState::NeedsUnlock);
    assert!(
        store
            .accept_master_key(&stale, BOB, &bob.keys.master.public_key_json())
            .is_err()
    );
}

/// TOFU のピンの登録が 409 になったら、記録を戻し、返ったピンで判定し直す。
#[test]
fn pin_conflict_falls_back_to_returned_pin() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut snapshot = Snapshot::new(&alice, &[&alice, &bob]);
    let result = evaluate(&mut store, &alice, &snapshot);
    store.discard_tofu_pin(&result.new_pins[0]);
    assert!(store.record(BOB).is_none());

    // 別の端末が先に、違うマスター鍵のピンを作っていた。
    let returned = alice
        .keys
        .user_signing
        .sign_tofu_pin(BOB, &Ed25519SecretKey::new().public_key())
        .unwrap();
    snapshot.pin(BOB, returned);
    let result = evaluate(&mut store, &alice, &snapshot);
    assert_eq!(
        state(&result, BOB),
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged)
    );
    assert_eq!(store.record(BOB).unwrap().confirmed_usk, None);
}

#[test]
fn trust_store_round_trips_and_checks_owner() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    evaluate(&mut store, &alice, &Snapshot::new(&alice, &[&alice, &bob]));
    assert!(store.record(BOB).unwrap().pin_pending);
    let json = store.to_json();
    let parsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["user_id"], ALICE);
    assert_eq!(
        parsed["user_signing_key_history"].as_array().unwrap().len(),
        1
    );
    assert!(parsed["records"][BOB]["confirmed_usk"].is_string());
    assert_eq!(parsed["records"][BOB]["pin_pending"], true);

    assert_eq!(TrustStore::from_json(&json, ALICE).unwrap(), store);
    assert!(matches!(
        TrustStore::from_json(&json, BOB),
        Err(CryptoError::InvalidTrustStore(_))
    ));
    for (field, value) in [
        ("version", json!(2)),
        ("own_master_key", json!("not a key")),
        ("user_signing_key_history", json!(["AAAA"])),
        (
            "records",
            json!({ "not-a-uuid": { "master_key": parsed["own_master_key"], "confirmed_usk": null } }),
        ),
    ] {
        let mut broken = parsed.clone();
        broken[field] = value;
        assert!(
            TrustStore::from_json(&broken.to_string(), ALICE).is_err(),
            "{field}"
        );
    }
}

#[test]
fn snapshot_pages_can_be_merged() {
    let (alice, bob) = alice_and_bob();
    let mut store = unlocked_store(&alice);
    let mut first = Snapshot::new(&alice, &[&alice]).get();
    let mut second = Snapshot::new(&alice, &[&bob]);
    second.value["user_signing_keys"] = json!({});
    first.extend(second.get());
    let pending = store.evaluate(&first, &alice.device(0), Some(&alice.keys.user_signing));
    store.confirm_tofu_pin(&pending.new_pins[0]);
    let result = store.evaluate(&first, &alice.device(0), Some(&alice.keys.user_signing));
    assert_only_signed_bob_devices(&result, &alice, &bob);
    assert!(KeySnapshot::from_json(&json!({ "signatures": [{ "signer_user_id": 1 }] })).is_err());
}

fn row_json(signer: &str, upload: &zoo_crypto::SignatureUpload) -> Value {
    serde_json::to_value(KeySignature {
        signer_user_id: signer.to_owned(),
        signer_key_id: upload.signer_key_id.clone(),
        target_user_id: upload.target_user_id.clone(),
        target_key_id: upload.target_key_id.clone(),
        signature: upload.signature.clone(),
    })
    .unwrap()
}
