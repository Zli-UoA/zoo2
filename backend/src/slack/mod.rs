//! Slack 連携 (#115)。取り込み状態と Slack ID の対応表は Electric の Shape に載せない。
pub mod api_import;
pub mod convert;
pub mod crypto;
pub mod export;
pub mod import;
pub mod link;
pub mod sync;
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

/// `SLACK_IMPORT_WORKER` が `false`/`0`/`off`/`disabled`/`no` のとき、この backend プロセスでは Slack の取り込みと
/// 差分同期のワーカーを動かさない (専用の `zoo_slack_worker` に任せる)。未設定・空・その他の値は有効 (従来どおり)
pub fn worker_enabled_from(raw: Option<&str>) -> bool {
    !matches!(
        raw.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("false" | "0" | "off" | "disabled" | "no")
    )
}

pub fn worker_enabled() -> bool {
    let raw = std::env::var("SLACK_IMPORT_WORKER").ok();
    let enabled = worker_enabled_from(raw.as_deref());
    // 書き間違い (`flase` など) は有効扱いになる。専用ワーカーに任せたつもりで二重に動かないよう警告する
    if let Some(value) = raw.as_deref().map(str::trim)
        && enabled
        && !value.is_empty()
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "1" | "on" | "enabled" | "yes"
        )
    {
        tracing::warn!(
            value,
            "Unrecognized SLACK_IMPORT_WORKER; the Slack workers stay enabled"
        );
    }
    enabled
}

#[cfg(test)]
mod worker_switch_tests {
    use super::worker_enabled_from;

    #[test]
    fn only_explicit_off_values_disable_the_workers() {
        for off in ["false", "0", "off", "disabled", "no", " FALSE ", "No"] {
            assert!(!worker_enabled_from(Some(off)), "{off}");
        }
        for on in ["true", "1", "", "flase", "yes"] {
            assert!(worker_enabled_from(Some(on)), "{on}");
        }
        assert!(worker_enabled_from(None));
    }
}
