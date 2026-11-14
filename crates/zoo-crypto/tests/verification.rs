//! 他のユーザーの絵文字 SAS 検証（#368。docs/e2ee.md の「絵文字 SAS 検証」）。
//!
//! 2 人のユーザーの端末どうしで、Olm で暗号化した `m.key.verification.*` をやり取りし、互いのマスター鍵を
//! 検証して署名するまでを、本物の暗号で確かめる。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use zoo_crypto::{
    CrossSigningKeys, CryptoError, DeviceAccount, KeySnapshot, ListedDevice, OlmSession, Rejection,
    SasPhase, SasVerification, TrustBasis, TrustStore, UserState, VERIFICATION_CANCEL_TYPE,
    VERIFICATION_DONE_TYPE, VERIFICATION_KEY_TYPE, VERIFICATION_MAC_TYPE, VERIFICATION_READY_TYPE,
    VERIFICATION_REQUEST_TYPE, VERIFICATION_START_TYPE, VERIFICATION_TIMEOUT_MS,
    VerificationIdentity, VerificationMessage, VerificationOutgoing, cancel_code,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const BOB: &str = "00000000-0000-0000-0000-00000000000b";
const NOW: u64 = 1_800_000_000_000;

struct Device {
    account: DeviceAccount,
    /// 受信で作った Olm セッション。
    inbound: Vec<OlmSession>,
    /// 送信用の Olm セッション（相手の識別鍵ごと）。
    outbound: BTreeMap<String, OlmSession>,
}

struct User {
    id: &'static str,
    keys: CrossSigningKeys,
    devices: Vec<Device>,
    trust: TrustStore,
}

impl User {
    fn new(id: &'static str, devices: usize) -> Self {
        let tag = id.as_bytes()[35];
        let keys = CrossSigningKeys::generate(id).unwrap();
        let mut trust = TrustStore::new(id).unwrap();
        trust.trust_own_master_key(&keys.master).unwrap();
        trust
            .record_own_user_signing_key(&keys.user_signing)
            .unwrap();
        Self {
            id,
            keys,
            devices: (0..devices)
                .map(|n| Device {
                    account: DeviceAccount::new(
                        id,
                        &format!("{tag:02x}{n:06x}-5b7d-4e9f-a1b2-c3d4e5f60718"),
                    )
                    .unwrap(),
                    inbound: Vec::new(),
                    outbound: BTreeMap::new(),
                })
                .collect(),
            trust,
        }
    }

    fn identity(&self, device: usize) -> VerificationIdentity {
        VerificationIdentity::new(&self.devices[device].account, &self.trust).unwrap()
    }

    fn device_index(&self, device_id: &str) -> usize {
        self.devices
            .iter()
            .position(|d| d.account.device_id() == device_id)
            .unwrap()
    }

    /// 自己署名鍵の署名を埋め込んだ端末鍵。
    fn signed_device_keys(&self, device: usize) -> Value {
        let mut keys = self.devices[device].account.device_keys();
        let upload = self.keys.self_signing.sign_device(&keys).unwrap();
        keys["signatures"][self.id][self.keys.self_signing.key_id()] = json!(upload.signature);
        keys
    }
}

/// `keys/query` の応答（`viewer` から見たもの）。`signer` の鍵で端末と自己署名鍵を署名したことにする
/// （サーバーが別のクロス署名鍵を見せる場合を作るため）。
fn keys_query(viewer: &User, users: &[(&User, &CrossSigningKeys)]) -> KeySnapshot {
    let mut value = json!({
        "user_ids": [],
        "device_keys": {},
        "master_keys": {},
        "self_signing_keys": {},
        "user_signing_keys": {
            viewer.id: viewer.keys.master.sign_subkey(&viewer.keys.user_signing).unwrap(),
        },
        "signatures": [],
        "tofu_pins": {},
    });
    for (user, keys) in users {
        value["user_ids"]
            .as_array_mut()
            .unwrap()
            .push(json!(user.id));
        value["master_keys"][user.id] = keys.master.public_key_json();
        value["self_signing_keys"][user.id] = keys.master.sign_subkey(&keys.self_signing).unwrap();
        value["device_keys"][user.id] = json!({});
        for device in &user.devices {
            let mut json = device.account.device_keys();
            let upload = keys.self_signing.sign_device(&json).unwrap();
            json["signatures"][user.id][keys.self_signing.key_id()] = json!(upload.signature);
            value["device_keys"][user.id][device.account.device_id()] = json;
        }
    }
    KeySnapshot::from_json(&value).unwrap()
}

