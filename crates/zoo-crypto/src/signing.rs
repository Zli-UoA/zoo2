//! Matrix の [Signing JSON](https://spec.matrix.org/latest/appendices/#signing-json) と同じ署名。
//!
//! `signatures` と `unsigned` を除いたオブジェクトを [Canonical JSON](crate::to_canonical_json)
//! にして Ed25519 で署名し、`signatures.<user_id>.<鍵 ID>` に入れる。

use serde_json::{Map, Value};
use vodozemac::{Ed25519PublicKey, Ed25519Signature};

use crate::{CryptoError, to_canonical_json};

/// 署名の対象になるバイト列（`signatures` と `unsigned` を除いた Canonical JSON）を返す。
///
/// `value` は JSON オブジェクトでなければならない。
pub fn signable_json(value: &Value) -> Result<String, CryptoError> {
    let Value::Object(map) = value else {
        return Err(CryptoError::CanonicalJson(
            "署名できるのは JSON オブジェクトだけです".to_owned(),
        ));
    };
    let mut map = map.clone();
    map.remove("signatures");
    map.remove("unsigned");
    to_canonical_json(&Value::Object(map))
}

/// `value` に署名し、`signatures.<user_id>.<key_id>` に追加する。他の署名は残す。
pub(crate) fn sign_json_with(
    value: &mut Value,
    user_id: &str,
    key_id: &str,
    sign: impl FnOnce(&[u8]) -> Ed25519Signature,
) -> Result<(), CryptoError> {
    let signature = sign(signable_json(value)?.as_bytes());
    let Value::Object(map) = value else {
        unreachable!("signable_json がオブジェクト以外を弾く");
    };
    let signatures = object_entry(map, "signatures")?;
    let by_user = object_entry(signatures, user_id)?;
    by_user.insert(key_id.to_owned(), Value::String(signature.to_base64()));
    Ok(())
}

fn object_entry<'a>(
    map: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, CryptoError> {
    map.entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| CryptoError::CanonicalJson(format!("{key} がオブジェクトではありません")))
}

/// `value` の `signatures.<user_id>.<key_id>` にある署名を、`public_key` で検証する。
///
/// 署名がなければ [`CryptoError::MissingSignature`]、一致しなければ [`CryptoError::Signature`] を返す。
pub fn verify_json_signature(
    value: &Value,
    user_id: &str,
    key_id: &str,
    public_key: &Ed25519PublicKey,
) -> Result<(), CryptoError> {
    let signature = value
        .get("signatures")
        .and_then(|s| s.get(user_id))
        .and_then(|s| s.get(key_id))
        .and_then(Value::as_str)
        .ok_or_else(|| CryptoError::MissingSignature {
            user_id: user_id.to_owned(),
            key_id: key_id.to_owned(),
        })?;
    let signature = Ed25519Signature::from_base64(signature)?;
    public_key.verify(signable_json(value)?.as_bytes(), &signature)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vodozemac::Ed25519SecretKey;

    use super::*;

    /// Matrix の付録の「Signing details」の例。
    /// 鍵の seed は `YJDBA9Xnr2sVqXD9Vj7XVUnmFZcZrlw8Md7kMW+3XA1`、鍵 ID は `ed25519:1`。
    /// 末尾の文字に余分なビットが立っていて厳密な base64 では読めないので、バイト列で書く。
    fn matrix_example_key() -> Ed25519SecretKey {
        Ed25519SecretKey::from_slice(&[
            96, 144, 193, 3, 213, 231, 175, 107, 21, 169, 112, 253, 86, 62, 215, 85, 73, 230, 21,
            151, 25, 174, 92, 60, 49, 222, 228, 49, 111, 183, 92, 13,
        ])
    }

    fn sign_example(value: &mut Value) {
        let key = matrix_example_key();
        sign_json_with(value, "domain", "ed25519:1", |m| key.sign(m)).unwrap();
    }

    #[test]
    fn matches_matrix_signing_examples() {
        let mut empty = json!({});
        sign_example(&mut empty);
        assert_eq!(
            empty,
            json!({ "signatures": { "domain": {
                "ed25519:1": "K8280/U9SSy9IVtjBuVeLr+HpOB4BQFWbg+UZaADMtTdGYI7Geitb76LTrr5QV/7Xg4ahLwYGYZzuHGZKM5ZAQ"
            } } })
        );

        let mut object = json!({ "one": 1, "two": "Two" });
        sign_example(&mut object);
        assert_eq!(
            object["signatures"]["domain"]["ed25519:1"],
            "KqmLSbO39/Bzb0QIYE82zqLwsA+PDzYIpIRA2sRQ4sL53+sN6/fpNSoqE7BP7vBZhG6kYdD13EIMJpvhJI+6Bw"
        );
        let public_key = matrix_example_key().public_key();
        verify_json_signature(&object, "domain", "ed25519:1", &public_key).unwrap();
    }

    #[test]
    fn ignores_unsigned_and_keeps_other_signatures() {
        let mut value = json!({
            "a": 1,
            "unsigned": { "age": 3 },
            "signatures": { "other": { "ed25519:x": "kept" } }
        });
        sign_example(&mut value);
        assert_eq!(value["signatures"]["other"]["ed25519:x"], "kept");
        assert_eq!(value["unsigned"]["age"], 3);

        // unsigned を変えても署名は有効なまま。
        value["unsigned"]["age"] = json!(10);
        let public_key = matrix_example_key().public_key();
        verify_json_signature(&value, "domain", "ed25519:1", &public_key).unwrap();
    }

    #[test]
    fn rejects_tampered_or_missing_signatures() {
        let public_key = matrix_example_key().public_key();
        let mut value = json!({ "a": 1 });
        sign_example(&mut value);

        let mut tampered = value.clone();
        tampered["a"] = json!(2);
        assert!(matches!(
            verify_json_signature(&tampered, "domain", "ed25519:1", &public_key),
            Err(CryptoError::Signature(_))
        ));

        assert!(matches!(
            verify_json_signature(&value, "domain", "ed25519:2", &public_key),
            Err(CryptoError::MissingSignature { .. })
        ));

        let other_key = Ed25519SecretKey::new().public_key();
        assert!(matches!(
            verify_json_signature(&value, "domain", "ed25519:1", &other_key),
            Err(CryptoError::Signature(_))
        ));
    }

    #[test]
    fn rejects_non_objects() {
        assert!(matches!(
            signable_json(&json!([1])),
            Err(CryptoError::CanonicalJson(_))
        ));
    }
}
