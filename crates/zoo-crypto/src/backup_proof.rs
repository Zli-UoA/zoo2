//! 鍵バックアップのアップロードの証明（docs/e2ee.md の鍵バックアップの API の「アップロードの証明」）。
//!
//! サーバーは版ごとに Curve25519 の鍵対（[`BackupUploadKey`]）を持ち、公開鍵を版の `upload_key` で返す。
//! バックアップ鍵の秘密鍵を持つ端末は、その公開鍵との ECDH から証明を作り、アップロードの
//! `X-Zoo-Backup-Proof` に入れる（[`BackupDecryptionKey::upload_proof`]）。サーバーは自分の秘密鍵と
//! 版の `auth_data.public_key` との ECDH から同じ値を作って比べる（[`BackupUploadKey::verify_proof`]）。
//!
//! バックアップの公開鍵は誰でも使えるので、`session_data` だけでは、上げたのがバックアップ鍵を持つ
//! 端末かどうかを確かめられない。アクセストークンだけを持つ者は、この証明を作れない。
//!
//! ```text
//! shared = X25519(秘密鍵, 相手の公開鍵)
//! proof  = HKDF-SHA256(ikm = shared, salt = 32 バイトの 0,
//!                      info = LP("zoo e2ee megolm backup upload v1") ‖ LP(user_id) ‖ LP(version), L = 32)
//! LP(x)  = UTF-8 のバイト長（u32、ビッグエンディアン）‖ UTF-8 のバイト列
//! ```
//!
//! ヘッダーの値は `proof` のパディングなしの標準の base64。

use hkdf::Hkdf;
use sha2::Sha256;
use vodozemac::{Curve25519PublicKey, Curve25519SecretKey, base64_decode, base64_encode};
use zeroize::{Zeroize, Zeroizing};

use crate::CryptoError;
use crate::backup::BackupDecryptionKey;
use crate::device::random_bytes;

/// アップロードの証明のヘッダー。
pub const BACKUP_PROOF_HEADER: &str = "X-Zoo-Backup-Proof";
/// HKDF の info の先頭。
const PROOF_INFO: &str = "zoo e2ee megolm backup upload v1";

/// サーバーが版ごとに持つ、アップロードの証明を確かめる鍵（Curve25519）。
///
/// `Debug` は公開鍵だけを出す。
pub struct BackupUploadKey {
    secret: Curve25519SecretKey,
    public: Curve25519PublicKey,
}

impl std::fmt::Debug for BackupUploadKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupUploadKey")
            .field("public_key", &self.public)
            .finish_non_exhaustive()
    }
}

impl BackupUploadKey {
    /// 新しい鍵を作る。
    ///
    /// # Panics
    ///
    /// OS の暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn generate() -> Self {
        let mut bytes: [u8; 32] = random_bytes();
        let key = Self::from_bytes(&bytes);
        bytes.zeroize();
        key
    }

    fn from_bytes(bytes: &[u8; 32]) -> Self {
        let secret = Curve25519SecretKey::from_slice(bytes);
        let public = Curve25519PublicKey::from(&secret);
        Self { secret, public }
    }

    /// 保存した秘密鍵（パディングなしの base64、32 バイト）から戻す。
    pub fn from_base64(value: &str) -> Result<Self, CryptoError> {
        let mut bytes = base64_decode(value).map_err(|_| {
            CryptoError::InvalidBackup("upload key が base64 ではありません".into())
        })?;
        let result = <[u8; 32]>::try_from(bytes.as_slice())
            .map(|array| {
                let mut array = array;
                let key = Self::from_bytes(&array);
                array.zeroize();
                key
            })
            .map_err(|_| {
                CryptoError::InvalidBackup("upload key が 32 バイトではありません".into())
            });
        bytes.zeroize();
        result
    }

    /// 保存する秘密鍵（パディングなしの base64）。
    pub fn to_base64(&self) -> Zeroizing<String> {
        let mut bytes = self.secret.to_bytes();
        let encoded = Zeroizing::new(base64_encode(bytes.as_ref()));
        bytes.zeroize();
        encoded
    }

    /// 公開鍵（版の `upload_key`）。
    pub fn public_key(&self) -> Curve25519PublicKey {
        self.public
    }

    /// `proof` が、`backup_key`（版の `auth_data.public_key`）の秘密鍵を持つ者が `user_id` の `version` に
    /// 向けて作った証明かを確かめる。比較は定数時間で行う。
    pub fn verify_proof(
        &self,
        backup_key: &Curve25519PublicKey,
        user_id: &str,
        version: &str,
        proof: &str,
    ) -> bool {
        let Ok(mut received) = base64_decode(proof) else {
            return false;
        };
        let ok = match derive_proof(&self.secret, backup_key, user_id, version) {
            Ok(expected) if received.len() == expected.len() => {
                let diff = expected
                    .iter()
                    .zip(&received)
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b));
                diff == 0
            }
            _ => false,
        };
        received.zeroize();
        ok
    }
}