/// 両方から見た、正しい `keys/query`。
fn honest(viewer: &User, users: &[&User]) -> KeySnapshot {
    let pairs: Vec<(&User, &CrossSigningKeys)> = users.iter().map(|u| (*u, &u.keys)).collect();
    keys_query(viewer, &pairs)
}

/// `from` の端末 `device` から `to` に、送るメッセージを Olm で暗号化して届け、受信時の検証を通す。
/// 宛先の端末ごとに `(端末の番号, 受け取ったメッセージ)` を返す。
fn deliver(
    from: &mut User,
    device: usize,
    to: &mut User,
    out: &VerificationOutgoing,
) -> Vec<(usize, VerificationMessage)> {
    let mut received = Vec::new();
    for target in &out.devices {
        let index = to.device_index(&target.device_id);
        let sender = &mut from.devices[device];
        let curve = target.curve25519.to_base64();
        if !sender.outbound.contains_key(&curve) {
            let receiver = &mut to.devices[index].account;
            receiver.generate_one_time_keys(1);
            let otk = receiver.one_time_keys().into_values().next().unwrap();
            receiver.mark_keys_as_published();
            let session = sender
                .account
                .create_outbound_olm_session(target, &otk)
                .unwrap();
            sender.outbound.insert(curve.clone(), session);
        }
        let session = sender.outbound.get_mut(&curve).unwrap();
        let content = sender
            .account
            .encrypt_verification(session, target, out.event_type, &out.content)
            .unwrap();
        let listed = from.signed_device_keys(device);
        let receiver = &mut to.devices[index];
        let decrypted = receiver
            .account
            .decrypt_to_device(&mut receiver.inbound, &content)
            .unwrap();
        let message = decrypted
            .verify_verification(&receiver.account, ListedDevice::Listed(&listed))
            .unwrap();
        assert_eq!(message.event_type, out.event_type);
        received.push((index, message));
    }
    received
}

fn only(mut received: Vec<(usize, VerificationMessage)>) -> VerificationMessage {
    assert_eq!(received.len(), 1);
    received.pop().unwrap().1
}

fn single(out: Vec<VerificationOutgoing>, event_type: &str) -> VerificationOutgoing {
    assert_eq!(
        out.iter().map(|o| o.event_type).collect::<Vec<_>>(),
        vec![event_type]
    );
    out.into_iter().next().unwrap()
}

/// Alice（端末 1 つ）が Bob（端末 2 つ）に検証を求め、Bob の端末 0 が受け入れて、双方が絵文字を
/// 見る段階まで進める。
struct Flow {
    alice: User,
    bob: User,
    a: SasVerification,
    b: SasVerification,
}

fn exchange_keys() -> Flow {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 2);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);

    let (mut a, request) =
        SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    assert_eq!(a.phase(), SasPhase::Requested);
    assert_eq!(request.event_type, VERIFICATION_REQUEST_TYPE);
    assert_eq!(request.devices.len(), 2);

    // Bob の両方の端末に要求が届く。
    let received = deliver(&mut alice, 0, &mut bob, &request);
    let mut incoming: Vec<SasVerification> = received
        .iter()
        .map(|(i, m)| SasVerification::from_request(bob.identity(*i), m, &bob_view, NOW).unwrap())
        .collect();
    assert!(incoming.iter().all(|v| v.phase() == SasPhase::Incoming));
    assert!(
        incoming
            .iter()
            .all(|v| v.transaction_id() == a.transaction_id())
    );
    let mut other = incoming.pop().unwrap();
    let mut b = incoming.pop().unwrap();
    assert_eq!(
        b.their_device_id(),
        Some(alice.devices[0].account.device_id())
    );

    // 端末 0 が受け入れる。Alice は start を送り、端末 1 には取りやめ（m.accepted）を送る。
    let ready = single(b.accept().unwrap(), VERIFICATION_READY_TYPE);
    let ready = only(deliver(&mut bob, 0, &mut alice, &ready));
    let out = a.receive(&ready, &alice_view);
    assert_eq!(
        out.iter().map(|o| o.event_type).collect::<Vec<_>>(),
        vec![VERIFICATION_START_TYPE, VERIFICATION_CANCEL_TYPE]
    );
    assert_eq!(out[1].content["code"], cancel_code::ACCEPTED);
    let cancel = only(deliver(&mut alice, 0, &mut bob, &out[1]));
    assert!(other.receive(&cancel, &bob_view).is_empty());
    assert_eq!(other.phase(), SasPhase::Cancelled);
    assert!(!other.cancellation().unwrap().by_us);

    let start = only(deliver(&mut alice, 0, &mut bob, &out[0]));
    let accept = single(b.receive(&start, &bob_view), "m.key.verification.accept");
    let accept = only(deliver(&mut bob, 0, &mut alice, &accept));
    let key = single(a.receive(&accept, &alice_view), VERIFICATION_KEY_TYPE);
    let key = only(deliver(&mut alice, 0, &mut bob, &key));
    let key = single(b.receive(&key, &bob_view), VERIFICATION_KEY_TYPE);
    let key = only(deliver(&mut bob, 0, &mut alice, &key));
    assert!(a.receive(&key, &alice_view).is_empty());

    assert_eq!(a.phase(), SasPhase::Compare);
    assert_eq!(b.phase(), SasPhase::Compare);
    Flow { alice, bob, a, b }
}

