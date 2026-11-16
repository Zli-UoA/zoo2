//! 新規投稿の本人だけに Zoo への移行案内を表示する。
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::state::AppState;

const TEXT: &str = "こんにちは！Zoo2移行チームです！\nSlack使用料削減のためZoo2でのメッセージングを非常に強く推奨しています！\n今後の投稿にはZoo(<https://zli.chat|https://zli.chat>) をご利用ください";
static SENDING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);
static SEEN: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn notice(payload: &Value) -> Option<Value> {
    if payload["type"] != "event_callback" {
        return None;
    }
    let event = &payload["event"];
    if event["type"] != "message"
        || matches!(event["channel_type"].as_str(), Some("im" | "mpim"))
        || event.get("bot_id").is_some()
        || event.get("app_id").is_some()
        || !matches!(
            event["subtype"].as_str(),
            None | Some("file_share" | "me_message" | "thread_broadcast")
        )
    {
        return None;
    }
    let user = event["user"].as_str()?;
    let channel = event["channel"].as_str()?;
    let valid = |id: &str| {
        !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric())
    };
    if !valid(user) || !valid(channel) || user == "USLACKBOT" {
        return None;
    }
    let ts = event["ts"].as_str().filter(|ts| super::is_valid_ts(ts))?;
    let mut body = json!({"channel": channel, "user": user, "text": TEXT});
    if let Some(thread) = event["thread_ts"].as_str() {
        if !super::is_valid_ts(thread) {
            return None;
        }
        // Slackは、まだ返信のない投稿のスレッドにはephemeralを表示しない。
        if thread != ts {
            body["thread_ts"] = json!(thread);
        }
    }
    Some(body)
}

/// 署名・アプリ・チームの確認とDBへの書き込みが済んでから呼ぶ。
/// API待ちを受信口へ持ち込まず、過負荷時は案内を省略する。
pub(super) fn dispatch(s: &AppState, payload: &Value, retry: bool) {
    let config = &s.config.slack;
    let Some(token) = config.import_bot_token.as_ref() else {
        return;
    };
    // ワークスペース共通のBotトークンを、別チームのイベントに使わない。
    if retry || config.team_id.as_deref() != payload["team_id"].as_str() || config.team_id.is_none()
    {
        return;
    }
    let Some(body) = notice(payload) else { return };
    let Ok(permit) = SENDING.try_acquire() else {
        return;
    };
    let key = format!(
        "{}:{}:{}",
        payload["team_id"], body["channel"], payload["event"]["ts"]
    );
    {
        let mut seen = SEEN.lock().expect("notice cache poisoned");
        seen.retain(|_, time| time.elapsed() < Duration::from_secs(600));
        if seen.contains_key(&key) || seen.len() >= 10000 {
            return;
        }
        seen.insert(key, Instant::now());
    }
    let client = s.http_client.clone();
    let token = token.clone();
    let url = format!(
        "{}/chat.postEphemeral",
        config.api_base_url.trim_end_matches('/')
    );
    tokio::spawn(async move {
        let _permit = permit;
        if !send(&client, &url, &token, &body).await {
            tracing::warn!("Slack migration notice could not be delivered");
        }
    });
}

async fn send(client: &reqwest::Client, url: &str, token: &str, body: &Value) -> bool {
    let result = client
        .post(url)
        .bearer_auth(token)
        .json(body)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => response
            .json::<Value>()
            .await
            .is_ok_and(|value| value["ok"] == true),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> Value {
        json!({"type":"event_callback", "team_id":"T1", "event":{
            "type":"message", "channel":"C1", "user":"U1", "ts":"1700000000.000001"}})
    }

    #[tokio::test]
    async fn posts_only_ephemeral_and_checks_slack_result() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, header, method, path},
        };
        let server = MockServer::start().await;
        let body = notice(&post()).unwrap();
        Mock::given(method("POST"))
            .and(path("/chat.postEphemeral"))
            .and(header("authorization", "Bearer test-token"))
            .and(body_json(body.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .expect(1)
            .mount(&server)
            .await;
        let url = format!("{}/chat.postEphemeral", server.uri());
        assert!(send(&reqwest::Client::new(), &url, "test-token", &body).await);
        server.verify().await;
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":false,"error":"missing_scope"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(!send(&reqwest::Client::new(), &url, "test-token", &body).await);
    }

    #[test]
    fn targets_only_author_and_correct_thread() {
        let mut payload = post();
        let body = notice(&payload).unwrap();
        assert_eq!(body["user"], "U1");
        assert!(body.get("thread_ts").is_none());
        assert!(body["text"].as_str().unwrap().contains("https://zli.chat"));
        payload["event"]["thread_ts"] = json!("1699999999.000001");
        assert_eq!(notice(&payload).unwrap()["thread_ts"], "1699999999.000001");
    }

    #[test]
    fn skips_bots_edits_deletes_and_system_events() {
        for subtype in [
            "bot_message",
            "message_changed",
            "message_deleted",
            "message_replied",
            "channel_join",
        ] {
            let mut payload = post();
            payload["event"]["subtype"] = json!(subtype);
            assert!(notice(&payload).is_none());
        }
        for field in ["bot_id", "app_id"] {
            let mut payload = post();
            payload["event"][field] = json!("B1");
            assert!(notice(&payload).is_none());
        }
        for (field, value) in [
            ("user", ""),
            ("user", "USLACKBOT"),
            ("ts", "bad"),
            ("thread_ts", "bad"),
        ] {
            let mut payload = post();
            payload["event"][field] = json!(value);
            assert!(notice(&payload).is_none());
        }
    }
}
