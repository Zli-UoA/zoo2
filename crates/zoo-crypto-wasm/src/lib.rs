//! `zoo-crypto` の wasm-bindgen バインディング（Web の React 向け、#352）。
//!
//! 仕様は `docs/e2ee.md`、各操作の意味は `crates/zoo-crypto` の同じ名前の型と関数の説明にある。
//! このクレートは型を JS に出すだけで、暗号の処理は持たない。
//!
//! JS との受け渡しの決まり:
//!
//! - サーバーの JSON と同じ形の値（端末鍵、署名、`ciphertext` など）は、JS のオブジェクトで受け取り、
//!   返す。中では `JSON.stringify` と `JSON.parse` を通すので、整数は整数のまま扱う
//!   （Canonical JSON は小数を拒む）。項目の名前はサーバーの JSON と同じ snake_case。
//! - 状態を持つもの（端末アカウント、セッション、SSSS など）はクラスにする。メソッドは camelCase。
//! - 32 バイトの鍵、シード、PRF の出力は `Uint8Array` で受け取り、返す。
//! - `epoch` や時刻は JS の `number` で、`Number.MAX_SAFE_INTEGER` までの整数だけ受け付ける。
//! - 失敗は `name` が `ZooCryptoError` の `Error` を投げる。`code` に `zoo_crypto::CryptoError` の
//!   列挙子の名前（引数の誤りは `InvalidArgument`）を、受信時の検証で破棄したときは `rejection` に
//!   `zoo_crypto::Rejection` の列挙子の名前を入れる。
//!
//! 引数にクラスを値で渡すメソッド（`OlmSessions.push` など）は、渡したオブジェクトを使えなくする
//! （wasm-bindgen の所有権の移動）。説明にそのことを書いてある。

mod backup;
mod convert;
mod cross_signing;
mod device;
mod megolm;
mod secret_storage;
mod setup;

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zoo_crypto as core;

pub use backup::{
    BackupDecryptionKey, BackupEncryptionKey, BackupKeyList, BackupUpload, BackupUploadState,
};
pub use cross_signing::{CrossSigningKeyPair, CrossSigningKeys, KeySnapshot, TrustStore};
pub use device::{
    DeviceAccount, DeviceKeys, OlmDecrypted, OlmSession, OlmSessions, ReceivedRoomKey,
};
pub use megolm::{BackupSession, InboundSessionStore, OutboundSession, RoomKey};
pub use secret_storage::{RecoveryKey, SecretStorage, SsssKey, UnlockedSecretStorage};
pub use setup::{OwnKeys, OwnSecrets};

use convert::{IntoJs, ed25519, from_js, to_js};

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
/** サーバーの JSON と同じ形の値。 */
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
/** サーバーの JSON と同じ形のオブジェクト。 */
export type JsonObject = { [key: string]: JsonValue };
export type KeyUsage = "master" | "self_signing" | "user_signing";
export type AddOutcome = "added" | "replaced" | "kept";
export interface IdentityKeys { curve25519: string; ed25519: string }
export interface DeviceKeysJson {
  user_id: string;
  device_id: string;
  algorithms: string[];
  curve25519: string;
  ed25519: string;
}
export interface SenderTrust { self_signing_key: string; signature?: string | null }
export interface SessionRef { channel_id: string; sender_key: string; session_id: string }
export interface PickledOlmSession { session_id: string; their_identity_key: string; pickle: string }
export interface MessageContent { body: string; format: string; kind: string }
export interface Mentions { user_ids: string[]; channel: boolean; here: boolean }
/** Matrix の EncryptedFile から url を除いたもの（#364）。 */
export interface EncryptedFile {
  v: "v2";
  key: { kty: "oct"; key_ops: string[]; alg: "A256CTR"; k: string; ext: true };
  iv: string;
  hashes: { sha256: string; [name: string]: string };
}
/** Megolm で暗号化する平文に入れる、1 つの添付（#364）。 */
export interface AttachmentInfo {
  attachment_id: string;
  name: string;
  mimetype: string;
  size: number;
  width?: number;
  height?: number;
  sensitive?: boolean;
  file: EncryptedFile;
}
/** encryptAttachment の結果。ciphertext を S3 に上げ、file を AttachmentInfo.file に入れる。 */
export interface EncryptedAttachment { ciphertext: Uint8Array; file: EncryptedFile }
export interface OutgoingMessage {
  message_id: string;
  parent_id?: string | null;
  revision: number;
  content: MessageContent;
  mentions?: Mentions;
  attachments?: AttachmentInfo[];
}
export interface EncryptedMessage {
  channel_id: string;
  message_id: string;
  parent_id?: string | null;
  user_id: string;
  message_kind: string;
  epoch: number;
  revision: number;
  ciphertext: JsonObject;
}
export interface MessagePayload {
  type: string;
  channel_id: string;
  message_id: string;
  parent_id: string | null;
  epoch: number;
  revision: number;
  content: MessageContent;
  mentions: Mentions;
  attachments?: AttachmentInfo[];
}
export type RevisionStatus =
  | { status: "latest" }
  | { status: "stale"; latest: number }
  | { status: "conflict" };