#[test]
fn two_users_verify_each_other() {
    let Flow {
        mut alice,
        mut bob,
        mut a,
        mut b,
    } = exchange_keys();
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    // 双方が同じ絵文字を見る。
    assert!(a.emoji_indices().is_some());
    assert_eq!(a.emoji_indices(), b.emoji_indices());
    assert!(a.emoji_indices().unwrap().iter().all(|i| *i < 64));

    // Alice が先に一致すると答える。Bob はまだ答えていないので MAC を確かめない。
    let mac = single(a.confirm().unwrap(), VERIFICATION_MAC_TYPE);
    assert_eq!(a.phase(), SasPhase::Confirmed);
    let mac = only(deliver(&mut alice, 0, &mut bob, &mac));
    assert!(b.receive(&mac, &bob_view).is_empty());
    assert_eq!(b.phase(), SasPhase::Compare);

    // Bob が答えると、自分の MAC と（Alice の MAC を確かめた）done を送る。
    let out = b.confirm().unwrap();
    assert_eq!(
        out.iter().map(|o| o.event_type).collect::<Vec<_>>(),
        vec![VERIFICATION_MAC_TYPE, VERIFICATION_DONE_TYPE]
    );
    let bob_mac = only(deliver(&mut bob, 0, &mut alice, &out[0]));
    let bob_done = only(deliver(&mut bob, 0, &mut alice, &out[1]));
    let done = single(a.receive(&bob_mac, &alice_view), VERIFICATION_DONE_TYPE);
    assert_eq!(a.phase(), SasPhase::Confirmed);
    assert!(a.receive(&bob_done, &alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::Done);
    let done = only(deliver(&mut alice, 0, &mut bob, &done));
    assert!(b.receive(&done, &bob_view).is_empty());
    assert_eq!(b.phase(), SasPhase::Done);

    assert_eq!(a.verified_master_key(), Some(bob.keys.master.public_key()));
    assert_eq!(
        b.verified_master_key(),
        Some(alice.keys.master.public_key())
    );

    // 検証できたら、相手のマスター鍵にユーザー署名鍵で署名する。ピンと署名を保存するまでは登録待ち。
    let accepted = a
        .sign_master_key(
            &mut alice.trust,
            &alice.keys.user_signing,
            &bob.keys.master.public_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.target_user_id, BOB);
    assert_eq!(
        accepted.master_key,
        bob.keys.master.public_key().to_base64()
    );
    assert_eq!(accepted.signature.target_user_id, BOB);
    assert!(alice.trust.record(BOB).unwrap().pin_pending);
    alice
        .trust
        .confirm_accepted_master_key(&accepted.target_user_id, &accepted.master_key);
    assert!(!alice.trust.record(BOB).unwrap().pin_pending);

    // サーバーに署名が載ると、配布先の判定で「検証済み」（規則 3）になる。
    let mut value = serde_json::to_value(KeySnapshotJson::from(&alice, &bob)).unwrap();
    value["signatures"].as_array_mut().unwrap().push(json!({
        "signer_user_id": ALICE,
        "signer_key_id": accepted.signature.signer_key_id,
        "target_user_id": BOB,
        "target_key_id": accepted.signature.target_key_id,
        "signature": accepted.signature.signature,
    }));
    let snapshot = KeySnapshot::from_json(&value).unwrap();
    let result = alice.trust.evaluate(
        &snapshot,
        alice.devices[0].account.device_id(),
        Some(&alice.keys.user_signing),
    );
    assert_eq!(
        result.users[BOB].state,
        UserState::Trusted(TrustBasis::UserSigningSignature)
    );

    // Bob も Alice のマスター鍵に署名できる。
    let accepted = b
        .sign_master_key(
            &mut bob.trust,
            &bob.keys.user_signing,
            &alice.keys.master.public_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.target_user_id, ALICE);
}

/// 判定に使う `keys/query` の JSON（Alice から見た Alice と Bob）。
struct KeySnapshotJson;

impl KeySnapshotJson {
    fn from(alice: &User, bob: &User) -> Value {
        let mut value = json!({
            "user_ids": [ALICE, BOB],
            "device_keys": {},
            "master_keys": {},
            "self_signing_keys": {},
            "user_signing_keys": {
                ALICE: alice.keys.master.sign_subkey(&alice.keys.user_signing).unwrap(),
            },
            "signatures": [],
            "tofu_pins": {},
        });
        for user in [alice, bob] {
            value["master_keys"][user.id] = user.keys.master.public_key_json();
            value["self_signing_keys"][user.id] = user
                .keys
                .master
                .sign_subkey(&user.keys.self_signing)
                .unwrap();
            value["device_keys"][user.id] = json!({});
            for i in 0..user.devices.len() {
                value["device_keys"][user.id][user.devices[i].account.device_id()] =
                    user.signed_device_keys(i);
            }
        }
        value
    }
}

#[test]
fn mismatch_cancels_on_both_sides() {
    let Flow {
        mut alice,
        mut bob,
        mut a,
        mut b,
    } = exchange_keys();
    let bob_view = honest(&bob, &[&alice, &bob]);
    let cancel = single(a.mismatch(), VERIFICATION_CANCEL_TYPE);
    assert_eq!(a.phase(), SasPhase::Cancelled);
    assert_eq!(a.cancellation().unwrap().code, cancel_code::MISMATCHED_SAS);
    let cancel = only(deliver(&mut alice, 0, &mut bob, &cancel));
    assert!(b.receive(&cancel, &bob_view).is_empty());
    assert_eq!(b.phase(), SasPhase::Cancelled);
    assert_eq!(b.cancellation().unwrap().code, cancel_code::MISMATCHED_SAS);
    assert!(b.confirm().is_err());
    assert!(b.verified_master_key().is_none());
}

/// サーバーが Alice に、Bob の偽のクロス署名鍵（攻撃者の鍵で Bob の端末を署名したもの）を見せても、
/// Bob の端末の MAC は本物のマスター鍵なので、検証は失敗する。
#[test]
fn substituted_master_key_is_detected_by_mac() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let forged = CrossSigningKeys::generate(BOB).unwrap();
    let alice_view = keys_query(&alice, &[(&alice, &alice.keys), (&bob, &forged)]);
    let bob_view = honest(&bob, &[&alice, &bob]);

    let (mut a, request) =
        SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let request = only(deliver(&mut alice, 0, &mut bob, &request));
    let mut b = SasVerification::from_request(bob.identity(0), &request, &bob_view, NOW).unwrap();
    let ready = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &single(b.accept().unwrap(), VERIFICATION_READY_TYPE),
    ));
    let start = single(a.receive(&ready, &alice_view), VERIFICATION_START_TYPE);
    let start = only(deliver(&mut alice, 0, &mut bob, &start));
    let accept = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &b.receive(&start, &bob_view)[0],
    ));
    let key = only(deliver(
        &mut alice,
        0,
        &mut bob,
        &a.receive(&accept, &alice_view)[0],
    ));
    let key = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &b.receive(&key, &bob_view)[0],
    ));
    a.receive(&key, &alice_view);

    a.confirm().unwrap();
    let out = b.confirm().unwrap();
    let mac = only(deliver(&mut bob, 0, &mut alice, &out[0]));
    let cancel = single(a.receive(&mac, &alice_view), VERIFICATION_CANCEL_TYPE);
    assert_eq!(cancel.content["code"], cancel_code::KEY_MISMATCH);
    assert_eq!(a.phase(), SasPhase::Cancelled);
    assert!(a.verified_master_key().is_none());
}

