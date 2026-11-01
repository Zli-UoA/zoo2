//! Slack 連携 (#115)。取り込み状態と Slack ID の対応表は Electric の Shape に載せない。
pub mod convert;
pub mod crypto;
pub mod export;
pub mod import;
pub mod link;
pub mod worker;

use sha2::{Digest, Sha256};

/// Slack 側の識別子から決定的な UUID を作る。再取り込み・差分同期でも同じ行を指すため冪等になる。
pub fn stable_id(namespace: &str, parts: &[&str]) -> String {
    let digest = Sha256::digest(format!("slack-{namespace}:{}", parts.join(":")));
    uuid::Uuid::from_slice(&digest[..16])
        .expect("16 bytes")
        .to_string()
}

pub fn message_id(team: &str, channel: &str, ts: &str) -> String {
    stable_id("message", &[team, channel, ts])
}

pub fn channel_id(team: &str, slack_channel: &str) -> String {
    stable_id("channel", &[team, slack_channel])
}

pub fn user_id(team: &str, slack_user: &str) -> String {
    stable_id("user", &[team, slack_user])
}

pub fn reaction_id(message: &str, user: &str, emoji: &str) -> String {
    stable_id("reaction", &[message, user, emoji])
}

pub fn attachment_id(team: &str, slack_file: &str) -> String {
    stable_id("file", &[team, slack_file])
}

/// Slack の ts (`1700000000.123456`) が数値として妥当か。SQL へ渡す前に検証する。
pub fn is_valid_ts(ts: &str) -> bool {
    let mut parts = ts.splitn(2, '.');
    let secs = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("0");
    !secs.is_empty()
        && secs.len() <= 12
        && secs.bytes().all(|b| b.is_ascii_digit())
        && !frac.is_empty()
        && frac.len() <= 9
        && frac.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_are_deterministic_and_namespaced() {
        let a = message_id("T1", "C1", "1700000000.000100");
        assert_eq!(a, message_id("T1", "C1", "1700000000.000100"));
        assert_ne!(a, message_id("T1", "C2", "1700000000.000100"));
        assert_ne!(user_id("T1", "U1"), channel_id("T1", "U1"));
        assert!(uuid::Uuid::parse_str(&a).is_ok());
    }

    #[test]
    fn validates_ts() {
        assert!(is_valid_ts("1700000000.000100"));
        assert!(is_valid_ts("1700000000"));
        assert!(!is_valid_ts(""));
        assert!(!is_valid_ts("1700000000."));
        assert!(!is_valid_ts("17e9"));
        assert!(!is_valid_ts("1;DROP"));
    }
}
