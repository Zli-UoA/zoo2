//! Slack の Events API が知らせた「確かめ直すメッセージ」の反映 (#293)。編集・削除・tombstone・リアクションの
//! 追加と削除・範囲外のスレッドの返信・Zoo にないメッセージ・失敗し続けるヒントを、モックの Slack API で確かめる。

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
async fn fixture(signing_secret: Option<&str>) -> anyhow::Result<Fixture> {
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

/// ヒントを処理したあとの投稿を取り込んである状態にする (Zoo に `ts_new` の投稿「new post」がある)
async fn imported(signing: bool) -> anyhow::Result<Fixture> {
    let f = fixture(signing.then_some(SECRET)).await?;
    deliver(&f, &message_event(&f.team, "C1", &f.ts_new, None)).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 1);
    Ok(f)
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

async fn message_row(f: &Fixture, ts: &str) -> Value {
    let id = backend::slack::message_id(&f.team, "C1", ts);
    sql(
        &f.state,
        "SELECT json_build_object('content',content,'revision',revision,'deleted',deleted_at IS NOT NULL,\
         'updated',extract(epoch FROM updated_at))::text FROM messages WHERE id=$1",
        &[&id],
    )
    .await
    .unwrap()
    .into_iter()
    .next()
    .unwrap_or(Value::Null)
}

async fn hints(f: &Fixture) -> Vec<Value> {
    sql(
        &f.state,
        "SELECT json_build_object('kind',kind,'attempts',attempts)::text FROM slack_message_hints WHERE team_id=$1 ORDER BY kind",
        &[&f.team],
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn test_hints_apply_an_edit_only_when_it_is_a_real_newer_edit() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    let before = message_row(&f, &ts).await;
    assert_eq!(before["content"], "new post");

    // `edited` のない変更 (リンクの展開など) では、本文が違っても書き換えない
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","ts":&ts,"user":"UA","text":"unfurled"}]),
    )
    .await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    assert_eq!(message_row(&f, &ts).await["content"], "new post");
    assert!(hints(&f).await.is_empty(), "the hint is consumed");

    // 本物の編集 (edited.ts が updated_at より新しい) は、本文を更新して revision を上げる
    f.slack.reset().await;
    let edited = now_secs();
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","ts":&ts,"user":"UA","text":"edited post",
        "edited":{"user":"UA","ts":format!("{edited}.000000")}}]),
    )
    .await;
    mount_basics(&f).await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    let after = message_row(&f, &ts).await;
    assert_eq!(after["content"], "edited post");
    assert_eq!(
        after["revision"].as_i64().unwrap(),
        before["revision"].as_i64().unwrap() + 1
    );

    // 同じ編集の再送、または古い編集では、書き換えない
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    assert_eq!(message_row(&f, &ts).await["revision"], after["revision"]);
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","ts":&ts,"user":"UA","text":"older edit",
        "edited":{"user":"UA","ts":format!("{}.000000", edited - 3600)}}]),
    )
    .await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    assert_eq!(message_row(&f, &ts).await["content"], "edited post");
    Ok(())
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

#[tokio::test]
async fn test_hints_delete_when_slack_says_it_is_gone_and_retry_while_it_still_exists()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    // API がまだメッセージを返すうちは、削除せず、ヒントを残して試行回数を増やす
    deliver(&f, &deleted_event(&f.team, "C1", &ts)).await?;
    assert_eq!(message_row(&f, &ts).await["deleted"], false);
    let rows = hints(&f).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0]["kind"].as_str(), rows[0]["attempts"].as_i64()),
        (Some("deleted"), Some(1))
    );

    // 消えたら (取り直しが成功して「ない」と答える)、論理削除してヒントを消す
    f.slack.reset().await;
    mount_basics(&f).await;
    sync_now(&f).await?;
    assert_eq!(message_row(&f, &ts).await["deleted"], true);
    assert!(hints(&f).await.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_hints_treat_a_tombstone_as_a_delete() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","subtype":"tombstone","ts":&ts,
        "text":"This message was deleted.","user":"USLACKBOT"}]),
    )
    .await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    assert_eq!(message_row(&f, &ts).await["deleted"], true);
    Ok(())
}