/// 中継が `key` を差し替えると、コミットメントが合わずに取りやめる。
#[test]
fn replaced_key_breaks_the_commitment() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    let (mut a, request) =
        SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let request = only(deliver(&mut alice, 0, &mut bob, &request));
    let mut b = SasVerification::from_request(bob.identity(0), &request, &bob_view, NOW).unwrap();
    let ready = only(deliver(&mut bob, 0, &mut alice, &b.accept().unwrap()[0]));
    let start = only(deliver(
        &mut alice,
        0,
        &mut bob,
        &a.receive(&ready, &alice_view)[0],
    ));
    let accept = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &b.receive(&start, &bob_view)[0],
    ));
    let key = only(deliver(
        &mut alice,
        0,
        &mut bob,
        &a.receive(&accept, &alice_view)[0],
    ));
    let mut key = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &b.receive(&key, &bob_view)[0],
    ));
    let other = vodozemac::sas::Sas::new().public_key().to_base64();
    key.content["key"] = json!(other);
    let cancel = single(a.receive(&key, &alice_view), VERIFICATION_CANCEL_TYPE);
    assert_eq!(cancel.content["code"], cancel_code::MISMATCHED_COMMITMENT);
}

#[test]
fn old_or_future_requests_are_ignored() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    let (_, request) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let request = only(deliver(&mut alice, 0, &mut bob, &request));
    let late = NOW + VERIFICATION_TIMEOUT_MS + 1;
    assert!(matches!(
        SasVerification::from_request(bob.identity(0), &request, &bob_view, late),
        Err(CryptoError::Verification(_))
    ));
    let early = NOW - 6 * 60 * 1000;
    assert!(matches!(
        SasVerification::from_request(bob.identity(0), &request, &bob_view, early),
        Err(CryptoError::Verification(_))
    ));
    assert!(
        SasVerification::from_request(bob.identity(0), &request, &bob_view, NOW + 1000).is_ok()
    );
}

