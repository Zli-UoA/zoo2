//! Olm での鍵の配布と、Megolm のメッセージの受信時の検証（docs/e2ee.md の「受信時の検証」）。

use serde_json::{Value, json};
use vodozemac::Ed25519SecretKey;
use vodozemac::megolm::{ExportedSessionKey, InboundGroupSession, SessionConfig, SessionKey};
use zoo_crypto::{
    AddOutcome, BackupSession, CryptoError, DecryptedMessage, DeviceAccount, DeviceKeys,
    EncryptedMessage, InboundSessionStore, ListedDevice, MAX_SAFE_INTEGER, Mentions,
    MessageContent, OlmSession, OutboundSession, OutgoingMessage, ROTATION_PERIOD_MESSAGES,
    ROTATION_PERIOD_MS, Rejection, RevisionStatus, RoomKey, SenderTrust, SessionOrigin,
    signable_json, verify_device_keys,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const ALICE_DEVICE: &str = "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const BOB: &str = "00000000-0000-0000-0000-00000000000b";
const BOB_DEVICE: &str = "2b8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const MALLORY: &str = "00000000-0000-0000-0000-00000000000c";
const MALLORY_DEVICE: &str = "3c8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const CHANNEL: &str = "c0000000-0000-4000-8000-000000000001";
const OTHER_CHANNEL: &str = "c0000000-0000-4000-8000-000000000002";
const EPOCH: u64 = 3;
const NOW_MS: u64 = 1_800_000_000_000;

struct Device {
    account: DeviceAccount,
    olm_sessions: Vec<OlmSession>,
    store: InboundSessionStore,
}

impl Device {
    fn new(user_id: &str, device_id: &str) -> Self {
        Self {
            account: DeviceAccount::new(user_id, device_id).unwrap(),
            olm_sessions: Vec::new(),
            store: InboundSessionStore::new(),
        }
    }

    fn device_keys(&self) -> DeviceKeys {
        verify_device_keys(
            &self.account.device_keys(),
            self.account.user_id(),
            self.account.device_id(),
        )
        .unwrap()
    }

    /// サーバーに置いたワンタイム鍵を 1 つ払い出す（`keys/claim` の応答の値）。
    fn claim_one_time_key(&mut self) -> Value {
        self.account.generate_one_time_keys(1);
        let key = self.account.one_time_keys().into_values().next().unwrap();
        self.account.mark_keys_as_published();
        key
    }
}

/// `from` から `to` に Olm セッションを作り、`room_key` を送る to-device の `content`。
fn send_room_key(
    from: &Device,
    session: &mut OlmSession,
    to: &Device,
    room_key: &RoomKey,
) -> Value {
    from.account
        .encrypt_room_key(session, &to.device_keys(), room_key)
        .unwrap()
}

fn olm_session(from: &Device, to: &mut Device) -> OlmSession {
    let one_time_key = to.claim_one_time_key();
    from.account
        .create_outbound_olm_session(&to.device_keys(), &one_time_key)
        .unwrap()
}

/// `to` が受け取り、`from` の端末を Shape の行として検証して保存する。
fn receive_room_key(from: &Device, to: &mut Device, content: &Value) -> AddOutcome {
    let decrypted = to
        .account
        .decrypt_to_device(&mut to.olm_sessions, content)
        .unwrap();
    let listed = from.account.device_keys();
    let room_key = decrypted
        .verify_room_key(&to.account, ListedDevice::Listed(&listed), None)
        .unwrap();
    to.store.add_room_key(room_key)
}

fn share(from: &Device, to: &mut Device, room_key: &RoomKey) -> AddOutcome {
    let mut session = olm_session(from, to);
    let content = send_room_key(from, &mut session, to, room_key);
    receive_room_key(from, to, &content)
}

fn message(message_id: &str, revision: u64, body: &str) -> OutgoingMessage {
    OutgoingMessage {
        message_id: message_id.to_owned(),
        parent_id: None,
        revision,
        content: MessageContent {
            body: body.to_owned(),
            format: "mfm".to_owned(),
            kind: "message".to_owned(),
        },
        mentions: Mentions::default(),
    }
}

/// サーバーの `messages` の行。
#[derive(Clone)]
struct Row {
    channel_id: String,
    message_id: String,
    parent_id: Option<String>,
    user_id: String,
    message_kind: String,
    epoch: u64,
    revision: u64,
    ciphertext: Value,
}

impl Row {
    fn new(sender: &Device, session: &mut OutboundSession, message: &OutgoingMessage) -> Self {
        Self {
            channel_id: session.channel_id().to_owned(),
            message_id: message.message_id.clone(),
            parent_id: message.parent_id.clone(),
            user_id: sender.account.user_id().to_owned(),
            message_kind: message.content.kind.clone(),
            epoch: session.epoch(),
            revision: message.revision,
            ciphertext: session.encrypt(&sender.account, message).unwrap(),
        }
    }

    fn view(&self) -> EncryptedMessage<'_> {
        EncryptedMessage {
            channel_id: &self.channel_id,
            message_id: &self.message_id,
            parent_id: self.parent_id.as_deref(),
            user_id: &self.user_id,
            message_kind: &self.message_kind,
            epoch: self.epoch,
            revision: self.revision,
            ciphertext: &self.ciphertext,
        }
    }
}

