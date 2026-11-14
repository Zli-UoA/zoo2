use zoo_crypto::{CryptoError, Rejection};

/// zoo-crypto の [`CryptoError`] を Kotlin の例外（`ZooCryptoException`）にしたもの。
///
/// 種類はコアの `CryptoError` の variant と 1 対 1 に対応する。`reason` はコアのメッセージ。
/// 呼び出し側が分岐に使う値（足りない鍵のセッション ID、巻き戻った版の番号、破棄の理由）は
/// 項目にも入れる。
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ZooCryptoError {
    /// `user_id` や `device_id` などが UUID の形式ではない。
    #[error("{reason}")]
    InvalidId { reason: String },
    /// Canonical JSON にできない。
    #[error("{reason}")]
    CanonicalJson { reason: String },
    /// 鍵の形式が正しくない。
    #[error("{reason}")]
    Key { reason: String },
    /// 端末鍵が正しくない。
    #[error("{reason}")]
    InvalidDeviceKeys { reason: String },
    /// 検証しようとした署名がない。
    #[error("{reason}")]
    MissingSignature { reason: String },
    /// 署名が正しくない。
    #[error("{reason}")]
    Signature { reason: String },
    /// pickle を復号できない（保存鍵が違うときもこれ）。
    #[error("{reason}")]
    Pickle { reason: String },
    /// 保存した状態の形式が正しくない。
    #[error("{reason}")]
    InvalidPickle { reason: String },
    /// 保存した状態が別のユーザーか端末のもの。
    #[error("{reason}")]
    PickleOwnerMismatch { reason: String },
    /// 受け取った JSON、平文、引数の形式が正しくない。
    #[error("{reason}")]
    InvalidPayload { reason: String },
    /// Olm の処理に失敗した。
    #[error("{reason}")]
    Olm { reason: String },
    /// Megolm のメッセージを復号できない。
    #[error("{reason}")]
    Megolm { reason: String },
    /// 復号に使うセッションの鍵がまだ届いていない。
    #[error("{reason}")]
    MissingRoomKey {
        reason: String,
        session_id: String,
        message_index: Option<u32>,
    },
    /// クロス署名鍵が正しくない。
    #[error("{reason}")]
    InvalidCrossSigningKey { reason: String },
    /// 配布先の判定の記録が正しくない。
    #[error("{reason}")]
    InvalidTrustStore { reason: String },
    /// 置き換わった自分のユーザー署名鍵に戻ろうとした。
    #[error("{reason}")]
    UserSigningKeyRollback { reason: String },
    /// 今のユーザー署名鍵ではない。
    #[error("{reason}")]
    StaleUserSigningKey { reason: String },
    /// リカバリーキーが正しくない。
    #[error("{reason}")]
    InvalidRecoveryKey { reason: String },
    /// SSSS の形式が正しくない。
    #[error("{reason}")]
    InvalidSecretStorage { reason: String },
    /// 解錠に使える鍵スロットがない。
    #[error("{reason}")]
    NoKeySlot { reason: String },
    /// 鍵スロットを開けない。
    #[error("{reason}")]
    KeySlotUnlock { reason: String },
    /// SSSS 鍵が確認値と一致しない。
    #[error("{reason}")]
    SsssKeyMismatch { reason: String },
    /// SSSS の秘密を復号できない。
    #[error("{reason}")]
    SecretDecryption { reason: String },
    /// 鍵バックアップの形式が正しくない。
    #[error("{reason}")]
    InvalidBackup { reason: String },
    /// 鍵バックアップの版の公開鍵が SSSS のバックアップ鍵と一致しない。
    #[error("{reason}")]
    BackupKeyMismatch { reason: String },
    /// サーバーのクロス署名鍵や鍵バックアップの今の版が、SSSS の鍵と違う。
    #[error("{reason}")]
    ServerKeyMismatch { reason: String },
    /// 暗号化した添付を復号できない（暗号文のハッシュが合わない。#364）。
    #[error("{reason}")]
    AttachmentDecryption { reason: String },
    /// SSSS を保存する前に止まった 2〜5 の失効は続けられない。最初からやり直す（#362）。
    #[error("{reason}")]
    RevocationRestart { reason: String },
    /// 絵文字 SAS の検証（#368）を、今の状態では行えない。
    #[error("{reason}")]
    Verification { reason: String },
    /// 鍵バックアップの版が巻き戻った。
    #[error("{reason}")]
    BackupVersionRollback {
        reason: String,
        max_seen: u64,
        received: u64,
    },
    /// 鍵バックアップのセッションを復号できない。
    #[error("{reason}")]
    BackupDecryption { reason: String },
    /// 受信時の検証に失敗した。呼び出し側は破棄する。
    #[error("{reason}")]
    Rejected { reason: String, kind: RejectionKind },
    /// オブジェクトを使い終わったあとに、もう一度使った（例：保存済みの `ReceivedRoomKey`）。
    #[error("{reason}")]
    AlreadyConsumed { reason: String },
    /// このバインディングが知らない新しい種類のエラー（コアの `CryptoError` は `non_exhaustive`）。
    #[error("{reason}")]
    Other { reason: String },
}

