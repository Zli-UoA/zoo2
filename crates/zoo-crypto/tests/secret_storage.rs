//! SSSS、鍵スロット、リカバリーキー（docs/e2ee.md の「SSSS（秘密の暗号化）」「鍵スロット」
//! 「リカバリーキーの形式」「失効の手順」）。

use serde_json::{Value, json};
use zoo_crypto::{
    BackupDecryptionKey, CrossSigningKeys, CryptoError, KeySlot, KeyUsage, REQUIRED_SECRETS,
    RecoveryKey, SECRET_PREVIOUS_MEGOLM_BACKUP, SecretStorage, SecretStorageKeysRequest, SlotKind,
    SlotSecret, SsssKey, UnlockedSecretStorage, new_prf_salt,
};

const ALICE: &str = "00000000-0000-0000-0000-00000000000a";
const BOB: &str = "00000000-0000-0000-0000-00000000000b";
const CREDENTIAL: &str = "AQIDBAUGBwg";
const OTHER_CREDENTIAL: &str = "CQoLDA0ODxA";

/// 初回設定で作るもの。
struct Setup {
    cross_signing: CrossSigningKeys,
    backup_key: BackupDecryptionKey,
    ssss_key: SsssKey,
    recovery_key: RecoveryKey,
    prf_output: [u8; 32],
    request: Value,
}

fn setup(user_id: &str) -> Setup {
    let cross_signing = CrossSigningKeys::generate(user_id).unwrap();
    let backup_key = BackupDecryptionKey::generate();
    let ssss_key = SsssKey::generate();
    let recovery_key = RecoveryKey::generate();
    let prf_output: [u8; 32] = rand_bytes();
    let slots = [
        KeySlot::new_passkey(user_id, &ssss_key, CREDENTIAL, &new_prf_salt(), &prf_output).unwrap(),
        KeySlot::new_recovery(user_id, &ssss_key, &recovery_key).unwrap(),
    ];
    let request = ssss_key
        .keys_request(&SecretStorageKeysRequest {
            expected_default_key_id: None,
            expected_secrets_revision: None,
            cross_signing: &cross_signing,
            backup_key: &backup_key,
            previous_backup_keys: &[],
            slots: &slots,
        })
        .unwrap();
    Setup {
        cross_signing,
        backup_key,
        ssss_key,
        recovery_key,
        prf_output,
        request,
    }
}

fn rand_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).unwrap();
    bytes
}

/// `POST /api/e2ee/secret_storage/keys` を受けたサーバーが `GET /api/e2ee/secret_storage` で返す形。
fn server_response(request: &Value, secrets_revision: u64) -> Value {
    let key_id = request["key_id"].as_str().unwrap();
    let mut secrets = request["secrets"].clone();
    for secret in secrets.as_object_mut().unwrap().values_mut() {
        secret["key_id"] = json!(key_id);
    }
    let slots: Vec<Value> = request["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| {
            let mut slot = slot.clone();
            slot["created_at"] = json!("2026-11-12T00:00:00Z");
            slot["last_used_at"] = Value::Null;
            slot
        })
        .collect();
    json!({
        "default_key_id": key_id,
        "keys": { key_id: request["key"] },
        "secrets": secrets,
        "secrets_revision": secrets_revision,
        "slots": slots,
    })
}

fn storage(user_id: &str, response: &Value) -> SecretStorage {
    SecretStorage::from_json(user_id, response)
        .unwrap()
        .unwrap()
}

fn assert_same_secrets(unlocked: &UnlockedSecretStorage, setup: &Setup) {
    for (usage, expected) in [
        (KeyUsage::Master, &setup.cross_signing.master),
        (KeyUsage::SelfSigning, &setup.cross_signing.self_signing),
        (KeyUsage::UserSigning, &setup.cross_signing.user_signing),
    ] {
        let key = unlocked.cross_signing_key(usage).unwrap().unwrap();
        assert_eq!(key.public_key(), expected.public_key());
    }
    assert_eq!(
        unlocked.backup_key().unwrap().unwrap().public_key(),
        setup.backup_key.public_key()
    );
    assert!(unlocked.previous_backup_keys().unwrap().is_empty());
}

