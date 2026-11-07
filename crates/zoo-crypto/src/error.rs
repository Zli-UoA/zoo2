use thiserror::Error;

/// zoo-crypto の操作が失敗した理由。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// `user_id` や `device_id` が、小文字でハイフン区切りの UUID ではない。
    #[error("{field} が UUID の形式ではありません: {value:?}")]
    InvalidId {
        /// 項目の名前（`user_id` など）。
        field: &'static str,
        /// 受け取った値。
        value: String,
    },

    /// JSON を Canonical JSON にできない（小数、範囲外の整数、オブジェクト以外への署名など）。
    #[error("Canonical JSON にできません: {0}")]
    CanonicalJson(String),

    /// 鍵の形式が正しくない（base64 でない、長さが違うなど）。
    #[error("鍵の形式が正しくありません: {0}")]
    Key(#[from] vodozemac::KeyError),

    /// 端末鍵の JSON に必要な項目がない、または期待した値と違う。
    #[error("端末鍵が正しくありません: {0}")]
    InvalidDeviceKeys(String),

    /// 検証しようとした署名が `signatures` にない。
    #[error("{user_id} の {key_id} の署名がありません")]
    MissingSignature {
        /// 署名した鍵の持ち主。
        user_id: String,
        /// 鍵 ID（`ed25519:<device_id>` など）。
        key_id: String,
    },

    /// 署名の形式が正しくない、または署名が一致しない。
    #[error("署名を検証できません: {0}")]
    Signature(#[from] vodozemac::SignatureError),

    /// vodozemac の pickle を復号できない。pickle key が違う場合もこれになる。
    #[error("pickle を復号できません: {0}")]
    Pickle(#[from] vodozemac::PickleError),

    /// 保存した状態の外側の形式が正しくない（JSON でない、版が違う、ID と鍵の対応が壊れている）。
    #[error("保存した状態の形式が正しくありません: {0}")]
    InvalidPickle(String),
}