fn decrypt(device: &mut Device, row: &Row) -> Result<DecryptedMessage, CryptoError> {
    device.store.decrypt(&row.view())
}

fn rejection(result: Result<DecryptedMessage, CryptoError>) -> Rejection {
    match result {
        Err(CryptoError::Rejected(reason)) => reason,
        other => panic!("破棄されるはずが {other:?}"),
    }
}

/// Alice が CHANNEL の送信用セッションを作り、Bob に配った状態。
fn alice_and_bob() -> (Device, Device, OutboundSession) {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    assert_eq!(
        share(&alice, &mut bob, &outbound.room_key()),
        AddOutcome::Added
    );
    (alice, bob, outbound)
}

/// 自己署名鍵 `ssk` で端末鍵に署名する（クロス署名、#350 の代わり）。
fn cross_sign(device_keys: &mut Value, user_id: &str, ssk: &Ed25519SecretKey) -> String {
    let signature = ssk
        .sign(signable_json(device_keys).unwrap().as_bytes())
        .to_base64();
    let key_id = format!("ed25519:{}", ssk.public_key().to_base64());
    device_keys["signatures"][user_id][key_id] = json!(signature);
    signature
}

#[test]
fn delivers_room_key_and_decrypts_messages() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "こんにちは"));
    let decrypted = decrypt(&mut bob, &row).unwrap();
    assert_eq!(decrypted.payload.channel_id, CHANNEL);
    assert_eq!(decrypted.payload.epoch, EPOCH);
    assert_eq!(decrypted.payload.content.body, "こんにちは");
    assert_eq!(decrypted.message_index, 0);
    assert_eq!(decrypted.origin, SessionOrigin::Olm);
    // クロス署名を確かめていないので「未検証の送信者」。
    assert!(!decrypted.sender_verified);
    assert_eq!(decrypted.revision, RevisionStatus::Latest);
}

#[test]
fn same_message_can_be_fetched_and_decrypted_again() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    let first = decrypt(&mut bob, &row).unwrap();
    for _ in 0..3 {
        let again = decrypt(&mut bob, &row).unwrap();
        assert_eq!(again, first);
    }
}

#[test]
fn sender_can_read_own_messages() {
    let mut alice = Device::new(ALICE, ALICE_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    assert_eq!(
        alice.store.add_outbound(&alice.account, &outbound),
        AddOutcome::Added
    );
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    let decrypted = decrypt(&mut alice, &row).unwrap();
    assert!(decrypted.sender_verified);
}

#[test]
fn second_room_key_reuses_the_olm_session() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let mut session = olm_session(&alice, &mut bob);
    for channel in [CHANNEL, OTHER_CHANNEL] {
        let outbound = OutboundSession::new(channel, EPOCH, NOW_MS);
        let content = send_room_key(&alice, &mut session, &bob, &outbound.room_key());
        assert_eq!(
            receive_room_key(&alice, &mut bob, &content),
            AddOutcome::Added
        );
    }
    assert_eq!(bob.olm_sessions.len(), 1);
}

