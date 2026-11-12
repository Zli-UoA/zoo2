//! 鍵バックアップ（docs/e2ee.md の「鍵バックアップ」と「受信時の検証」の「鍵バックアップから取り込んだとき」）。

use serde_json::{Value, json};
use zoo_crypto::{
    BACKUP_ALGORITHM, BackupDecryptionKey, BackupRow, BackupUploadState, BackupVersionInfo,
    CrossSigningKeys, CryptoError, DeviceAccount, EncryptedMessage, InboundSessionStore, Mentions,
    MessageContent, OutboundSession, OutgoingMessage, SessionOrigin, SessionRef, VersionChange,
    previous_versions_to_migrate, select_previous_backup_keys, verify_backup_version,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const ALICE_DEVICE: &str = "1a8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const ALICE_NEW_DEVICE: &str = "4d8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const BOB: &str = "00000000-0000-0000-0000-00000000000b";
const CHANNEL: &str = "c0000000-0000-4000-8000-000000000001";
const OTHER_CHANNEL: &str = "c0000000-0000-4000-8000-000000000002";
const MESSAGE: &str = "d0000000-0000-4000-8000-000000000001";
const EPOCH: u64 = 3;
const NOW_MS: u64 = 1_800_000_000_000;

fn version(number: &str, backup_key: &BackupDecryptionKey, keys: &CrossSigningKeys) -> Value {
    json!({
        "version": number,
        "algorithm": BACKUP_ALGORITHM,
        "auth_data": backup_key.auth_data(&keys.master).unwrap(),
        "count": 0,
    })
}

/// Alice の端末が送信用セッションを作り、1 通送った状態。
struct Sender {
    account: DeviceAccount,
    store: InboundSessionStore,
    outbound: OutboundSession,
    row: Value,
}

fn sender() -> Sender {
    let account = DeviceAccount::new(ALICE, ALICE_DEVICE).unwrap();
    let mut store = InboundSessionStore::new();
    let mut outbound = OutboundSession::new(CHANNEL, EPOCH, NOW_MS);
    store.add_outbound(&account, &outbound);
    let message = OutgoingMessage {
        message_id: MESSAGE.to_owned(),
        parent_id: None,
        revision: 0,
        content: MessageContent {
            body: "hello".to_owned(),
            format: "mfm".to_owned(),
            kind: "message".to_owned(),
        },
        mentions: Mentions::default(),
    };
    let row = outbound.encrypt(&account, &message).unwrap();
    Sender {
        account,
        store,
        outbound,
        row,
    }
}

fn decrypt(
    store: &mut InboundSessionStore,
    ciphertext: &Value,
) -> Result<zoo_crypto::DecryptedMessage, CryptoError> {
    store.decrypt(&EncryptedMessage {
        channel_id: CHANNEL,
        message_id: MESSAGE,
        parent_id: None,
        user_id: ALICE,
        message_kind: "message",
        epoch: EPOCH,
        revision: 0,
        ciphertext,
    })
}

/// `store` のセッションをすべて、版 1 にアップロードしたときの行。
fn uploaded_rows(
    store: &InboundSessionStore,
    backup_key: &BackupDecryptionKey,
    keys: &CrossSigningKeys,
) -> Vec<BackupRow> {
    let mut state = BackupUploadState::new(ALICE).unwrap();
    state.set_secrets_revision(Some(1));
    state
        .accept_current_version(
            &version("1", backup_key, keys),
            &keys.master.public_key(),
            backup_key,
        )
        .unwrap();
    state.enqueue_all(store);
    rows(&state.next_upload(store, 1000).unwrap().unwrap().body)
}

/// サーバーに置いた行（アップロードの本文の `sessions`）。
fn rows(body: &Value) -> Vec<BackupRow> {
    serde_json::from_value(body["sessions"].clone()).unwrap()
}

#[test]
fn uploads_and_restores_sessions_on_a_new_device() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let mut alice = sender();

    let mut state = BackupUploadState::new(ALICE).unwrap();
    state.set_secrets_revision(Some(7));
    // 今の版を検証するまではアップロードしない。
    state.enqueue_all(&alice.store);
    assert_eq!(state.pending_len(), 1);
    assert!(state.next_upload(&alice.store, 1000).unwrap().is_none());

    let change = state
        .accept_current_version(
            &version("1", &backup_key, &keys),
            &keys.master.public_key(),
            &backup_key,
        )
        .unwrap();
    assert_eq!(change, VersionChange::Changed);
    let upload = state.next_upload(&alice.store, 1000).unwrap().unwrap();
    assert_eq!(upload.version, "1");
    assert_eq!(upload.secrets_revision, 7);
    let rows = rows(&upload.body);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].channel_id, CHANNEL);
    assert_eq!(
        rows[0].sender_key,
        alice.account.identity_keys().curve25519.to_base64()
    );
    assert_eq!(rows[0].session_id, alice.outbound.session_id());
    assert_eq!(rows[0].first_message_index, 0);
    assert_eq!(rows[0].forwarded_count, 0);
    assert!(rows[0].is_verified);
    state.upload_succeeded(&upload);
    assert_eq!(state.pending_len(), 0);
    assert!(state.next_upload(&alice.store, 1000).unwrap().is_none());

    // 平文の形は docs の BackedUpSessionData と同じ。
    let plaintext = backup_key.decrypt(&rows[0].session_data).unwrap();
    assert_eq!(plaintext.algorithm, "m.megolm.v1.aes-sha2");
    assert_eq!(plaintext.epoch, EPOCH);
    assert_eq!(plaintext.sender_user_id, ALICE);
    assert_eq!(plaintext.sender_device_id, ALICE_DEVICE);
    assert_eq!(
        plaintext.sender_claimed_keys.ed25519,
        alice.account.identity_keys().ed25519.to_base64()
    );

    // 新しい端末は SSSS から取り出したバックアップ鍵で取り込み、過去のメッセージを読める。
    let _new_device = DeviceAccount::new(ALICE, ALICE_NEW_DEVICE).unwrap();
    let mut restored = InboundSessionStore::new();
    assert_eq!(backup_key.import_rows(&mut restored, &rows), 0);
    let message = decrypt(&mut restored, &alice.row).unwrap();
    assert_eq!(message.payload.content.body, "hello");
    // バックアップから取り込んだセッションでは、行の is_verified にかかわらず送信者を確かめられない。
    assert_eq!(message.origin, SessionOrigin::Backup);
    assert!(!message.sender_verified);

    // 送信側の端末は Olm 由来で読める。
    assert_eq!(
        decrypt(&mut alice.store, &alice.row).unwrap().origin,
        SessionOrigin::Olm
    );
}

