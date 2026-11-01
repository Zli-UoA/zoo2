//! Slack のユーザートークンの暗号化と、OAuth の state の署名。

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit as HmacKeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::AppError;

const NONCE_LEN: usize = 12;

/// AES-256-GCM で暗号化し、`base64(nonce || ciphertext)` を返す
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> Result<String, AppError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| AppError::Internal(e.into()))?;
    let random = uuid::Uuid::new_v4();
    let nonce_bytes = &random.as_bytes()[..NONCE_LEN];
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(nonce_bytes), plaintext.as_bytes())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("Failed to encrypt Slack token")))?;
    let mut out = nonce_bytes.to_vec();
    out.extend(ciphertext);
    Ok(STANDARD.encode(out))
}

pub fn decrypt(key: &[u8; 32], encoded: &str) -> Result<String, AppError> {
    let invalid = || AppError::Internal(anyhow::anyhow!("Failed to decrypt Slack token"));
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid())?;
    if bytes.len() <= NONCE_LEN {
        return Err(invalid());
    }
    let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| AppError::Internal(e.into()))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| invalid())?;
    String::from_utf8(plaintext).map_err(|_| invalid())
}

/// 連携を始めたユーザーと有効期限を、改ざんできない形で Slack に往復させる
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct OAuthState {
    pub user_id: String,
    pub nonce: String,
    pub expires_at: i64,
}

fn mac(key: &[u8; 32], payload: &str) -> Hmac<Sha256> {
    let mut mac =
        <Hmac<Sha256> as HmacKeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(b"slack-oauth-state:");
    mac.update(payload.as_bytes());
    mac
}

pub fn sign_state(key: &[u8; 32], state: &OAuthState) -> String {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(state).expect("serializable"));
    let signature = URL_SAFE_NO_PAD.encode(mac(key, &payload).finalize().into_bytes());
    format!("{payload}.{signature}")
}

/// 署名と有効期限を確かめる。`now` は UNIX 秒。
pub fn verify_state(key: &[u8; 32], raw: &str, now: i64) -> Option<OAuthState> {
    let (payload, signature) = raw.split_once('.')?;
    let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
    mac(key, payload).verify_slice(&signature).ok()?;
    let state: OAuthState = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    (state.expires_at > now).then_some(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn round_trips_tokens_with_random_nonces() {
        let a = encrypt(&KEY, "xoxp-secret").unwrap();
        let b = encrypt(&KEY, "xoxp-secret").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt(&KEY, &a).unwrap(), "xoxp-secret");
        assert!(decrypt(&[8; 32], &a).is_err());
        assert!(decrypt(&KEY, "AAAA").is_err());
    }

    #[test]
    fn verifies_signed_state() {
        let state = OAuthState {
            user_id: "u1".into(),
            nonce: "n".into(),
            expires_at: 100,
        };
        let raw = sign_state(&KEY, &state);
        assert_eq!(verify_state(&KEY, &raw, 99), Some(state));
        assert_eq!(verify_state(&KEY, &raw, 100), None, "expired");
        assert_eq!(verify_state(&[8; 32], &raw, 99), None, "other key");
        let (payload, signature) = raw.split_once('.').unwrap();
        let forged = URL_SAFE_NO_PAD.encode(br#"{"user_id":"u2","nonce":"n","expires_at":100}"#);
        assert_eq!(
            verify_state(&KEY, &format!("{forged}.{signature}"), 99),
            None
        );
        assert_eq!(verify_state(&KEY, payload, 99), None);
    }
}
