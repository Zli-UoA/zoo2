//! 端末のアカウント（識別鍵、ワンタイム鍵、フォールバック鍵）と、その保存。
//!
//! 形式は docs/e2ee.md の「端末鍵とワンタイム鍵」と同じ（Matrix の Device keys と
//! One-time and fallback keys に合わせる）。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vodozemac::olm::{Account, AccountPickle, IdentityKeys};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature, KeyId};

use crate::signing::sign_json_with;
use crate::{CryptoError, verify_json_signature};

/// Olm の方式名。
pub const OLM_ALGORITHM: &str = "m.olm.v1.curve25519-aes-sha2";
/// Megolm の方式名。
pub const MEGOLM_ALGORITHM: &str = "m.megolm.v1.aes-sha2";
/// ワンタイム鍵とフォールバック鍵の種類。
pub const SIGNED_CURVE25519: &str = "signed_curve25519";

/// 保存形式の版。形式を変えたら上げる。
const PICKLE_VERSION: u32 = 1;
/// 保存した状態の ID と鍵の対応に付ける署名の、用途を区別する文字列。
const PICKLE_BINDING_CONTEXT: &str = "zoo e2ee device pickle v1";

/// 1 つの端末の鍵を持つアカウント。
///
/// vodozemac の [`Account`] に、持ち主の `user_id` と `device_id` を組み合わせたもの。
/// 秘密鍵は外に出さず、公開鍵は署名付きの JSON で返す。
pub struct DeviceAccount {
    user_id: String,
    device_id: String,
    account: Account,
}

impl DeviceAccount {
    /// 新しい識別鍵（Curve25519 と Ed25519）を作る。
    ///
    /// `user_id` と `device_id` は小文字でハイフン区切りの UUID でなければならない。
    /// `device_id` はクライアントが UUID v4 で作る（サーバーへの登録前に署名に入るため）。
    pub fn new(user_id: &str, device_id: &str) -> Result<Self, CryptoError> {
        validate_uuid("user_id", user_id)?;
        validate_uuid("device_id", device_id)?;
        Ok(Self {
            user_id: user_id.to_owned(),
            device_id: device_id.to_owned(),
            account: Account::new(),
        })
    }

    /// 持ち主のユーザー ID。
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// 端末 ID。
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// 識別鍵（Curve25519）と端末の署名鍵（Ed25519）の公開鍵。
    pub fn identity_keys(&self) -> IdentityKeys {
        self.account.identity_keys()
    }

    /// 端末の署名鍵の鍵 ID（`ed25519:<device_id>`）。
    pub fn signing_key_id(&self) -> String {
        format!("ed25519:{}", self.device_id)
    }

    /// 端末の署名鍵で `value` に署名し、`signatures.<user_id>.ed25519:<device_id>` に追加する。
    ///
    /// `value` は JSON オブジェクトで、Canonical JSON にできる値だけを含まなければならない。
    pub fn sign_json(&self, value: &mut Value) -> Result<(), CryptoError> {
        let key_id = self.signing_key_id();
        sign_json_with(value, &self.user_id, &key_id, |message| {
            self.account.sign(message)
        })
    }

    /// サーバーに登録する、署名付きの端末鍵。
    ///
    /// ```json
    /// {"user_id", "device_id", "algorithms", "keys": {"curve25519:<device_id>", "ed25519:<device_id>"}, "signatures"}
    /// ```
    pub fn device_keys(&self) -> Value {
        let keys = self.identity_keys();
        let mut value = json!({
            "user_id": self.user_id,
            "device_id": self.device_id,
            "algorithms": [OLM_ALGORITHM, MEGOLM_ALGORITHM],
            "keys": {
                format!("curve25519:{}", self.device_id): keys.curve25519.to_base64(),
                format!("ed25519:{}", self.device_id): keys.ed25519.to_base64(),
            },
        });
        self.sign_owned(&mut value);
        value
    }

    /// サーバーに置いておくワンタイム鍵の数（vodozemac の値で 50）。
    pub fn max_number_of_one_time_keys(&self) -> usize {
        self.account.max_number_of_one_time_keys()
    }