impl BackupDecryptionKey {
    /// `user_id` の版 `version` へのアップロードの証明（`X-Zoo-Backup-Proof` の値）を作る。
    /// `upload_key` は版の `upload_key`（サーバーの公開鍵）。
    pub fn upload_proof(
        &self,
        upload_key: &Curve25519PublicKey,
        user_id: &str,
        version: &str,
    ) -> Result<String, CryptoError> {
        let secret = Curve25519SecretKey::from_slice(&self.to_bytes());
        let proof = derive_proof(&secret, upload_key, user_id, version)?;
        Ok(base64_encode(proof.as_slice()))
    }
}

fn derive_proof(
    secret: &Curve25519SecretKey,
    public: &Curve25519PublicKey,
    user_id: &str,
    version: &str,
) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    let shared = secret
        .diffie_hellman(public)
        .ok_or_else(|| CryptoError::InvalidBackup("鍵交換の結果が 0 になる公開鍵です".into()))?;
    let mut info = Vec::new();
    for part in [PROOF_INFO, user_id, version] {
        let len = u32::try_from(part.len()).expect("ID は 4 GiB より短い");
        info.extend_from_slice(&len.to_be_bytes());
        info.extend_from_slice(part.as_bytes());
    }
    let mut okm = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(&[0u8; 32]), shared.as_bytes())
        .expand(&info, okm.as_mut_slice())
        .expect("32 バイトは HKDF-SHA256 の上限より短い");
    Ok(okm)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "00000000-0000-4000-8000-000000000001";

    #[test]
    fn the_server_accepts_only_proofs_from_the_backup_key_holder() {
        let backup = BackupDecryptionKey::generate();
        let server = BackupUploadKey::generate();
        let proof = backup
            .upload_proof(&server.public_key(), USER, "3")
            .unwrap();
        assert!(server.verify_proof(&backup.public_key(), USER, "3", &proof));

        // 別のバックアップ鍵、別のユーザー、別の版、別のサーバーの鍵では通らない。
        let other = BackupDecryptionKey::generate();
        let forged = other.upload_proof(&server.public_key(), USER, "3").unwrap();
        assert!(!server.verify_proof(&backup.public_key(), USER, "3", &forged));
        assert!(!server.verify_proof(
            &backup.public_key(),
            "00000000-0000-4000-8000-000000000002",
            "3",
            &proof
        ));
        assert!(!server.verify_proof(&backup.public_key(), USER, "4", &proof));
        let other_server = BackupUploadKey::generate();
        assert!(!other_server.verify_proof(&backup.public_key(), USER, "3", &proof));
        // 形の崩れた値も通らない。
        for bad in ["", "!!", "AAAA", &proof[..proof.len() - 2]] {
            assert!(
                !server.verify_proof(&backup.public_key(), USER, "3", bad),
                "{bad}"
            );
        }
    }

    #[test]
    fn upload_keys_round_trip_through_base64() {
        let server = BackupUploadKey::generate();
        let restored = BackupUploadKey::from_base64(&server.to_base64()).unwrap();
        assert_eq!(restored.public_key(), server.public_key());
        assert!(BackupUploadKey::from_base64("AAAA").is_err());
        assert!(BackupUploadKey::from_base64("!!").is_err());
        assert!(!format!("{server:?}").contains(server.to_base64().as_str()));
    }

    /// `crates/zoo-crypto/tests/vectors/gen_backup_upload_proof_v1.py`（Python の `cryptography`）で、
    /// `zoo-crypto` とは別に計算した値。
    #[test]
    fn matches_the_independent_vector() {
        let backup = BackupDecryptionKey::from_bytes(&[0x11; 32]);
        let server = BackupUploadKey::from_bytes(&[0x22; 32]);
        let proof = backup
            .upload_proof(&server.public_key(), USER, "12")
            .unwrap();
        assert_eq!(proof, VECTOR_PROOF);
        assert!(server.verify_proof(&backup.public_key(), USER, "12", VECTOR_PROOF));
    }

    const VECTOR_PROOF: &str = "Wst7kaXNOseF/BRemOR8xJJTEtm6vOlvoe6korBz8hw";
}