#[test]
fn rejects_one_time_keys_with_broken_signatures() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let mut one_time_key = bob.claim_one_time_key();
    // サーバーが別の鍵に差し替えた。
    one_time_key["key"] = json!(
        Device::new(MALLORY, MALLORY_DEVICE)
            .device_keys()
            .curve25519
            .to_base64()
    );
    assert!(matches!(
        alice
            .account
            .create_outbound_olm_session(&bob.device_keys(), &one_time_key),
        Err(CryptoError::Signature(_))
    ));
}

#[test]
fn ignores_to_device_messages_for_other_devices() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let mut mallory = Device::new(MALLORY, MALLORY_DEVICE);
    let mut session = olm_session(&alice, &mut bob);
    let outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    let content = send_room_key(&alice, &mut session, &bob, &outbound.room_key());
    assert!(matches!(
        mallory
            .account
            .decrypt_to_device(&mut mallory.olm_sessions, &content),
        Err(CryptoError::InvalidPayload(_))
    ));
}

#[test]
fn verifies_sender_through_self_signing_key() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let ssk = Ed25519SecretKey::new();
    let mut signed_device_keys = alice.account.device_keys();
    let signature = cross_sign(&mut signed_device_keys, ALICE, &ssk);
    let unsigned_device_keys = alice.account.device_keys();

    let receive = |listed: ListedDevice<'_>, trust: Option<&SenderTrust>| {
        let mut bob = Device::new(BOB, BOB_DEVICE);
        let mut session = olm_session(&alice, &mut bob);
        let room_key = OutboundSession::new(CHANNEL, EPOCH, NOW_MS).room_key();
        let content = send_room_key(&alice, &mut session, &bob, &room_key);
        bob.account
            .decrypt_to_device(&mut bob.olm_sessions, &content)
            .unwrap()
            .verify_room_key(&bob.account, listed, trust)
            .unwrap()
            .sender_verified()
    };

    let trust = SenderTrust {
        self_signing_key: ssk.public_key(),
        signature: None,
    };
    // sender_device_keys には自己署名鍵の署名がない（encrypt_room_key は自己署名だけを入れる）ので、
    // e2ee_key_signatures の署名を渡したときだけ検証済みになる。
    assert!(!receive(
        ListedDevice::Listed(&signed_device_keys),
        Some(&trust)
    ));
    let with_row_signature = SenderTrust {
        signature: Some(signature),
        ..trust.clone()
    };
    assert!(receive(
        ListedDevice::Listed(&unsigned_device_keys),
        Some(&with_row_signature)
    ));
    // 自己署名だけ（クロス署名を確かめられない）なら検証済みにならない。
    assert!(!receive(ListedDevice::Listed(&unsigned_device_keys), None));
    // 別の自己署名鍵では検証済みにならない。
    let other = SenderTrust {
        self_signing_key: Ed25519SecretKey::new().public_key(),
        signature: with_row_signature.signature.clone(),
    };
    assert!(!receive(
        ListedDevice::Listed(&unsigned_device_keys),
        Some(&other)
    ));
    // Shape にも keys/query にもない端末は、署名が正しくても検証済みにならない。
    assert!(!receive(ListedDevice::NotListed, Some(&with_row_signature)));
}

#[test]
fn verified_sender_is_reported_on_decrypted_messages() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let ssk = Ed25519SecretKey::new();
    let mut signed = alice.account.device_keys();
    let signature = cross_sign(&mut signed, ALICE, &ssk);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    let mut session = olm_session(&alice, &mut bob);
    let content = send_room_key(&alice, &mut session, &bob, &outbound.room_key());
    let listed = alice.account.device_keys();
    let room_key = bob
        .account
        .decrypt_to_device(&mut bob.olm_sessions, &content)
        .unwrap()
        .verify_room_key(
            &bob.account,
            ListedDevice::Listed(&listed),
            Some(&SenderTrust {
                self_signing_key: ssk.public_key(),
                signature: Some(signature),
            }),
        )
        .unwrap();
    bob.store.add_room_key(room_key);
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    assert!(decrypt(&mut bob, &row).unwrap().sender_verified);
}

