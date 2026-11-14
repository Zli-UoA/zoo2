//! 暗号化した添付（#364）。
//!
//! 形式は Matrix の [`EncryptedFile`](https://spec.matrix.org/latest/client-server-api/#extensions-to-mroommessage-msgtypes)
//! （`v2`、AES-256-CTR、JWK の鍵、SHA-256 のハッシュ）と同じで、`url` の代わりに
//! `attachments.id`（[`AttachmentInfo::attachment_id`]）で場所を示す。
//! 仕様は docs/e2ee.md の「暗号化した添付」。
//!
//! 鍵、IV、ハッシュ、ファイル名、MIME、画像の大きさは、Megolm で暗号化する平文
//! （[`MessagePayload::attachments`](crate::MessagePayload::attachments)）にだけ入れ、
//! サーバーには暗号文と、決まったファイル名と Content-Type だけを渡す。

use std::collections::BTreeMap;

use ctr::cipher::{KeyIvInit as _, StreamCipher as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use vodozemac::{base64_decode, base64_encode};
use zeroize::Zeroizing;

use crate::CryptoError;
use crate::device::{random_bytes, validate_uuid};
use crate::megolm::check_safe_integer;

type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

/// `EncryptedFile.v`。
pub const ATTACHMENT_VERSION: &str = "v2";
/// JWK の `alg`。
pub const ATTACHMENT_KEY_ALGORITHM: &str = "A256CTR";
/// 暗号化チャンネルの `POST /api/attachments/presign` に渡すファイル名。本当の名前は平文に入れる。
pub const ENCRYPTED_ATTACHMENT_FILE_NAME: &str = "encrypted.bin";
/// 暗号化チャンネルの `POST /api/attachments/presign` と S3 へのアップロードに使う Content-Type。
pub const ENCRYPTED_ATTACHMENT_CONTENT_TYPE: &str = "application/octet-stream";
/// 1 つのメッセージに付けられる添付の数の上限（平文チャンネルの `attachment_ids` と同じ）。
pub const MAX_ATTACHMENTS: usize = 10;
/// ファイル名の上限（文字数。`POST /api/attachments/presign` と同じ）。
pub const MAX_ATTACHMENT_NAME_CHARS: usize = 255;
/// MIME の上限（文字数。`POST /api/attachments/presign` と同じ）。
pub const MAX_ATTACHMENT_MIMETYPE_CHARS: usize = 128;

/// AES-256-CTR の鍵（JWK、[RFC 7517](https://www.rfc-editor.org/rfc/rfc7517)）。Matrix と同じ形。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentKey {
    /// `oct`。
    pub kty: String,
    /// `encrypt` と `decrypt` を含む。
    pub key_ops: Vec<String>,
    /// `A256CTR`。
    pub alg: String,
    /// 32 バイトの鍵（パディングなしの base64url）。
    pub k: String,
    /// `true`。
    pub ext: bool,
}

impl std::fmt::Debug for AttachmentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentKey")
            .field("kty", &self.kty)
            .field("key_ops", &self.key_ops)
            .field("alg", &self.alg)
            .finish_non_exhaustive()
    }
}

/// 暗号化したファイルの鍵、IV、ハッシュ（Matrix の `EncryptedFile` から `url` を除いたもの）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedFile {
    /// `v2`。
    pub v: String,
    /// 鍵。
    pub key: AttachmentKey,
    /// 16 バイトの IV（パディングなしの base64）。前半 8 バイトが乱数、後半 8 バイト（カウンター）が 0。
    pub iv: String,
    /// 暗号文のハッシュ。`sha256`（パディングなしの base64）を必ず含む。
    pub hashes: BTreeMap<String, String>,
}

/// Megolm で暗号化する平文に入れる、1 つの添付。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// `attachments.id`（`POST /api/attachments/presign` の `attachment_id`）。
    pub attachment_id: String,
    /// 元のファイル名。
    pub name: String,
    /// 元の MIME。
    pub mimetype: String,
    /// 元の大きさ（バイト）。AES-CTR なので暗号文と同じ。
    pub size: u64,
    /// 画像の幅（ピクセル）。画像でなければ省く。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// 画像の高さ（ピクセル）。画像でなければ省く。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// センシティブ（ぼかして表示する）か。サーバーの `is_sensitive` の代わり。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// 鍵、IV、ハッシュ。
    pub file: EncryptedFile,
}

