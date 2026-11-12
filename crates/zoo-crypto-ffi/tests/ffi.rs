//! バインディングの API を Rust から通しで確かめる（Kotlin の単体テストと同じ流れ）。

use zoo_crypto_ffi::*;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const ALICE_DEVICE: &str = "00000000-0000-4000-8000-0000000000a1";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const BOB_DEVICE: &str = "00000000-0000-4000-8000-0000000000b1";
const CHANNEL: &str = "00000000-0000-4000-8000-0000000000c1";
const MESSAGE: &str = "00000000-0000-4000-8000-0000000000d1";

fn message(body: &str) -> OutgoingMessage {
    OutgoingMessage {
        message_id: MESSAGE.into(),
        parent_id: None,
        revision: 0,
        content: MessageContent {
            body: body.into(),
            format: "mfm".into(),
            kind: "message".into(),
        },
        mentions: Mentions::default(),
    }
}

fn row(ciphertext: String) -> EncryptedMessage {
    EncryptedMessage {
        channel_id: CHANNEL.into(),
        message_id: MESSAGE.into(),
        parent_id: None,
        user_id: ALICE.into(),
        message_kind: "message".into(),
        epoch: 1,
        revision: 0,
        ciphertext,
    }
}

#[test]
fn pickle_round_trip_and_wrong_key() {
    let account = DeviceAccount::new(ALICE.into(), ALICE_DEVICE.into()).unwrap();
    let key = vec![7u8; 32];
    let pickle = account.pickle(key.clone()).unwrap();
    let restored =
        DeviceAccount::from_pickle(pickle.clone(), key, ALICE.into(), ALICE_DEVICE.into()).unwrap();
    assert_eq!(restored.curve25519_key(), account.curve25519_key());
    assert_eq!(restored.ed25519_key(), account.ed25519_key());

    let wrong =
        DeviceAccount::from_pickle(pickle, vec![8u8; 32], ALICE.into(), ALICE_DEVICE.into());
    assert!(matches!(wrong, Err(ZooCryptoError::Pickle { .. })));
    assert!(matches!(
        account.pickle(vec![0u8; 31]),
        Err(ZooCryptoError::InvalidPayload { .. })
    ));
}

#[test]
fn megolm_own_message_round_trip() {
    let account = DeviceAccount::new(ALICE.into(), ALICE_DEVICE.into()).unwrap();
    let outbound = OutboundSession::new(CHANNEL.into(), 1, 0);
    let inbound = InboundSessionStore::new();
    assert_eq!(inbound.add_outbound(&account, &outbound), AddOutcome::Added);

    let ciphertext = outbound.encrypt(&account, message("こんにちは")).unwrap();
    let decrypted = inbound.decrypt(row(ciphertext)).unwrap();
    assert_eq!(decrypted.payload.content.body, "こんにちは");
    assert_eq!(decrypted.message_index, 0);
    assert_eq!(decrypted.origin, SessionOrigin::Olm);
    assert_eq!(decrypted.revision, RevisionStatus::Latest);
}

