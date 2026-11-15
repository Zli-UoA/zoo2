//! 他のユーザーの絵文字 SAS 検証（#368。docs/e2ee.md の「絵文字 SAS 検証」）。
//!
//! 2 人のユーザーの端末どうしで、Olm で暗号化した `m.key.verification.*` をやり取りし、互いのマスター鍵を
//! 検証して署名するまでを、本物の暗号で確かめる。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use zoo_crypto::{
    CrossSigningKeys, CryptoError, DeviceAccount, KeySnapshot, ListedDevice, OlmSession,
    QrCodeData, Rejection, SasPhase, SasVerification, TrustBasis, TrustStore, UserState,
    VERIFICATION_CANCEL_TYPE, VERIFICATION_DONE_TYPE, VERIFICATION_KEY_TYPE, VERIFICATION_MAC_TYPE,
    VERIFICATION_READY_TYPE, VERIFICATION_REQUEST_TYPE, VERIFICATION_START_TYPE,
    VERIFICATION_TIMEOUT_MS, VerificationIdentity, VerificationMessage, VerificationMethod,
    VerificationOutgoing, cancel_code,
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

    // 端末 0 が受け入れる。Alice は端末 1 に取りやめ（m.accepted）を送る。Bob も QR に対応しているので、
    // 双方が方式を選ぶ段階になる。
    let ready = single(b.accept().unwrap(), VERIFICATION_READY_TYPE);
    let ready = only(deliver(&mut bob, 0, &mut alice, &ready));
    let cancel = single(a.receive(&ready, &alice_view), VERIFICATION_CANCEL_TYPE);
    assert_eq!(cancel.content["code"], cancel_code::ACCEPTED);
    let cancel = only(deliver(&mut alice, 0, &mut bob, &cancel));
    assert!(other.receive(&cancel, &bob_view).is_empty());
    assert_eq!(other.phase(), SasPhase::Cancelled);
    assert!(!other.cancellation().unwrap().by_us);
    assert_eq!(a.phase(), SasPhase::ChooseMethod);
    assert_eq!(b.phase(), SasPhase::ChooseMethod);

    // Alice が絵文字を選ぶ。
    let start = single(a.start_sas().unwrap(), VERIFICATION_START_TYPE);
    assert_eq!(a.method(), Some(VerificationMethod::Sas));
    let start = only(deliver(&mut alice, 0, &mut bob, &start));
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
    assert!(a.receive(&ready, &alice_view).is_empty());
    let start = single(a.start_sas().unwrap(), VERIFICATION_START_TYPE);
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
    assert!(a.receive(&ready, &alice_view).is_empty());
    let start = only(deliver(&mut alice, 0, &mut bob, &a.start_sas().unwrap()[0]));
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

// ---- QR コードによる検証（docs/e2ee.md の「QR コードによる検証」） ----