#[test]
fn unlocks_with_passkey_and_recovery_key() {
    let s = setup(ALICE);
    assert_eq!(s.request["expected_default_key_id"], Value::Null);
    let secrets = s.request["secrets"].as_object().unwrap();
    for name in REQUIRED_SECRETS {
        assert!(secrets.contains_key(name), "{name}");
    }
    assert!(!secrets.contains_key(SECRET_PREVIOUS_MEGOLM_BACKUP));

    let storage = storage(ALICE, &server_response(&s.request, 1));
    assert_eq!(storage.default_key_id(), s.ssss_key.key_id());
    assert_eq!(storage.passkey_slots().count(), 1);

    let by_passkey = storage
        .unlock_with_passkey(CREDENTIAL, &s.prf_output)
        .unwrap();
    assert_eq!(by_passkey.secrets_revision(), Some(1));
    assert_eq!(by_passkey.key().as_bytes(), s.ssss_key.as_bytes());
    assert_same_secrets(&by_passkey, &s);

    let typed = s.recovery_key.to_display();
    let parsed = RecoveryKey::parse(&typed.replace(' ', "\n")).unwrap();
    let by_recovery = storage.unlock_with_recovery_key(&parsed).unwrap();
    assert_same_secrets(&by_recovery, &s);
}

#[test]
fn no_secret_storage_before_initial_setup() {
    let empty = json!({"default_key_id": null, "keys": {}, "secrets": {}, "secrets_revision": null, "slots": []});
    assert!(SecretStorage::from_json(ALICE, &empty).unwrap().is_none());
}

#[test]
fn wrong_prf_output_or_recovery_key_cannot_unlock() {
    let s = setup(ALICE);
    let storage = storage(ALICE, &server_response(&s.request, 1));
    let mut wrong_prf = s.prf_output;
    wrong_prf[0] ^= 1;
    assert!(matches!(
        storage.unlock_with_passkey(CREDENTIAL, &wrong_prf),
        Err(CryptoError::KeySlotUnlock)
    ));
    assert!(matches!(
        storage.unlock_with_passkey(OTHER_CREDENTIAL, &s.prf_output),
        Err(CryptoError::NoKeySlot)
    ));
    assert!(matches!(
        storage.unlock_with_recovery_key(&RecoveryKey::generate()),
        Err(CryptoError::KeySlotUnlock)
    ));
}

#[test]
fn rejects_malformed_recovery_keys() {
    let key = RecoveryKey::generate();
    let display = key.to_display();
    assert_eq!(display.len(), 48 + 11);
    assert!(display.split(' ').all(|group| group.len() == 4));
    assert_eq!(RecoveryKey::parse(&display).unwrap(), key);

    let compact: String = display.chars().filter(|c| *c != ' ').collect();
    let mut malformed = vec![
        compact[1..].to_owned(),        // 1 文字欠けた
        format!("{compact}1"),          // 1 文字多い
        format!("0{}", &compact[1..]),  // Base58 にない文字（0）
        format!("{}l", &compact[..47]), // Base58 にない文字（l）
    ];
    // 1 文字の打ち間違いはパリティか先頭の 2 バイトで見つかる。
    for i in [0, 10, 47] {
        let mut chars: Vec<char> = compact.chars().collect();
        chars[i] = if chars[i] == 'z' { 'y' } else { 'z' };
        malformed.push(chars.into_iter().collect());
    }
    for input in malformed {
        assert!(
            matches!(
                RecoveryKey::parse(&input),
                Err(CryptoError::InvalidRecoveryKey(_))
            ),
            "{input}"
        );
    }
}