#[test]
fn each_upload_uses_a_new_ephemeral_key() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let alice = sender();
    let mut state = BackupUploadState::new(ALICE).unwrap();
    state.set_secrets_revision(Some(1));
    state
        .accept_current_version(
            &version("1", &backup_key, &keys),
            &keys.master.public_key(),
            &backup_key,
        )
        .unwrap();
    state.enqueue_all(&alice.store);
    let first = rows(&state.next_upload(&alice.store, 1000).unwrap().unwrap().body);
    let second = rows(&state.next_upload(&alice.store, 1000).unwrap().unwrap().body);
    assert_ne!(
        first[0].session_data.ephemeral,
        second[0].session_data.ephemeral
    );
    assert_ne!(
        first[0].session_data.ciphertext,
        second[0].session_data.ciphertext
    );
}

#[test]
fn rejects_unsigned_foreign_or_mismatched_versions() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let master = keys.master.public_key();
    let good = version("1", &backup_key, &keys);
    assert_eq!(
        verify_backup_version(&good, ALICE, &master, &backup_key)
            .unwrap()
            .version,
        "1"
    );

    // 署名がない。
    let mut unsigned = good.clone();
    unsigned["auth_data"]
        .as_object_mut()
        .unwrap()
        .remove("signatures");
    assert!(matches!(
        verify_backup_version(&unsigned, ALICE, &master, &backup_key),
        Err(CryptoError::MissingSignature { .. })
    ));
    // 別のマスター鍵の署名（サーバーが作った版）。
    let attacker = CrossSigningKeys::generate(ALICE).unwrap();
    let forged = version("1", &backup_key, &attacker);
    assert!(verify_backup_version(&forged, ALICE, &master, &backup_key).is_err());
    // 署名のあとで公開鍵を差し替えた。
    let mut swapped = good.clone();
    swapped["auth_data"]["public_key"] =
        json!(BackupDecryptionKey::generate().public_key().to_base64());
    assert!(matches!(
        verify_backup_version(&swapped, ALICE, &master, &backup_key),
        Err(CryptoError::Signature(_))
    ));
    // 正しく署名されているが、SSSS のバックアップ鍵と合わない（失効の途中か改ざん）。
    let other = BackupDecryptionKey::generate();
    assert!(matches!(
        verify_backup_version(&version("1", &other, &keys), ALICE, &master, &backup_key),
        Err(CryptoError::BackupKeyMismatch)
    ));
    // 方式が違う、番号が 10 進でない。
    let mut algorithm = good.clone();
    algorithm["algorithm"] = json!("m.megolm_backup.v1.curve25519-aes-sha2");
    assert!(matches!(
        verify_backup_version(&algorithm, ALICE, &master, &backup_key),
        Err(CryptoError::InvalidBackup(_))
    ));
    for number in ["", "01", "x", "-1"] {
        let mut bad = good.clone();
        bad["version"] = json!(number);
        assert!(matches!(
            verify_backup_version(&bad, ALICE, &master, &backup_key),
            Err(CryptoError::InvalidBackup(_))
        ));
    }
}