/// Alice（端末 1 つ）が Bob（端末 1 つ）に検証を求め、Bob が受け入れて、双方が方式を選ぶ段階まで進める。
/// `alice_view` と `bob_view` は、それぞれから見た `keys/query`。
fn choose_method(
    alice: &mut User,
    bob: &mut User,
    alice_view: &KeySnapshot,
    bob_view: &KeySnapshot,
) -> (SasVerification, SasVerification) {
    let (mut a, request) =
        SasVerification::request(alice.identity(0), alice_view, BOB, NOW).unwrap();
    assert_eq!(
        request.content["methods"],
        json!([
            "m.qr_code.show.v1",
            "m.qr_code.scan.v1",
            "m.reciprocate.v1",
            "m.sas.v1"
        ])
    );
    let request = only(deliver(alice, 0, bob, &request));
    let mut b = SasVerification::from_request(bob.identity(0), &request, bob_view, NOW).unwrap();
    // 受け入れる前は、QR を出さない。
    assert!(b.qr_code().is_none());
    let ready = only(deliver(bob, 0, alice, &b.accept().unwrap()[0]));
    assert!(a.receive(&ready, alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::ChooseMethod);
    assert_eq!(b.phase(), SasPhase::ChooseMethod);
    (a, b)
}

fn honest_pair() -> (User, User, KeySnapshot, KeySnapshot) {
    let alice = User::new(ALICE, 1);
    let bob = User::new(BOB, 1);
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    (alice, bob, alice_view, bob_view)
}

#[test]
fn qr_code_has_the_matrix_layout() {
    let (alice, bob, alice_view, _) = honest_pair();
    let (a, _) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    // 要求した側は、相手の受け入れより前から QR を出す。
    let bytes = a.qr_code().unwrap();
    assert_eq!(&bytes[..6], b"MATRIX");
    assert_eq!(bytes[6], 0x02);
    assert_eq!(bytes[7], 0x00);
    let txn = a.transaction_id().as_bytes();
    assert_eq!(u16::from_be_bytes([bytes[8], bytes[9]]) as usize, txn.len());
    assert_eq!(&bytes[10..10 + txn.len()], txn);
    let data = QrCodeData::from_bytes(&bytes).unwrap();
    assert_eq!(data.transaction_id, a.transaction_id());
    assert_eq!(data.their_master_key, alice.keys.master.public_key());
    assert_eq!(data.our_master_key, bob.keys.master.public_key());
    assert_eq!(data.secret.len(), 16);
    // 検証ごとに共有の秘密を作る。
    let (other, _) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    assert_ne!(
        QrCodeData::from_bytes(&other.qr_code().unwrap())
            .unwrap()
            .secret,
        data.secret
    );

    // 形の正しくないものは受け付けない。
    let mut self_verifying = bytes.clone();
    self_verifying[7] = 0x01;
    assert!(matches!(
        QrCodeData::from_bytes(&self_verifying),
        Err(CryptoError::InvalidPayload(_))
    ));
    let short_secret = &bytes[..bytes.len() - 9];
    assert!(QrCodeData::from_bytes(short_secret).is_err());
    assert!(QrCodeData::from_bytes(&bytes[..bytes.len() - 8]).is_ok());
    assert!(QrCodeData::from_bytes(b"MATRIX").is_err());
    let mut other_version = bytes.clone();
    other_version[6] = 0x01;
    assert!(QrCodeData::from_bytes(&other_version).is_err());
}

/// Bob が Alice の QR を読み取り、Alice が「読み取った」と答えると、双方が相手のマスター鍵を検証できる。
#[test]
fn qr_two_users_verify_each_other() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    assert!(b.can_scan_qr_code());
    assert!(b.qr_code().is_some());

    let start = single(
        b.scan_qr_code(&a.qr_code().unwrap()).unwrap(),
        VERIFICATION_START_TYPE,
    );
    assert_eq!(start.content["method"], "m.reciprocate.v1");
    assert_eq!(b.phase(), SasPhase::QrScanned);
    assert_eq!(b.method(), Some(VerificationMethod::QrCode));
    // 読み取ったあとは、自分の QR も絵文字も出さない。
    assert!(b.qr_code().is_none());
    assert!(!b.can_start_sas());

    let start = only(deliver(&mut bob, 0, &mut alice, &start));
    assert!(a.receive(&start, &alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::QrConfirm);
    assert!(a.qr_code().is_none());
    // 利用者が答えるまでは検証済みにしない。
    assert!(a.verified_master_key().is_none());

    let done = single(a.confirm_qr_scanned().unwrap(), VERIFICATION_DONE_TYPE);
    assert_eq!(a.phase(), SasPhase::Done);
    assert_eq!(a.verified_master_key(), Some(bob.keys.master.public_key()));
    // 読み取った側は、表示した側の done を受けるまで検証済みにしない。
    assert!(b.verified_master_key().is_none());
    let done = only(deliver(&mut alice, 0, &mut bob, &done));
    let done = single(b.receive(&done, &bob_view), VERIFICATION_DONE_TYPE);
    assert_eq!(b.phase(), SasPhase::Done);
    assert_eq!(
        b.verified_master_key(),
        Some(alice.keys.master.public_key())
    );
    // 表示した側は、あとから届いた done を無視する。
    let done = only(deliver(&mut bob, 0, &mut alice, &done));
    assert!(a.receive(&done, &alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::Done);

    // SAS と同じく、相手のマスター鍵に署名し、記録はピンを保存するまで登録待ちにする。
    let accepted = a
        .sign_master_key(
            &mut alice.trust,
            &alice.keys.user_signing,
            &bob.keys.master.public_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.target_user_id, BOB);
    assert!(alice.trust.record(BOB).unwrap().pin_pending);
    let accepted = b
        .sign_master_key(
            &mut bob.trust,
            &bob.keys.user_signing,
            &alice.keys.master.public_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.target_user_id, ALICE);
    assert!(bob.trust.record(ALICE).unwrap().pin_pending);
}

/// 要求した側が、相手の QR を読み取ることもできる。
#[test]
fn requester_can_scan_the_other_qr() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let start = single(
        a.scan_qr_code(&b.qr_code().unwrap()).unwrap(),
        VERIFICATION_START_TYPE,
    );
    let start = only(deliver(&mut alice, 0, &mut bob, &start));
    assert!(b.receive(&start, &bob_view).is_empty());
    assert_eq!(b.phase(), SasPhase::QrConfirm);
    let done = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &b.confirm_qr_scanned().unwrap()[0],
    ));
    single(a.receive(&done, &alice_view), VERIFICATION_DONE_TYPE);
    assert_eq!(a.verified_master_key(), Some(bob.keys.master.public_key()));
    assert_eq!(
        b.verified_master_key(),
        Some(alice.keys.master.public_key())
    );
}