#[test]
fn rejects_messages_moved_to_another_channel() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let mut room_key = outbound.room_key();
    let mut row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    row.channel_id = OTHER_CHANNEL.to_owned();
    // 別のチャンネルの行では、セッションが見つからない。
    assert!(matches!(
        decrypt(&mut bob, &row),
        Err(CryptoError::MissingRoomKey { .. })
    ));
    // 送信者が同じセッションを別のチャンネルのものとしても配っていても、ペイロードの channel_id で破棄する。
    room_key.channel_id = OTHER_CHANNEL.to_owned();
    let mut other_bob = Device::new(BOB, BOB_DEVICE);
    share(&alice, &mut other_bob, &room_key);
    assert_eq!(
        rejection(decrypt(&mut other_bob, &row)),
        Rejection::ChannelMismatch
    );
}

#[test]
fn rejects_messages_moved_to_another_row() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));

    let mut moved = row.clone();
    moved.message_id = "m2".to_owned();
    assert_eq!(
        rejection(decrypt(&mut bob, &moved)),
        Rejection::MessageIdMismatch
    );

    let mut moved = row.clone();
    moved.parent_id = Some("m0".to_owned());
    assert_eq!(
        rejection(decrypt(&mut bob, &moved)),
        Rejection::ParentMismatch
    );

    let mut moved = row.clone();
    moved.message_kind = "emote".to_owned();
    assert_eq!(
        rejection(decrypt(&mut bob, &moved)),
        Rejection::KindMismatch
    );

    // 失敗したあとも、正しい行は復号できる。
    decrypt(&mut bob, &row).unwrap();
}

#[test]
fn rejects_spoofed_senders() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));

    let mut spoofed = row.clone();
    spoofed.user_id = MALLORY.to_owned();
    assert_eq!(
        rejection(decrypt(&mut bob, &spoofed)),
        Rejection::SenderMismatch
    );

    let mut spoofed = row.clone();
    spoofed.ciphertext["device_id"] = json!(MALLORY_DEVICE);
    assert_eq!(
        rejection(decrypt(&mut bob, &spoofed)),
        Rejection::DeviceMismatch
    );

    // 別の端末の識別鍵に書き換えると、そのキーのセッションがないので復号できない。
    let mut spoofed = row.clone();
    spoofed.ciphertext["sender_key"] = json!(bob.device_keys().curve25519.to_base64());
    assert!(matches!(
        decrypt(&mut bob, &spoofed),
        Err(CryptoError::MissingRoomKey { .. })
    ));
}

#[test]
fn forwarded_room_key_does_not_replace_the_original_sender() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    // メンバーの Mallory が、Alice のセッション鍵を自分の zoo.room_key として Bob に転送する。
    let mallory = Device::new(MALLORY, MALLORY_DEVICE);
    assert_eq!(
        share(&mallory, &mut bob, &outbound.room_key()),
        AddOutcome::Added
    );
    // Alice のメッセージは、Alice の識別鍵の下にある Alice のセッションで復号され、送信者は Alice のまま。
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    decrypt(&mut bob, &row).unwrap();
    // Mallory の識別鍵に書き換えても、行の送信者が Alice なのでなりすましとして破棄する。
    let mut spoofed = row.clone();
    spoofed.ciphertext["sender_key"] = json!(mallory.device_keys().curve25519.to_base64());
    spoofed.ciphertext["device_id"] = json!(MALLORY_DEVICE);
    assert_eq!(
        rejection(decrypt(&mut bob, &spoofed)),
        Rejection::SenderMismatch
    );
}

