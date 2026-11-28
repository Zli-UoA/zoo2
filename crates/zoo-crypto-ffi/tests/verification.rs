//! 他のユーザーの検証（絵文字 SAS と QR コード）のバインディングを、Kotlin と同じ流れで通しで確かめる。
//! メッセージは Olm で暗号化して届け、受信時の検証（`OlmDecrypted.verify_verification`）を通す。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use zoo_crypto_ffi::*;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const ALICE_DEVICE: &str = "00000000-0000-4000-8000-0000000000a1";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const BOB_DEVICE: &str = "00000000-0000-4000-8000-0000000000b1";
const NOW: u64 = 1_800_000_000_000;

struct User {
    id: &'static str,
    keys: Arc<CrossSigningKeys>,
    account: Arc<DeviceAccount>,
    trust: Arc<TrustStore>,
    /// 受信で作った Olm セッション。
    inbound: Arc<OlmSessionStore>,
    /// 送信用の Olm セッション。
    outbound: Arc<OlmSessionStore>,
    /// 送信用のセッション ID（相手の識別鍵ごと）。
    outbound_ids: HashMap<String, String>,
}

impl User {
    fn new(id: &'static str, device: &str) -> Self {
        let keys = CrossSigningKeys::generate(id.into()).unwrap();
        let trust = TrustStore::new(id.into()).unwrap();
        trust.trust_own_master_key(&keys.master()).unwrap();
        trust
            .record_own_user_signing_key(&keys.user_signing())
            .unwrap();
        Self {
            id,
            keys,
            account: DeviceAccount::new(id.into(), device.into()).unwrap(),
            trust,
            inbound: OlmSessionStore::new(),
            outbound: OlmSessionStore::new(),
            outbound_ids: HashMap::new(),
        }
    }

    /// 自己署名鍵の署名を埋め込んだ端末鍵。
    fn signed_device_keys(&self) -> Value {
        let mut keys: Value = serde_json::from_str(&self.account.device_keys()).unwrap();
        let ssk = self.keys.self_signing();
        let upload = ssk.sign_device(keys.to_string()).unwrap();
        keys["signatures"][self.id][ssk.key_id()] = json!(upload.signature);
        keys
    }

    fn master_key_json(&self) -> String {
        self.keys.master().public_key_json()
    }
}

/// `viewer` から見た `keys/query` の応答（JSON）。
fn keys_query(viewer: &User, users: &[&User]) -> String {
    let master = viewer.keys.master();
    let mut value = json!({
        "user_ids": users.iter().map(|u| u.id).collect::<Vec<_>>(),
        "device_keys": {},
        "master_keys": {},
        "self_signing_keys": {},
        "user_signing_keys": {
            viewer.id: serde_json::from_str::<Value>(
                &master.sign_subkey(&viewer.keys.user_signing()).unwrap()
            ).unwrap(),
        },
        "signatures": [],
        "tofu_pins": {},
    });
    for user in users {
        let master = user.keys.master();
        value["master_keys"][user.id] = serde_json::from_str(&master.public_key_json()).unwrap();
        value["self_signing_keys"][user.id] =
            serde_json::from_str(&master.sign_subkey(&user.keys.self_signing()).unwrap()).unwrap();
        value["device_keys"][user.id] =
            json!({ user.account.device_id(): user.signed_device_keys() });
    }
    value.to_string()
}