/// サーバーが Alice に Bob の偽のマスター鍵を見せると、Alice の QR の 2 つめの鍵が Bob の本物の鍵と
/// 合わないので、Bob が読み取ったときに取りやめる。どちらも署名しない。
#[test]
fn qr_with_forged_key_shown_to_the_displayer_is_rejected() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let forged = CrossSigningKeys::generate(BOB).unwrap();
    let alice_view = keys_query(&alice, &[(&alice, &alice.keys), (&bob, &forged)]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);

    let cancel = single(
        b.scan_qr_code(&a.qr_code().unwrap()).unwrap(),
        VERIFICATION_CANCEL_TYPE,
    );
    assert_eq!(cancel.content["code"], cancel_code::KEY_MISMATCH);
    assert_eq!(b.phase(), SasPhase::Cancelled);
    let cancel = only(deliver(&mut bob, 0, &mut alice, &cancel));
    assert!(a.receive(&cancel, &alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::Cancelled);
    assert!(a.verified_master_key().is_none());
    assert!(b.verified_master_key().is_none());
}

/// サーバーが Bob に Alice の偽のマスター鍵を見せると、QR の 1 つめの鍵（Alice の本物）と合わない。
#[test]
fn qr_with_forged_key_shown_to_the_scanner_is_rejected() {
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let forged = CrossSigningKeys::generate(ALICE).unwrap();
    let alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = keys_query(&bob, &[(&alice, &forged), (&bob, &bob.keys)]);
    let (a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let cancel = single(
        b.scan_qr_code(&a.qr_code().unwrap()).unwrap(),
        VERIFICATION_CANCEL_TYPE,
    );
    assert_eq!(cancel.content["code"], cancel_code::KEY_MISMATCH);
    assert!(b.verified_master_key().is_none());

    // 表示した側の QR の鍵と相手の端末は、ready のときに決まる。そのあとでサーバーが別の鍵を見せても、
    // start の処理には使わない（署名の直前の keys/query の照合は sign_master_key が行う）。
    let mut alice = User::new(ALICE, 1);
    let mut bob = User::new(BOB, 1);
    let forged_bob = CrossSigningKeys::generate(BOB).unwrap();
    let honest_alice_view = honest(&alice, &[&alice, &bob]);
    let bob_view = honest(&bob, &[&alice, &bob]);
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &honest_alice_view, &bob_view);
    let start = single(
        b.scan_qr_code(&a.qr_code().unwrap()).unwrap(),
        VERIFICATION_START_TYPE,
    );
    let start = only(deliver(&mut bob, 0, &mut alice, &start));
    let forged_view = keys_query(&alice, &[(&alice, &alice.keys), (&bob, &forged_bob)]);
    assert!(a.receive(&start, &forged_view).is_empty());
    assert_eq!(a.phase(), SasPhase::QrConfirm);
    a.confirm_qr_scanned().unwrap();
    assert!(matches!(
        a.sign_master_key(
            &mut alice.trust,
            &alice.keys.user_signing,
            &forged_bob.master.public_key_json()
        ),
        Err(CryptoError::Verification(_))
    ));
}