#[test]
fn tampered_slots_are_rejected() {
    let s = setup(ALICE);
    let response = server_response(&s.request, 1);
    let tamper = |field: &str, f: &dyn Fn(&mut Vec<u8>)| {
        let mut response = response.clone();
        let slot = &mut response["slots"][0];
        let mut bytes = vodozemac::base64_decode(slot[field].as_str().unwrap()).unwrap();
        f(&mut bytes);
        slot[field] = json!(vodozemac::base64_encode(bytes));
        storage(ALICE, &response).unlock_with_passkey(CREDENTIAL, &s.prf_output)
    };
    // 暗号文、タグ、nonce、prf_salt のどれを書き換えても開けない。
    assert!(matches!(
        tamper("ciphertext", &|b| b[0] ^= 1),
        Err(CryptoError::KeySlotUnlock)
    ));
    assert!(matches!(
        tamper("ciphertext", &|b| b[47] ^= 1),
        Err(CryptoError::KeySlotUnlock)
    ));
    assert!(matches!(
        tamper("nonce", &|b| b[0] ^= 1),
        Err(CryptoError::KeySlotUnlock)
    ));
    // prf_salt は AAD に入れない（docs のとおり）。書き換えられると認証器が別の PRF 出力を返すので、
    // 開けなくなるだけ（サーバーにできる妨害で、鍵は漏れない）。同じ PRF 出力を渡せば開ける。
    assert!(tamper("prf_salt", &|b| b[0] ^= 1).is_ok());
    let mut other_output = s.prf_output;
    other_output[31] ^= 1;
    let mut salted = response.clone();
    salted["slots"][0]["prf_salt"] = json!(vodozemac::base64_encode([9u8; 32]));
    assert!(matches!(
        storage(ALICE, &salted).unlock_with_passkey(CREDENTIAL, &other_output),
        Err(CryptoError::KeySlotUnlock)
    ));
    // 長さが違うものは形式の誤り。
    assert!(matches!(
        tamper("ciphertext", &|b| {
            b.pop();
        }),
        Err(CryptoError::InvalidSecretStorage(_))
    ));
}

#[test]
fn slots_moved_to_another_user_key_or_slot_cannot_be_opened() {
    let s = setup(ALICE);
    let response = server_response(&s.request, 1);
    let slot: KeySlot = serde_json::from_value(response["slots"][0].clone()).unwrap();
    assert_eq!(slot.kind, SlotKind::Passkey);
    let prf_salt = slot.prf_salt_bytes().unwrap().unwrap();
    let secret = || SlotSecret::Passkey {
        credential_id: CREDENTIAL,
        prf_salt: &prf_salt,
        prf_output: &s.prf_output,
    };
    assert!(slot.open(ALICE, secret()).is_ok());

    // 別のユーザー：Bob の SSSS として出しても開けない。
    assert!(matches!(
        slot.open(BOB, secret()),
        Err(CryptoError::KeySlotUnlock)
    ));
    let mut bobs = response.clone();
    for secret in bobs["secrets"].as_object_mut().unwrap().values_mut() {
        secret["key_id"] = response["default_key_id"].clone();
    }
    assert!(matches!(
        storage(BOB, &bobs).unlock_with_passkey(CREDENTIAL, &s.prf_output),
        Err(CryptoError::KeySlotUnlock)
    ));

    // 別の SSSS 鍵：key_id を書き換えると開けない。
    let other_key = SsssKey::generate();
    let mut moved = slot.clone();
    moved.key_id = other_key.key_id().to_owned();
    assert!(matches!(
        moved.open(ALICE, secret()),
        Err(CryptoError::KeySlotUnlock)
    ));

    // 別のスロット：同じパスキーの別スロットの ID に付け替えると開けない。
    let mut moved = slot.clone();
    moved.id = "11111111-2222-4333-8444-555555555555".to_owned();
    assert!(matches!(
        moved.open(ALICE, secret()),
        Err(CryptoError::KeySlotUnlock)
    ));

    // 別の credential ID：別のパスキーのスロットとして出しても、呼び出し側が渡す credential ID と
    // 合わないので開けない。
    let mut moved = slot.clone();
    moved.credential_id = Some(OTHER_CREDENTIAL.to_owned());
    assert!(matches!(
        moved.open(ALICE, secret()),
        Err(CryptoError::KeySlotUnlock)
    ));

    // リカバリーキーで、パスキーのスロットは開けない。
    assert!(matches!(
        slot.open(ALICE, SlotSecret::Recovery(&s.recovery_key)),
        Err(CryptoError::KeySlotUnlock)
    ));
}