/// 受信時の検証で破棄する理由（[`Rejection`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RejectionKind {
    InvalidSenderDeviceKeys,
    SenderDeviceKeysConflict,
    UnknownSenderDevice,
    SenderKeyMismatch,
    SenderSigningKeyMismatch,
    RecipientMismatch,
    ChannelMismatch,
    MessageIdMismatch,
    ParentMismatch,
    KindMismatch,
    SenderMismatch,
    DeviceMismatch,
    EpochMismatch,
    Replay,
    RevisionMismatch,
    /// このバインディングが知らない新しい理由。
    Other,
}

impl From<&Rejection> for RejectionKind {
    fn from(rejection: &Rejection) -> Self {
        match rejection {
            Rejection::InvalidSenderDeviceKeys(_) => Self::InvalidSenderDeviceKeys,
            Rejection::SenderDeviceKeysConflict => Self::SenderDeviceKeysConflict,
            Rejection::UnknownSenderDevice => Self::UnknownSenderDevice,
            Rejection::SenderKeyMismatch => Self::SenderKeyMismatch,
            Rejection::SenderSigningKeyMismatch => Self::SenderSigningKeyMismatch,
            Rejection::RecipientMismatch => Self::RecipientMismatch,
            Rejection::ChannelMismatch => Self::ChannelMismatch,
            Rejection::MessageIdMismatch => Self::MessageIdMismatch,
            Rejection::ParentMismatch => Self::ParentMismatch,
            Rejection::KindMismatch => Self::KindMismatch,
            Rejection::SenderMismatch => Self::SenderMismatch,
            Rejection::DeviceMismatch => Self::DeviceMismatch,
            Rejection::EpochMismatch => Self::EpochMismatch,
            Rejection::Replay { .. } => Self::Replay,
            Rejection::RevisionMismatch => Self::RevisionMismatch,
            _ => Self::Other,
        }
    }
}

impl From<CryptoError> for ZooCryptoError {
    fn from(error: CryptoError) -> Self {
        let reason = error.to_string();
        match error {
            CryptoError::InvalidId { .. } => Self::InvalidId { reason },
            CryptoError::CanonicalJson(_) => Self::CanonicalJson { reason },
            CryptoError::Key(_) => Self::Key { reason },
            CryptoError::InvalidDeviceKeys(_) => Self::InvalidDeviceKeys { reason },
            CryptoError::MissingSignature { .. } => Self::MissingSignature { reason },
            CryptoError::Signature(_) => Self::Signature { reason },
            CryptoError::Pickle(_) => Self::Pickle { reason },
            CryptoError::InvalidPickle(_) => Self::InvalidPickle { reason },
            CryptoError::PickleOwnerMismatch { .. } => Self::PickleOwnerMismatch { reason },
            CryptoError::InvalidPayload(_) => Self::InvalidPayload { reason },
            CryptoError::Olm(_) => Self::Olm { reason },
            CryptoError::Megolm(_) => Self::Megolm { reason },
            CryptoError::MissingRoomKey {
                session_id,
                message_index,
            } => Self::MissingRoomKey {
                reason,
                session_id,
                message_index,
            },
            CryptoError::InvalidCrossSigningKey(_) => Self::InvalidCrossSigningKey { reason },
            CryptoError::InvalidTrustStore(_) => Self::InvalidTrustStore { reason },
            CryptoError::UserSigningKeyRollback(_) => Self::UserSigningKeyRollback { reason },
            CryptoError::StaleUserSigningKey(_) => Self::StaleUserSigningKey { reason },
            CryptoError::InvalidRecoveryKey(_) => Self::InvalidRecoveryKey { reason },
            CryptoError::InvalidSecretStorage(_) => Self::InvalidSecretStorage { reason },
            CryptoError::NoKeySlot => Self::NoKeySlot { reason },
            CryptoError::KeySlotUnlock => Self::KeySlotUnlock { reason },
            CryptoError::SsssKeyMismatch => Self::SsssKeyMismatch { reason },
            CryptoError::SecretDecryption(_) => Self::SecretDecryption { reason },
            CryptoError::InvalidBackup(_) => Self::InvalidBackup { reason },
            CryptoError::BackupKeyMismatch => Self::BackupKeyMismatch { reason },
            CryptoError::ServerKeyMismatch(_) => Self::ServerKeyMismatch { reason },
            CryptoError::AttachmentDecryption(_) => Self::AttachmentDecryption { reason },
            CryptoError::RevocationRestart => Self::RevocationRestart { reason },
            CryptoError::Verification(_) => Self::Verification { reason },
            CryptoError::BackupVersionRollback { max_seen, received } => {
                Self::BackupVersionRollback {
                    reason,
                    max_seen,
                    received,
                }
            }
            CryptoError::BackupDecryption(_) => Self::BackupDecryption { reason },
            CryptoError::Rejected(rejection) => Self::Rejected {
                kind: RejectionKind::from(&rejection),
                reason,
            },
            _ => Self::Other { reason },
        }
    }
}
