use thiserror::Error;

/// zoo-crypto の操作が失敗した理由。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// `user_id` や `device_id` が、小文字でハイフン区切りの UUID ではない。
    /// `device_id` は、UUID v4（version が 4 で variant が RFC 4122）でない場合もこれになる。
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

    /// vodozemac の pickle を復号できない。保存鍵が違う場合もこれになる。
    #[error("pickle を復号できません: {0}")]
    Pickle(#[from] vodozemac::PickleError),

    /// 保存した状態の外側の形式が正しくない（JSON でない、版が違う、ID が UUID でない、
    /// ID と鍵の対応が壊れている）。
    #[error("保存した状態の形式が正しくありません: {0}")]
    InvalidPickle(String),

    /// 保存した状態は正しいが、別のユーザーか端末のもの。
    #[error(
        "保存した状態は {user_id} の {device_id} のもので、{expected_user_id} の {expected_device_id} のものではありません"
    )]
    PickleOwnerMismatch {
        /// 期待したユーザー ID。
        expected_user_id: String,
        /// 期待した端末 ID。
        expected_device_id: String,
        /// 保存した状態のユーザー ID。
        user_id: String,
        /// 保存した状態の端末 ID。
        device_id: String,
    },

    /// 受け取った JSON や平文の形式が正しくない（項目がない、型が違う、方式が違うなど）。
    #[error("形式が正しくありません: {0}")]
    InvalidPayload(String),

    /// Olm のセッションを作れない、または暗号化や復号に失敗した。
    #[error("Olm の処理に失敗しました: {0}")]
    Olm(String),

    /// Megolm のメッセージを復号できない（署名や MAC が一致しないなど）。
    #[error("Megolm のメッセージを復号できません: {0}")]
    Megolm(String),

    /// 復号に使うセッションの鍵がまだ届いていない。`message_index` があれば、
    /// セッションはあるが、そのインデックスより後からしか読めない。
    #[error("{session_id} の鍵がまだ届いていません（インデックス {message_index:?}）")]
    MissingRoomKey {
        /// Megolm セッション ID。
        session_id: String,
        /// 読めなかったメッセージのインデックス。
        message_index: Option<u32>,
    },

    /// 受信時の検証に失敗した。呼び出し側は破棄する。
    #[error("受信時の検証に失敗しました: {0}")]
    Rejected(Rejection),
}

/// 受信時の検証で破棄する理由（docs/e2ee.md の「受信時の検証」）。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum Rejection {
    /// `sender_device_keys` の `user_id` か `device_id` が平文の送信者と違う、または自己署名が壊れている。
    #[error("sender_device_keys が正しくありません: {0}")]
    InvalidSenderDeviceKeys(String),
    /// `sender_device_keys` の鍵が、Shape か `keys/query` の端末鍵と違う。
    #[error("sender_device_keys が登録済みの端末鍵と違います")]
    SenderDeviceKeysConflict,
    /// `sender_device_keys` がなく、Shape にも `keys/query` にも送信端末がない。
    #[error("送信端末の端末鍵がありません")]
    UnknownSenderDevice,
    /// Olm セッションの相手の識別鍵が、送信端末の識別鍵と違う。
    #[error("Olm セッションの相手が送信端末ではありません")]
    SenderKeyMismatch,
    /// 平文の `keys.ed25519` が、送信端末の署名鍵と違う。
    #[error("keys.ed25519 が送信端末の署名鍵と違います")]
    SenderSigningKeyMismatch,
    /// `recipient` か `recipient_keys.ed25519` が自分ではない。
    #[error("宛先がこの端末ではありません")]
    RecipientMismatch,
    /// ペイロードの `channel_id` が行のチャンネルと違う（別のチャンネルへの移植）。
    #[error("channel_id が違います")]
    ChannelMismatch,
    /// ペイロードの `message_id` が行の `id` と違う（別の行への移植）。
    #[error("message_id が違います")]
    MessageIdMismatch,
    /// ペイロードの `parent_id` が行の `parent_id` と違う（別のスレッドへの移植）。
    #[error("parent_id が違います")]
    ParentMismatch,
    /// ペイロードの `content.kind` が行の `message_kind` と違う。
    #[error("content.kind が違います")]
    KindMismatch,
    /// 行の `user_id` がセッションの送信ユーザーと違う（なりすまし）。
    #[error("送信者がセッションの送信ユーザーと違います")]
    SenderMismatch,
    /// `ciphertext.device_id` がセッションの送信端末と違う（なりすまし）。
    #[error("送信端末がセッションの送信端末と違います")]
    DeviceMismatch,
    /// ペイロード、行、セッションの `epoch` が一致しない。
    #[error("epoch が一致しません")]
    EpochMismatch,
    /// 同じ `(session_id, message_index)` を、別の `message_id` で先に復号している（リプレイ）。
    #[error("リプレイです（最初のメッセージは {first_message_id}）")]
    Replay {
        /// 最初に復号した `message_id`。
        first_message_id: String,
    },
    /// ペイロードの `revision` が行の `revision` と違う。
    #[error("revision が違います")]
    RevisionMismatch,
}
