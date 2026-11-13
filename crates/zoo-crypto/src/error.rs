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

    /// クロス署名鍵の JSON が正しくない（`user_id` や `usage` が違う、`keys` が 1 つの Ed25519 鍵でない）、
    /// または鍵を用途の違う操作に使った（自己署名鍵で他のユーザーのマスター鍵に署名するなど）。
    #[error("クロス署名鍵が正しくありません: {0}")]
    InvalidCrossSigningKey(String),

    /// 配布先の判定の記録（[`TrustStore`](crate::TrustStore)）の保存形式が正しくない、または別のユーザーのもの。
    #[error("配布先の判定の記録が正しくありません: {0}")]
    InvalidTrustStore(String),

    /// 一度置き換わった自分のユーザー署名鍵に戻ろうとした（巻き戻し）。
    #[error("一度置き換わったユーザー署名鍵です: {0}")]
    UserSigningKeyRollback(String),

    /// 渡したユーザー署名鍵が、記録にある今のユーザー署名鍵と違う。
    #[error("今のユーザー署名鍵ではありません: {0}")]
    StaleUserSigningKey(String),

    /// リカバリーキーの形式が正しくない（Base58 でない、長さ、先頭の 2 バイト、パリティが違う）。
    #[error("リカバリーキーが正しくありません: {0}")]
    InvalidRecoveryKey(String),

    /// SSSS の応答、秘密、鍵スロットの形式が正しくない。
    #[error("Secret Storage の形式が正しくありません: {0}")]
    InvalidSecretStorage(String),

    /// 解錠に使える鍵スロットがない（その credential ID のスロットやリカバリーキーのスロットがない）。
    #[error("使える鍵スロットがありません")]
    NoKeySlot,

    /// 鍵スロットを開けない。PRF 出力やリカバリーキーが違う、スロットが改ざんされた、
    /// 別のユーザー、別の SSSS 鍵、別のスロットに付け替えられた、のどれか（区別できない）。
    #[error("鍵スロットを開けません")]
    KeySlotUnlock,

    /// SSSS 鍵が確認値と合わない。
    #[error("SSSS 鍵が確認値と一致しません")]
    SsssKeyMismatch,

    /// SSSS の秘密を復号できない（別の SSSS 鍵で暗号化された、MAC が合わない）。
    #[error("秘密を復号できません: {0}")]
    SecretDecryption(String),

    /// 鍵バックアップの版や保存した状態の形式が正しくない。
    #[error("鍵バックアップの形式が正しくありません: {0}")]
    InvalidBackup(String),

    /// 版の `auth_data.public_key` が、SSSS のバックアップ鍵と一致しない。
    #[error("鍵バックアップの版の公開鍵が SSSS のバックアップ鍵と一致しません")]
    BackupKeyMismatch,

    /// 今の版として、前に見たより小さい番号の版が返った。
    #[error("鍵バックアップの版が巻き戻りました（見た最大は {max_seen}、返ったのは {received}）")]
    BackupVersionRollback {
        /// 今の版として見た番号の最大。
        max_seen: u64,
        /// 返った番号。
        received: u64,
    },

    /// 鍵バックアップのセッションを復号できない、または行と平文が合わない。
    #[error("鍵バックアップのセッションを復号できません: {0}")]
    BackupDecryption(String),

    /// 受信時の検証に失敗した。呼び出し側は破棄する。
    #[error("受信時の検証に失敗しました: {0}")]
    Rejected(Rejection),

    /// サーバーにあるクロス署名鍵や鍵バックアップの今の版が、SSSS の鍵と違う。
    /// 失効が途中で止まっているか、SSSS を書き換えられた可能性がある（docs/e2ee.md の
    /// 「新しい端末での解錠」の【決定】）。サーバーに鍵や版が「ない」だけなら、このエラーにはならない。
    #[error("サーバーの鍵が SSSS の鍵と一致しません: {0}")]
    ServerKeyMismatch(String),
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