#[test]
fn refuses_version_rollback_and_stops_on_conflicts() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let master = keys.master.public_key();
    let alice = sender();
    let mut state = BackupUploadState::new(ALICE).unwrap();
    state.set_secrets_revision(Some(1));
    state.enqueue_all(&alice.store);

    assert_eq!(
        state
            .accept_current_version(&version("3", &backup_key, &keys), &master, &backup_key)
            .unwrap(),
        VersionChange::Changed
    );
    assert_eq!(
        state
            .accept_current_version(&version("3", &backup_key, &keys), &master, &backup_key)
            .unwrap(),
        VersionChange::Unchanged
    );
    // 漏れた鍵の古い版に戻そうとしても使わない。
    assert!(matches!(
        state.accept_current_version(&version("2", &backup_key, &keys), &master, &backup_key),
        Err(CryptoError::BackupVersionRollback {
            max_seen: 3,
            received: 2
        })
    ));
    assert!(state.current_version().is_none());
    assert!(state.next_upload(&alice.store, 1000).unwrap().is_none());

    // 保存した最大の番号は、起動し直しても残る。
    let saved = state.to_json();
    let mut restored = BackupUploadState::from_json(&saved, ALICE).unwrap();
    assert_eq!(restored.max_seen_version(), Some(3));
    assert_eq!(restored.pending_len(), 1);
    assert_eq!(restored.secrets_revision(), Some(1));
    assert!(restored.current_version().is_none());
    assert!(matches!(
        restored.accept_current_version(&version("2", &backup_key, &keys), &master, &backup_key),
        Err(CryptoError::BackupVersionRollback { .. })
    ));
    assert!(matches!(
        BackupUploadState::from_json(&saved, BOB),
        Err(CryptoError::InvalidBackup(_))
    ));

    // 409 を受けたら止まり、アップロード待ちは残る。
    restored
        .accept_current_version(&version("3", &backup_key, &keys), &master, &backup_key)
        .unwrap();
    let upload = restored.next_upload(&alice.store, 1000).unwrap().unwrap();
    restored.suspend();
    assert!(restored.next_upload(&alice.store, 1000).unwrap().is_none());
    // 止めたあとに届いた成功は、待ちを外さない（新しい版にも上げ直す）。
    restored.upload_succeeded(&upload);
    assert_eq!(restored.pending_len(), 1);
}

#[test]
fn version_change_requires_reupload_of_everything() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let old_key = BackupDecryptionKey::generate();
    let new_key = BackupDecryptionKey::generate();
    let master = keys.master.public_key();
    let alice = sender();
    let mut state = BackupUploadState::new(ALICE).unwrap();
    state.set_secrets_revision(Some(1));
    state
        .accept_current_version(&version("1", &old_key, &keys), &master, &old_key)
        .unwrap();
    state.enqueue_all(&alice.store);
    let upload = state.next_upload(&alice.store, 1000).unwrap().unwrap();
    state.upload_succeeded(&upload);
    assert_eq!(state.pending_len(), 0);

    // 失効で新しい版になったら、手元のセッションをすべて上げ直す。
    assert_eq!(
        state
            .accept_current_version(&version("2", &new_key, &keys), &master, &new_key)
            .unwrap(),
        VersionChange::Changed
    );
    state.enqueue_all(&alice.store);
    let upload = state.next_upload(&alice.store, 1000).unwrap().unwrap();
    assert_eq!(upload.version, "2");
    // 新しい版の行は新しい鍵でだけ復号できる。
    let rows = rows(&upload.body);
    assert!(new_key.decrypt(&rows[0].session_data).is_ok());
    assert!(matches!(
        old_key.decrypt(&rows[0].session_data),
        Err(CryptoError::BackupDecryption(_))
    ));
}