#[test]
fn room_key_over_olm_then_megolm() {
    let alice = DeviceAccount::new(ALICE.into(), ALICE_DEVICE.into()).unwrap();
    let bob = DeviceAccount::new(BOB.into(), BOB_DEVICE.into()).unwrap();
    bob.generate_one_time_keys(1);
    let otks: serde_json::Value = serde_json::from_str(&bob.one_time_keys()).unwrap();
    let (_, otk) = otks.as_object().unwrap().iter().next().unwrap();
    bob.mark_keys_as_published();

    let alice_sessions = OlmSessionStore::new();
    let bob_keys = verify_device_keys(bob.device_keys(), BOB.into(), BOB_DEVICE.into()).unwrap();
    let session_id = alice
        .create_outbound_olm_session(&alice_sessions, &bob_keys, otk.to_string())
        .unwrap();

    let outbound = OutboundSession::new(CHANNEL.into(), 1, 0);
    let content = alice
        .encrypt_room_key(&alice_sessions, session_id, &bob_keys, outbound.room_key())
        .unwrap();

    let bob_sessions = OlmSessionStore::new();
    let decrypted = bob.decrypt_to_device(&bob_sessions, content).unwrap();
    assert_eq!(bob_sessions.len(), 1);
    assert_eq!(decrypted.event_type().as_deref(), Some("zoo.room_key"));
    let room_key = decrypted
        .verify_room_key(&bob, Some(alice.device_keys()), None)
        .unwrap();
    assert!(!room_key.sender_verified());

    let inbound = InboundSessionStore::new();
    assert_eq!(inbound.add_room_key(&room_key).unwrap(), AddOutcome::Added);
    assert!(matches!(
        inbound.add_room_key(&room_key),
        Err(ZooCryptoError::AlreadyConsumed { .. })
    ));

    let ciphertext = outbound.encrypt(&alice, message("hi")).unwrap();
    let decrypted = inbound.decrypt(row(ciphertext)).unwrap();
    assert_eq!(decrypted.payload.content.body, "hi");
    assert!(!decrypted.sender_verified);
}

#[test]
fn missing_room_key_carries_session_id() {
    let account = DeviceAccount::new(ALICE.into(), ALICE_DEVICE.into()).unwrap();
    let outbound = OutboundSession::new(CHANNEL.into(), 1, 0);
    let ciphertext = outbound.encrypt(&account, message("x")).unwrap();
    match InboundSessionStore::new().decrypt(row(ciphertext)) {
        Err(ZooCryptoError::MissingRoomKey { session_id, .. }) => {
            assert_eq!(session_id, outbound.session_id())
        }
        other => panic!("MissingRoomKey ではありません: {:?}", other.err()),
    }
}

#[test]
fn secret_storage_round_trip_with_recovery_key() {
    let keys = CrossSigningKeys::generate(ALICE.into()).unwrap();
    let backup = BackupDecryptionKey::generate();
    let ssss = SsssKey::generate();
    let recovery = RecoveryKey::generate();
    let slot = new_recovery_key_slot(ALICE.into(), &ssss, &recovery).unwrap();
    let request: serde_json::Value = serde_json::from_str(
        &ssss
            .keys_request(None, None, &keys, &backup, vec![], vec![slot])
            .unwrap(),
    )
    .unwrap();

    // サーバーが返す形にする（default_key_id、keys、secrets、slots）。
    let key_id = request["key_id"].as_str().unwrap();
    let response = serde_json::json!({
        "default_key_id": key_id,
        "keys": { key_id: request["key"] },
        "secrets": request["secrets"],
        "secrets_revision": 1,
        "slots": request["slots"],
    });
    let storage = parse_secret_storage(ALICE.into(), response.to_string())
        .unwrap()
        .unwrap();
    let parsed = RecoveryKey::parse(recovery.to_display()).unwrap();
    let unlocked = storage.unlock_with_recovery_key(&parsed).unwrap();
    assert_eq!(unlocked.key().key_id(), ssss.key_id());
    assert_eq!(
        unlocked
            .cross_signing_key(KeyUsage::Master)
            .unwrap()
            .unwrap()
            .public_key(),
        keys.master().public_key()
    );
    assert_eq!(
        unlocked.backup_key().unwrap().unwrap().public_key(),
        backup.public_key()
    );

    let wrong = storage.unlock_with_recovery_key(&RecoveryKey::generate());
    assert!(matches!(wrong, Err(ZooCryptoError::KeySlotUnlock { .. })));
}

#[test]
fn trust_store_round_trip() {
    let keys = CrossSigningKeys::generate(ALICE.into()).unwrap();
    let store = TrustStore::new(ALICE.into()).unwrap();
    store.trust_own_master_key(&keys.master()).unwrap();
    let restored = TrustStore::from_json(store.to_json(), ALICE.into()).unwrap();
    assert_eq!(restored.own_master_key(), Some(keys.master().public_key()));

    let distribution = restored
        .evaluate(vec!["{}".into()], ALICE_DEVICE.into(), None)
        .unwrap();
    assert!(distribution.devices.is_empty());
}