#[test]
fn requests_from_devices_without_cross_signing_are_ignored() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let (_, request) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let request = only(deliver(&mut alice, 0, &mut bob, &request));
    // Bob から見た Alice の端末が、Alice の自己署名鍵で署名されていない。
    let other = CrossSigningKeys::generate(ALICE).unwrap();
    let mut bob_view = honest(&bob, &[&bob]);
    let forged = keys_query(&bob, &[(&alice, &other)]);
    bob_view
        .master_keys
        .insert(ALICE.into(), alice.keys.master.public_key_json());
    bob_view.self_signing_keys.insert(
        ALICE.into(),
        alice
            .keys
            .master
            .sign_subkey(&alice.keys.self_signing)
            .unwrap(),
    );
    bob_view.device_keys.extend(forged.device_keys);
    assert!(matches!(
        SasVerification::from_request(bob.identity(0), &request, &bob_view, NOW),
        Err(CryptoError::Verification(_))
    ));
}

/// 遅れて届いた要求の期限は、受け取った時刻ではなく要求の時刻から 10 分。
#[test]
fn late_request_expires_ten_minutes_after_its_timestamp() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    let (_, request) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let request = only(deliver(&mut alice, 0, &mut bob, &request));
    let received = NOW + 9 * 60 * 1000;
    let mut b =
        SasVerification::from_request(bob.identity(0), &request, &bob_view, received).unwrap();
    assert_eq!(b.started_at_ms(), NOW);
    assert!(b.expire(NOW + VERIFICATION_TIMEOUT_MS - 1).is_empty());
    let cancel = single(
        b.expire(NOW + VERIFICATION_TIMEOUT_MS),
        VERIFICATION_CANCEL_TYPE,
    );
    assert_eq!(cancel.content["code"], cancel_code::TIMEOUT);
}

#[test]
fn verification_times_out_after_ten_minutes() {
    let alice = User::new(ALICE, 1);
    let bob = User::new(BOB, 2);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let (mut a, _) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    assert!(a.expire(NOW + VERIFICATION_TIMEOUT_MS - 1).is_empty());
    let cancel = single(
        a.expire(NOW + VERIFICATION_TIMEOUT_MS),
        VERIFICATION_CANCEL_TYPE,
    );
    // 相手の端末が決まっていなければ、要求を送ったすべての端末に取りやめを送る。
    assert_eq!(cancel.devices.len(), 2);
    assert_eq!(cancel.content["code"], cancel_code::TIMEOUT);
    assert_eq!(a.phase(), SasPhase::Cancelled);
    assert!(a.cancellation().unwrap().by_us);
}

