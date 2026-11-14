//! Slack の Events API の受信口 (#293)。
//!
//! 方針は「Slack API のポーリング (差分同期、`sync.rs`) を正とし、イベントは『このチャンネルを今すぐ同期して』
//! という合図にだけ使う」。この受信口は、署名を検証し、イベントからチームとチャンネルだけを取り出して、
//! `slack_channel_wakes` に 1 行書き、すぐ 200 を返す (Slack は 3 秒以内の応答を求める)。反映は差分同期が行う。
//!
//! - 認証のない経路なので、署名の検証より前に DB には触れない。ボディの上限は `MAX_BODY`。
//! - イベントの本文は読み捨てる。保存もログにも出さない (非公開チャンネルの内容を含みうる)。
//! - 登録されていないワークスペース・チャンネルは、何も書かずに 200 を返す (存在の有無を教えない)。
//! - 再送 (`X-Slack-Retry-Num`) や重複は、同じ行への UPSERT なので、結果が変わらない。
//! - 署名の Secret (`SLACK_SIGNING_SECRET`) が未設定なら 404 (機能が無効)。

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

use super::export::SKIPPED_SUBTYPES;
use crate::{error::AppError, state::AppState};

/// 受け付けるボディの上限。Slack のイベントは数 KB だが、ブロックや添付を含む投稿は大きくなる
const MAX_BODY: usize = 256 * 1024;
/// 署名に使うタイムスタンプと現在時刻の許容差 (秒)。古い要求の使い回し (リプレイ) を防ぐ
const MAX_SKEW_SECS: i64 = 5 * 60;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/api/slack/events",
        post(receive).layer(DefaultBodyLimit::max(MAX_BODY)),
    )
}

