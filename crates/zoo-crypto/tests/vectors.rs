//! SSSS、鍵スロット、リカバリーキー、鍵バックアップのテストベクター（docs/e2ee.md の「鍵スロット」）。
//!
//! `tests/vectors/secret_storage_v1.json` の値と一致するかを確かめる。ベクターは乱数の代わりに
//! 決まったバイト列を入れて作った。値は、この実装とは別に Python の `cryptography` で計算し直して
//! 一致を確かめてある（手順は JSON の `generated_by`）。形式を変えるときは、版を上げた別のファイルにする。

use serde::Deserialize;
use zoo_crypto::{
    BackedUpSessionData, BackupDecryptionKey, KeySlot, RecoveryKey, SlotSecret, SsssKey,
};

const VECTORS: &str = include_str!("vectors/secret_storage_v1.json");

#[derive(Deserialize)]
struct Vectors {
    user_id: String,
    ssss: Ssss,
    passkey_slot: PasskeySlot,
    recovery_slot: RecoverySlot,
    backup: Backup,
}

#[derive(Deserialize)]
struct Ssss {
    key_id: String,
    key: String,
    check_iv: String,
    check_mac: String,
    secret_name: String,
    secret_plaintext: String,
    secret_iv: String,
    secret_ciphertext: String,
    secret_mac: String,
}

#[derive(Deserialize)]
struct PasskeySlot {
    slot_id: String,
    credential_id: String,
    prf_salt: String,
    prf_output: String,
    nonce: String,
    aad: String,
    kek: String,
    ciphertext: String,
}

#[derive(Deserialize)]
struct RecoverySlot {
    slot_id: String,
    recovery_key: String,
    recovery_key_display: String,
    nonce: String,
    aad: String,
    kek: String,
    ciphertext: String,
}

#[derive(Deserialize)]
struct Backup {
    private_key: String,
    public_key: String,
    ephemeral_private_key: String,
    plaintext: String,
    ephemeral: String,
    ciphertext: String,
    mac: String,
}

fn hex<const N: usize>(s: &str) -> [u8; N] {
    assert_eq!(s.len(), N * 2, "{s}");
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

fn vectors() -> Vectors {
    serde_json::from_str(VECTORS).unwrap()
}

#[test]
fn ssss_key_check_and_secret_match_vectors() {
    let v = vectors().ssss;
    let key = SsssKey::from_bytes(&v.key_id, hex(&v.key)).unwrap();

    let description = key.description_with_iv(hex(&v.check_iv));
    assert_eq!(description.algorithm, "m.secret_storage.v1.aes-hmac-sha2");
    assert_eq!(
        description.iv,
        vodozemac::base64_encode(hex::<16>(&v.check_iv))
    );
    assert_eq!(description.mac, v.check_mac);
    key.check(&description).unwrap();

    let secret = key.encrypt_secret_with_iv(&v.secret_name, &v.secret_plaintext, hex(&v.secret_iv));
    assert_eq!(secret.ciphertext, v.secret_ciphertext);
    assert_eq!(secret.mac, v.secret_mac);
    assert_eq!(
        *key.decrypt_secret(&v.secret_name, &secret).unwrap(),
        v.secret_plaintext
    );
}

#[test]
fn passkey_slot_matches_vectors() {
    let all = vectors();
    let v = all.passkey_slot;
    let key = SsssKey::from_bytes(&all.ssss.key_id, hex(&all.ssss.key)).unwrap();
    let prf_salt = hex::<32>(&v.prf_salt);
    let prf_output = hex::<32>(&v.prf_output);
    let secret = || SlotSecret::Passkey {
        credential_id: &v.credential_id,
        prf_salt: &prf_salt,
        prf_output: &prf_output,
    };
    let slot = KeySlot::seal(&all.user_id, &key, &v.slot_id, secret(), hex(&v.nonce)).unwrap();
    assert_eq!(slot.ciphertext, v.ciphertext);
    assert_eq!(
        slot.prf_salt.as_deref(),
        Some(&*vodozemac::base64_encode(prf_salt))
    );
    // aad と kek は Python で計算し直すための中間値。ここでは長さだけを確かめる。
    assert_eq!(v.aad.len() % 2, 0);
    assert_eq!(v.kek.len(), 64);
    let opened = slot.open(&all.user_id, secret()).unwrap();
    assert_eq!(opened.as_bytes(), key.as_bytes());
}

#[test]
fn recovery_slot_and_key_match_vectors() {
    let all = vectors();
    let v = all.recovery_slot;
    let key = SsssKey::from_bytes(&all.ssss.key_id, hex(&all.ssss.key)).unwrap();
    let recovery_key = RecoveryKey::from_bytes(hex(&v.recovery_key));
    assert_eq!(*recovery_key.to_display(), v.recovery_key_display);
    assert_eq!(
        RecoveryKey::parse(&v.recovery_key_display).unwrap(),
        recovery_key
    );

    let slot = KeySlot::seal(
        &all.user_id,
        &key,
        &v.slot_id,
        SlotSecret::Recovery(&recovery_key),
        hex(&v.nonce),
    )
    .unwrap();
    assert_eq!(slot.ciphertext, v.ciphertext);
    assert_eq!(slot.credential_id, None);
    assert_eq!(slot.prf_salt, None);
    assert_eq!(v.kek.len(), 64);
    assert_eq!(v.aad.len() % 2, 0);
    let opened = slot
        .open(&all.user_id, SlotSecret::Recovery(&recovery_key))
        .unwrap();
    assert_eq!(opened.as_bytes(), key.as_bytes());
}

#[test]
fn backup_encryption_matches_vectors() {
    let v = vectors().backup;
    let key = BackupDecryptionKey::from_bytes(&hex(&v.private_key));
    assert_eq!(key.public_key().to_base64(), v.public_key);
    let data: BackedUpSessionData = serde_json::from_str(&v.plaintext).unwrap();
    assert_eq!(serde_json::to_string(&data).unwrap(), v.plaintext);

    let session_data = key
        .encryption_key()
        .encrypt_with_ephemeral(&data, &hex(&v.ephemeral_private_key))
        .unwrap();
    assert_eq!(session_data.ephemeral, v.ephemeral);
    assert_eq!(session_data.ciphertext, v.ciphertext);
    assert_eq!(session_data.mac, v.mac);
    assert_eq!(key.decrypt(&session_data).unwrap(), data);
}