#[test]
fn rejects_epoch_mismatches() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let mut room_key = outbound.room_key();
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));

    let mut stale = row.clone();
    stale.epoch = EPOCH + 1;
    assert_eq!(
        rejection(decrypt(&mut bob, &stale)),
        Rejection::EpochMismatch
    );

    // セッションの epoch（配布されたもの）がペイロードと違う。
    room_key.epoch = EPOCH - 1;
    let mut other_bob = Device::new(BOB, BOB_DEVICE);
    share(&alice, &mut other_bob, &room_key);
    assert_eq!(
        rejection(decrypt(&mut other_bob, &row)),
        Rejection::EpochMismatch
    );
}

#[test]
fn checks_revisions() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let original = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    let edited = Row::new(&alice, &mut outbound, &message("m1", 1, "b"));

    let mut mismatch = edited.clone();
    mismatch.revision = 0;
    assert_eq!(
        rejection(decrypt(&mut bob, &mismatch)),
        Rejection::RevisionMismatch
    );

    assert_eq!(
        decrypt(&mut bob, &edited).unwrap().revision,
        RevisionStatus::Latest
    );
    // サーバーが古い版を返した。
    assert_eq!(
        decrypt(&mut bob, &original).unwrap().revision,
        RevisionStatus::Stale { latest: 1 }
    );
    // 同じ版で中身が違う。
    let conflicting = Row::new(&alice, &mut outbound, &message("m1", 1, "c"));
    assert_eq!(
        decrypt(&mut bob, &conflicting).unwrap().revision,
        RevisionStatus::Conflict
    );
    // 最初に見た版の再取得は、最新のまま。
    assert_eq!(
        decrypt(&mut bob, &edited).unwrap().revision,
        RevisionStatus::Latest
    );
}

#[test]
fn keeps_the_session_with_the_smaller_first_known_index() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    let early = outbound.room_key();
    let first = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    let late = outbound.room_key();

    assert_eq!(share(&alice, &mut bob, &late), AddOutcome::Added);
    assert!(matches!(
        decrypt(&mut bob, &first),
        Err(CryptoError::MissingRoomKey {
            message_index: Some(0),
            ..
        })
    ));
    assert_eq!(share(&alice, &mut bob, &early), AddOutcome::Replaced);
    decrypt(&mut bob, &first).unwrap();
    assert_eq!(share(&alice, &mut bob, &late), AddOutcome::Kept);
    decrypt(&mut bob, &first).unwrap();
}

/// `room_key` のセッションを `index` から書き出した、バックアップの中身。
fn backup(room_key: &RoomKey, sender: &Device, index: u32) -> BackupSession {
    let mut session = InboundGroupSession::new(
        &SessionKey::from_base64(&room_key.session_key).unwrap(),
        SessionConfig::version_1(),
    );
    BackupSession {
        channel_id: room_key.channel_id.clone(),
        sender_key: sender.device_keys().curve25519,
        session_id: room_key.session_id.clone(),
        sender_user_id: sender.account.user_id().to_owned(),
        sender_device_id: sender.account.device_id().to_owned(),
        epoch: room_key.epoch,
        session_key: session.export_at(index).unwrap().to_base64(),
    }
}

#[test]
fn backup_sessions_never_replace_olm_sessions() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    let from_start = outbound.room_key();
    let before = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    share(&alice, &mut bob, &outbound.room_key());
    let after = Row::new(&alice, &mut outbound, &message("m2", 0, "b"));

    // バックアップの平文の送信者が違っても、first known index が小さくても、Olm 由来は残る。
    let mut forged = backup(&from_start, &alice, 0);
    forged.sender_user_id = MALLORY.to_owned();
    assert_eq!(
        bob.store.import_backup_session(forged).unwrap(),
        AddOutcome::Added
    );
    let decrypted = decrypt(&mut bob, &after).unwrap();
    assert_eq!(decrypted.origin, SessionOrigin::Olm);

    // Olm 由来の first known index より前は、バックアップ由来で復号するが、送信者が違うので破棄する。
    assert_eq!(
        rejection(decrypt(&mut bob, &before)),
        Rejection::SenderMismatch
    );
}