/// `from` から `to` に、送るメッセージを Olm で暗号化して届け、受信時の検証を通す（Web の `sasEncrypt`
/// と、`decryptToDevice` の検証のメッセージの分岐）。
fn deliver(from: &mut User, to: &User, out: &VerificationOutgoing) -> VerificationMessage {
    assert_eq!(out.devices.len(), 1);
    let target = &out.devices[0];
    assert_eq!(target.device_id(), to.account.device_id());
    let curve = target.curve25519_key();
    let session_id = match from.outbound_ids.get(&curve) {
        Some(id) => id.clone(),
        None => {
            // `keys/claim` の代わり。
            to.account.generate_one_time_keys(1);
            let otks: Value = serde_json::from_str(&to.account.one_time_keys()).unwrap();
            let otk = otks.as_object().unwrap().values().next().unwrap().clone();
            to.account.mark_keys_as_published();
            let id = from
                .account
                .create_outbound_olm_session(&from.outbound, target, otk.to_string())
                .unwrap();
            from.outbound_ids.insert(curve, id.clone());
            id
        }
    };
    let content = from
        .account
        .encrypt_verification(
            &from.outbound,
            session_id,
            target,
            out.event_type.clone(),
            out.content.clone(),
        )
        .unwrap();
    let decrypted = to.account.decrypt_to_device(&to.inbound, content).unwrap();
    assert_eq!(
        decrypted.event_type().as_deref(),
        Some(out.event_type.as_str())
    );
    let message = decrypted
        .verify_verification(&to.account, Some(from.signed_device_keys().to_string()))
        .unwrap();
    assert_eq!(message.sender_user_id, from.id);
    assert_eq!(message.event_type, out.event_type);
    // iOS の暗号ストアへの保存を挟んでも、検証済みの端末と内容を保持する。
    let expected_sender = message.sender_user_id.clone();
    let expected_device = message.sender_device.clone();
    let expected_type = message.event_type.clone();
    let expected_content: serde_json::Value = serde_json::from_str(&message.content).unwrap();
    let stored = verification_message_to_json(message).unwrap();
    let restored = verification_message_from_json(stored).unwrap();
    assert_eq!(restored.sender_user_id, expected_sender);
    assert_eq!(restored.event_type, expected_type);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&restored.content).unwrap(),
        expected_content
    );
    assert_eq!(restored.sender_device.user_id(), expected_device.user_id());
    assert_eq!(
        restored.sender_device.device_id(),
        expected_device.device_id()
    );
    assert_eq!(
        restored.sender_device.algorithms(),
        expected_device.algorithms()
    );
    assert_eq!(
        restored.sender_device.curve25519_key(),
        expected_device.curve25519_key()
    );
    assert_eq!(
        restored.sender_device.ed25519_key(),
        expected_device.ed25519_key()
    );
    restored
}

fn single(out: Vec<VerificationOutgoing>, event_type: &str) -> VerificationOutgoing {
    assert_eq!(
        out.iter()
            .map(|o| o.event_type.as_str())
            .collect::<Vec<_>>(),
        vec![event_type]
    );
    out.into_iter().next().unwrap()
}

fn content(out: &VerificationOutgoing) -> Value {
    serde_json::from_str(&out.content).unwrap()
}

/// Alice が Bob に検証を求め、Bob が受け入れて、双方が方式を選ぶ段階まで進める。
fn choose_method(alice: &mut User, bob: &mut User) -> (Arc<SasVerification>, Arc<SasVerification>) {
    let alice_view = keys_query(alice, &[alice, bob]);
    let bob_view = keys_query(bob, &[alice, bob]);

    let targets = verification_devices(alice_view.clone(), BOB.into()).unwrap();
    assert_eq!(targets.master_key, bob.keys.master().public_key());
    assert_eq!(targets.devices.len(), 1);

    let SasRequest {
        verification: a,
        outgoing,
    } = request_verification(
        &alice.account,
        &alice.trust,
        alice_view.clone(),
        BOB.into(),
        NOW,
    )
    .unwrap();
    assert_eq!(a.view().phase, SasPhase::Requested);
    assert!(a.view().we_requested);
    let request = single(outgoing, "m.key.verification.request");
    let request = deliver(alice, bob, &request);
    let b =
        SasVerification::from_request(&bob.account, &bob.trust, request, bob_view, NOW).unwrap();
    assert_eq!(b.view().phase, SasPhase::Incoming);
    assert_eq!(b.transaction_id(), a.transaction_id());
    assert_eq!(b.view().their_device_id.as_deref(), Some(ALICE_DEVICE));
    assert_eq!(b.view().started_at, NOW);

    let ready = single(b.accept().unwrap(), "m.key.verification.ready");
    let ready = deliver(bob, alice, &ready);
    assert!(a.receive(ready, alice_view).unwrap().is_empty());
    assert_eq!(a.view().phase, SasPhase::ChooseMethod);
    assert_eq!(b.view().phase, SasPhase::ChooseMethod);
    (a, b)
}