    /// ワンタイム鍵を `count` 個作る。まだ公開していない鍵として
    /// [`one_time_keys`](Self::one_time_keys) に現れる。
    ///
    /// vodozemac が持てる数を超えると、古い鍵から捨てられる。
    pub fn generate_one_time_keys(&mut self, count: usize) {
        self.account.generate_one_time_keys(count);
    }

    /// 新しいフォールバック鍵を作る。1 つ前のフォールバック鍵は、遅れて届くメッセージのために残す。
    pub fn generate_fallback_key(&mut self) {
        self.account.generate_fallback_key();
    }

    /// まだ公開していないワンタイム鍵を、署名付きの JSON で返す。
    ///
    /// キーは `signed_curve25519:<key_id>`、値は `{"key", "signatures"}`。
    pub fn one_time_keys(&self) -> Map<String, Value> {
        self.signed_keys(self.account.one_time_keys(), false)
    }

    /// まだ公開していないフォールバック鍵を、署名付きの JSON で返す（0 個か 1 個）。
    ///
    /// キーは `signed_curve25519:<key_id>`、値は `{"key", "fallback": true, "signatures"}`。
    pub fn fallback_keys(&self) -> Map<String, Value> {
        self.signed_keys(self.account.fallback_key(), true)
    }

    /// 今の未公開のワンタイム鍵とフォールバック鍵を、公開済みにする。
    /// サーバーへのアップロードが成功してから呼ぶ。
    pub fn mark_keys_as_published(&mut self) {
        self.account.mark_keys_as_published();
    }

    /// 状態を `pickle_key` で暗号化した文字列にする。
    ///
    /// 中身は vodozemac の暗号化した pickle と、`user_id`、`device_id` を入れた JSON。
    /// `user_id` と `device_id` は平文だが、端末の署名鍵の署名で pickle と結び付けてある。
    pub fn pickle(&self, pickle_key: &[u8; 32]) -> String {
        let envelope = PickleEnvelope {
            version: PICKLE_VERSION,
            user_id: self.user_id.clone(),
            device_id: self.device_id.clone(),
            account: self.account.pickle().encrypt(pickle_key),
            binding: self
                .account
                .sign(binding_message(&self.user_id, &self.device_id))
                .to_base64(),
        };
        serde_json::to_string(&envelope).expect("文字列と数値だけの構造体は必ず JSON にできる")
    }

    /// [`pickle`](Self::pickle) で作った文字列から状態を戻す。
    ///
    /// `pickle_key` が違えば [`CryptoError::Pickle`] を返す。外側の形式が壊れている、
    /// または `user_id` と `device_id` が書き換えられていれば [`CryptoError::InvalidPickle`] を返す。
    pub fn from_pickle(pickle: &str, pickle_key: &[u8; 32]) -> Result<Self, CryptoError> {
        let envelope: PickleEnvelope = serde_json::from_str(pickle)
            .map_err(|e| CryptoError::InvalidPickle(format!("JSON として読めません: {e}")))?;
        if envelope.version != PICKLE_VERSION {
            return Err(CryptoError::InvalidPickle(format!(
                "対応していない版です: {}",
                envelope.version
            )));
        }
        let account = Account::from_pickle(AccountPickle::from_encrypted(
            &envelope.account,
            pickle_key,
        )?);
        let binding = Ed25519Signature::from_base64(&envelope.binding)
            .map_err(|e| CryptoError::InvalidPickle(format!("署名の形式が違います: {e}")))?;
        account
            .ed25519_key()
            .verify(
                &binding_message(&envelope.user_id, &envelope.device_id),
                &binding,
            )
            .map_err(|_| {
                CryptoError::InvalidPickle("user_id と device_id が鍵と対応していません".to_owned())
            })?;
        Ok(Self {
            user_id: envelope.user_id,
            device_id: envelope.device_id,
            account,
        })
    }

    fn signed_keys(
        &self,
        keys: impl IntoIterator<Item = (KeyId, Curve25519PublicKey)>,
        fallback: bool,
    ) -> Map<String, Value> {
        keys.into_iter()
            .map(|(key_id, key)| {
                let mut value = json!({ "key": key.to_base64() });
                if fallback {
                    value["fallback"] = Value::Bool(true);
                }
                self.sign_owned(&mut value);
                (format!("{SIGNED_CURVE25519}:{}", key_id.to_base64()), value)
            })
            .collect()
    }