#[test]
fn backup_sessions_decrypt_older_messages_with_a_mark() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    let from_start = outbound.room_key();
    let before = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));

    assert_eq!(
        bob.store
            .import_backup_session(backup(&from_start, &alice, 0))
            .unwrap(),
        AddOutcome::Added
    );
    // あとから Olm で受け取っても、バックアップ由来は消えない。
    share(&alice, &mut bob, &outbound.room_key());
    let after = Row::new(&alice, &mut outbound, &message("m2", 0, "b"));

    let old = decrypt(&mut bob, &before).unwrap();
    assert_eq!(old.origin, SessionOrigin::Backup);
    assert!(!old.sender_verified);
    let new = decrypt(&mut bob, &after).unwrap();
    assert_eq!(new.origin, SessionOrigin::Olm);
}

#[test]
fn backup_sessions_replace_each_other_only_when_better() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let room_key = OutboundSession::new(CHANNEL, EPOCH, NOW_MS).room_key();

    let store = &mut bob.store;
    assert_eq!(
        store
            .import_backup_session(backup(&room_key, &alice, 3))
            .unwrap(),
        AddOutcome::Added
    );
    assert_eq!(
        store
            .import_backup_session(backup(&room_key, &alice, 5))
            .unwrap(),
        AddOutcome::Kept
    );
    assert_eq!(
        store
            .import_backup_session(backup(&room_key, &alice, 3))
            .unwrap(),
        AddOutcome::Kept
    );
    // 平文の送信者が違えば、小さくても置き換えない。
    let mut other_sender = backup(&room_key, &alice, 0);
    other_sender.sender_device_id = MALLORY_DEVICE.to_owned();
    assert_eq!(
        store.import_backup_session(other_sender).unwrap(),
        AddOutcome::Kept
    );

    // 同じ session_id で、つながらないラチェットと小さい first known index（Unconnected）。
    let real = backup(&room_key, &alice, 3);
    let mut bytes = ExportedSessionKey::from_base64(&real.session_key)
        .unwrap()
        .to_bytes();
    // 先頭は版（1 バイト）、次がインデックス（4 バイト）、その後ろ 128 バイトがラチェット。
    bytes[1..5].copy_from_slice(&0u32.to_be_bytes());
    for b in &mut bytes[5..133] {
        *b ^= 0x5a;
    }
    let forged = BackupSession {
        session_key: ExportedSessionKey::from_bytes(&bytes).unwrap().to_base64(),
        ..real
    };
    assert_eq!(
        store.import_backup_session(forged).unwrap(),
        AddOutcome::Kept
    );

    assert_eq!(
        store
            .import_backup_session(backup(&room_key, &alice, 1))
            .unwrap(),
        AddOutcome::Replaced
    );
}

#[test]
fn rejects_backup_sessions_with_another_session_id() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut store = InboundSessionStore::new();
    let room_key = OutboundSession::new(CHANNEL, EPOCH, NOW_MS).room_key();
    let mut wrong = backup(&room_key, &alice, 0);
    wrong.session_id = OutboundSession::new(CHANNEL, EPOCH, NOW_MS).session_id();
    assert!(matches!(
        store.import_backup_session(wrong),
        Err(CryptoError::InvalidPayload(_))
    ));
}

#[test]
fn decides_when_to_rotate_outbound_sessions() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    assert!(!outbound.needs_rotation(NOW_MS, EPOCH));
    assert!(outbound.needs_rotation(NOW_MS, EPOCH + 1));
    assert!(!outbound.needs_rotation(NOW_MS + ROTATION_PERIOD_MS - 1, EPOCH));
    assert!(outbound.needs_rotation(NOW_MS + ROTATION_PERIOD_MS, EPOCH));
    // 時計が戻っても作り直さない。
    assert!(!outbound.needs_rotation(NOW_MS - 1, EPOCH));

    for i in 0..ROTATION_PERIOD_MESSAGES {
        assert!(!outbound.needs_rotation(NOW_MS, EPOCH));
        outbound
            .encrypt(&alice.account, &message(&format!("m{i}"), 0, "a"))
            .unwrap();
    }
    assert!(outbound.needs_rotation(NOW_MS, EPOCH));
}

