//! リカバリーキー（docs/e2ee.md の「リカバリーキーの形式」）。
//!
//! エンコードは Matrix の Cryptographic key representation と同じ。ランダムな 32 バイトの前に
//! `0x8B 0x01` を置き、34 バイトをすべて XOR したパリティを末尾に足した 35 バイトを、Bitcoin と
//! 同じ alphabet の Base58 にする（48 文字）。表示は 4 文字ごとに空白を入れる。
//!
//! zoo のリカバリーキーは SSSS 鍵そのものではなく、SSSS 鍵を包む鍵スロットの鍵なので、
//! Matrix のリカバリーキーとは互換性がない。

use zeroize::{Zeroize, Zeroizing};

use crate::CryptoError;
use crate::device::random_bytes;

/// 先頭に置く 2 バイト。
const PREFIX: [u8; 2] = [0x8b, 0x01];
/// エンコードする前のバイト長（前置き 2、鍵 32、パリティ 1）。
const DECODED_LEN: usize = 35;
/// エンコードしたあとの文字数（空白を除く）。
pub const RECOVERY_KEY_CHARS: usize = 48;
/// Bitcoin の Base58 の alphabet。
const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// リカバリーキーの 32 バイト。ユーザーが控え、リカバリーキーの鍵スロットの KEK を導く。
///
/// `Debug` は中身を出さない。落とすときにメモリーから消す。
#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryKey(..)")
    }
}

impl RecoveryKey {
    /// 新しいリカバリーキーを作る。
    ///
    /// # Panics
    ///
    /// OS やブラウザーの暗号用乱数を取れないとき（vodozemac の鍵の生成と同じ）。
    pub fn generate() -> Self {
        Self(Zeroizing::new(random_bytes()))
    }

    /// 32 バイトから作る（テストベクター用）。
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// 32 バイトの中身。
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// ユーザーに見せる形（48 文字を 4 文字ごとに空白で区切ったもの）。
    pub fn to_display(&self) -> Zeroizing<String> {
        let mut bytes = Zeroizing::new([0u8; DECODED_LEN]);
        bytes[..2].copy_from_slice(&PREFIX);
        bytes[2..34].copy_from_slice(self.0.as_ref());
        bytes[34] = bytes[..34].iter().fold(0, |acc, b| acc ^ b);
        let encoded = base58_encode(bytes.as_ref());
        let mut display = Zeroizing::new(String::with_capacity(encoded.len() + encoded.len() / 4));
        for (i, c) in encoded.chars().enumerate() {
            if i > 0 && i % 4 == 0 {
                display.push(' ');
            }
            display.push(c);
        }
        display
    }

    /// ユーザーが入力したリカバリーキーを読む。空白（改行やタブを含む）は無視する。
    ///
    /// Base58 でない、長さ、先頭の 2 バイト、パリティのどれかが違えば
    /// [`CryptoError::InvalidRecoveryKey`] を返す。
    pub fn parse(input: &str) -> Result<Self, CryptoError> {
        let compact: Zeroizing<String> =
            Zeroizing::new(input.chars().filter(|c| !c.is_whitespace()).collect());
        if compact.chars().count() != RECOVERY_KEY_CHARS {
            return Err(CryptoError::InvalidRecoveryKey(format!(
                "{RECOVERY_KEY_CHARS} 文字ではありません"
            )));
        }
        let decoded = Zeroizing::new(base58_decode(&compact)?);
        if decoded.len() != DECODED_LEN {
            return Err(CryptoError::InvalidRecoveryKey(
                "復号したバイト長が違います".into(),
            ));
        }
        if decoded[..2] != PREFIX {
            return Err(CryptoError::InvalidRecoveryKey(
                "先頭の 2 バイトが違います".into(),
            ));
        }
        if decoded.iter().fold(0, |acc, b| acc ^ b) != 0 {
            return Err(CryptoError::InvalidRecoveryKey(
                "パリティが一致しません（打ち間違いの可能性があります）".into(),
            ));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&decoded[2..34]);
        let recovery_key = Self::from_bytes(key);
        key.zeroize();
        Ok(recovery_key)
    }
}

/// Base58 にする。先頭の 0 のバイトは `1` にする（リカバリーキーは先頭が 0x8B なので出ない）。
fn base58_encode(input: &[u8]) -> String {
    // 58 進の桁（下位から）。
    let mut digits: Zeroizing<Vec<u8>> =
        Zeroizing::new(Vec::with_capacity(input.len() * 138 / 100 + 1));
    for &byte in input {
        let mut carry = u32::from(byte);
        for digit in digits.iter_mut() {
            carry += u32::from(*digit) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = input.iter().take_while(|&&b| b == 0).count();
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    out.extend(
        digits
            .iter()
            .rev()
            .map(|&d| ALPHABET[usize::from(d)] as char),
    );
    out
}

/// Base58 を読む。alphabet にない文字があれば [`CryptoError::InvalidRecoveryKey`] を返す。
fn base58_decode(input: &str) -> Result<Vec<u8>, CryptoError> {
    // 256 進の桁（下位から）。
    let mut bytes: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(input.len()));
    for c in input.bytes() {
        let value = ALPHABET
            .iter()
            .position(|&a| a == c)
            .ok_or_else(|| CryptoError::InvalidRecoveryKey("Base58 にない文字があります".into()))?;
        let mut carry = value as u32;
        for byte in bytes.iter_mut() {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = input.bytes().take_while(|&c| c == b'1').count();
    let mut out = vec![0u8; zeros];
    out.extend(bytes.iter().rev());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base58_matches_bitcoin_vectors() {
        // https://github.com/bitcoin/bitcoin/blob/master/src/test/data/base58_encode_decode.json
        let vectors: [(&[u8], &str); 5] = [
            (b"", ""),
            (&[0x61], "2g"),
            (&[0x62, 0x62, 0x62], "a3gV"),
            (&[0x00, 0x00, 0x28, 0x7f, 0xb4, 0xcd], "111233QC4"),
            (&[0x51, 0x6b, 0x6f, 0xcd, 0x0f], "ABnLTmg"),
        ];
        for (bytes, text) in vectors {
            assert_eq!(base58_encode(bytes), text);
            assert_eq!(base58_decode(text).unwrap(), bytes);
        }
    }
}
