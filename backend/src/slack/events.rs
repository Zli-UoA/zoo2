//! Slack の Events API の受信口 (#293)。
//!
//! 方針は「Slack API のポーリング (差分同期、`sync.rs`) を正とし、イベントは『このチャンネルを今すぐ同期して』
//! という合図にだけ使う」。この受信口は、署名を検証し、イベントからチームとチャンネルだけを取り出して、
//! `slack_channel_wakes` に 1 行書き (編集・削除・リアクションなら、確かめ直すメッセージの指定を `slack_message_hints` にも書き)、すぐ 200 を返す (Slack は 3 秒以内の応答を求める)。反映は差分同期が行う。
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

/// 同期のあとに確かめ直す、Slack のメッセージ 1 件の指定 (`slack_message_hints` の 1 行)。本文は持たない
#[derive(Debug, PartialEq, Eq)]
pub struct Hint {
    pub ts: String,
    /// `changed` / `deleted` / `reaction` / `reply`
    pub kind: &'static str,
    /// `reaction` のときの、リアクションした人 (Slack の ID) と、リアクションの名前。ほかは空
    pub user: String,
    pub reaction: String,
    /// スレッドの返信なら、親の `ts`。そうでなければ空
    pub thread_ts: String,
}

/// 受け取ったイベントから取り出すもの
#[derive(Debug, PartialEq, Eq)]
pub struct Signal {
    pub team: String,
    pub channel: String,
    pub hint: Option<Hint>,
}

fn reaction_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b':' | b'\''))
}

