//! 端末のアカウント（識別鍵、ワンタイム鍵、フォールバック鍵）と、その保存。
//!
//! 形式は docs/e2ee.md の「端末鍵とワンタイム鍵」と同じ（Matrix の Device keys と
//! One-time and fallback keys に合わせる）。

use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::Sha256;
use vodozemac::olm::{Account, AccountPickle, IdentityKeys};
use vodozemac::{
    Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature, KeyId, base64_decode, base64_encode,
};
use zeroize::Zeroizing;

use crate::signing::sign_json_with;
use crate::{CryptoError, verify_json_signature};

/// Olm の方式名。
pub const OLM_ALGORITHM: &str = "m.olm.v1.curve25519-aes-sha2";
/// Megolm の方式名。
pub const MEGOLM_ALGORITHM: &str = "m.megolm.v1.aes-sha2";
/// ワンタイム鍵とフォールバック鍵の種類。
pub const SIGNED_CURVE25519: &str = "signed_curve25519";

/// 保存形式の版。形式を変えたら上げる。
///
/// 2 で、保存ごとの salt（`salt`）を足した。1 は公開前の形式で、読み込みには対応しない。
const PICKLE_VERSION: u32 = 2;
/// 保存ごとの pickle key を導く HKDF の info。
const PICKLE_KEY_INFO: &[u8] = b"zoo e2ee device pickle key v1";
/// 保存ごとの salt の長さ（バイト）。
const PICKLE_SALT_LEN: usize = 32;
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
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    ///
    /// `user_id` と `device_id` は小文字でハイフン区切りの UUID でなければならない。
    /// `device_id` はクライアントが UUID v4 で作る（サーバーへの登録前に署名に入るため）ので、
    /// version が 4 で variant が RFC 4122 のものだけを受け付ける。nil UUID と v1 などは
    /// [`CryptoError::InvalidId`] になる。`user_id` は PostgreSQL の `uuid` 型の値なので、表記だけを確かめる。
    pub fn new(user_id: &str, device_id: &str) -> Result<Self, CryptoError> {
        validate_ids(user_id, device_id)?;
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
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn generate_one_time_keys(&mut self, count: usize) {
        self.account.generate_one_time_keys(count);
    }

    /// 新しいフォールバック鍵を作る。1 つ前のフォールバック鍵は、遅れて届くメッセージのために残す。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
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

    /// 状態を、呼び出し側の 32 バイトの保存鍵 `storage_key` から導いた鍵で暗号化した文字列にする。
    ///
    /// 中身は vodozemac の暗号化した pickle と、`salt`、`user_id`、`device_id` を入れた JSON。
    /// `user_id` と `device_id` は平文だが、端末の署名鍵の署名で pickle と結び付けてある。
    ///
    /// `storage_key` は vodozemac にそのまま渡さない。呼ぶたびに新しい 32 バイトの salt を作り、
    /// `HKDF-SHA256(ikm = storage_key, salt, info = "zoo e2ee device pickle key v1")` で
    /// その保存だけの pickle key を導いて暗号化する。vodozemac の pickle は pickle key から
    /// AES 鍵と IV を決定的に導くので、同じ pickle key を使い回すと、保存のたびに同じ IV で
    /// AES-CBC を使うことになる。そうなると、複数の版を手に入れた攻撃者は、先頭から同じ
    /// 暗号文ブロックを比べて、どこが変わったかを知ることができる。このため、
    /// 状態が変わっていなくても、呼ぶたびに出力は変わる。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn pickle(&self, storage_key: &[u8; 32]) -> String {
        let mut salt = [0u8; PICKLE_SALT_LEN];
        getrandom::fill(&mut salt).expect("OS かブラウザーの暗号用乱数を取れない");
        let pickle_key = derive_pickle_key(storage_key, &salt);
        let envelope = PickleEnvelope {
            version: PICKLE_VERSION,
            user_id: self.user_id.clone(),
            device_id: self.device_id.clone(),
            salt: base64_encode(salt),
            account: self.account.pickle().encrypt(&pickle_key),
            binding: self
                .account
                .sign(binding_message(&self.user_id, &self.device_id))
                .to_base64(),
        };
        serde_json::to_string(&envelope).expect("文字列と数値だけの構造体は必ず JSON にできる")
    }

    /// [`pickle`](Self::pickle) で作った文字列から、`user_id` の `device_id` の状態を戻す。
    ///
    /// - `storage_key` が違えば [`CryptoError::Pickle`] を返す。
    /// - 外側の形式が壊れている、中の `user_id` と `device_id` が [`new`](Self::new) の条件を
    ///   満たさない、または `user_id` と `device_id` が書き換えられていれば
    ///   [`CryptoError::InvalidPickle`] を返す。
    /// - pickle 自体は正しいが、持ち主が引数の `user_id` と `device_id` と違えば
    ///   [`CryptoError::PickleOwnerMismatch`] を返す。
    ///
    /// 最後の照合は、同じ `storage_key` を複数のユーザーや端末で使っている場合に、
    /// 別のユーザーや端末の pickle を丸ごと差し替えられても気付けるようにするため。
    /// `binding` はその pickle 自身の署名鍵で検証するので、丸ごと差し替えた pickle も通る。
    /// 呼び出し側は、今ログインしているユーザーと、そのユーザーで登録した端末の ID を渡す。
    pub fn from_pickle(
        pickle: &str,
        storage_key: &[u8; 32],
        user_id: &str,
        device_id: &str,
    ) -> Result<Self, CryptoError> {
        let envelope: PickleEnvelope = serde_json::from_str(pickle)
            .map_err(|e| CryptoError::InvalidPickle(format!("JSON として読めません: {e}")))?;
        if envelope.version != PICKLE_VERSION {
            return Err(CryptoError::InvalidPickle(format!(
                "対応していない版です: {}",
                envelope.version
            )));
        }
        validate_ids(&envelope.user_id, &envelope.device_id)
            .map_err(|e| CryptoError::InvalidPickle(e.to_string()))?;
        let salt: [u8; PICKLE_SALT_LEN] = base64_decode(&envelope.salt)
            .ok()
            .and_then(|salt| salt.try_into().ok())
            .ok_or_else(|| {
                CryptoError::InvalidPickle(format!(
                    "salt が {PICKLE_SALT_LEN} バイトの base64 ではありません"
                ))
            })?;
        let pickle_key = derive_pickle_key(storage_key, &salt);
        let account = Account::from_pickle(AccountPickle::from_encrypted(
            &envelope.account,
            &pickle_key,
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
        if envelope.user_id != user_id || envelope.device_id != device_id {
            return Err(CryptoError::PickleOwnerMismatch {
                expected_user_id: user_id.to_owned(),
                expected_device_id: device_id.to_owned(),
                user_id: envelope.user_id,
                device_id: envelope.device_id,
            });
        }
        Ok(Self {
            user_id: envelope.user_id,
            device_id: envelope.device_id,
            account,
        })
    }

    pub(crate) fn account(&self) -> &Account {
        &self.account
    }

    pub(crate) fn account_mut(&mut self) -> &mut Account {
        &mut self.account
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
    /// この保存の pickle key を導いた salt（32 バイト、パディングなしの base64）。
    salt: String,
    /// vodozemac の `AccountPickle::encrypt` の出力。
    account: String,
    /// [`binding_message`] への端末の署名鍵の署名。
    binding: String,
}

/// 保存鍵と salt から、1 回の保存だけで使う pickle key を導く。
fn derive_pickle_key(storage_key: &[u8; 32], salt: &[u8; PICKLE_SALT_LEN]) -> Zeroizing<[u8; 32]> {
    let mut pickle_key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(salt), storage_key)
        .expand(PICKLE_KEY_INFO, pickle_key.as_mut_slice())
        .expect("32 バイトは HKDF-SHA256 で導ける長さ（255 × 32 バイト以下）");
    pickle_key
}

/// `LP(PICKLE_BINDING_CONTEXT)‖LP(user_id)‖LP(device_id)`。LP は u32 ビッグエンディアンの長さと UTF-8 のバイト列。
///
/// 署名する対象を JSON ではなく長さ付きのバイト列にして、この署名がサーバーに送る
/// 署名付き JSON の署名と取り違えられないようにする。
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

/// `user_id` は UUID の表記、`device_id` は UUID v4 かを確かめる（[`DeviceAccount::new`] の条件）。
fn validate_ids(user_id: &str, device_id: &str) -> Result<(), CryptoError> {
    validate_uuid("user_id", user_id, false)?;
    validate_uuid("device_id", device_id, true)
}

/// 小文字でハイフン区切りの UUID（`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`）かを確かめる。
/// `v4` なら、version が 4 で variant が RFC 4122（`10xx`）であることも確かめる。
///
/// 署名する JSON に入るので、PostgreSQL の `uuid` 型が返す表記とそろえておく必要がある。
fn validate_uuid(field: &'static str, value: &str, v4: bool) -> Result<(), CryptoError> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
        // 14 文字目は version、19 文字目の上位 2 ビットは variant。
        && (!v4 || (bytes[14] == b'4' && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')));
    if valid {
        Ok(())
    } else {
        Err(CryptoError::InvalidId {
            field,
            value: value.to_owned(),
        })
    }
}