/// Slack の署名 (`v0=` + hex(HMAC-SHA256(secret, `v0:{timestamp}:{body}`))) を検証する。
/// タイムスタンプが `now` から 5 分以上離れているもの (未来も含む) は、署名が正しくても拒否する。
pub fn verify_signature(
    secret: &str,
    timestamp: &str,
    signature: &str,
    body: &[u8],
    now: i64,
) -> bool {
    let Ok(ts) = timestamp.trim().parse::<i64>() else {
        return false;
    };
    // `i64::MIN` などでも桁あふれしない比べ方にする (桁あふれは panic になりうる)
    if now.abs_diff(ts) > MAX_SKEW_SECS as u64 {
        return false;
    }
    let Some(hex) = signature.strip_prefix("v0=") else {
        return false;
    };
    if hex.len() != 64 || !hex.is_ascii() {
        return false;
    }
    let Ok(expected) = (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(b"v0:");
    mac.update(timestamp.as_bytes());
    mac.update(b":");
    mac.update(body);
    // 定数時間で比べる
    mac.verify_slice(&expected).is_ok()
}

/// Slack の ID (`T…`・`C…`・`G…`) として妥当か。SQL へ渡す前の確認で、巨大な値や変な文字を落とす
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// 同期を起こす合図になるイベントから、(チームの ID, チャンネルの ID) を取り出す。
///
/// 合図にするのは新しい投稿 (`message`。subtype なし・`thread_broadcast`・`file_share`・bot の投稿など) だけ。
/// 入退室・トピックなどのシステム通知と、編集・削除 (`message_changed` / `message_deleted`) は、この版では
/// 同期しても反映できない (編集・削除の反映は後続の変更) ので、合図にしない。
/// 取り出すのは ID だけで、本文は見ない。
pub fn wake_target(payload: &Value) -> Option<(String, String)> {
    if payload["type"] != "event_callback" {
        return None;
    }
    let event = &payload["event"];
    if event["type"] != "message" {
        return None;
    }
    if event["subtype"]
        .as_str()
        .is_some_and(|s| SKIPPED_SUBTYPES.contains(&s))
    {
        return None;
    }
    let team = payload["team_id"].as_str()?;
    let channel = event["channel"].as_str()?;
    (valid_id(team) && valid_id(channel)).then(|| (team.to_string(), channel.to_string()))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn receive(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(secret) = s.config.slack.signing_secret.as_deref() else {
        return Err(AppError::NotFound("Slack events are not configured".into()));
    };
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let verified = match (get("x-slack-request-timestamp"), get("x-slack-signature")) {
        (Some(timestamp), Some(signature)) => {
            verify_signature(secret, timestamp, signature, &body, now_secs())
        }
        _ => false,
    };
    if !verified {
        return Err(AppError::Unauthorized("Invalid Slack signature".into()));
    }
    // 署名のあとなので、ここからは Slack からの要求。形が違うものは、再送されても直らないので 200 で受け流す
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return Ok(StatusCode::OK.into_response());
    };
    if payload["type"] == "url_verification" {
        let challenge = payload["challenge"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        return Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            challenge,
        )
            .into_response());
    }
    let Some((team, channel)) = wake_target(&payload) else {
        return Ok(StatusCode::OK.into_response());
    };
    if !s.config.slack.events_enabled() || !s.config.slack.team_allowed(&team) {
        return Ok(StatusCode::OK.into_response());
    }
    // `slack_channels` に行があるチャンネルだけを書く (対応付けのないチャンネルは何もしない)。
    // DB が失敗したら 500 を返し、Slack の再送 (最大 3 回) に任せる
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let mut statement = toasty::sql::statement(
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) \
         SELECT team_id,slack_channel_id FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2 \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now()",
    );
    statement = statement.bind(team.as_str()).bind(channel.as_str());
    statement.exec(&mut tx).await?;
    tx.commit().await?;
    Ok(StatusCode::OK.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sign(secret: &str, ts: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("v0:{ts}:").as_bytes());
        mac.update(body);
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("v0={hex}")
    }

    #[test]
    fn accepts_a_correct_signature() {
        let body = br#"{"type":"event_callback"}"#;
        let sig = sign("secret", "1700000000", body);
        assert!(verify_signature(
            "secret",
            "1700000000",
            &sig,
            body,
            1_700_000_100
        ));
    }

    #[test]
    fn rejects_wrong_secret_tampered_body_and_bad_formats() {
        let body = br#"{"a":1}"#;
        let sig = sign("secret", "1700000000", body);
        let now = 1_700_000_000;
        assert!(!verify_signature("other", "1700000000", &sig, body, now));
        assert!(!verify_signature(
            "secret",
            "1700000000",
            &sig,
            br#"{"a":2}"#,
            now
        ));
        // 別のタイムスタンプ (署名はタイムスタンプも対象)
        assert!(!verify_signature("secret", "1700000001", &sig, body, now));
        assert!(!verify_signature("secret", "1700000000", "", body, now));
        assert!(!verify_signature(
            "secret",
            "1700000000",
            &sig.replacen("v0=", "v1=", 1),
            body,
            now
        ));
        assert!(!verify_signature(
            "secret",
            "1700000000",
            &format!("v0={}", "z".repeat(64)),
            body,
            now
        ));
        assert!(!verify_signature(
            "secret",
            "1700000000",
            "v0=ab",
            body,
            now
        ));
        assert!(!verify_signature("secret", "abc", &sig, body, now));
        assert!(!verify_signature("secret", "", &sig, body, now));
        // 桁あふれする値でも、panic せずに拒否する
        for (extreme, opposite) in [(i64::MIN, i64::MAX), (i64::MAX, i64::MIN)] {
            let ts = extreme.to_string();
            let sig = sign("secret", &ts, body);
            assert!(!verify_signature("secret", &ts, &sig, body, now));
            assert!(!verify_signature("secret", &ts, &sig, body, opposite));
        }
    }

    #[test]
    fn rejects_stale_and_future_timestamps() {
        let body = b"{}";
        let sig = sign("secret", "1700000000", body);
        assert!(verify_signature(
            "secret",
            "1700000000",
            &sig,
            body,
            1_700_000_300
        ));
        assert!(!verify_signature(
            "secret",
            "1700000000",
            &sig,
            body,
            1_700_000_301
        ));
        assert!(verify_signature(
            "secret",
            "1700000000",
            &sig,
            body,
            1_699_999_700
        ));
        assert!(!verify_signature(
            "secret",
            "1700000000",
            &sig,
            body,
            1_699_999_699
        ));
    }

    fn message(subtype: Option<&str>) -> Value {
        let mut event =
            json!({"type": "message", "channel": "C123", "ts": "1700000000.000100", "text": "hi"});
        if let Some(s) = subtype {
            event["subtype"] = json!(s);
        }
        json!({"type": "event_callback", "team_id": "T1", "event": event})
    }

    #[test]
    fn wakes_for_new_posts_only() {
        let target = Some(("T1".to_string(), "C123".to_string()));
        assert_eq!(wake_target(&message(None)), target);
        for subtype in [
            "thread_broadcast",
            "file_share",
            "bot_message",
            "me_message",
        ] {
            assert_eq!(wake_target(&message(Some(subtype))), target, "{subtype}");
        }
        for subtype in [
            "channel_join",
            "group_topic",
            "message_changed",
            "message_deleted",
            "tombstone",
        ] {
            assert_eq!(wake_target(&message(Some(subtype))), None, "{subtype}");
        }
    }

    #[test]
    fn ignores_other_events_and_bad_ids() {
        let mut reaction = message(None);
        reaction["event"]["type"] = json!("reaction_added");
        assert_eq!(wake_target(&reaction), None);
        let mut other = message(None);
        other["type"] = json!("url_verification");
        assert_eq!(wake_target(&other), None);
        let mut bad = message(None);
        bad["event"]["channel"] = json!("C1'; DROP TABLE x;--");
        assert_eq!(wake_target(&bad), None);
        let mut long = message(None);
        long["team_id"] = json!("T".repeat(33));
        assert_eq!(wake_target(&long), None);
        let mut missing = message(None);
        missing["event"].as_object_mut().unwrap().remove("channel");
        assert_eq!(wake_target(&missing), None);
    }
}