#[test]
fn outbound_payload_has_session_channel_and_epoch() {
    let (alice, mut bob, mut outbound) = alice_and_bob();
    let row = Row::new(&alice, &mut outbound, &message("m1", 0, "a"));
    assert_eq!(row.ciphertext["algorithm"], "m.megolm.v1.aes-sha2");
    assert_eq!(row.ciphertext["device_id"], ALICE_DEVICE);
    assert_eq!(row.ciphertext["session_id"], json!(outbound.session_id()));
    let payload = decrypt(&mut bob, &row).unwrap().payload;
    assert_eq!(payload.payload_type, "zoo.message");
    assert_eq!(
        (payload.channel_id.as_str(), payload.epoch),
        (CHANNEL, EPOCH)
    );
}

#[test]
fn rejects_invalid_outgoing_messages() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    let mut bad = message("m1", 0, "a");
    bad.content.kind = "system".to_owned();
    assert!(matches!(
        outbound.encrypt(&alice.account, &bad),
        Err(CryptoError::InvalidPayload(_))
    ));
    let too_long = message("m1", 0, &"あ".repeat(zoo_crypto::MAX_BODY_CHARS + 1));
    assert!(matches!(
        outbound.encrypt(&alice.account, &too_long),
        Err(CryptoError::InvalidPayload(_))
    ));
    assert_eq!(outbound.message_count(), 0);
}

#[test]
fn accepts_epoch_and_revision_up_to_the_canonical_json_limit() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, MAX_SAFE_INTEGER, NOW_MS);
    share(&alice, &mut bob, &outbound.room_key());
    let row = Row::new(&alice, &mut outbound, &message("m1", MAX_SAFE_INTEGER, "a"));
    let decrypted = decrypt(&mut bob, &row).unwrap();
    assert_eq!(decrypted.payload.epoch, MAX_SAFE_INTEGER);
    assert_eq!(decrypted.payload.revision, MAX_SAFE_INTEGER);
}

#[test]
fn rejects_epoch_and_revision_beyond_the_canonical_json_limit_when_sending() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    assert!(matches!(
        outbound.encrypt(&alice.account, &message("m1", MAX_SAFE_INTEGER + 1, "a")),
        Err(CryptoError::InvalidPayload(_))
    ));
    let mut outbound = OutboundSession::new(CHANNEL, MAX_SAFE_INTEGER + 1, NOW_MS);
    assert!(matches!(
        outbound.encrypt(&alice.account, &message("m1", 0, "a")),
        Err(CryptoError::InvalidPayload(_))
    ));
    assert_eq!(outbound.message_count(), 0);
}

#[test]
fn rejects_room_keys_with_epoch_beyond_the_limit() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let room_key = OutboundSession::new(CHANNEL, MAX_SAFE_INTEGER + 1, NOW_MS).room_key();
    let mut session = olm_session(&alice, &mut bob);
    let content = send_room_key(&alice, &mut session, &bob, &room_key);
    let listed = alice.account.device_keys();
    let result = bob
        .account
        .decrypt_to_device(&mut bob.olm_sessions, &content)
        .unwrap()
        .verify_room_key(&bob.account, ListedDevice::Listed(&listed), None);
    assert!(matches!(result, Err(CryptoError::InvalidPayload(_))));
}

#[test]
fn debug_output_does_not_contain_session_keys() {
    let alice = Device::new(ALICE, ALICE_DEVICE);
    let mut bob = Device::new(BOB, BOB_DEVICE);
    let room_key = OutboundSession::new(CHANNEL, EPOCH, NOW_MS).room_key();
    assert!(!format!("{room_key:?}").contains(&room_key.session_key));

    let backup = backup(&room_key, &alice, 0);
    assert!(!format!("{backup:?}").contains(&backup.session_key));

    let mut session = olm_session(&alice, &mut bob);
    let content = send_room_key(&alice, &mut session, &bob, &room_key);
    let decrypted = bob
        .account
        .decrypt_to_device(&mut bob.olm_sessions, &content)
        .unwrap();
    let debug = format!("{decrypted:?}");
    assert!(!debug.contains(&room_key.session_key));
    assert!(debug.contains(ALICE));
}
