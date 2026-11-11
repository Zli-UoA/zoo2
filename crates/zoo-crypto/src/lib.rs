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

mod canonical_json;
mod device;
mod error;
mod inbound;
mod megolm;
mod olm;
mod signing;

pub use canonical_json::to_canonical_json;
pub use device::{
    DeviceAccount, DeviceKeys, MEGOLM_ALGORITHM, OLM_ALGORITHM, SIGNED_CURVE25519,
    verify_device_keys,
};
pub use error::{CryptoError, Rejection};
pub use inbound::{
    AddOutcome, BackupSession, DecryptedMessage, EncryptedMessage, InboundSessionStore,
    RevisionStatus, SessionOrigin,
};
pub use megolm::{
    MAX_BODY_CHARS, MESSAGE_TYPE, Mentions, MessageContent, MessagePayload, OutboundSession,
    OutgoingMessage, ROTATION_PERIOD_MESSAGES, ROTATION_PERIOD_MS,
};
pub use olm::{
    ListedDevice, OlmDecrypted, OlmSession, ROOM_KEY_TYPE, ReceivedRoomKey, RoomKey, SenderTrust,
};
pub use signing::{signable_json, verify_json_signature};
pub use vodozemac::olm::IdentityKeys;
pub use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, PickleError};