/// `start` の `secret` が QR の共有の秘密と違えば、表示した側が `m.key_mismatch` で取りやめる。
#[test]
fn qr_wrong_secret_is_rejected() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let start = single(
        b.scan_qr_code(&a.qr_code().unwrap()).unwrap(),
        VERIFICATION_START_TYPE,
    );
    let mut start = only(deliver(&mut bob, 0, &mut alice, &start));
    start.content["secret"] = json!(vodozemac::base64_encode([7u8; 16]));
    let cancel = single(a.receive(&start, &alice_view), VERIFICATION_CANCEL_TYPE);
    assert_eq!(cancel.content["code"], cancel_code::KEY_MISMATCH);
    assert_eq!(a.phase(), SasPhase::Cancelled);
    assert!(a.confirm_qr_scanned().is_err());
    assert!(a.verified_master_key().is_none());
}

/// 前の検証の QR（撮った画像）は、`transaction_id` が違うので受け付けない。取りやめずに読み直せる。
#[test]
fn qr_from_another_transaction_is_not_accepted() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (old, _) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let old_qr = old.qr_code().unwrap();
    let (a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    assert!(matches!(
        b.scan_qr_code(&old_qr),
        Err(CryptoError::Verification(_))
    ));
    assert_eq!(b.phase(), SasPhase::ChooseMethod);
    // 形の正しくないものも同じ。
    assert!(matches!(
        b.scan_qr_code(b"https://example.com"),
        Err(CryptoError::InvalidPayload(_))
    ));
    assert_eq!(b.phase(), SasPhase::ChooseMethod);
    // 正しい QR は読み取れる。
    single(
        b.scan_qr_code(&a.qr_code().unwrap()).unwrap(),
        VERIFICATION_START_TYPE,
    );
}

/// 期限（要求から 10 分）を過ぎた検証の QR は、どちらの側でも使えない。
#[test]
fn qr_verification_expires() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let qr = a.qr_code().unwrap();
    let start = single(b.scan_qr_code(&qr).unwrap(), VERIFICATION_START_TYPE);
    let start = only(deliver(&mut bob, 0, &mut alice, &start));
    assert!(a.receive(&start, &alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::QrConfirm);

    // 利用者が答える前に期限が過ぎた。
    let cancel = single(
        a.expire(NOW + VERIFICATION_TIMEOUT_MS),
        VERIFICATION_CANCEL_TYPE,
    );
    assert_eq!(cancel.content["code"], cancel_code::TIMEOUT);
    assert!(a.confirm_qr_scanned().is_err());
    assert!(a.verified_master_key().is_none());
    assert!(a.qr_code().is_none());
    let cancel = only(deliver(&mut alice, 0, &mut bob, &cancel));
    assert!(b.receive(&cancel, &bob_view).is_empty());
    assert_eq!(b.phase(), SasPhase::Cancelled);
    assert!(b.verified_master_key().is_none());

    // 読み取る前に期限が過ぎた側は、QR を読み取れない。
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    single(
        b.expire(NOW + VERIFICATION_TIMEOUT_MS),
        VERIFICATION_CANCEL_TYPE,
    );
    assert!(b.scan_qr_code(&a.qr_code().unwrap()).is_err());
    assert!(b.qr_code().is_none());
}

/// 表示した側の利用者が「読み取っていない」と答えたら、`m.user` で取りやめる。
#[test]
fn displayer_can_deny_the_scan() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let start = single(
        b.scan_qr_code(&a.qr_code().unwrap()).unwrap(),
        VERIFICATION_START_TYPE,
    );
    let start = only(deliver(&mut bob, 0, &mut alice, &start));
    a.receive(&start, &alice_view);
    let cancel = single(a.cancel(), VERIFICATION_CANCEL_TYPE);
    assert_eq!(cancel.content["code"], cancel_code::USER);
    let cancel = only(deliver(&mut alice, 0, &mut bob, &cancel));
    b.receive(&cancel, &bob_view);
    assert_eq!(b.phase(), SasPhase::Cancelled);
    assert!(b.verified_master_key().is_none());
}

