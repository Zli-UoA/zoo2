use serde_json::{Value, json};
use zoo_crypto::{
    CryptoError, DeviceAccount, MEGOLM_ALGORITHM, OLM_ALGORITHM, PickleError, verify_device_keys,
    verify_json_signature,
};

const USER_ID: &str = "00000000-0000-0000-0000-000000000001";
const DEVICE_ID: &str = "3f8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const PICKLE_KEY: [u8; 32] = [7; 32];

fn account() -> DeviceAccount {
    DeviceAccount::new(USER_ID, DEVICE_ID).unwrap()
}

#[test]
fn creates_distinct_identity_keys() {
    let a = account();
    let b = account();
    assert_ne!(a.identity_keys(), b.identity_keys());
    assert_eq!(a.user_id(), USER_ID);
    assert_eq!(a.device_id(), DEVICE_ID);
    assert_eq!(a.signing_key_id(), format!("ed25519:{DEVICE_ID}"));
}

#[test]
fn rejects_ids_that_are_not_lowercase_uuids() {
    for (user_id, device_id) in [
        ("", DEVICE_ID),
        ("not-a-uuid", DEVICE_ID),
        (USER_ID, "3F8A1C2E-5B7D-4E9F-A1B2-C3D4E5F60718"),
        (USER_ID, "3f8a1c2e5b7d4e9fa1b2c3d4e5f60718"),
        (USER_ID, "3f8a1c2e-5b7d-4e9f-a1b2-c3d4e5f6071g"),
    ] {
        assert!(
            matches!(
                DeviceAccount::new(user_id, device_id),
                Err(CryptoError::InvalidId { .. })
            ),
            "{user_id} {device_id}"
        );
    }
}

#[test]
fn exports_signed_device_keys() {
    let account = account();
    let keys = account.identity_keys();
    let device_keys = account.device_keys();

    assert_eq!(device_keys["user_id"], USER_ID);
    assert_eq!(device_keys["device_id"], DEVICE_ID);
    assert_eq!(
        device_keys["algorithms"],
        json!([OLM_ALGORITHM, MEGOLM_ALGORITHM])
    );
    assert_eq!(
        device_keys["keys"],
        json!({
            format!("curve25519:{DEVICE_ID}"): keys.curve25519.to_base64(),
            format!("ed25519:{DEVICE_ID}"): keys.ed25519.to_base64(),
        })
    );

    let verified = verify_device_keys(&device_keys, USER_ID, DEVICE_ID).unwrap();
    assert_eq!(verified.curve25519, keys.curve25519);
    assert_eq!(verified.ed25519, keys.ed25519);
    assert_eq!(verified.algorithms, [OLM_ALGORITHM, MEGOLM_ALGORITHM]);
}

#[test]
fn rejects_tampered_device_keys() {
    let account = account();
    let device_keys = account.device_keys();

    // 識別鍵を別の端末のものに差し替える。
    let mut tampered = device_keys.clone();
    tampered["keys"][format!("curve25519:{DEVICE_ID}")] = json!(
        DeviceAccount::new(USER_ID, DEVICE_ID)
            .unwrap()
            .identity_keys()
            .curve25519
            .to_base64()
    );
    assert!(matches!(
        verify_device_keys(&tampered, USER_ID, DEVICE_ID),
        Err(CryptoError::Signature(_))
    ));

    // 署名鍵ごと差し替えても、元の署名は合わない。
    let mut swapped = device_keys.clone();
    swapped["keys"][format!("ed25519:{DEVICE_ID}")] = json!(
        DeviceAccount::new(USER_ID, DEVICE_ID)
            .unwrap()
            .identity_keys()
            .ed25519
            .to_base64()
    );
    assert!(matches!(
        verify_device_keys(&swapped, USER_ID, DEVICE_ID),
        Err(CryptoError::Signature(_))
    ));

    // 方式の一覧を書き換える。
    let mut algorithms = device_keys.clone();
    algorithms["algorithms"] = json!([OLM_ALGORITHM]);
    assert!(matches!(
        verify_device_keys(&algorithms, USER_ID, DEVICE_ID),
        Err(CryptoError::Signature(_))
    ));

    // 署名を消す。
    let mut unsigned = device_keys.clone();
    unsigned["signatures"] = json!({});
    assert!(matches!(
        verify_device_keys(&unsigned, USER_ID, DEVICE_ID),
        Err(CryptoError::MissingSignature { .. })
    ));

    // 期待と違う持ち主や端末として受け取る。
    let other_user = "00000000-0000-0000-0000-000000000002";
    assert!(matches!(
        verify_device_keys(&device_keys, other_user, DEVICE_ID),
        Err(CryptoError::InvalidDeviceKeys(_))
    ));
    let other_device = "4f8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    assert!(matches!(
        verify_device_keys(&device_keys, USER_ID, other_device),
        Err(CryptoError::InvalidDeviceKeys(_))
    ));
}

#[test]
fn signs_arbitrary_json_with_the_device_key() {
    let account = account();
    let mut value = json!({ "a": [1, "b"], "unsigned": { "x": 1 } });
    account.sign_json(&mut value).unwrap();
    let ed25519 = account.identity_keys().ed25519;
    verify_json_signature(&value, USER_ID, &account.signing_key_id(), &ed25519).unwrap();

    let mut float = json!({ "a": 1.5 });
    assert!(matches!(
        account.sign_json(&mut float),
        Err(CryptoError::CanonicalJson(_))
    ));
}