impl AttachmentInfo {
    /// 形式を確かめる。
    pub(crate) fn validate(&self) -> Result<(), CryptoError> {
        validate_uuid("attachment_id", &self.attachment_id, false)?;
        let name_chars = self.name.chars().count();
        if self.name.trim().is_empty()
            || name_chars > MAX_ATTACHMENT_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid(format!(
                "attachments.name は 1〜{MAX_ATTACHMENT_NAME_CHARS} 文字で、制御文字を含まない文字列です"
            )));
        }
        if self.mimetype.is_empty()
            || self.mimetype.chars().count() > MAX_ATTACHMENT_MIMETYPE_CHARS
            || self.mimetype.chars().any(|c| c.is_control())
        {
            return Err(invalid(format!(
                "attachments.mimetype は 1〜{MAX_ATTACHMENT_MIMETYPE_CHARS} 文字で、制御文字を含まない文字列です"
            )));
        }
        check_safe_integer("attachments.size", self.size)?;
        if self.width.is_some() != self.height.is_some() {
            return Err(invalid(
                "attachments.width と height は両方あるか、両方ないかのどちらかです".into(),
            ));
        }
        self.file.parse().map(|_| ())
    }
}

/// [`encrypt_attachment`] の結果。
#[derive(Clone)]
pub struct EncryptedAttachment {
    /// S3 にアップロードする暗号文。平文と同じ長さ。
    pub ciphertext: Vec<u8>,
    /// 平文の [`AttachmentInfo::file`] に入れる値。
    pub file: EncryptedFile,
}

impl std::fmt::Debug for EncryptedAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedAttachment")
            .field("ciphertext_len", &self.ciphertext.len())
            .field("file", &self.file)
            .finish()
    }
}

/// `plaintext` を、新しい鍵と IV の AES-256-CTR で暗号化する。
///
/// 鍵は添付ごとに作り直し、使い回さない。ハッシュは暗号文の SHA-256。
///
/// # Panics
///
/// OS やブラウザーの暗号用乱数を取れないとき。
pub fn encrypt_attachment(plaintext: &[u8]) -> EncryptedAttachment {
    let key = Zeroizing::new(random_bytes::<32>());
    let mut iv = [0u8; 16];
    iv[..8].copy_from_slice(&random_bytes::<8>());
    encrypt_with(plaintext, &key, iv)
}

fn encrypt_with(plaintext: &[u8], key: &[u8; 32], iv: [u8; 16]) -> EncryptedAttachment {
    let mut ciphertext = plaintext.to_vec();
    Aes256Ctr::new(key.into(), &iv.into()).apply_keystream(&mut ciphertext);
    let hash = Sha256::digest(&ciphertext);
    EncryptedAttachment {
        file: EncryptedFile {
            v: ATTACHMENT_VERSION.to_owned(),
            key: AttachmentKey {
                kty: "oct".to_owned(),
                key_ops: vec!["encrypt".to_owned(), "decrypt".to_owned()],
                alg: ATTACHMENT_KEY_ALGORITHM.to_owned(),
                k: base64url_encode(key),
                ext: true,
            },
            iv: base64_encode(iv),
            hashes: BTreeMap::from([("sha256".to_owned(), base64_encode(hash))]),
        },
        ciphertext,
    }
}

/// S3 から取得した `ciphertext` の SHA-256 を `file.hashes.sha256` と比べてから復号する。
///
/// ハッシュが合わない、または `file` の形式が正しくないときは
/// [`CryptoError::AttachmentDecryption`] か [`CryptoError::InvalidPayload`] を返し、平文を返さない。
pub fn decrypt_attachment(file: &EncryptedFile, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let parsed = file.parse()?;
    let actual = Sha256::digest(ciphertext);
    if !constant_time_eq(actual.as_slice(), &parsed.sha256) {
        return Err(CryptoError::AttachmentDecryption(
            "暗号文のハッシュが一致しません".into(),
        ));
    }
    let mut plaintext = ciphertext.to_vec();
    Aes256Ctr::new((&*parsed.key).into(), &parsed.iv.into()).apply_keystream(&mut plaintext);
    Ok(plaintext)
}

struct ParsedFile {
    key: Zeroizing<[u8; 32]>,
    iv: [u8; 16],
    sha256: [u8; 32],
}

impl EncryptedFile {
    fn parse(&self) -> Result<ParsedFile, CryptoError> {
        if self.v != ATTACHMENT_VERSION {
            return Err(invalid(format!(
                "file.v が {ATTACHMENT_VERSION} ではありません"
            )));
        }
        let key = &self.key;
        if key.kty != "oct"
            || key.alg != ATTACHMENT_KEY_ALGORITHM
            || !key.ext
            || !key.key_ops.iter().any(|op| op == "encrypt")
            || !key.key_ops.iter().any(|op| op == "decrypt")
        {
            return Err(invalid(
                "file.key は kty が oct、alg が A256CTR、ext が true、key_ops に encrypt と decrypt を含む JWK です"
                    .into(),
            ));
        }
        let raw_key = Zeroizing::new(
            base64url_decode(&key.k)
                .ok_or_else(|| invalid("file.key.k が base64url ではありません".into()))?,
        );
        let key: [u8; 32] = raw_key
            .as_slice()
            .try_into()
            .map_err(|_| invalid("file.key.k が 32 バイトではありません".into()))?;
        let iv: [u8; 16] = decode_fixed(&self.iv, "file.iv")?;
        let sha256 = self
            .hashes
            .get("sha256")
            .ok_or_else(|| invalid("file.hashes.sha256 がありません".into()))?;
        let sha256: [u8; 32] = decode_fixed(sha256, "file.hashes.sha256")?;
        Ok(ParsedFile {
            key: Zeroizing::new(key),
            iv,
            sha256,
        })
    }
}