/// 方式を選ぶ段階で、受けた側も絵文字を選べる。両方が同時に選んだら、`user_id` の小さい側の `start` を使う。
#[test]
fn simultaneous_sas_starts_use_the_smaller_user_id() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let a_start = single(a.start_sas().unwrap(), VERIFICATION_START_TYPE);
    let b_start = single(b.start_sas().unwrap(), VERIFICATION_START_TYPE);
    let a_start = only(deliver(&mut alice, 0, &mut bob, &a_start));
    let b_start = only(deliver(&mut bob, 0, &mut alice, &b_start));
    // ALICE < BOB なので、Alice は Bob の start を無視し、Bob は Alice の start に accept を返す。
    assert!(a.receive(&b_start, &alice_view).is_empty());
    let accept = single(b.receive(&a_start, &bob_view), "m.key.verification.accept");
    let accept = only(deliver(&mut bob, 0, &mut alice, &accept));
    let key = only(deliver(
        &mut alice,
        0,
        &mut bob,
        &single(a.receive(&accept, &alice_view), VERIFICATION_KEY_TYPE),
    ));
    let key = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &single(b.receive(&key, &bob_view), VERIFICATION_KEY_TYPE),
    ));
    assert!(a.receive(&key, &alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::Compare);
    assert_eq!(b.phase(), SasPhase::Compare);
    assert_eq!(a.emoji_indices(), b.emoji_indices());
}

/// 受けた側が絵文字を選んでも、要求した側の QR を読み取っても、SAS の start を受けた側は、QR の
/// reciprocate を受け付けない（方式は 1 つの検証の中で変えない）。
#[test]
fn method_does_not_change_after_start() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let qr = a.qr_code().unwrap();
    let sas_start = single(b.start_sas().unwrap(), VERIFICATION_START_TYPE);
    // Bob はもう SAS を始めたので、QR を読み取れない。
    assert!(b.scan_qr_code(&qr).is_err());
    let sas_start = only(deliver(&mut bob, 0, &mut alice, &sas_start));
    single(
        a.receive(&sas_start, &alice_view),
        "m.key.verification.accept",
    );
    assert_eq!(a.method(), Some(VerificationMethod::Sas));
    assert!(a.qr_code().is_none());
}

/// #368 のクライアント（QR に対応していない）とも、今までどおり絵文字で検証できる。
#[test]
fn peers_without_qr_support_fall_back_to_sas() {
    // 受けた側が #368 のクライアント：ready に m.sas.v1 しかないので、要求した側はすぐに start を送る。
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, request) =
        SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let request = only(deliver(&mut alice, 0, &mut bob, &request));
    let mut b = SasVerification::from_request(bob.identity(0), &request, &bob_view, NOW).unwrap();
    let mut ready = only(deliver(&mut bob, 0, &mut alice, &b.accept().unwrap()[0]));
    ready.content["methods"] = json!(["m.sas.v1"]);
    let start = single(a.receive(&ready, &alice_view), VERIFICATION_START_TYPE);
    assert_eq!(start.content["method"], "m.sas.v1");
    assert_eq!(a.phase(), SasPhase::Exchanging);
    assert!(a.qr_code().is_none());

    // 要求した側が #368 のクライアント：受けた側は QR を出さず、絵文字も選ばず、相手の start を待つ。
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (_, request) = SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    let mut request = only(deliver(&mut alice, 0, &mut bob, &request));
    request.content["methods"] = json!(["m.sas.v1"]);
    let mut b = SasVerification::from_request(bob.identity(0), &request, &bob_view, NOW).unwrap();
    b.accept().unwrap();
    assert_eq!(b.phase(), SasPhase::Exchanging);
    assert!(b.qr_code().is_none());
    assert!(!b.can_scan_qr_code());
    assert!(!b.can_start_sas());
    assert!(b.start_sas().is_err());
}