async fn reactions_of(f: &Fixture, ts: &str) -> i64 {
    let id = backend::slack::message_id(&f.team, "C1", ts);
    sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM message_reactions WHERE message_id=$1",
        &[&id],
    )
    .await
    .unwrap()[0]["n"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn test_hints_add_and_remove_reactions_with_normalized_emoji() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    let with = |reactions: Value| json!([{"type":"message","ts":&ts,"user":"UA","text":"new post","reactions":reactions}]);

    // 追加
    mock_fetch(
        &f,
        &ts,
        with(json!([{"name":"thumbsup","users":["UA"],"count":1}])),
    )
    .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", true),
    )
    .await?;
    assert_eq!(reactions_of(&f, &ts).await, 1);

    // 肌色の違う名前でも、同じ絵文字なら「まだいる」ので消さない
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(
        &f,
        &ts,
        with(json!([{"name":"thumbsup::skin-tone-3","users":["UA"],"count":1}])),
    )
    .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", false),
    )
    .await?;
    assert_eq!(reactions_of(&f, &ts).await, 1);

    // users が一部しか返らない (count のほうが多い) ときは reactions.get で確かめ、いるなら消さない
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(
        &f,
        &ts,
        with(json!([{"name":"thumbsup","users":["UB"],"count":2}])),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/reactions.get"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
            "message": {"reactions": [{"name":"thumbsup","users":["UB","UA"],"count":2}]}})))
        .mount(&f.slack)
        .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", false),
    )
    .await?;
    // 全員そろった API の状態にいる UB のリアクションは、追加される (UA の行は消さない)
    assert_eq!(reactions_of(&f, &ts).await, 2);

    // 確かめられない (reactions.get が失敗) ときは消さず、ヒントを残す
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(
        &f,
        &ts,
        with(json!([{"name":"thumbsup","users":["UB"],"count":2}])),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/reactions.get"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&f.slack)
        .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", false),
    )
    .await?;
    assert_eq!(reactions_of(&f, &ts).await, 2);
    assert_eq!(hints(&f).await.len(), 1);

    // 完全に消えていれば、その人の 1 行だけを消す (UB のリアクションは残る)
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(&f, &ts, with(json!([]))).await;
    sync_now(&f).await?;
    assert_eq!(reactions_of(&f, &ts).await, 1);
    assert!(hints(&f).await.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_hints_attach_a_reply_outside_the_watched_threads_to_its_parent() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let reply = format!("{}.000900", now_secs() - 30);
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("ts", parent.as_str()))
        .and(query_param("latest", reply.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
            "messages": [{"type":"message","ts":&reply,"user":"UA","text":"late reply","thread_ts":&parent}],
            "has_more": false})))
        .with_priority(1)
        .mount(&f.slack)
        .await;
    let event = json!({"type":"event_callback","team_id":&f.team,"event":{"type":"message",
        "channel":"C1","user":"UA","ts":&reply,"thread_ts":&parent,"text":"late reply"}});
    deliver(&f, &event).await?;
    let rows = sql(
        &f.state,
        "SELECT json_build_object('parent',parent_id,'content',content)::text FROM messages WHERE id=$1",
        &[&backend::slack::message_id(&f.team, "C1", &reply)],
    )
    .await?;
    assert_eq!(rows[0]["content"], "late reply");
    assert_eq!(
        rows[0]["parent"],
        backend::slack::message_id(&f.team, "C1", &parent).as_str()
    );
    Ok(())
}

#[tokio::test]
async fn test_hints_do_not_create_messages_zoo_does_not_have() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let old = format!("{}.000300", now_secs() - 86_400);
    mock_fetch(
        &f,
        &old,
        json!([{"type":"message","ts":&old,"user":"UA","text":"old post",
        "edited":{"user":"UA","ts":format!("{}.000000", now_secs())},
        "reactions":[{"name":"thumbsup","users":["UA"],"count":1}]}]),
    )
    .await;
    deliver(&f, &changed_event(&f.team, "C1", &old)).await?;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &old, "UA", "thumbsup", true),
    )
    .await?;
    deliver(&f, &deleted_event(&f.team, "C1", &old)).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 1);
    assert!(message_row(&f, &old).await.is_null());
    assert!(
        hints(&f).await.iter().all(|h| h["kind"] == "deleted"),
        "{:?}",
        hints(&f).await
    );
    Ok(())
}

#[tokio::test]
async fn test_hints_stop_after_repeated_failures() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("latest", ts.as_str()))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&f.slack)
        .await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    let rows = hints(&f).await;
    assert_eq!(rows[0]["attempts"], 1);
    assert_eq!(message_row(&f, &ts).await["content"], "new post");
    // 試行の上限に達したヒントは、取り直さない
    sql(
        &f.state,
        "UPDATE slack_message_hints SET attempts=5 WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    let before = fetch_calls(&f.slack).await;
    sync_now(&f).await?;
    assert_eq!(fetch_calls(&f.slack).await, before);
    Ok(())
}

