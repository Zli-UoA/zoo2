//! ループ防止 (#293)。Zoo から Slack へ出した投稿 (`slack_message_map` の `zoo`、このアプリの `app_id`) を、Slack から
//! 取り込み直さないこと。Zoo で作った投稿への Slack の返信・リアクションは、その投稿に付くこと。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

use backend::{
    apps::query, config::Config, db::init_db, routes::create_router, slack::sync, state::AppState,
};

const SECRET: &str = "test-signing-secret";

/// `sync::run_once` は DB 上のどのチャンネルの項目でも拾う。設定の違うテスト同士が互いの項目を処理しないよう、
/// このファイルのテストは順に実行する。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn sign(secret: &str, timestamp: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("v0={hex}")
}

async fn post_event(
    app: &Router,
    secret: &str,
    timestamp: u64,
    body: &Value,
) -> anyhow::Result<(StatusCode, String)> {
    let bytes = serde_json::to_vec(body)?;
    let ts = timestamp.to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/slack/events")
                .header("content-type", "application/json")
                .header("x-slack-request-timestamp", &ts)
                .header("x-slack-signature", sign(secret, &ts, &bytes))
                .body(Body::from(bytes))?,
        )
        .await?;
    let status = response.status();
    let text = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, String::from_utf8_lossy(&text).into_owned()))
}

async fn sql(state: &AppState, statement: &str, params: &[&str]) -> anyhow::Result<Vec<Value>> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn create_user(app: &Router, name: &str) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/users")
                .header("X-Dev-User", &id)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "id": id,
                    "user_name": format!("{name}_{}", uuid::Uuid::new_v4().simple()),
                    "display_name": name,
                }))?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(id)
}

async fn create_channel(app: &Router, owner: &str) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/channels")
                .header("X-Dev-User", owner)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "id": id,
                    "channel_name": format!("events_{}", uuid::Uuid::new_v4().simple()),
                    "is_private": false,
                }))?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(id)
}

async fn drain(state: &AppState) {
    for _ in 0..100 {
        if !sync::run_once(state).await.unwrap() {
            return;
        }
    }
    panic!("the sync queue did not drain");
}

async fn count_messages(state: &AppState, channel: &str) -> i64 {
    sql(
        state,
        "SELECT json_build_object('n',count(*))::text FROM messages WHERE channel_id=$1 AND deleted_at IS NULL",
        &[channel],
    )
    .await
    .unwrap()[0]["n"]
        .as_i64()
        .unwrap()
}

async fn wakes(state: &AppState, team: &str, channel: &str) -> i64 {
    sql(
        state,
        "SELECT json_build_object('n',count(*))::text FROM slack_channel_wakes WHERE team_id=$1 AND slack_channel_id=$2",
        &[team, channel],
    )
    .await
    .unwrap()[0]["n"]
        .as_i64()
        .unwrap()
}

fn message_event(team: &str, channel: &str, ts: &str, subtype: Option<&str>) -> Value {
    let mut event =
        json!({"type": "message", "channel": channel, "user": "UA", "ts": ts, "text": "hello"});
    if let Some(subtype) = subtype {
        event["subtype"] = json!(subtype);
    }
    json!({"type": "event_callback", "team_id": team, "api_app_id": "A1", "event": event})
}

struct Fixture {
    state: AppState,
    app: Router,
    team: String,
    channel: String,
    slack: MockServer,
    ts_new: String,
}

/// 連携済みのユーザーと、対応付けたチャンネル (`C1`) と、同期の項目 (次の実行は 1 時間後、前回の同期は 1 時間前) を作る
async fn fixture(signing_secret: Option<&str>, app_id: Option<&str>) -> anyhow::Result<Fixture> {
    let key = [5u8; 32];
    let team = format!("TEVT{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let slack = MockServer::start().await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key = Some(base64::engine::general_purpose::STANDARD.encode(key));
    config.slack.api_base_url = slack.uri();
    config.slack.signing_secret = signing_secret.map(str::to_string);
    config.slack.app_id = app_id.map(str::to_string);
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, "events_alice").await?;
    let enc = backend::slack::crypto::encrypt(&key, "xoxp-alice")?;
    sql(
        &state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'UA',$3) \
         RETURNING json_build_object('ok',true)::text",
        &[&alice, &team, &enc],
    )
    .await?;
    let channel = create_channel(&app, &alice).await?;
    let cursor = format!("{}.000000", now_secs() - 7200);
    sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C1',$2,$3) \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &channel, &cursor],
    )
    .await?;
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        sync::enqueue_team(&mut tx, &team).await?;
        tx.commit().await?;
    }
    // 一度同期したあとの状態。定期の実行も、最短間隔も、まだ来ない (次の実行は 1 時間後)
    set_sync_state(&state, &team, "1 hour", "1 hour").await?;

    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {"id": "C1", "is_private": false},
        })))
        .mount(&slack)
        .await;
    let ts_new = format!("{}.000100", now_secs() - 60);
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [{"type": "message", "ts": &ts_new, "user": "UA", "text": "new post"}],
            "has_more": false,
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "messages": [], "has_more": false})),
        )
        .mount(&slack)
        .await;
    Ok(Fixture {
        state,
        app,
        team,
        channel,
        slack,
        ts_new,
    })
}