/// 要求した側が、相手の受け入れより前に絵文字を選ぶと、ready を受けてすぐに SAS を始める。
#[test]
fn choosing_sas_before_ready_starts_it_on_ready() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, request) =
        SasVerification::request(alice.identity(0), &alice_view, BOB, NOW).unwrap();
    assert!(a.can_start_sas());
    assert!(a.start_sas().unwrap().is_empty());
    assert!(a.qr_code().is_none());
    assert!(!a.can_start_sas());
    let request = only(deliver(&mut alice, 0, &mut bob, &request));
    let mut b = SasVerification::from_request(bob.identity(0), &request, &bob_view, NOW).unwrap();
    let ready = only(deliver(&mut bob, 0, &mut alice, &b.accept().unwrap()[0]));
    let start = single(a.receive(&ready, &alice_view), VERIFICATION_START_TYPE);
    let start = only(deliver(&mut alice, 0, &mut bob, &start));
    single(b.receive(&start, &bob_view), "m.key.verification.accept");
    assert_eq!(b.phase(), SasPhase::Exchanging);
}

/// 双方が同時に相手の QR を読み取ったら、Matrix の規則どおり `user_id` が大きい側（Bob）の `start` を
/// 無視し、Alice が Bob の QR を読み取ったものとして進める。双方が検証済みになる。
#[test]
fn simultaneous_qr_scans_use_the_smaller_user_id() {
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let (a_qr, b_qr) = (a.qr_code().unwrap(), b.qr_code().unwrap());
    let a_start = single(a.scan_qr_code(&b_qr).unwrap(), VERIFICATION_START_TYPE);
    let b_start = single(b.scan_qr_code(&a_qr).unwrap(), VERIFICATION_START_TYPE);
    assert_eq!(a.phase(), SasPhase::QrScanned);
    assert_eq!(b.phase(), SasPhase::QrScanned);
    let a_start = only(deliver(&mut alice, 0, &mut bob, &a_start));
    let b_start = only(deliver(&mut bob, 0, &mut alice, &b_start));

    // ALICE < BOB：Alice は Bob の start を無視し、Bob の done を待つ。
    assert!(a.receive(&b_start, &alice_view).is_empty());
    assert_eq!(a.phase(), SasPhase::QrScanned);
    // Bob は自分の読み取りを捨て、Alice が自分の QR を読み取ったものとして利用者に聞く。
    assert!(b.receive(&a_start, &bob_view).is_empty());
    assert_eq!(b.phase(), SasPhase::QrConfirm);

    let done = only(deliver(
        &mut bob,
        0,
        &mut alice,
        &single(b.confirm_qr_scanned().unwrap(), VERIFICATION_DONE_TYPE),
    ));
    let done = only(deliver(
        &mut alice,
        0,
        &mut bob,
        &single(a.receive(&done, &alice_view), VERIFICATION_DONE_TYPE),
    ));
    assert!(b.receive(&done, &bob_view).is_empty());
    assert_eq!(a.verified_master_key(), Some(bob.keys.master.public_key()));
    assert_eq!(
        b.verified_master_key(),
        Some(alice.keys.master.public_key())
    );

    // 大きい側が処理する start も、共有の秘密を確かめる。
    let (mut alice, mut bob, alice_view, bob_view) = honest_pair();
    let (mut a, mut b) = choose_method(&mut alice, &mut bob, &alice_view, &bob_view);
    let (a_qr, b_qr) = (a.qr_code().unwrap(), b.qr_code().unwrap());
    let a_start = single(a.scan_qr_code(&b_qr).unwrap(), VERIFICATION_START_TYPE);
    b.scan_qr_code(&a_qr).unwrap();
    let mut a_start = only(deliver(&mut alice, 0, &mut bob, &a_start));
    a_start.content["secret"] = json!(vodozemac::base64_encode([7u8; 16]));
    let cancel = single(b.receive(&a_start, &bob_view), VERIFICATION_CANCEL_TYPE);
    assert_eq!(cancel.content["code"], cancel_code::KEY_MISMATCH);
}