fn assert_signed_key(account: &DeviceAccount, key_id: &str, value: &Value, fallback: bool) {
    assert!(key_id.starts_with("signed_curve25519:"), "{key_id}");
    assert!(value["key"].as_str().is_some_and(|k| !k.is_empty()));
    assert_eq!(
        value.get("fallback"),
        fallback.then_some(&Value::Bool(true))
    );
    let ed25519 = account.identity_keys().ed25519;
    verify_json_signature(value, USER_ID, &account.signing_key_id(), &ed25519).unwrap();

    let mut tampered = value.clone();
    tampered["key"] = json!(account.identity_keys().curve25519.to_base64());
    assert!(matches!(
        verify_json_signature(&tampered, USER_ID, &account.signing_key_id(), &ed25519),
        Err(CryptoError::Signature(_))
    ));
}

#[test]
fn generates_and_publishes_one_time_keys() {
    let mut account = account();
    assert_eq!(account.max_number_of_one_time_keys(), 50);
    assert!(account.one_time_keys().is_empty());

    account.generate_one_time_keys(10);
    let keys = account.one_time_keys();
    assert_eq!(keys.len(), 10);
    for (key_id, value) in &keys {
        assert_signed_key(&account, key_id, value, false);
    }

    account.mark_keys_as_published();
    assert!(account.one_time_keys().is_empty());

    // 公開済みの鍵は返さず、新しく作った鍵だけを返す。
    account.generate_one_time_keys(5);
    let next = account.one_time_keys();
    assert_eq!(next.len(), 5);
    assert!(next.keys().all(|key_id| !keys.contains_key(key_id)));
}

#[test]
fn generates_and_publishes_fallback_keys() {
    let mut account = account();
    assert!(account.fallback_keys().is_empty());

    account.generate_fallback_key();
    let keys = account.fallback_keys();
    assert_eq!(keys.len(), 1);
    let (key_id, value) = keys.iter().next().unwrap();
    assert_signed_key(&account, key_id, value, true);

    account.mark_keys_as_published();
    assert!(account.fallback_keys().is_empty());

    account.generate_fallback_key();
    let next = account.fallback_keys();
    assert_eq!(next.len(), 1);
    assert_ne!(next.keys().next(), Some(key_id));
}

#[test]
fn pickle_round_trip_keeps_keys_and_publish_state() {
    let mut account = account();
    account.generate_one_time_keys(3);
    account.generate_fallback_key();
    account.mark_keys_as_published();
    account.generate_one_time_keys(2);
    let unpublished = account.one_time_keys();

    let pickle = account.pickle(&PICKLE_KEY);
    let restored = DeviceAccount::from_pickle(&pickle, &PICKLE_KEY).unwrap();

    assert_eq!(restored.user_id(), USER_ID);
    assert_eq!(restored.device_id(), DEVICE_ID);
    assert_eq!(restored.identity_keys(), account.identity_keys());
    assert_eq!(restored.device_keys(), account.device_keys());
    assert!(restored.fallback_keys().is_empty());
    let restored_keys = restored.one_time_keys();
    assert_eq!(
        restored_keys.keys().collect::<Vec<_>>(),
        unpublished.keys().collect::<Vec<_>>()
    );

    // 秘密鍵は pickle の平文に出ない。
    let identity = account.identity_keys();
    assert!(!pickle.contains(&identity.curve25519.to_base64()));
    assert!(!pickle.contains(&identity.ed25519.to_base64()));
}

#[test]
fn pickle_with_wrong_key_fails() {
    let pickle = account().pickle(&PICKLE_KEY);
    let mut wrong_key = PICKLE_KEY;
    wrong_key[0] ^= 1;
    assert!(matches!(
        DeviceAccount::from_pickle(&pickle, &wrong_key),
        Err(CryptoError::Pickle(PickleError::Decryption(_)))
    ));
}

#[test]
fn pickle_rejects_tampering_and_unknown_formats() {
    let pickle = account().pickle(&PICKLE_KEY);
    let envelope: Value = serde_json::from_str(&pickle).unwrap();

    // ID を書き換えると、鍵との対応の署名が合わない。
    let other_device = "4f8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
    for (field, value) in [
        ("device_id", other_device),
        ("user_id", "00000000-0000-0000-0000-000000000002"),
    ] {
        let mut tampered = envelope.clone();
        tampered[field] = json!(value);
        assert!(
            matches!(
                DeviceAccount::from_pickle(&tampered.to_string(), &PICKLE_KEY),
                Err(CryptoError::InvalidPickle(_))
            ),
            "{field}"
        );
    }

    // 別のアカウントの pickle と ID の組み合わせも受け付けない。
    let other: Value = serde_json::from_str(
        &DeviceAccount::new(USER_ID, other_device)
            .unwrap()
            .pickle(&PICKLE_KEY),
    )
    .unwrap();
    let mut mixed = envelope.clone();
    mixed["account"] = other["account"].clone();
    assert!(matches!(
        DeviceAccount::from_pickle(&mixed.to_string(), &PICKLE_KEY),
        Err(CryptoError::InvalidPickle(_))
    ));

    let mut version = envelope.clone();
    version["version"] = json!(2);
    assert!(matches!(
        DeviceAccount::from_pickle(&version.to_string(), &PICKLE_KEY),
        Err(CryptoError::InvalidPickle(_))
    ));

    assert!(matches!(
        DeviceAccount::from_pickle("not json", &PICKLE_KEY),
        Err(CryptoError::InvalidPickle(_))
    ));

    let mut garbage = envelope.clone();
    garbage["account"] = json!("AAAA");
    assert!(matches!(
        DeviceAccount::from_pickle(&garbage.to_string(), &PICKLE_KEY),
        Err(CryptoError::Pickle(_))
    ));
}
