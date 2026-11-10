use hkdf::Hkdf;
use serde_json::{Value, json};
use sha2::Sha256;
use vodozemac::olm::{Account, AccountPickle};
use vodozemac::{base64_decode, base64_encode};
use zoo_crypto::{
    CryptoError, DeviceAccount, MEGOLM_ALGORITHM, OLM_ALGORITHM, PickleError, verify_device_keys,
    verify_json_signature,
};

const USER_ID: &str = "00000000-0000-0000-0000-000000000001";
const DEVICE_ID: &str = "3f8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const OTHER_USER_ID: &str = "00000000-0000-0000-0000-000000000002";
const OTHER_DEVICE_ID: &str = "4f8a1c2e-5b7d-4e9f-a1b2-c3d4e5f60718";
const STORAGE_KEY: [u8; 32] = [7; 32];
/// nil UUID。
const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";
/// UUID v1（RFC 4122 の付録にある DNS の名前空間）。
const V1_UUID: &str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";

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
        // device_id は UUID v4 だけを受け付ける。
        (USER_ID, NIL_UUID),
        (USER_ID, V1_UUID),
        // version は 4 だが、variant が RFC 4122（8、9、a、b）ではない。
        (USER_ID, "3f8a1c2e-5b7d-4e9f-c1b2-c3d4e5f60718"),
        (USER_ID, "3f8a1c2e-5b7d-4e9f-71b2-c3d4e5f60718"),
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
fn accepts_any_uuid_version_for_user_id_but_only_v4_for_device_id() {
    // user_id は PostgreSQL の uuid 型の値なので、表記だけを確かめる。
    for user_id in [NIL_UUID, V1_UUID, USER_ID] {
        DeviceAccount::new(user_id, DEVICE_ID).unwrap();
    }
    for device_id in [
        DEVICE_ID,
        "3f8a1c2e-5b7d-4e9f-8000-000000000000",
        "3f8a1c2e-5b7d-4e9f-bfff-ffffffffffff",
    ] {
        DeviceAccount::new(USER_ID, device_id).unwrap();
    }
    assert!(matches!(
        DeviceAccount::new(USER_ID, NIL_UUID),
        Err(CryptoError::InvalidId {
            field: "device_id",
            ..
        })
    ));
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
    assert!(matches!(
        verify_device_keys(&device_keys, OTHER_USER_ID, DEVICE_ID),
        Err(CryptoError::InvalidDeviceKeys(_))
    ));
    assert!(matches!(
        verify_device_keys(&device_keys, USER_ID, OTHER_DEVICE_ID),
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

fn restore(pickle: &str) -> Result<DeviceAccount, CryptoError> {
    DeviceAccount::from_pickle(pickle, &STORAGE_KEY, USER_ID, DEVICE_ID)
}

/// docs/e2ee.md の保存形式（版 2）に従って、保存ごとの pickle key を導く。
fn derive_pickle_key(storage_key: &[u8; 32], salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), storage_key)
        .expand(b"zoo e2ee device pickle key v1", &mut key)
        .unwrap();
    key
}

/// 保存した状態から、vodozemac の平文の pickle を取り出す。
fn open_inner(pickle: &str, storage_key: &[u8; 32]) -> AccountPickle {
    let envelope: Value = serde_json::from_str(pickle).unwrap();
    let salt = base64_decode(envelope["salt"].as_str().unwrap()).unwrap();
    AccountPickle::from_encrypted(
        envelope["account"].as_str().unwrap(),
        &derive_pickle_key(storage_key, &salt),
    )
    .unwrap()
}

/// `LP(context)‖LP(user_id)‖LP(device_id)`（docs/e2ee.md の `binding`）。
fn binding_message(user_id: &str, device_id: &str) -> Vec<u8> {
    let mut message = Vec::new();
    for part in ["zoo e2ee device pickle v1", user_id, device_id] {
        message.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
        message.extend_from_slice(part.as_bytes());
    }
    message
}

/// 鍵を持つ側が、任意の ID で正しく署名した保存形式を作る（DeviceAccount::new を通さない）。
fn forge_pickle(account: &Account, user_id: &str, device_id: &str) -> String {
    let salt = [9u8; 32];
    json!({
        "version": 2,
        "user_id": user_id,
        "device_id": device_id,
        "salt": base64_encode(salt),
        "account": account.pickle().encrypt(&derive_pickle_key(&STORAGE_KEY, &salt)),
        "binding": account.sign(binding_message(user_id, device_id)).to_base64(),
    })
    .to_string()
}

/// `account` の暗号文の先頭 32 バイト（AES-CBC の最初の 2 ブロック）。
fn ciphertext_prefix(pickle: &str) -> Vec<u8> {
    let envelope: Value = serde_json::from_str(pickle).unwrap();
    base64_decode(envelope["account"].as_str().unwrap()).unwrap()[..32].to_vec()
}

fn salt(pickle: &str) -> String {
    let envelope: Value = serde_json::from_str(pickle).unwrap();
    envelope["salt"].as_str().unwrap().to_owned()
}

#[test]
fn pickle_round_trip_keeps_keys_and_publish_state() {
    let mut account = account();
    account.generate_one_time_keys(3);
    account.generate_fallback_key();
    account.mark_keys_as_published();
    account.generate_one_time_keys(2);
    let unpublished = account.one_time_keys();

    let pickle = account.pickle(&STORAGE_KEY);
    let restored = restore(&pickle).unwrap();

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

    // 暗号化されているので、識別鍵（公開鍵）も平文の pickle に出ない。
    // 秘密鍵が出ないことは pickle_does_not_contain_secret_keys で確かめる。
    let identity = account.identity_keys();
    assert!(!pickle.contains(&identity.curve25519.to_base64()));
    assert!(!pickle.contains(&identity.ed25519.to_base64()));
}

#[test]
fn pickle_uses_a_fresh_key_on_every_save() {
    let mut account = account();
    let first = account.pickle(&STORAGE_KEY);
    let second = account.pickle(&STORAGE_KEY);
    account.generate_one_time_keys(1);
    let third = account.pickle(&STORAGE_KEY);

    // 状態が同じでも、同じ保存鍵でも、salt と暗号文の先頭が毎回変わる。
    let pickles = [&first, &second, &third];
    for (i, a) in pickles.iter().enumerate() {
        for b in &pickles[i + 1..] {
            assert_ne!(salt(a), salt(b));
            assert_ne!(ciphertext_prefix(a), ciphertext_prefix(b));
        }
    }

    // どれも元に戻せる。
    for pickle in pickles {
        assert_eq!(
            restore(pickle).unwrap().identity_keys(),
            account.identity_keys()
        );
    }
    assert_eq!(restore(&third).unwrap().one_time_keys().len(), 1);

    // 比べ方の確認：vodozemac に同じ pickle key を直接渡すと、先頭は一致する（v1 の問題）。
    let fixed = |account: &DeviceAccount| {
        let inner = open_inner(&account.pickle(&STORAGE_KEY), &STORAGE_KEY);
        let ciphertext = Account::from_pickle(inner).pickle().encrypt(&STORAGE_KEY);
        base64_decode(ciphertext).unwrap()[..32].to_vec()
    };
    assert_eq!(fixed(&account), fixed(&account));
}

/// JSON の値から、32 バイトのバイト列（数の配列）をすべて集める。
fn collect_32_byte_arrays(value: &Value, out: &mut Vec<Vec<u8>>) {
    match value {
        Value::Array(items) => match as_32_bytes(value) {
            Some(bytes) => out.push(bytes),
            None => items
                .iter()
                .for_each(|item| collect_32_byte_arrays(item, out)),
        },
        Value::Object(map) => map
            .values()
            .for_each(|item| collect_32_byte_arrays(item, out)),
        _ => {}
    }
}

fn as_32_bytes(value: &Value) -> Option<Vec<u8>> {
    let bytes: Vec<u8> = value
        .as_array()?
        .iter()
        .map(|item| item.as_u64().and_then(|n| u8::try_from(n).ok()))
        .collect::<Option<_>>()?;
    (bytes.len() == 32).then_some(bytes)
}

/// 秘密鍵を文字列にするときに使われそうな表記。
fn encodings(secret: &[u8]) -> Vec<String> {
    let unpadded = base64_encode(secret);
    let padded = format!("{unpadded}{}", "=".repeat((4 - unpadded.len() % 4) % 4));
    let url_safe = |s: &str| s.replace('+', "-").replace('/', "_");
    let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    vec![
        url_safe(&unpadded),
        url_safe(&padded),
        unpadded,
        padded,
        hex.to_uppercase(),
        hex,
        serde_json::to_string(secret).unwrap(),
    ]
}

/// `haystack` に平文で入っている秘密鍵の表記。
fn leaked_encodings(haystack: &str, secrets: &[Vec<u8>]) -> Vec<String> {
    secrets
        .iter()
        .flat_map(|secret| encodings(secret))
        .filter(|encoded| haystack.contains(encoded.as_str()))
        .collect()
}

#[test]
fn pickle_does_not_contain_secret_keys() {
    let mut account = account();
    account.generate_one_time_keys(2);
    // 2 回作り、previous_fallback_key も入れる。
    account.generate_fallback_key();
    account.generate_fallback_key();
    let pickle = account.pickle(&STORAGE_KEY);

    // 包みの中の vodozemac の pickle を復号し、秘密鍵のフィールドを明示して取り出す
    // （vodozemac 0.11.2 の AccountPickle の serde の形）。
    let inner = serde_json::to_value(open_inner(&pickle, &STORAGE_KEY)).unwrap();
    let field = |pointer: &str| {
        as_32_bytes(
            inner
                .pointer(pointer)
                .unwrap_or_else(|| panic!("{pointer} がない")),
        )
        .unwrap_or_else(|| panic!("{pointer} が 32 バイトの配列ではない"))
    };
    let mut secrets = vec![
        field("/signing_key/Normal"),
        field("/diffie_hellman_key"),
        field("/fallback_keys/fallback_key/key"),
        field("/fallback_keys/previous_fallback_key/key"),
    ];
    let private_keys = inner["one_time_keys"]["private_keys"].as_object().unwrap();
    assert_eq!(private_keys.len(), 2);
    secrets.extend(private_keys.values().map(|key| as_32_bytes(key).unwrap()));
    // 署名鍵、識別鍵、フォールバック鍵 2 つ、ワンタイム鍵 2 つ。
    assert_eq!(secrets.len(), 6);
    let mut distinct = secrets.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 6);

    // 他に 32 バイトの配列はワンタイム鍵の公開鍵（2 つ）だけ。秘密鍵の表現が増えたら、ここで気付く。
    let mut all = Vec::new();
    collect_32_byte_arrays(&inner, &mut all);
    assert_eq!(
        inner["one_time_keys"]["public_keys"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(all.len(), secrets.len() + 2);

    // 検査の確認：暗号化していない pickle に向けると、すべての秘密鍵を見つける。
    let unencrypted = inner.to_string();
    for secret in &secrets {
        assert!(
            !leaked_encodings(&unencrypted, std::slice::from_ref(secret)).is_empty(),
            "平文の pickle で秘密鍵を見つけられない"
        );
    }

    let leaked = leaked_encodings(&pickle, &secrets);
    assert!(leaked.is_empty(), "秘密鍵が平文で入っている: {leaked:?}");
}

#[test]
fn pickle_with_wrong_key_fails() {
    let pickle = account().pickle(&STORAGE_KEY);
    let mut wrong_key = STORAGE_KEY;
    wrong_key[0] ^= 1;
    assert!(matches!(
        DeviceAccount::from_pickle(&pickle, &wrong_key, USER_ID, DEVICE_ID),
        Err(CryptoError::Pickle(PickleError::Decryption(_)))
    ));
}

#[test]
fn pickle_from_another_user_or_device_is_rejected() {
    // 同じ保存鍵で保存した、別のユーザーの別の端末の pickle を丸ごと差し替える。
    let other = DeviceAccount::new(OTHER_USER_ID, OTHER_DEVICE_ID).unwrap();
    let pickle = other.pickle(&STORAGE_KEY);

    for (user_id, device_id) in [
        (USER_ID, DEVICE_ID),
        (USER_ID, OTHER_DEVICE_ID),
        (OTHER_USER_ID, DEVICE_ID),
    ] {
        let result = DeviceAccount::from_pickle(&pickle, &STORAGE_KEY, user_id, device_id);
        match result {
            Err(CryptoError::PickleOwnerMismatch {
                expected_user_id,
                expected_device_id,
                user_id: actual_user_id,
                device_id: actual_device_id,
            }) => {
                assert_eq!(expected_user_id, user_id);
                assert_eq!(expected_device_id, device_id);
                assert_eq!(actual_user_id, OTHER_USER_ID);
                assert_eq!(actual_device_id, OTHER_DEVICE_ID);
            }
            other => panic!("{user_id} {device_id}: {other:?}"),
        }
    }

    // 持ち主の ID を渡せば戻せる。
    let restored =
        DeviceAccount::from_pickle(&pickle, &STORAGE_KEY, OTHER_USER_ID, OTHER_DEVICE_ID).unwrap();
    assert_eq!(restored.identity_keys(), other.identity_keys());
}

#[test]
fn pickle_rejects_ids_that_new_would_reject() {
    let account = Account::new();

    // 作り方の確認：正しい ID なら、この方法で作った保存形式は読める。
    let valid = forge_pickle(&account, USER_ID, DEVICE_ID);
    assert_eq!(
        restore(&valid).unwrap().identity_keys(),
        account.identity_keys()
    );

    // binding は正しいが、ID が DeviceAccount::new の条件を満たさない。
    for (user_id, device_id) in [
        ("not-a-uuid", DEVICE_ID),
        ("00000000-0000-0000-0000-00000000000A", DEVICE_ID),
        (USER_ID, NIL_UUID),
        (USER_ID, V1_UUID),
        (USER_ID, "3F8A1C2E-5B7D-4E9F-A1B2-C3D4E5F60718"),
    ] {
        let pickle = forge_pickle(&account, user_id, device_id);
        let result = DeviceAccount::from_pickle(&pickle, &STORAGE_KEY, user_id, device_id);
        assert!(
            matches!(result, Err(CryptoError::InvalidPickle(_))),
            "{user_id} {device_id}: {result:?}"
        );
    }
}

#[test]
fn pickle_rejects_tampering_and_unknown_formats() {
    let pickle = account().pickle(&STORAGE_KEY);
    let envelope: Value = serde_json::from_str(&pickle).unwrap();

    // ID を書き換えると、鍵との対応の署名が合わない（書き換えた ID を期待しても通らない）。
    for (field, user_id, device_id) in [
        ("device_id", USER_ID, OTHER_DEVICE_ID),
        ("user_id", OTHER_USER_ID, DEVICE_ID),
    ] {
        let mut tampered = envelope.clone();
        tampered[field] = json!(if field == "user_id" {
            user_id
        } else {
            device_id
        });
        assert!(
            matches!(
                DeviceAccount::from_pickle(&tampered.to_string(), &STORAGE_KEY, user_id, device_id),
                Err(CryptoError::InvalidPickle(_))
            ),
            "{field}"
        );
    }

    // 別のアカウントの pickle と ID の組み合わせも受け付けない。
    let other: Value = serde_json::from_str(
        &DeviceAccount::new(USER_ID, OTHER_DEVICE_ID)
            .unwrap()
            .pickle(&STORAGE_KEY),
    )
    .unwrap();
    let mut mixed = envelope.clone();
    mixed["account"] = other["account"].clone();
    mixed["salt"] = other["salt"].clone();
    assert!(matches!(
        restore(&mixed.to_string()),
        Err(CryptoError::InvalidPickle(_))
    ));

    for version in [1, 3] {
        let mut changed = envelope.clone();
        changed["version"] = json!(version);
        assert!(matches!(
            restore(&changed.to_string()),
            Err(CryptoError::InvalidPickle(_))
        ));
    }

    assert!(matches!(
        restore("not json"),
        Err(CryptoError::InvalidPickle(_))
    ));

    // salt がない、または 32 バイトでない。
    let mut no_salt = envelope.clone();
    no_salt.as_object_mut().unwrap().remove("salt");
    assert!(matches!(
        restore(&no_salt.to_string()),
        Err(CryptoError::InvalidPickle(_))
    ));
    let mut short_salt = envelope.clone();
    short_salt["salt"] = json!(base64_encode([0u8; 16]));
    assert!(matches!(
        restore(&short_salt.to_string()),
        Err(CryptoError::InvalidPickle(_))
    ));

    // salt を別の 32 バイトにすると、導く鍵が変わって復号できない。
    let mut other_salt = envelope.clone();
    other_salt["salt"] = json!(base64_encode([0u8; 32]));
    assert!(matches!(
        restore(&other_salt.to_string()),
        Err(CryptoError::Pickle(PickleError::Decryption(_)))
    ));

    let mut garbage = envelope.clone();
    garbage["account"] = json!("AAAA");
    assert!(matches!(
        restore(&garbage.to_string()),
        Err(CryptoError::Pickle(_))
    ));
}