#[test]
fn signing_requires_the_verified_master_key() {
    let Flow {
        mut alice,
        mut bob,
        mut a,
        mut b,
    } = exchange_keys();
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    // 終わる前は署名できない。
    assert!(
        a.sign_master_key(
            &mut alice.trust,
            &alice.keys.user_signing,
            &bob.keys.master.public_key_json()
        )
        .is_err()
    );
    let mac = only(deliver(&mut alice, 0, &mut bob, &a.confirm().unwrap()[0]));
    b.receive(&mac, &bob_view);
    let out = b.confirm().unwrap();
    let mac = only(deliver(&mut bob, 0, &mut alice, &out[0]));
    let done = only(deliver(&mut bob, 0, &mut alice, &out[1]));
    a.receive(&mac, &alice_view);
    a.receive(&done, &alice_view);
    assert_eq!(a.phase(), SasPhase::Done);
    // 検証のあとで Bob がマスター鍵を作り直していたら、新しい鍵には署名しない。
    let replaced = CrossSigningKeys::generate(BOB).unwrap();
    assert!(matches!(
        a.sign_master_key(
            &mut alice.trust,
            &alice.keys.user_signing,
            &replaced.master.public_key_json()
        ),
        Err(CryptoError::Verification(_))
    ));
    assert!(alice.trust.record(BOB).is_none());
}

/// 受信時の検証：`keys/query` にない端末からの検証のメッセージは捨てる。
#[test]
fn unlisted_sender_device_is_rejected() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let (_, request) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let target = &request.devices[0];
    let otk = {
        let receiver = &mut bob.devices[0].account;
        receiver.generate_one_time_keys(1);
        let otk = receiver.one_time_keys().into_values().next().unwrap();
        receiver.mark_keys_as_published();
        otk
    };
    let sender = &mut alice.devices[0];
    let mut session = sender
        .account
        .create_outbound_olm_session(target, &otk)
        .unwrap();
    let content = sender
        .account
        .encrypt_verification(&mut session, target, request.event_type, &request.content)
        .unwrap();
    let receiver = &mut bob.devices[0];
    let decrypted = receiver
        .account
        .decrypt_to_device(&mut receiver.inbound, &content)
        .unwrap();
    assert!(matches!(
        decrypted.verify_verification(&receiver.account, ListedDevice::NotListed),
        Err(CryptoError::Rejected(Rejection::UnknownSenderDevice))
    ));
    // zoo.room_key としては受け付けない。
    let listed = alice.signed_device_keys(0);
    assert!(
        decrypted
            .verify_room_key(&receiver.account, ListedDevice::Listed(&listed), None)
            .is_err()
    );
    // 検証の type 以外は暗号化しない。
    assert!(
        alice.devices[0]
            .account
            .encrypt_verification(&mut session, target, "zoo.room_key", &json!({}))
            .is_err()
    );
}

/// 自分は SAS で検証しない。
#[test]
fn cannot_request_verification_of_self() {
    let alice = User::new(ALICE, 2);
    let view = honest(&alice, &[&alice]);
    assert!(matches!(
        SasVerification::request(alice.identity(0), &view, ALICE, NOW),
        Err(CryptoError::Verification(_))
    ));
}

/// 受け入れ（#362）と SAS の記録は、ピンを保存できるまで登録待ちで、ピンがなければ配らない。
#[test]
fn accepted_master_key_waits_for_the_pin() {
    let mut alice = User::new(ALICE, 1);
    let bob = User::new(BOB, 1);
    let accepted = alice
        .trust
        .accept_master_key(
            &alice.keys.user_signing,
            BOB,
            &bob.keys.master.public_key_json(),
        )
        .unwrap();
    let snapshot = KeySnapshot::from_json(&KeySnapshotJson::from(&alice, &bob)).unwrap();
    let device = alice.devices[0].account.device_id().to_owned();
    let result = alice
        .trust
        .evaluate(&snapshot, &device, Some(&alice.keys.user_signing));
    assert_eq!(result.users[BOB].state, UserState::PinPending);
    assert_eq!(result.new_pins.len(), 1);
    alice
        .trust
        .confirm_accepted_master_key(BOB, &accepted.master_key);
    let result = alice
        .trust
        .evaluate(&snapshot, &device, Some(&alice.keys.user_signing));
    assert_eq!(
        result.users[BOB].state,
        UserState::Trusted(TrustBasis::Record)
    );
}