export interface DecryptedMessage {
  payload: MessagePayload;
  message_index: number;
  origin: "olm" | "backup";
  sender_verified: boolean;
  revision: RevisionStatus;
}
export interface SignatureUpload {
  target_user_id: string;
  target_key_id: string;
  signer_key_id: string;
  signature: string;
}
export interface TrustRecord { master_key: string; confirmed_usk: string | null; pin_pending: boolean }
export interface NewTofuPin { target_user_id: string; master_key: string; pin: JsonObject }
export interface AcceptedMasterKey { pin: JsonObject; signature: SignatureUpload }
export type UserState =
  | {
      state: "trusted";
      basis: "own" | "user_signing_signature" | "shared_pin" | "record";
    }
  | { state: "no_cross_signing_keys" }
  | { state: "invalid_cross_signing_keys" }
  | {
      state: "needs_reverification";
      reason: "master_key_changed" | "not_confirmed" | "user_signing_key_changed";
    }
  | { state: "needs_unlock" }
  | { state: "pin_pending" }
  | { state: "blocked" };
export type DeviceExclusion =
  | "current_device"
  | "invalid_device_keys"
  | "invalid_self_signing_key"
  | "unsigned"
  | "bad_signature";
export type TrustWarning =
  | "own_identity_unknown"
  | "own_master_key_changed"
  | "untrusted_user_signing_key"
  | "user_signing_key_rollback"
  | "user_signing_key_changed";
export interface Distribution {
  devices: DeviceKeys[];
  users: Record<string, { state: UserState; excluded_devices: Record<string, DeviceExclusion> }>;
  new_pins: NewTofuPin[];
  warnings: TrustWarning[];
}
export interface SsssKeyDescription { algorithm: string; iv: string; mac: string }
export interface EncryptedSecret { key_id?: string | null; iv: string; ciphertext: string; mac: string }
export interface NamedSecret { name: string; secret: EncryptedSecret }
export interface KeySlot {
  id: string;
  key_id: string;
  kind: "passkey" | "recovery";
  credential_id?: string | null;
  prf_salt?: string | null;
  nonce: string;
  ciphertext: string;
  label?: string | null;
}
export interface KeysRequestOptions {
  expected_default_key_id?: string | null;
  expected_secrets_revision?: number | null;
}
export interface SessionData { ephemeral: string; ciphertext: string; mac: string }
export interface BackedUpSessionData {
  algorithm: string;
  channel_id: string;
  epoch: number;
  session_id: string;
  session_key: string;
  sender_key: string;
  sender_claimed_keys: { ed25519: string };
  sender_user_id: string;
  sender_device_id: string;
}
export interface BackupRow {
  channel_id: string;
  sender_key: string;
  session_id: string;
  first_message_index: number;
  forwarded_count: number;
  is_verified: boolean;
  session_data: SessionData;
}
export interface VerifiedBackupVersion { version: string; number: number; public_key: string }
export interface PreviousBackupSelection { keys: BackupKeyList; versions_without_key: string[] }
export interface ZooCryptoConstants {
  OLM_ALGORITHM: string;
  MEGOLM_ALGORITHM: string;
  SIGNED_CURVE25519: string;
  ROOM_KEY_TYPE: string;
  MESSAGE_TYPE: string;
  MAX_BODY_CHARS: number;
  MAX_SAFE_INTEGER: number;
  ROTATION_PERIOD_MESSAGES: number;
  ROTATION_PERIOD_MS: number;
  TOFU_PIN_TYPE: string;
  SSSS_ALGORITHM: string;
  SECRET_MEGOLM_BACKUP: string;
  SECRET_PREVIOUS_MEGOLM_BACKUP: string;
  REQUIRED_SECRETS: string[];
  RECOVERY_KEY_CHARS: number;
  BACKUP_ALGORITHM: string;
  MAX_UPLOAD_SESSIONS: number;
  ENCRYPTED_ATTACHMENT_FILE_NAME: string;
  ENCRYPTED_ATTACHMENT_CONTENT_TYPE: string;
  MAX_ATTACHMENTS: number;
}
"#;