/// 絵文字を比べる段階まで進める。
fn compare(alice: &mut User, bob: &mut User) -> (Arc<SasVerification>, Arc<SasVerification>) {
    let (a, b) = choose_method(alice, bob);
    let alice_view = keys_query(alice, &[alice, bob]);
    let bob_view = keys_query(bob, &[alice, bob]);
    assert!(a.view().can_start_sas);
    let start = single(a.start_sas().unwrap(), "m.key.verification.start");
    assert_eq!(a.view().method, Some(VerificationMethod::Sas));
    let start = deliver(alice, bob, &start);
    let accept = single(
        b.receive(start, bob_view.clone()).unwrap(),
        "m.key.verification.accept",
    );
    let accept = deliver(bob, alice, &accept);
    let key = single(
        a.receive(accept, alice_view.clone()).unwrap(),
        "m.key.verification.key",
    );
    let key = deliver(alice, bob, &key);
    let key = single(b.receive(key, bob_view).unwrap(), "m.key.verification.key");
    let key = deliver(bob, alice, &key);
    assert!(a.receive(key, alice_view).unwrap().is_empty());
    assert_eq!(a.view().phase, SasPhase::Compare);
    assert_eq!(b.view().phase, SasPhase::Compare);
    (a, b)
}

#[test]
fn emoji_sas_two_users_verify_each_other() {
    let mut alice = User::new(ALICE, ALICE_DEVICE);
    let mut bob = User::new(BOB, BOB_DEVICE);
    let (a, b) = compare(&mut alice, &mut bob);
    let alice_view = keys_query(&alice, &[&alice, &bob]);
    let bob_view = keys_query(&bob, &[&alice, &bob]);

    // 双方が同じ絵文字を見る。
    let emoji = a.view().emoji.unwrap();
    assert_eq!(emoji.len(), 7);
    assert!(emoji.iter().all(|i| *i < 64));
    assert_eq!(b.view().emoji, Some(emoji));

    let mac = single(a.confirm().unwrap(), "m.key.verification.mac");
    assert_eq!(a.view().phase, SasPhase::Confirmed);
    let mac = deliver(&mut alice, &bob, &mac);
    assert!(b.receive(mac, bob_view.clone()).unwrap().is_empty());

    let out = b.confirm().unwrap();
    assert_eq!(
        out.iter()
            .map(|o| o.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["m.key.verification.mac", "m.key.verification.done"]
    );
    let bob_mac = deliver(&mut bob, &alice, &out[0]);
    let bob_done = deliver(&mut bob, &alice, &out[1]);
    let done = single(
        a.receive(bob_mac, alice_view.clone()).unwrap(),
        "m.key.verification.done",
    );
    assert!(a.receive(bob_done, alice_view).unwrap().is_empty());
    assert_eq!(a.view().phase, SasPhase::Done);
    let done = deliver(&mut alice, &bob, &done);
    assert!(b.receive(done, bob_view).unwrap().is_empty());
    assert_eq!(b.view().phase, SasPhase::Done);

    assert_eq!(
        a.view().verified_master_key,
        Some(bob.keys.master().public_key())
    );
    assert_eq!(
        b.view().verified_master_key,
        Some(alice.keys.master().public_key())
    );

    // 相手のマスター鍵に署名する（Web の `sasSign`）。記録はピンを保存するまで登録待ち。
    let accepted = a
        .sign_master_key(
            &alice.trust,
            &alice.keys.user_signing(),
            bob.master_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.signature.target_user_id, BOB);
    assert_eq!(
        accepted.signature.signer_key_id,
        alice.keys.user_signing().key_id()
    );
    let pin: Value = serde_json::from_str(&accepted.pin).unwrap();
    assert_eq!(pin["master_key"], bob.keys.master().public_key());
    assert!(alice.trust.record(BOB.into()).unwrap().pin_pending);
    alice
        .trust
        .confirm_accepted_master_key(BOB.into(), bob.keys.master().public_key());
    assert!(!alice.trust.record(BOB.into()).unwrap().pin_pending);

    // 検証したあとで作り直されたマスター鍵には署名しない。
    let other = CrossSigningKeyPair::generate(BOB.into(), KeyUsage::Master).unwrap();
    assert!(matches!(
        a.sign_master_key(
            &alice.trust,
            &alice.keys.user_signing(),
            other.public_key_json()
        ),
        Err(ZooCryptoError::Verification { .. })
    ));

    let accepted = b
        .sign_master_key(
            &bob.trust,
            &bob.keys.user_signing(),
            alice.master_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.signature.target_user_id, ALICE);
}

#[test]
fn qr_code_two_users_verify_each_other() {
    let mut alice = User::new(ALICE, ALICE_DEVICE);
    let mut bob = User::new(BOB, BOB_DEVICE);
    let (a, b) = choose_method(&mut alice, &mut bob);
    let alice_view = keys_query(&alice, &[&alice, &bob]);
    let bob_view = keys_query(&bob, &[&alice, &bob]);

    let qr = a.view().qr_code.unwrap();
    assert_eq!(&qr[..6], b"MATRIX");
    assert!(b.view().can_scan_qr_code);

    // 形の正しくない QR は、取りやめずに読み直す。
    assert!(matches!(
        b.scan_qr_code(b"MATRIX".to_vec()),
        Err(ZooCryptoError::InvalidPayload { .. })
    ));
    assert_eq!(b.view().phase, SasPhase::ChooseMethod);

    let start = single(b.scan_qr_code(qr).unwrap(), "m.key.verification.start");
    assert_eq!(content(&start)["method"], "m.reciprocate.v1");
    assert_eq!(b.view().phase, SasPhase::QrScanned);
    assert_eq!(b.view().method, Some(VerificationMethod::QrCode));
    assert!(b.view().qr_code.is_none());

    let start = deliver(&mut bob, &alice, &start);
    assert!(a.receive(start, alice_view).unwrap().is_empty());
    assert_eq!(a.view().phase, SasPhase::QrConfirm);
    assert!(a.view().verified_master_key.is_none());

    let done = single(a.confirm_qr_scanned().unwrap(), "m.key.verification.done");
    assert_eq!(a.view().phase, SasPhase::Done);
    assert_eq!(
        a.view().verified_master_key,
        Some(bob.keys.master().public_key())
    );
    assert!(b.view().verified_master_key.is_none());
    let done = deliver(&mut alice, &bob, &done);
    single(
        b.receive(done, bob_view).unwrap(),
        "m.key.verification.done",
    );
    assert_eq!(b.view().phase, SasPhase::Done);
    assert_eq!(
        b.view().verified_master_key,
        Some(alice.keys.master().public_key())
    );

    let accepted = b
        .sign_master_key(
            &bob.trust,
            &bob.keys.user_signing(),
            alice.master_key_json(),
        )
        .unwrap();
    assert_eq!(accepted.signature.target_user_id, ALICE);
}

#[test]
fn mismatch_cancels_on_both_sides() {
    let mut alice = User::new(ALICE, ALICE_DEVICE);
    let mut bob = User::new(BOB, BOB_DEVICE);
    let (a, b) = compare(&mut alice, &mut bob);
    let bob_view = keys_query(&bob, &[&alice, &bob]);

    let cancel = single(a.mismatch(), "m.key.verification.cancel");
    let view = a.view();
    assert_eq!(view.phase, SasPhase::Cancelled);
    assert_eq!(
        view.cancel,
        Some(SasCancellation {
            code: "m.mismatched_sas".into(),
            by_us: true
        })
    );
    let cancel = deliver(&mut alice, &bob, &cancel);
    assert!(b.receive(cancel, bob_view).unwrap().is_empty());
    let view = b.view();
    assert_eq!(view.phase, SasPhase::Cancelled);
    assert_eq!(view.cancel.unwrap().code, "m.mismatched_sas");
    assert!(b.confirm().is_err());
    assert!(view.verified_master_key.is_none());
    assert!(matches!(
        b.sign_master_key(
            &bob.trust,
            &bob.keys.user_signing(),
            alice.master_key_json()
        ),
        Err(ZooCryptoError::Verification { .. })
    ));
}

#[test]
fn cancel_and_expire() {
    let mut alice = User::new(ALICE, ALICE_DEVICE);
    let mut bob = User::new(BOB, BOB_DEVICE);
    let alice_view = keys_query(&alice, &[&alice, &bob]);
    let bob_view = keys_query(&bob, &[&alice, &bob]);

    // 要求を断る。
    let request = request_verification(
        &alice.account,
        &alice.trust,
        alice_view.clone(),
        BOB.into(),
        NOW,
    )
    .unwrap();
    let a = request.verification;
    let message = deliver(&mut alice, &bob, &request.outgoing[0]);
    let b = SasVerification::from_request(&bob.account, &bob.trust, message, bob_view.clone(), NOW)
        .unwrap();
    let cancel = single(b.cancel(), "m.key.verification.cancel");
    assert_eq!(content(&cancel)["code"], "m.user");
    let cancel = deliver(&mut bob, &alice, &cancel);
    assert!(a.receive(cancel, alice_view.clone()).unwrap().is_empty());
    assert_eq!(a.view().phase, SasPhase::Cancelled);
    assert!(!a.view().cancel.unwrap().by_us);

    // 10 分たったら取りやめる。
    let request =
        request_verification(&alice.account, &alice.trust, alice_view, BOB.into(), NOW).unwrap();
    let a = request.verification;
    assert!(a.expire(NOW + 1000).is_empty());
    let cancel = single(
        a.expire(NOW + 10 * 60 * 1000 + 1),
        "m.key.verification.cancel",
    );
    assert_eq!(content(&cancel)["code"], "m.timeout");
    assert_eq!(a.view().phase, SasPhase::Cancelled);

    // 古い要求は受け付けない（捨てる）。
    let message = deliver(&mut alice, &bob, &request.outgoing[0]);
    assert!(
        SasVerification::from_request(
            &bob.account,
            &bob.trust,
            message,
            bob_view,
            NOW + 11 * 60 * 1000
        )
        .is_err()
    );
}

#[test]
fn verification_needs_unlocked_identity_and_listed_device() {
    let alice = User::new(ALICE, ALICE_DEVICE);
    let bob = User::new(BOB, BOB_DEVICE);
    // 自分のマスター鍵を信頼していない（解錠の前）。
    let locked = TrustStore::new(ALICE.into()).unwrap();
    assert!(matches!(
        request_verification(
            &alice.account,
            &locked,
            keys_query(&alice, &[&alice, &bob]),
            BOB.into(),
            NOW
        ),
        Err(ZooCryptoError::Verification { .. })
    ));

    // 送信端末が `keys/query` にない検証のメッセージは捨てる。
    let request = request_verification(
        &bob.account,
        &bob.trust,
        keys_query(&bob, &[&alice, &bob]),
        ALICE.into(),
        NOW,
    )
    .unwrap();
    let out = &request.outgoing[0];
    let target = &out.devices[0];
    alice.account.generate_one_time_keys(1);
    let otks: Value = serde_json::from_str(&alice.account.one_time_keys()).unwrap();
    let otk = otks
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .to_string();
    let session_id = bob
        .account
        .create_outbound_olm_session(&bob.outbound, target, otk)
        .unwrap();
    let content = bob
        .account
        .encrypt_verification(
            &bob.outbound,
            session_id,
            target,
            out.event_type.clone(),
            out.content.clone(),
        )
        .unwrap();
    let decrypted = alice
        .account
        .decrypt_to_device(&alice.inbound, content)
        .unwrap();
    assert!(matches!(
        decrypted.verify_verification(&alice.account, None),
        Err(ZooCryptoError::Rejected { .. })
    ));

    // 送信用のセッションがなければ暗号化しない（先に `keys/claim` の鍵でセッションを作る）。
    assert!(matches!(
        bob.account.encrypt_verification(
            &bob.outbound,
            "missing".into(),
            target,
            "m.key.verification.request".into(),
            "{}".into()
        ),
        Err(ZooCryptoError::Olm { .. })
    ));
}
