//! zoo2 の E2EE の暗号コア。仕様は `docs/e2ee.md` にある。
//!
//! vodozemac 0.11 の上に、全クライアント（React、Compose、TUI）で共有する処理を置く。
//! このクレートは I/O を持たない。サーバーの JSON を受け取り、送る JSON や保存する状態を返す。
//! 保存先と通信は、各プラットフォームのバインディングと呼び出し側が実装する。
//!
//! 今あるもの（#348）:
//!
//! - [`DeviceAccount`]：端末の識別鍵、ワンタイム鍵、フォールバック鍵と、署名付きの公開鍵の書き出し
//! - [`DeviceAccount::pickle`] と [`DeviceAccount::from_pickle`]：呼び出し側の 32 バイトの保存鍵から保存ごとに導いた鍵で暗号化した保存と、持ち主を照合する復元
//! - [`to_canonical_json`]、[`verify_json_signature`]、[`verify_device_keys`]：Matrix と同じ Canonical JSON と署名の検証
//!
//! #349 で足したもの:
//!
//! - [`DeviceAccount::create_outbound_olm_session`]、[`DeviceAccount::encrypt_room_key`]、
//!   [`DeviceAccount::decrypt_to_device`]：Olm のセッションと、Megolm セッション鍵の配布（`zoo.room_key`）
//! - [`OlmDecrypted::verify_room_key`]：受け取った `zoo.room_key` の検証（受信時の検証の手順 1〜4）
//! - [`OutboundSession`]：チャンネルごとの送信用セッションと、作り直しの判断
//! - [`InboundSessionStore`]：受信用セッションの保存の規則と、復号したメッセージの照合（リプレイの検出を含む）
//!
//! #350 で足したもの:
//!
//! - [`CrossSigningKeys`]、[`CrossSigningKeyPair`]：マスター鍵、自己署名鍵、ユーザー署名鍵の生成と、
//!   端末、他のユーザーのマスター鍵、TOFU のピンへの署名
//! - [`verify_cross_signing_key`]：サーバーから受け取ったクロス署名鍵の検証
//! - [`TrustStore`]：TOFU の記録（`confirmed_usk` を含む）、自分のユーザー署名鍵の履歴と、
//!   [`TrustStore::evaluate`] による鍵の配布先の判定（規則 1〜6）
//!
//! #351 で足したもの:
//!
//! - [`SsssKey`]：SSSS 鍵、確認値、秘密の暗号化（`m.secret_storage.v1.aes-hmac-sha2`）
//! - [`KeySlot`]：パスキー（PRF）とリカバリーキーの鍵スロット（AES-256-GCM）
//! - [`SecretStorage`]、[`UnlockedSecretStorage`]：`GET /api/e2ee/secret_storage` の解錠と秘密の取り出し
//! - [`RecoveryKey`]：リカバリーキーの生成、表示、読み取り（Base58）
//! - [`BackupDecryptionKey`]、[`BackupEncryptionKey`]：鍵バックアップ（`zoo.megolm_backup.v1.curve25519-aes-sha2`）
//! - [`verify_backup_version`]、[`BackupUploadState`]：今の版の検証と、アップロード待ちの状態機械
//! - [`select_previous_backup_keys`]、[`previous_versions_to_migrate`]：失効のときの古い版の鍵の退避と移行
//!
//! #359 で足したもの:
//!
//! - [`OlmSession::pickle`]、[`InboundSessionStore::pickle_session`]、[`TrustStore::pickle`] と、
//!   それぞれの戻し方：暗号ストアに置く保存形式（保存鍵から保存ごとに導いた鍵の AES-256-GCM で、
//!   持ち主の端末に結び付ける）

mod backup;
mod canonical_json;
mod cross_signing;
mod device;
mod error;
mod inbound;
mod megolm;
mod olm;
mod recovery_key;
mod secret_storage;
mod setup;
mod signing;
mod store;
mod trust;

pub use backup::{
    BACKUP_ALGORITHM, BackedUpSessionData, BackupDecryptionKey, BackupEncryptionKey, BackupRow,
    BackupUpload, BackupUploadState, BackupVersionInfo, MAX_UPLOAD_SESSIONS,
    PreviousBackupSelection, SenderClaimedKeys, SessionData, VerifiedBackupVersion, VersionChange,
    previous_versions_to_migrate, select_previous_backup_keys, verify_backup_version,
};
pub use canonical_json::to_canonical_json;
pub use cross_signing::{
    CrossSigningKeyPair, CrossSigningKeys, KeySignature, KeyUsage, SignatureUpload, TOFU_PIN_TYPE,
    cross_signing_key_id, verify_cross_signing_key,
};
pub use device::{
    DeviceAccount, DeviceKeys, MEGOLM_ALGORITHM, OLM_ALGORITHM, SIGNED_CURVE25519,
    verify_device_keys,
};
pub use error::{CryptoError, Rejection};
pub use inbound::{
    AddOutcome, BackupSession, DecryptedMessage, EncryptedMessage, InboundSessionStore,
    RevisionStatus, SessionOrigin, SessionRef,
};
pub use megolm::{
    MAX_BODY_CHARS, MAX_SAFE_INTEGER, MESSAGE_TYPE, Mentions, MessageContent, MessagePayload,
    OutboundSession, OutgoingMessage, ROTATION_PERIOD_MESSAGES, ROTATION_PERIOD_MS,
};
pub use olm::{
    ListedDevice, OlmDecrypted, OlmSession, ROOM_KEY_TYPE, ReceivedRoomKey, RoomKey, SenderTrust,
};
pub use recovery_key::{RECOVERY_KEY_CHARS, RecoveryKey};
pub use secret_storage::{
    EncryptedSecret, KeySlot, REQUIRED_SECRETS, SECRET_MEGOLM_BACKUP,
    SECRET_PREVIOUS_MEGOLM_BACKUP, SSSS_ALGORITHM, SecretStorage, SecretStorageKeysRequest,
    SlotKind, SlotSecret, SsssKey, SsssKeyDescription, UnlockedSecretStorage, new_prf_salt,
};
pub use setup::{
    OwnKeys, OwnSecrets, ResumePlan, ServerKeyState, StartupCheck, plan_resume, startup_check,
};
pub use signing::{signable_json, verify_json_signature};
pub use trust::{
    AcceptedMasterKey, DeviceExclusion, Distribution, KeySnapshot, NewTofuPin,
    ReverificationReason, TrustBasis, TrustRecord, TrustStore, TrustWarning, UserState, UserTrust,
};
pub use vodozemac::olm::IdentityKeys;
pub use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, PickleError};