fn invalid(message: String) -> CryptoError {
    CryptoError::InvalidPayload(message)
}

/// パディングあり、なしのどちらの base64 も受け付ける（Matrix のクライアントにはパディングを付けるものがある）。
fn decode_fixed<const N: usize>(value: &str, field: &str) -> Result<[u8; N], CryptoError> {
    base64_decode(value.trim_end_matches('='))
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| invalid(format!("{field} が {N} バイトの base64 ではありません")))
}

fn base64url_encode(bytes: &[u8]) -> String {
    base64_encode(bytes).replace('+', "-").replace('/', "_")
}

fn base64url_decode(value: &str) -> Option<Vec<u8>> {
    if value.contains(['+', '/']) {
        return None;
    }
    base64_decode(
        value
            .trim_end_matches('=')
            .replace('-', "+")
            .replace('_', "/"),
    )
    .ok()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAINTEXT: &[u8] = b"zoo2 attachment plaintext \x00\x01\x02 with binary bytes";

    #[test]
    fn round_trip_and_ciphertext_differs_from_plaintext() {
        let encrypted = encrypt_attachment(PLAINTEXT);
        assert_eq!(encrypted.ciphertext.len(), PLAINTEXT.len());
        assert_ne!(encrypted.ciphertext, PLAINTEXT);
        assert!(
            !encrypted
                .ciphertext
                .windows(8)
                .any(|w| PLAINTEXT.windows(8).any(|p| p == w)),
            "暗号文に平文の 8 バイトの並びが残っている"
        );
        let decrypted = decrypt_attachment(&encrypted.file, &encrypted.ciphertext).unwrap();
        assert_eq!(decrypted, PLAINTEXT);
    }

    #[test]
    fn empty_file_round_trips() {
        let encrypted = encrypt_attachment(b"");
        assert!(encrypted.ciphertext.is_empty());
        assert!(decrypt_attachment(&encrypted.file, &[]).unwrap().is_empty());
    }

    #[test]
    fn each_attachment_gets_a_fresh_key_and_iv_with_a_zero_counter() {
        let a = encrypt_attachment(PLAINTEXT);
        let b = encrypt_attachment(PLAINTEXT);
        assert_ne!(a.file.key.k, b.file.key.k);
        assert_ne!(a.file.iv, b.file.iv);
        assert_ne!(a.ciphertext, b.ciphertext);
        let iv = base64_decode(&a.file.iv).unwrap();
        assert_eq!(iv.len(), 16);
        assert_eq!(&iv[8..], &[0u8; 8]);
    }

    #[test]
    fn json_shape_matches_matrix() {
        let encrypted = encrypt_attachment(PLAINTEXT);
        let json = serde_json::to_value(&encrypted.file).unwrap();
        assert_eq!(json["v"], "v2");
        assert_eq!(json["key"]["kty"], "oct");
        assert_eq!(json["key"]["alg"], "A256CTR");
        assert_eq!(json["key"]["ext"], true);
        assert_eq!(
            json["key"]["key_ops"],
            serde_json::json!(["encrypt", "decrypt"])
        );
        let k = json["key"]["k"].as_str().unwrap();
        assert!(!k.contains(['+', '/', '=']));
        assert_eq!(base64url_decode(k).unwrap().len(), 32);
        assert!(json["hashes"]["sha256"].is_string());
    }

    /// Matrix の仕様のとおり、ハッシュは暗号文の SHA-256。
    #[test]
    fn hash_is_over_the_ciphertext() {
        let encrypted = encrypt_attachment(PLAINTEXT);
        let expected = base64_encode(Sha256::digest(&encrypted.ciphertext));
        assert_eq!(encrypted.file.hashes["sha256"], expected);
    }

    /// 決まった鍵と IV での結果（AES-256-CTR、カウンターは 128 ビットのビッグエンディアン）。
    #[test]
    fn known_answer() {
        // NIST SP 800-38A F.5.5 CTR-AES256.Encrypt の 1 ブロック目
        let key: [u8; 32] = hex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
        let iv: [u8; 16] = hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
        let plaintext: [u8; 16] = hex("6bc1bee22e409f96e93d7e117393172a");
        let encrypted = encrypt_with(&plaintext, &key, iv);
        assert_eq!(
            encrypted.ciphertext,
            hex::<16>("601ec313775789a5b7a7f504bbf3d228")
        );
        assert_eq!(
            decrypt_attachment(&encrypted.file, &encrypted.ciphertext).unwrap(),
            plaintext
        );
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let encrypted = encrypt_attachment(PLAINTEXT);
        let mut tampered = encrypted.ciphertext.clone();
        tampered[0] ^= 1;
        assert!(matches!(
            decrypt_attachment(&encrypted.file, &tampered),
            Err(CryptoError::AttachmentDecryption(_))
        ));
        assert!(matches!(
            decrypt_attachment(&encrypted.file, &encrypted.ciphertext[1..]),
            Err(CryptoError::AttachmentDecryption(_))
        ));
    }

    #[test]
    fn malformed_files_are_rejected() {
        let encrypted = encrypt_attachment(PLAINTEXT);
        let cases: Vec<fn(&mut EncryptedFile)> = vec![
            |f| f.v = "v1".into(),
            |f| f.key.alg = "A128CTR".into(),
            |f| f.key.kty = "RSA".into(),
            |f| f.key.ext = false,
            |f| f.key.key_ops = vec!["decrypt".into()],
            |f| f.key.k = "AAAA".into(),
            |f| f.key.k = base64_encode([0xffu8; 32]),
            |f| f.iv = base64_encode([0u8; 8]),
            |f| {
                f.hashes.remove("sha256");
            },
            |f| {
                f.hashes.insert("sha256".into(), "%%".into());
            },
        ];
        for (i, mutate) in cases.iter().enumerate() {
            let mut file = encrypted.file.clone();
            mutate(&mut file);
            assert!(
                matches!(
                    decrypt_attachment(&file, &encrypted.ciphertext),
                    Err(CryptoError::InvalidPayload(_))
                ),
                "case {i}"
            );
        }
    }

    #[test]
    fn padded_base64_from_other_clients_is_accepted() {
        let mut encrypted = encrypt_attachment(PLAINTEXT);
        encrypted.file.iv = format!("{}==", encrypted.file.iv);
        let hash = encrypted.file.hashes["sha256"].clone();
        encrypted
            .file
            .hashes
            .insert("sha256".into(), format!("{hash}="));
        assert_eq!(
            decrypt_attachment(&encrypted.file, &encrypted.ciphertext).unwrap(),
            PLAINTEXT
        );
    }

    #[test]
    fn debug_does_not_print_the_key() {
        let encrypted = encrypt_attachment(PLAINTEXT);
        let debug = format!("{encrypted:?}");
        assert!(!debug.contains(&encrypted.file.key.k));
    }

    fn info(file: EncryptedFile) -> AttachmentInfo {
        AttachmentInfo {
            attachment_id: "0b0d5c5e-6f2a-4c1e-9d7a-2f3b4c5d6e7f".into(),
            name: "photo.png".into(),
            mimetype: "image/png".into(),
            size: PLAINTEXT.len() as u64,
            width: Some(640),
            height: Some(480),
            sensitive: false,
            file,
        }
    }

    #[test]
    fn attachment_info_validation() {
        let file = encrypt_attachment(PLAINTEXT).file;
        assert!(info(file.clone()).validate().is_ok());
        let cases: Vec<fn(&mut AttachmentInfo)> = vec![
            |a| a.attachment_id = "not-a-uuid".into(),
            |a| a.name = String::new(),
            |a| a.name = "  ".into(),
            |a| a.name = "a\nb".into(),
            |a| a.name = "a".repeat(256),
            |a| a.mimetype = String::new(),
            |a| a.mimetype = "a".repeat(129),
            |a| a.size = crate::MAX_SAFE_INTEGER + 1,
            |a| a.width = None,
            |a| a.file.v = "v1".into(),
        ];
        for (i, mutate) in cases.iter().enumerate() {
            let mut a = info(file.clone());
            mutate(&mut a);
            assert!(a.validate().is_err(), "case {i}");
        }
    }

    #[test]
    fn optional_fields_are_omitted_from_json() {
        let mut a = info(encrypt_attachment(PLAINTEXT).file);
        a.width = None;
        a.height = None;
        let json = serde_json::to_value(&a).unwrap();
        assert!(json.get("width").is_none());
        assert!(json.get("sensitive").is_none());
        a.sensitive = true;
        assert_eq!(serde_json::to_value(&a).unwrap()["sensitive"], true);
    }

    fn hex<const N: usize>(s: &str) -> [u8; N] {
        let bytes: Vec<u8> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect();
        bytes.try_into().unwrap()
    }
}