/// 同期の項目の日時を、今から `next` 後に次の実行・`last` 前に前回の同期、に書き換える。待ちは外す
async fn set_sync_state(
    state: &AppState,
    team: &str,
    next: &str,
    last: &str,
) -> anyhow::Result<()> {
    sql(
        state,
        &format!(
            "UPDATE slack_channel_syncs SET available_at=now()+interval '{next}',\
             last_synced_at=now()-interval '{last}',not_before=NULL,lease_token=NULL,lease_until=NULL,running=false \
             WHERE team_id=$1 RETURNING json_build_object('ok',true)::text"
        ),
        &[team],
    )
    .await?;
    Ok(())
}

/// 合図の最初の時刻を `ago` 前にして、デバウンスを過ぎた状態にする
async fn age_wake(state: &AppState, team: &str, ago: &str) -> anyhow::Result<()> {
    sql(
        state,
        &format!(
            "UPDATE slack_channel_wakes SET first_at=now()-interval '{ago}' WHERE team_id=$1 \
             RETURNING json_build_object('ok',true)::text"
        ),
        &[team],
    )
    .await?;
    Ok(())
}

fn reaction_event(
    team: &str,
    channel: &str,
    ts: &str,
    user: &str,
    name: &str,
    added: bool,
) -> Value {
    json!({"type": "event_callback", "team_id": team, "event": {
        "type": if added { "reaction_added" } else { "reaction_removed" },
        "user": user, "reaction": name,
        "item": {"type": "message", "channel": channel, "ts": ts}}})
}

fn changed_event(team: &str, channel: &str, ts: &str) -> Value {
    json!({"type": "event_callback", "team_id": team, "event": {
        "type": "message", "subtype": "message_changed", "channel": channel,
        "message": {"ts": ts, "text": "x"}}})
}

fn deleted_event(team: &str, channel: &str, ts: &str) -> Value {
    json!({"type": "event_callback", "team_id": team, "event": {
        "type": "message", "subtype": "message_deleted", "channel": channel, "deleted_ts": ts}})
}

/// イベントを受け、合図のデバウンスと最短間隔を過ぎた状態にして、同期を回す
async fn deliver(f: &Fixture, event: &Value) -> anyhow::Result<()> {
    let (status, _) = post_event(&f.app, SECRET, now_secs(), event).await?;
    assert_eq!(status, StatusCode::OK);
    sync_now(f).await
}

async fn sync_now(f: &Fixture) -> anyhow::Result<()> {
    // 合図がなくても (再試行など)、同期が始まるように、合図の行を作ってから、デバウンスと最短間隔を過ぎた状態にする
    sql(
        &f.state,
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) VALUES($1,'C1') \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now() RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    age_wake(&f.state, &f.team, "10 seconds").await?;
    set_sync_state(&f.state, &f.team, "1 hour", "1 hour").await?;
    drain(&f.state).await;
    Ok(())
}

/// 取り直しの要求 (`latest` が `ts`) への応答を、優先して返す
async fn mock_fetch(f: &Fixture, ts: &str, messages: Value) {
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        .and(query_param("latest", ts))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "messages": messages, "has_more": false})),
        )
        .with_priority(1)
        .mount(&f.slack)
        .await;
}

/// `reset` で消えたモックのうち、同期に要るものを入れ直す
async fn mount_basics(f: &Fixture) {
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "channel": {"id": "C1", "is_private": false}})),
        )
        .mount(&f.slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "messages": [], "has_more": false})),
        )
        .mount(&f.slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "messages": [], "has_more": false})),
        )
        .mount(&f.slack)
        .await;
}

/// Zoo で作った投稿を 1 つ用意し、Slack の `ts` に結び付ける (#294 が `chat.postMessage` のあとにすること)
async fn zoo_post(f: &Fixture, ts: &str) -> anyhow::Result<String> {
    let user = sql(
        &f.state,
        "SELECT json_build_object('id',user_id)::text FROM slack_user_links WHERE team_id=$1",
        &[&f.team],
    )
    .await?[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let id = uuid::Uuid::new_v4().to_string();
    sql(
        &f.state,
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'from zoo') RETURNING json_build_object('ok',true)::text",
        &[&id, &user, &f.channel],
    )
    .await?;
    sql(
        &f.state,
        "INSERT INTO slack_message_map(team_id,slack_channel_id,ts,message_id,origin) VALUES($1,'C1',$2,$3,'zoo') \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team, ts, &id],
    )
    .await?;
    Ok(id)
}