#[test]
fn rejects_key_check_mismatch() {
    let s = setup(ALICE);
    let mut response = server_response(&s.request, 1);
    // 同じ key_id で別の鍵の確認値（サーバーが SSSS 鍵の記述をすり替えた）。
    let impostor = SsssKey::from_bytes(s.ssss_key.key_id(), [7u8; 32]).unwrap();
    response["keys"][s.ssss_key.key_id()] = serde_json::to_value(impostor.description()).unwrap();
    let storage = storage(ALICE, &response);
    assert!(matches!(
        storage.unlock_with_passkey(CREDENTIAL, &s.prf_output),
        Err(CryptoError::SsssKeyMismatch)
    ));
    assert!(matches!(
        storage.unlock_with_recovery_key(&s.recovery_key),
        Err(CryptoError::SsssKeyMismatch)
    ));

    // 方式名が違う記述は形式の誤り。
    let mut description = s.ssss_key.description();
    description.algorithm = "m.secret_storage.v1.curve25519-aes-sha2".into();
    assert!(matches!(
        s.ssss_key.check(&description),
        Err(CryptoError::InvalidSecretStorage(_))
    ));
}

#[test]
fn secrets_moved_or_tampered_are_rejected() {
    let s = setup(ALICE);
    let mut response = server_response(&s.request, 1);
    // 自己署名鍵の暗号文をマスター鍵の名前に置く。
    response["secrets"]["m.cross_signing.master"] =
        response["secrets"]["m.cross_signing.self_signing"].clone();
    // バックアップ鍵の MAC を壊す。
    let mut mac = vodozemac::base64_decode(
        response["secrets"]["m.megolm_backup.v1"]["mac"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    mac[0] ^= 1;
    response["secrets"]["m.megolm_backup.v1"]["mac"] = json!(vodozemac::base64_encode(mac));
    let unlocked = storage(ALICE, &response)
        .unlock_with_recovery_key(&s.recovery_key)
        .unwrap();
    assert!(matches!(
        unlocked.cross_signing_key(KeyUsage::Master),
        Err(CryptoError::SecretDecryption(_))
    ));
    assert!(matches!(
        unlocked.backup_key(),
        Err(CryptoError::SecretDecryption(_))
    ));
    assert!(
        unlocked
            .cross_signing_key(KeyUsage::UserSigning)
            .unwrap()
            .is_some()
    );
}

#[test]
fn rotated_ssss_key_hides_new_secrets_from_the_old_key() {
    let old = setup(ALICE);
    let old_response = server_response(&old.request, 1);

    // 失効の 2〜5：新しい SSSS 鍵、クロス署名鍵、バックアップ鍵を作り、残したリカバリーキーで
    // 新しいスロットを作る。古いバックアップ鍵は zoo.megolm_backup.previous に退避する。
    let new_cross_signing = CrossSigningKeys::generate(ALICE).unwrap();
    let new_backup = BackupDecryptionKey::generate();
    let new_key = SsssKey::generate();
    let new_prf: [u8; 32] = rand_bytes();
    let slots = [
        KeySlot::new_passkey(ALICE, &new_key, CREDENTIAL, &new_prf_salt(), &new_prf).unwrap(),
        KeySlot::new_recovery(ALICE, &new_key, &old.recovery_key).unwrap(),
    ];
    let request = new_key
        .keys_request(&SecretStorageKeysRequest {
            expected_default_key_id: Some(old.ssss_key.key_id()),
            expected_secrets_revision: Some(1),
            cross_signing: &new_cross_signing,
            backup_key: &new_backup,
            previous_backup_keys: std::slice::from_ref(&old.backup_key),
            slots: &slots,
        })
        .unwrap();
    assert_eq!(
        request["expected_default_key_id"],
        json!(old.ssss_key.key_id())
    );
    assert_eq!(request["expected_secrets_revision"], json!(1));
    let mut new_response = server_response(&request, 2);
    // 古いスロットは消えている。サーバーが古いスロットを残して返しても、既定の鍵のものではないので使わない。
    new_response["slots"]
        .as_array_mut()
        .unwrap()
        .push(old_response["slots"][0].clone());
    let new_storage = storage(ALICE, &new_response);
    assert_eq!(new_storage.slots().len(), 2);

    // 古い SSSS 鍵では、新しい秘密も新しい鍵の確認値も通らない。
    let new_description =
        serde_json::from_value(new_response["keys"][new_key.key_id()].clone()).unwrap();
    assert!(matches!(
        old.ssss_key.check(&new_description),
        Err(CryptoError::SsssKeyMismatch)
    ));
    for name in REQUIRED_SECRETS {
        let secret = serde_json::from_value(new_response["secrets"][name].clone()).unwrap();
        assert!(matches!(
            old.ssss_key.decrypt_secret(name, &secret),
            Err(CryptoError::SecretDecryption(_))
        ));
        // key_id を消して送り直しても、MAC が合わない。
        let mut secret: zoo_crypto::EncryptedSecret = secret;
        secret.key_id = None;
        assert!(matches!(
            old.ssss_key.decrypt_secret(name, &secret),
            Err(CryptoError::SecretDecryption(_))
        ));
    }
    // 古いパスキーの PRF 出力（古い salt のもの）では、新しいスロットを開けない。
    assert!(matches!(
        new_storage.unlock_with_passkey(CREDENTIAL, &old.prf_output),
        Err(CryptoError::KeySlotUnlock)
    ));

    // 漏れていないリカバリーキーでは新しい SSSS を開け、新しい鍵と退避した古い鍵を取り出せる。
    let unlocked = new_storage
        .unlock_with_recovery_key(&old.recovery_key)
        .unwrap();
    assert_eq!(
        unlocked
            .cross_signing_key(KeyUsage::Master)
            .unwrap()
            .unwrap()
            .public_key(),
        new_cross_signing.master.public_key()
    );
    assert_eq!(
        unlocked.backup_key().unwrap().unwrap().public_key(),
        new_backup.public_key()
    );
    let previous = unlocked.previous_backup_keys().unwrap();
    assert_eq!(previous.len(), 1);
    assert_eq!(previous[0].public_key(), old.backup_key.public_key());
}

#[test]
fn device_only_revocation_rewrites_secrets_under_the_same_key() {
    let s = setup(ALICE);
    let new_backup = BackupDecryptionKey::generate();
    let new_keys = CrossSigningKeys::generate(ALICE).unwrap();
    let request = s.ssss_key.secrets_request(
        Some(1),
        &[
            s.ssss_key.encrypt_cross_signing_key(&new_keys.self_signing),
            s.ssss_key.encrypt_cross_signing_key(&new_keys.user_signing),
            (
                "m.megolm_backup.v1",
                s.ssss_key.encrypt_backup_key(&new_backup),
            ),
            (
                SECRET_PREVIOUS_MEGOLM_BACKUP,
                s.ssss_key
                    .encrypt_previous_backup_keys(std::slice::from_ref(&s.backup_key)),
            ),
        ],
    );
    assert_eq!(request["key_id"], json!(s.ssss_key.key_id()));
    assert_eq!(request["expected_secrets_revision"], json!(1));
    let secrets = request["secrets"].as_object().unwrap();
    assert_eq!(secrets.len(), 4);
    assert!(!secrets.contains_key("m.cross_signing.master"));

    let mut response = server_response(&s.request, 2);
    for (name, secret) in secrets {
        response["secrets"][name] = secret.clone();
        response["secrets"][name]["key_id"] = json!(s.ssss_key.key_id());
    }
    let unlocked = storage(ALICE, &response)
        .unlock_with_passkey(CREDENTIAL, &s.prf_output)
        .unwrap();
    assert_eq!(
        unlocked
            .cross_signing_key(KeyUsage::Master)
            .unwrap()
            .unwrap()
            .public_key(),
        s.cross_signing.master.public_key()
    );
    assert_eq!(
        unlocked
            .cross_signing_key(KeyUsage::SelfSigning)
            .unwrap()
            .unwrap()
            .public_key(),
        new_keys.self_signing.public_key()
    );
    assert_eq!(
        unlocked.previous_backup_keys().unwrap()[0].public_key(),
        s.backup_key.public_key()
    );
}

#[test]
fn previous_backup_keys_must_match_their_public_keys() {
    let s = setup(ALICE);
    let other = BackupDecryptionKey::generate();
    let forged = json!({"keys": [{
        "public_key": other.public_key().to_base64(),
        "private_key": *s.backup_key.to_base64(),
    }]})
    .to_string();
    let mut response = server_response(&s.request, 1);
    response["secrets"][SECRET_PREVIOUS_MEGOLM_BACKUP] = serde_json::to_value(
        s.ssss_key
            .encrypt_secret(SECRET_PREVIOUS_MEGOLM_BACKUP, &forged),
    )
    .unwrap();
    let unlocked = storage(ALICE, &response)
        .unlock_with_recovery_key(&s.recovery_key)
        .unwrap();
    assert!(matches!(
        unlocked.previous_backup_keys(),
        Err(CryptoError::InvalidSecretStorage(_))
    ));
}

#[test]
fn keys_request_requires_slots_of_the_same_key() {
    let s = setup(ALICE);
    let request = |slots: &[KeySlot]| {
        s.ssss_key.keys_request(&SecretStorageKeysRequest {
            expected_default_key_id: None,
            expected_secrets_revision: None,
            cross_signing: &s.cross_signing,
            backup_key: &s.backup_key,
            previous_backup_keys: &[],
            slots,
        })
    };
    assert!(matches!(
        request(&[]),
        Err(CryptoError::InvalidSecretStorage(_))
    ));
    let other_key = SsssKey::generate();
    let other_slot = KeySlot::new_recovery(ALICE, &other_key, &s.recovery_key).unwrap();
    assert!(matches!(
        request(&[other_slot]),
        Err(CryptoError::InvalidSecretStorage(_))
    ));
}

#[test]
fn validates_uuid_notation_of_key_and_slot_ids() {
    let upper = "5D6A1D1C-0C4F-4C3A-9B1E-2F8A7E6D5C4B";
    assert!(matches!(
        SsssKey::from_bytes(upper, [0; 32]),
        Err(CryptoError::InvalidId {
            field: "key_id",
            ..
        })
    ));
    let key = SsssKey::generate();
    let recovery_key = RecoveryKey::generate();
    assert!(matches!(
        KeySlot::seal(
            ALICE,
            &key,
            upper,
            SlotSecret::Recovery(&recovery_key),
            [0; 12]
        ),
        Err(CryptoError::InvalidId {
            field: "slot_id",
            ..
        })
    ));
    assert!(matches!(
        KeySlot::new_recovery("not-a-uuid", &key, &recovery_key),
        Err(CryptoError::InvalidId {
            field: "user_id",
            ..
        })
    ));
    assert!(matches!(
        KeySlot::new_passkey(ALICE, &key, "not base64url!", &[0; 32], &[0; 32]),
        Err(CryptoError::InvalidId {
            field: "credential_id",
            ..
        })
    ));

    // サーバーが返した ID の表記が違えば、形式の誤りとして扱う。
    let s = setup(ALICE);
    let response = server_response(&s.request, 1);
    let mut bad_default = response.clone();
    bad_default["default_key_id"] = json!(s.ssss_key.key_id().to_uppercase());
    assert!(matches!(
        SecretStorage::from_json(ALICE, &bad_default),
        Err(CryptoError::InvalidSecretStorage(_))
    ));
    let mut bad_slot = response.clone();
    bad_slot["slots"][1]["id"] = json!(upper);
    assert!(matches!(
        storage(ALICE, &bad_slot).unlock_with_recovery_key(&s.recovery_key),
        Err(CryptoError::InvalidSecretStorage(_))
    ));

    // 作る ID は小文字でハイフン区切りの UUID v4。
    let slot = KeySlot::new_recovery(ALICE, &key, &recovery_key).unwrap();
    for id in [key.key_id(), slot.id.as_str()] {
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(
            id.chars()
                .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }
}

#[test]
fn debug_output_hides_secrets() {
    let s = setup(ALICE);
    for text in [
        format!("{:?}", s.ssss_key),
        format!("{:?}", s.recovery_key),
        format!("{:?}", s.backup_key),
    ] {
        assert!(
            !text.contains(&vodozemac::base64_encode(s.ssss_key.as_bytes())),
            "{text}"
        );
        assert!(!text.contains(&*s.recovery_key.to_display()), "{text}");
        assert!(!text.contains(&*s.backup_key.to_base64()), "{text}");
    }
}