/// イベントから、同期を起こすチャンネルと、確かめ直すメッセージを取り出す。取り出すのは ID と `ts`、
/// リアクションの名前だけで、本文は見ない。
///
/// - 新しい投稿 (`message`。subtype なし・`thread_broadcast`・`file_share`・bot の投稿など): チャンネルを起こす。
///   スレッドの返信なら `reply` のヒントも付ける (差分同期の監視範囲 (3 日・20 件) の外のスレッドも取れるようにする)。
/// - `message_changed` / `message_deleted`: チャンネルを起こし、`changed` / `deleted` のヒントを付ける。
/// - `reaction_added` / `reaction_removed` (メッセージへのものだけ): チャンネルを起こし、`reaction` のヒントを付ける。
/// - 入退室・トピックなどのシステム通知と、そのほかのイベントは無視する。
pub fn signal(payload: &Value) -> Option<Signal> {
    if payload["type"] != "event_callback" {
        return None;
    }
    let team = payload["team_id"].as_str()?;
    let event = &payload["event"];
    let ts_ok = |ts: &str| super::is_valid_ts(ts);
    let thread_of = |message: &Value, ts: &str| -> String {
        message["thread_ts"]
            .as_str()
            .filter(|t| *t != ts && ts_ok(t))
            .unwrap_or_default()
            .to_string()
    };
    let (channel, hint) = match event["type"].as_str()? {
        "message" => {
            let channel = event["channel"].as_str()?;
            match event["subtype"].as_str() {
                Some("message_changed") => {
                    let message = &event["message"];
                    let ts = message["ts"].as_str().filter(|t| ts_ok(t))?;
                    let hint = Hint {
                        ts: ts.into(),
                        kind: "changed",
                        user: String::new(),
                        reaction: String::new(),
                        thread_ts: thread_of(message, ts),
                    };
                    (channel, Some(hint))
                }
                Some("message_deleted") => {
                    let ts = event["deleted_ts"].as_str().filter(|t| ts_ok(t))?;
                    let hint = Hint {
                        ts: ts.into(),
                        kind: "deleted",
                        user: String::new(),
                        reaction: String::new(),
                        thread_ts: thread_of(&event["previous_message"], ts),
                    };
                    (channel, Some(hint))
                }
                Some(subtype) if SKIPPED_SUBTYPES.contains(&subtype) => return None,
                _ => {
                    let hint = event["ts"]
                        .as_str()
                        .filter(|t| ts_ok(t))
                        .map(|ts| (ts, thread_of(event, ts)))
                        .filter(|(_, thread)| !thread.is_empty())
                        .map(|(ts, thread_ts)| Hint {
                            ts: ts.into(),
                            kind: "reply",
                            user: String::new(),
                            reaction: String::new(),
                            thread_ts,
                        });
                    (channel, hint)
                }
            }
        }
        "reaction_added" | "reaction_removed" => {
            let item = &event["item"];
            if item["type"] != "message" {
                return None;
            }
            let user = event["user"].as_str().filter(|u| valid_id(u))?;
            let reaction = event["reaction"].as_str().filter(|r| reaction_name_ok(r))?;
            let ts = item["ts"].as_str().filter(|t| ts_ok(t))?;
            let hint = Hint {
                ts: ts.into(),
                kind: "reaction",
                user: user.into(),
                reaction: reaction.into(),
                thread_ts: String::new(),
            };
            (item["channel"].as_str()?, Some(hint))
        }
        _ => return None,
    };
    (valid_id(team) && valid_id(channel)).then(|| Signal {
        team: team.to_string(),
        channel: channel.to_string(),
        hint,
    })
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
    // 別の Slack アプリ宛てのイベントは無視する (`SLACK_APP_ID` を設定したときだけ)
    if let Some(app) = s.config.slack.app_id.as_deref()
        && payload["api_app_id"].as_str().is_some_and(|a| a != app)
    {
        return Ok(StatusCode::OK.into_response());
    }
    let Some(signal) = signal(&payload) else {
        return Ok(StatusCode::OK.into_response());
    };
    let (team, channel) = (signal.team.as_str(), signal.channel.as_str());
    if !s.config.slack.events_enabled() || !s.config.slack.team_allowed(team) {
        return Ok(StatusCode::OK.into_response());
    }
    // `slack_channels` に行があるチャンネルだけを書く (対応付けのないチャンネルは何もしない)。合図と確かめ直す
    // メッセージは、同じトランザクションで書く。DB が失敗したら 500 を返し、Slack の再送 (最大 3 回) に任せる
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let mut statement = toasty::sql::statement(
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) \
         SELECT team_id,slack_channel_id FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2 \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now()",
    );
    statement = statement.bind(team).bind(channel);
    statement.exec(&mut tx).await?;
    if let Some(hint) = &signal.hint {
        let mut statement = toasty::sql::statement(
            "INSERT INTO slack_message_hints(team_id,slack_channel_id,message_ts,kind,slack_user_id,reaction,thread_ts) \
             SELECT team_id,slack_channel_id,$3,$4,$5,$6,$7 FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2 \
             ON CONFLICT (team_id,slack_channel_id,message_ts,kind,slack_user_id,reaction) \
             DO UPDATE SET updated_at=now(),thread_ts=EXCLUDED.thread_ts",
        );
        statement = statement
            .bind(team)
            .bind(channel)
            .bind(hint.ts.as_str())
            .bind(hint.kind)
            .bind(hint.user.as_str())
            .bind(hint.reaction.as_str())
            .bind(hint.thread_ts.as_str());
        statement.exec(&mut tx).await?;
    }
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

    fn signal_of(payload: &Value) -> Option<(String, String, Option<Hint>)> {
        signal(payload).map(|s| (s.team, s.channel, s.hint))
    }

    #[test]
    fn new_posts_wake_the_channel_and_replies_add_a_hint() {
        let (team, channel, hint) = signal_of(&message(None)).unwrap();
        assert_eq!((team.as_str(), channel.as_str()), ("T1", "C123"));
        assert_eq!(hint, None);
        for subtype in [
            "thread_broadcast",
            "file_share",
            "bot_message",
            "me_message",
        ] {
            assert!(signal_of(&message(Some(subtype))).is_some(), "{subtype}");
        }
        let mut reply = message(None);
        reply["event"]["thread_ts"] = json!("1699999999.000100");
        assert_eq!(
            signal_of(&reply).unwrap().2,
            Some(Hint {
                ts: "1700000000.000100".into(),
                kind: "reply",
                user: String::new(),
                reaction: String::new(),
                thread_ts: "1699999999.000100".into(),
            })
        );
        // 親自身 (thread_ts が ts と同じ) はヒントなし
        let mut parent = message(None);
        parent["event"]["thread_ts"] = json!("1700000000.000100");
        assert_eq!(signal_of(&parent).unwrap().2, None);
    }

    #[test]
    fn system_notices_are_ignored() {
        for subtype in ["channel_join", "group_topic", "tombstone", "bot_add"] {
            assert_eq!(signal_of(&message(Some(subtype))), None, "{subtype}");
        }
    }

    #[test]
    fn edits_and_deletes_carry_the_message_ts() {
        let mut changed = message(Some("message_changed"));
        changed["event"]["message"] =
            json!({"ts": "1700000005.000100", "thread_ts": "1700000000.000100", "text": "x"});
        let hint = signal_of(&changed).unwrap().2.unwrap();
        assert_eq!(
            (hint.kind, hint.ts.as_str(), hint.thread_ts.as_str()),
            ("changed", "1700000005.000100", "1700000000.000100")
        );
        let mut deleted = message(Some("message_deleted"));
        deleted["event"]["deleted_ts"] = json!("1700000007.000100");
        deleted["event"]["previous_message"] = json!({"ts": "1700000007.000100"});
        let hint = signal_of(&deleted).unwrap().2.unwrap();
        assert_eq!(
            (hint.kind, hint.ts.as_str(), hint.thread_ts.as_str()),
            ("deleted", "1700000007.000100", "")
        );
        // ts がないもの・不正なものは無視する
        let mut bad = message(Some("message_deleted"));
        bad["event"]["deleted_ts"] = json!("1;DROP");
        assert_eq!(signal_of(&bad), None);
    }

    #[test]
    fn reactions_carry_the_user_and_name() {
        let payload = json!({"type": "event_callback", "team_id": "T1", "event": {
            "type": "reaction_removed", "user": "UA", "reaction": "+1::skin-tone-2",
            "item": {"type": "message", "channel": "C123", "ts": "1700000000.000100"}}});
        let (_, channel, hint) = signal_of(&payload).unwrap();
        assert_eq!(channel, "C123");
        let hint = hint.unwrap();
        assert_eq!(
            (hint.kind, hint.user.as_str(), hint.reaction.as_str()),
            ("reaction", "UA", "+1::skin-tone-2")
        );
        // ファイルへのリアクションや、不正な名前は無視する
        let mut file = payload.clone();
        file["event"]["item"]["type"] = json!("file");
        assert_eq!(signal_of(&file), None);
        let mut bad = payload.clone();
        bad["event"]["reaction"] = json!("a b'; DROP");
        assert_eq!(signal_of(&bad), None);
    }

    #[test]
    fn ignores_other_events_and_bad_ids() {
        let mut other = message(None);
        other["event"]["type"] = json!("channel_created");
        assert_eq!(signal_of(&other), None);
        let mut verification = message(None);
        verification["type"] = json!("url_verification");
        assert_eq!(signal_of(&verification), None);
        let mut bad = message(None);
        bad["event"]["channel"] = json!("C1'; DROP TABLE x;--");
        assert_eq!(signal_of(&bad), None);
        let mut long = message(None);
        long["team_id"] = json!("T".repeat(33));
        assert_eq!(signal_of(&long), None);
        let mut missing = message(None);
        missing["event"].as_object_mut().unwrap().remove("channel");
        assert_eq!(signal_of(&missing), None);
    }
}