/// コアの定数（アルゴリズム名、上限など）。
#[wasm_bindgen(unchecked_return_type = "ZooCryptoConstants")]
pub fn constants() -> Result<JsValue, JsValue> {
    to_js(&json!({
        "OLM_ALGORITHM": core::OLM_ALGORITHM,
        "MEGOLM_ALGORITHM": core::MEGOLM_ALGORITHM,
        "SIGNED_CURVE25519": core::SIGNED_CURVE25519,
        "ROOM_KEY_TYPE": core::ROOM_KEY_TYPE,
        "MESSAGE_TYPE": core::MESSAGE_TYPE,
        "MAX_BODY_CHARS": core::MAX_BODY_CHARS,
        "MAX_SAFE_INTEGER": core::MAX_SAFE_INTEGER,
        "ROTATION_PERIOD_MESSAGES": core::ROTATION_PERIOD_MESSAGES,
        "ROTATION_PERIOD_MS": core::ROTATION_PERIOD_MS,
        "TOFU_PIN_TYPE": core::TOFU_PIN_TYPE,
        "SSSS_ALGORITHM": core::SSSS_ALGORITHM,
        "SECRET_MEGOLM_BACKUP": core::SECRET_MEGOLM_BACKUP,
        "SECRET_PREVIOUS_MEGOLM_BACKUP": core::SECRET_PREVIOUS_MEGOLM_BACKUP,
        "REQUIRED_SECRETS": core::REQUIRED_SECRETS,
        "RECOVERY_KEY_CHARS": core::RECOVERY_KEY_CHARS,
        "BACKUP_ALGORITHM": core::BACKUP_ALGORITHM,
        "MAX_UPLOAD_SESSIONS": core::MAX_UPLOAD_SESSIONS,
        "ENCRYPTED_ATTACHMENT_FILE_NAME": core::ENCRYPTED_ATTACHMENT_FILE_NAME,
        "ENCRYPTED_ATTACHMENT_CONTENT_TYPE": core::ENCRYPTED_ATTACHMENT_CONTENT_TYPE,
        "MAX_ATTACHMENTS": core::MAX_ATTACHMENTS,
    }))
}

/// 添付の中身を、新しい鍵と IV の AES-256-CTR で暗号化する（#364）。
///
/// `ciphertext` を S3 にアップロードし、`file` を `AttachmentInfo.file` に入れて Megolm で暗号化して送る。
#[wasm_bindgen(js_name = encryptAttachment, unchecked_return_type = "EncryptedAttachment")]
pub fn encrypt_attachment(plaintext: &[u8]) -> Result<JsValue, JsValue> {
    let encrypted = core::encrypt_attachment(plaintext);
    let result = js_sys::Object::new();
    convert::set(
        &result,
        "ciphertext",
        &js_sys::Uint8Array::from(encrypted.ciphertext.as_slice()).into(),
    )?;
    convert::set(&result, "file", &to_js(&encrypted.file)?)?;
    Ok(result.into())
}

/// S3 から取得した暗号文のハッシュを確かめてから復号する（#364）。合わなければ投げる
/// （`code` は `AttachmentDecryption`、`file` の形式の誤りは `InvalidPayload`）。
#[wasm_bindgen(js_name = decryptAttachment)]
pub fn decrypt_attachment(
    #[wasm_bindgen(unchecked_param_type = "EncryptedFile")] file: JsValue,
    ciphertext: &[u8],
) -> Result<Vec<u8>, JsValue> {
    let file: core::EncryptedFile = from_js(&file, "file")?;
    core::decrypt_attachment(&file, ciphertext).js()
}

/// Matrix と同じ Canonical JSON の文字列にする。
#[wasm_bindgen(js_name = toCanonicalJson)]
pub fn to_canonical_json(
    #[wasm_bindgen(unchecked_param_type = "JsonValue")] value: JsValue,
) -> Result<String, JsValue> {
    let value: Value = from_js(&value, "value")?;
    core::to_canonical_json(&value).js()
}

/// 署名の対象の文字列（`signatures` と `unsigned` を除いた Canonical JSON）。
#[wasm_bindgen(js_name = signableJson)]
pub fn signable_json(
    #[wasm_bindgen(unchecked_param_type = "JsonObject")] value: JsValue,
) -> Result<String, JsValue> {
    let value: Value = from_js(&value, "value")?;
    core::signable_json(&value).js()
}

/// `value` の `signatures[userId][keyId]` を、Ed25519 の公開鍵 `publicKey`（base64）で検証する。
/// 合わなければ投げる。
#[wasm_bindgen(js_name = verifyJsonSignature)]
pub fn verify_json_signature(
    #[wasm_bindgen(unchecked_param_type = "JsonObject")] value: JsValue,
    user_id: &str,
    key_id: &str,
    public_key: &str,
) -> Result<(), JsValue> {
    let value: Value = from_js(&value, "value")?;
    core::verify_json_signature(&value, user_id, key_id, &ed25519(public_key)?).js()
}