    /// このモジュールで組み立てた、文字列と真偽値だけのオブジェクトに署名する。
    fn sign_owned(&self, value: &mut Value) {
        self.sign_json(value)
            .expect("文字列と真偽値だけのオブジェクトは必ず Canonical JSON にできる");
    }
}

impl std::fmt::Debug for DeviceAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceAccount")
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .field("identity_keys", &self.identity_keys())
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
struct PickleEnvelope {
    version: u32,
    user_id: String,
    device_id: String,
    /// vodozemac の `AccountPickle::encrypt` の出力。
    account: String,
    /// [`binding_message`] への端末の署名鍵の署名。
    binding: String,
}

/// `LP(PICKLE_BINDING_CONTEXT)‖LP(user_id)‖LP(device_id)`。LP は u32 ビッグエンディアンの長さと UTF-8 のバイト列。
///
/// JSON ではない形にして、サーバーに送る署名付き JSON と取り違えないようにする。
fn binding_message(user_id: &str, device_id: &str) -> Vec<u8> {
    let mut message = Vec::new();
    for part in [PICKLE_BINDING_CONTEXT, user_id, device_id] {
        let len = u32::try_from(part.len()).expect("ID は 4 GiB より短い");
        message.extend_from_slice(&len.to_be_bytes());
        message.extend_from_slice(part.as_bytes());
    }
    message
}

/// 他の端末の、検証済みの端末鍵。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceKeys {
    /// 持ち主のユーザー ID。
    pub user_id: String,
    /// 端末 ID。
    pub device_id: String,
    /// 端末が対応する方式。
    pub algorithms: Vec<String>,
    /// 識別鍵。
    pub curve25519: Curve25519PublicKey,
    /// 端末の署名鍵。
    pub ed25519: Ed25519PublicKey,
}

/// サーバーから受け取った端末鍵の JSON を検証する。
///
/// `user_id` と `device_id` が期待した値と一致し、`keys` に `curve25519:<device_id>` と
/// `ed25519:<device_id>` があり、その Ed25519 鍵の自己署名が正しいことを確かめる。
/// 自己署名は「この JSON がその鍵で作られた」ことしか示さない。端末を信頼してよいかは、
/// クロス署名（#350）で判断する。
pub fn verify_device_keys(
    value: &Value,
    user_id: &str,
    device_id: &str,
) -> Result<DeviceKeys, CryptoError> {
    let field = |name: &str| value.get(name).and_then(Value::as_str);
    if field("user_id") != Some(user_id) {
        return Err(CryptoError::InvalidDeviceKeys(format!(
            "user_id が {user_id} ではありません"
        )));
    }
    if field("device_id") != Some(device_id) {
        return Err(CryptoError::InvalidDeviceKeys(format!(
            "device_id が {device_id} ではありません"
        )));
    }
    let algorithms = value
        .get("algorithms")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| {
            CryptoError::InvalidDeviceKeys("algorithms が文字列の配列ではありません".to_owned())
        })?;
    let key = |algorithm: &str| {
        let key_id = format!("{algorithm}:{device_id}");
        value
            .get("keys")
            .and_then(|keys| keys.get(&key_id))
            .and_then(Value::as_str)
            .ok_or_else(|| CryptoError::InvalidDeviceKeys(format!("keys に {key_id} がありません")))
    };
    let curve25519 = Curve25519PublicKey::from_base64(key("curve25519")?)?;
    let ed25519 = Ed25519PublicKey::from_base64(key("ed25519")?)?;
    verify_json_signature(value, user_id, &format!("ed25519:{device_id}"), &ed25519)?;
    Ok(DeviceKeys {
        user_id: user_id.to_owned(),
        device_id: device_id.to_owned(),
        algorithms,
        curve25519,
        ed25519,
    })
}

/// 小文字でハイフン区切りの UUID（`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`）かを確かめる。
///
/// 署名する JSON に入るので、PostgreSQL の `uuid` 型が返す表記とそろえておく必要がある。
fn validate_uuid(field: &'static str, value: &str) -> Result<(), CryptoError> {
    let valid = value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        });
    if valid {
        Ok(())
    } else {
        Err(CryptoError::InvalidId {
            field,
            value: value.to_owned(),
        })
    }
}