async fn row(f: &Fixture, id: &str) -> Value {
    sql(
        &f.state,
        "SELECT json_build_object('content',content,'deleted',deleted_at IS NOT NULL,'revision',revision)::text FROM messages WHERE id=$1",
        &[id],
    )
    .await
    .unwrap()
    .into_iter()
    .next()
    .unwrap_or(Value::Null)
}

#[tokio::test]
async fn test_sync_skips_posts_of_our_own_app_and_zoo_origin_posts() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), Some("A_ZOO")).await?;
    let echoed = format!("{}.000400", now_secs() - 120);
    let mapped_ts = format!("{}.000500", now_secs() - 90);
    let normal = format!("{}.000600", now_secs() - 60);
    let mapped_id = zoo_post(&f, &mapped_ts).await?;
    f.slack.reset().await;
    mount_basics(&f).await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok": true, "messages": [
            // このアプリの投稿 (app_id が一致): 取り込まない
            {"type":"message","ts":&echoed,"user":"UA","text":"bridge echo","app_id":"A_ZOO"},
            // `slack_message_map` にある (Zoo で作った) 投稿: 取り込まない
            {"type":"message","ts":&mapped_ts,"user":"UA","text":"from zoo"},
            // ほかのアプリや人の投稿: 取り込む
            {"type":"message","ts":&normal,"user":"UA","text":"normal","app_id":"A_OTHER"}],
            "has_more": false})),
        )
        .with_priority(2)
        .mount(&f.slack)
        .await;
    deliver(&f, &message_event(&f.team, "C1", &normal, None)).await?;
    // Zoo で作った 1 件と、取り込んだ 1 件だけ
    assert_eq!(count_messages(&f.state, &f.channel).await, 2);
    // どちらも、決定的 ID の行としては取り込まれていない
    for ts in [&echoed, &mapped_ts] {
        let id = backend::slack::message_id(&f.team, "C1", ts);
        assert!(row(&f, &id).await.is_null(), "{ts} was imported again");
    }
    assert_eq!(row(&f, &mapped_id).await["content"], "from zoo");
    Ok(())
}

#[tokio::test]
async fn test_slack_replies_and_reactions_attach_to_the_zoo_post() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    let parent_ts = format!("{}.000500", now_secs() - 90);
    let reply_ts = format!("{}.000700", now_secs() - 30);
    let mapped_id = zoo_post(&f, &parent_ts).await?;
    f.slack.reset().await;
    mount_basics(&f).await;
    // 返信: 親は Zoo の投稿になる
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("ts", parent_ts.as_str()))
        .and(query_param("latest", reply_ts.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
            "messages": [{"type":"message","ts":&reply_ts,"user":"UA","text":"slack reply","thread_ts":&parent_ts}],
            "has_more": false})))
        .with_priority(1)
        .mount(&f.slack)
        .await;
    let event = json!({"type":"event_callback","team_id":&f.team,"event":{"type":"message",
        "channel":"C1","user":"UA","ts":&reply_ts,"thread_ts":&parent_ts,"text":"slack reply"}});
    deliver(&f, &event).await?;
    let rows = sql(
        &f.state,
        "SELECT json_build_object('parent',parent_id)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(&f.team, "C1", &reply_ts)],
    )
    .await?;
    assert_eq!(rows[0]["parent"], mapped_id.as_str());

    // リアクション: Zoo の投稿に付く。Slack での編集・削除は、Zoo が正なので反映しない
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(
        &f,
        &parent_ts,
        json!([{"type":"message","ts":&parent_ts,"user":"UA","text":"edited on slack",
        "edited":{"user":"UA","ts":format!("{}.000000", now_secs())},
        "reactions":[{"name":"thumbsup","users":["UA"],"count":1}]}]),
    )
    .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &parent_ts, "UA", "thumbsup", true),
    )
    .await?;
    deliver(&f, &changed_event(&f.team, "C1", &parent_ts)).await?;
    let reactions = sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM message_reactions WHERE message_id=$1",
        &[&mapped_id],
    )
    .await?;
    assert_eq!(reactions[0]["n"], 1);
    let after = row(&f, &mapped_id).await;
    assert_eq!(after["content"], "from zoo");
    assert_eq!(after["deleted"], false);

    // Slack で削除されても、Zoo の投稿は消えない
    f.slack.reset().await;
    mount_basics(&f).await;
    deliver(&f, &deleted_event(&f.team, "C1", &parent_ts)).await?;
    assert_eq!(row(&f, &mapped_id).await["deleted"], false);
    Ok(())
}

#[tokio::test]
async fn test_events_for_another_app_are_ignored() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), Some("A_ZOO")).await?;
    let mut event = message_event(&f.team, "C1", &f.ts_new, None);
    event["api_app_id"] = json!("A_OTHER");
    let (status, _) = post_event(&f.app, SECRET, now_secs(), &event).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 0);
    event["api_app_id"] = json!("A_ZOO");
    let (status, _) = post_event(&f.app, SECRET, now_secs(), &event).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 1);
    Ok(())
}