async fn fetch_calls(slack: &MockServer) -> usize {
    slack
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.query_pairs().any(|(k, _)| k == "latest"))
        .count()
}

#[tokio::test]
async fn test_events_record_one_hint_per_message_and_kind() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET)).await?;
    let ts = f.ts_new.clone();
    let reply = json!({"type":"event_callback","team_id":&f.team,"event":{"type":"message",
        "channel":"C1","user":"UA","ts":&ts,"thread_ts":"1700000000.000100","text":"r"}});
    for event in [
        reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", true),
        reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", false),
        changed_event(&f.team, "C1", &ts),
        changed_event(&f.team, "C1", &ts),
        deleted_event(&f.team, "C1", &ts),
        reply,
        // 対応付けのないチャンネルには、何も書かない
        reaction_event(&f.team, "C999", &ts, "UA", "thumbsup", true),
    ] {
        let (status, _) = post_event(&f.app, SECRET, now_secs(), &event).await?;
        assert_eq!(status, StatusCode::OK);
    }
    let kinds: Vec<String> = hints(&f)
        .await
        .iter()
        .map(|h| h["kind"].as_str().unwrap().to_string())
        .collect();
    // 同じ (メッセージ, 種類, ユーザー, リアクション) は 1 行にまとまる。追加と削除は同じ行
    assert_eq!(kinds, ["changed", "deleted", "reaction", "reply"]);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 1);
    assert_eq!(wakes(&f.state, &f.team, "C999").await, 0);
    Ok(())
}

#[tokio::test]
async fn test_a_new_event_revives_a_hint_that_hit_the_attempt_limit() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET)).await?;
    let ts = f.ts_new.clone();
    let (status, _) = post_event(
        &f.app,
        SECRET,
        now_secs(),
        &changed_event(&f.team, "C1", &ts),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    sql(
        &f.state,
        "UPDATE slack_message_hints SET attempts=5,created_at=now()-interval '2 days' WHERE team_id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    // 上限に達した行 (掃除待ち) に、同じメッセージの新しいイベントが届いたら、新しい確認として処理できる状態に戻す
    let (status, _) = post_event(
        &f.app,
        SECRET,
        now_secs(),
        &changed_event(&f.team, "C1", &ts),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let rows = sql(
        &f.state,
        "SELECT json_build_object('attempts',attempts,'fresh',created_at>now()-interval '1 minute')::text \
         FROM slack_message_hints WHERE team_id=$1",
        &[&f.team],
    )
    .await?;
    assert_eq!(
        (rows[0]["attempts"].as_i64(), rows[0]["fresh"].as_bool()),
        (Some(0), Some(true))
    );
    Ok(())
}

#[tokio::test]
async fn test_a_truncated_list_of_another_emoji_does_not_block_a_removal() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    let id = backend::slack::message_id(&f.team, "C1", &ts);
    let heart = backend::slack::convert::reaction_emoji("heart").unwrap();
    let user = sql(
        &f.state,
        "SELECT json_build_object('id',user_id)::text FROM slack_users WHERE team_id=$1 AND slack_user_id='UA'",
        &[&f.team],
    )
    .await?[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    sql(
        &f.state,
        "INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES($1,$2,$3,$4) RETURNING json_build_object('ok',true)::text",
        &[&uuid::Uuid::new_v4().to_string(), &id, &user, &heart],
    )
    .await?;
    // 無関係な thumbsup の users が一部 (count のほうが多い) でも、heart の削除は止めない (reactions.get は呼ばない)
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","ts":&ts,"user":"UA","text":"new post",
        "reactions":[{"name":"thumbsup","users":["UB"],"count":3}]}]),
    )
    .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &ts, "UA", "heart", false),
    )
    .await?;
    let hearts = sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM message_reactions WHERE message_id=$1 AND emoji=$2",
        &[&id, &heart],
    )
    .await?;
    assert_eq!(hearts[0]["n"], 0, "the heart was removed");
    assert!(hints(&f).await.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_additions_use_the_full_reaction_list_when_users_were_truncated() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","ts":&ts,"user":"UA","text":"new post",
        "reactions":[{"name":"thumbsup","users":["UB"],"count":3}]}]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/reactions.get"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
            "message": {"reactions": [{"name":"thumbsup","users":["UB","UC","UA"],"count":3}]}})))
        .mount(&f.slack)
        .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", true),
    )
    .await?;
    // 省略された UC も、完全な一覧から入る (UB・UC・UA の 3 行)
    assert_eq!(reactions_of(&f, &ts).await, 3);
    Ok(())
}