#[test]
fn tampered_or_mismatched_rows_are_dropped() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let alice = sender();
    let good = uploaded_rows(&alice.store, &backup_key, &keys).remove(0);

    let mut bad_mac = good.clone();
    let mut mac = vodozemac::base64_decode(&bad_mac.session_data.mac).unwrap();
    mac[31] ^= 1;
    bad_mac.session_data.mac = vodozemac::base64_encode(mac);

    let mut bad_ciphertext = good.clone();
    let mut ciphertext = vodozemac::base64_decode(&bad_ciphertext.session_data.ciphertext).unwrap();
    ciphertext[0] ^= 1;
    bad_ciphertext.session_data.ciphertext = vodozemac::base64_encode(ciphertext);

    // 行の値と平文の値が違う（別のチャンネル、別の送信者、別のセッションへの移植）。
    let mut other_channel = good.clone();
    other_channel.channel_id = OTHER_CHANNEL.to_owned();
    let mut other_sender = good.clone();
    other_sender.sender_key = DeviceAccount::new(BOB, ALICE_NEW_DEVICE)
        .unwrap()
        .identity_keys()
        .curve25519
        .to_base64();
    let mut other_session = good.clone();
    other_session.session_id = "AAAA".to_owned();

    for row in [
        &bad_mac,
        &bad_ciphertext,
        &other_channel,
        &other_sender,
        &other_session,
    ] {
        assert!(matches!(
            backup_key.decrypt_row(row),
            Err(CryptoError::BackupDecryption(_))
        ));
    }
    // 別のバックアップ鍵では復号できない。
    assert!(BackupDecryptionKey::generate().decrypt_row(&good).is_err());

    // 平文の session_id と session_key が合わない（平文ごと作り直した）ものは取り込みで捨てる。
    let mut other_store = InboundSessionStore::new();
    other_store.add_outbound(
        &alice.account,
        &OutboundSession::new(CHANNEL, EPOCH, NOW_MS),
    );
    let other_row = uploaded_rows(&other_store, &backup_key, &keys).remove(0);
    let mut forged = backup_key.decrypt(&good.session_data).unwrap();
    forged.session_key = backup_key
        .decrypt(&other_row.session_data)
        .unwrap()
        .session_key;
    let forged_row = BackupRow {
        session_data: backup_key.encryption_key().encrypt(&forged).unwrap(),
        ..good.clone()
    };
    let mut store = InboundSessionStore::new();
    assert_eq!(
        backup_key.import_rows(
            &mut store,
            &[
                bad_mac,
                bad_ciphertext,
                other_channel,
                forged_row,
                good.clone()
            ],
        ),
        4
    );
    // 正しい行だけが入る。
    assert_eq!(store.session_refs().len(), 1);
    assert_eq!(
        decrypt(&mut store, &alice.row).unwrap().origin,
        SessionOrigin::Backup
    );
}

#[test]
fn row_claims_do_not_make_the_sender_verified() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let alice = sender();
    let mut row = uploaded_rows(&alice.store, &backup_key, &keys).remove(0);
    row.is_verified = true;
    let mut store = InboundSessionStore::new();
    assert_eq!(backup_key.import_rows(&mut store, &[row]), 0);
    let message = decrypt(&mut store, &alice.row).unwrap();
    assert_eq!(message.origin, SessionOrigin::Backup);
    assert!(!message.sender_verified);

    // 取り込んだセッションは、同じ版には上げ直さない（呼び出し側は enqueue しない）が、
    // 新しい版に変わったときは enqueue_all で上げ直し、そのときは is_verified を false にする。
    let mut next = BackupUploadState::new(ALICE).unwrap();
    next.set_secrets_revision(Some(2));
    let new_key = BackupDecryptionKey::generate();
    next.accept_current_version(
        &version("2", &new_key, &keys),
        &keys.master.public_key(),
        &new_key,
    )
    .unwrap();
    next.enqueue_all(&store);
    let reuploaded = rows(&next.next_upload(&store, 1000).unwrap().unwrap().body);
    assert_eq!(reuploaded.len(), 1);
    assert!(!reuploaded[0].is_verified);
}

#[test]
fn upload_batches_are_limited() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let account = DeviceAccount::new(ALICE, ALICE_DEVICE).unwrap();
    let mut store = InboundSessionStore::new();
    for _ in 0..3 {
        store.add_outbound(&account, &OutboundSession::new(CHANNEL, EPOCH, NOW_MS));
    }
    let mut state = BackupUploadState::new(ALICE).unwrap();
    state.set_secrets_revision(Some(1));
    state
        .accept_current_version(
            &version("1", &backup_key, &keys),
            &keys.master.public_key(),
            &backup_key,
        )
        .unwrap();
    for session in store.session_refs() {
        state.enqueue(session);
    }
    // 手元にないセッション（消したもの）は送らずに待ちから外す。
    state.enqueue(SessionRef {
        channel_id: CHANNEL.to_owned(),
        sender_key: "gone".to_owned(),
        session_id: "gone".to_owned(),
    });
    let mut sent = 0;
    while let Some(upload) = state.next_upload(&store, 2).unwrap() {
        assert!(upload.sessions.len() <= 2);
        sent += rows(&upload.body).len();
        state.upload_succeeded(&upload);
    }
    assert_eq!(sent, 3);
    assert_eq!(state.pending_len(), 0);
}

#[test]
fn selects_previous_backup_keys_by_public_key() {
    let current = BackupDecryptionKey::generate();
    let older = BackupDecryptionKey::generate();
    let deleted = BackupDecryptionKey::generate();
    let unknown = BackupDecryptionKey::generate();
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let listed = |number: &str, key: &BackupDecryptionKey, deleted: bool| {
        let mut v = version(number, key, &keys);
        v["deleted"] = json!(deleted);
        v
    };
    let lost = BackupDecryptionKey::generate();
    let versions = BackupVersionInfo::list_from_json(&json!({"versions": [
        listed("1", &deleted, true),
        listed("2", &older, false),
        listed("3", &lost, false),
        listed("4", &current, false),
    ]}))
    .unwrap();

    let selection = select_previous_backup_keys(
        [
            current.clone(),
            older.clone(),
            deleted.clone(),
            unknown.clone(),
            older.clone(),
        ],
        &versions,
    );
    let selected: Vec<_> = selection.keys.iter().map(|k| k.public_key()).collect();
    // 消していない版の鍵とどの版とも合わない鍵は入れ、消した版の鍵は除き、重複はまとめる。
    assert_eq!(
        selected,
        vec![
            current.public_key(),
            older.public_key(),
            unknown.public_key()
        ]
    );
    assert_eq!(selection.versions_without_key, vec!["3".to_owned()]);

    // 解錠した端末は、今の版でない、消していない版のうち鍵のあるものを移行する。
    let migrate = previous_versions_to_migrate(&selection.keys, &versions, Some("4"));
    let migrate: Vec<_> = migrate
        .iter()
        .map(|(v, k)| (v.as_str(), k.public_key()))
        .collect();
    assert_eq!(migrate, vec![("2", older.public_key())]);
}

#[test]
fn success_of_an_older_upload_keeps_sessions_enqueued_again() {
    let keys = CrossSigningKeys::generate(ALICE).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let alice = sender();
    let mut state = BackupUploadState::new(ALICE).unwrap();
    state.set_secrets_revision(Some(1));
    state
        .accept_current_version(
            &version("1", &backup_key, &keys),
            &keys.master.public_key(),
            &backup_key,
        )
        .unwrap();
    state.enqueue_all(&alice.store);
    let in_flight = state.next_upload(&alice.store, 1000).unwrap().unwrap();

    // 応答を待つ間に、同じセッションがより小さい first known index の鍵に置き換わって入れ直された。
    let session = in_flight.sessions[0].clone();
    state.enqueue(session.clone());
    state.upload_succeeded(&in_flight);
    assert_eq!(state.pending_len(), 1);

    // 次のアップロードで送り、その成功で外れる。
    let next = state.next_upload(&alice.store, 1000).unwrap().unwrap();
    assert_eq!(next.sessions, vec![session]);
    state.upload_succeeded(&next);
    assert_eq!(state.pending_len(), 0);
}
