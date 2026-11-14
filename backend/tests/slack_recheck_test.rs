//! 直近 24 時間の履歴の再確認 (#293)。取りこぼした編集・削除を、30 分に 1 回の突き合わせで反映すること。Zoo で作った
//! 投稿や ZIP で取り込んだ投稿 (対応表にない) は、消えたと見なさないこと。履歴が不完全なときに大量に消さないこと。

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

fn m(ts: &str, text: &str) -> Value {
    json!({"type": "message", "ts": ts, "user": "UA", "text": text})
}

/// 履歴の応答を差し替える (再確認も、通常の同期も、同じ応答を受ける)
async fn history(f: &Fixture, messages: Value) {
    f.slack.reset().await;
    mount_basics(f).await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "messages": messages, "has_more": false})),
        )
        .with_priority(2)
        .mount(&f.slack)
        .await;
}

async fn trigger(f: &Fixture, due: bool) -> anyhow::Result<()> {
    sql(
        &f.state,
        &format!(
            "UPDATE slack_channel_syncs SET rechecked_at=now()-interval '{}' WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
            if due { "1 hour" } else { "1 minute" }
        ),
        &[&f.team],
    )
    .await?;
    deliver(f, &message_event(&f.team, "C1", &f.ts_new, None)).await
}

fn ts_at(offset: u64) -> String {
    format!("{}.000{:03}", now_secs() - 3000 + offset, offset % 1000)
}

#[tokio::test]
async fn test_recheck_applies_missed_edits_and_deletes_only_what_slack_dropped()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    let (t1, t2) = (ts_at(10), ts_at(20));
    history(&f, json!([m(&t1, "first"), m(&t2, "second")])).await;
    deliver(&f, &message_event(&f.team, "C1", &t1, None)).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 2);
    // Zoo で作った投稿 (対応表にない) は、Slack の履歴になくても消えない
    let user = sql(
        &f.state,
        "SELECT json_build_object('id',user_id)::text FROM slack_user_links WHERE team_id=$1",
        &[&f.team],
    )
    .await?[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let native = uuid::Uuid::new_v4().to_string();
    sql(
        &f.state,
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'native') RETURNING json_build_object('ok',true)::text",
        &[&native, &user, &f.channel],
    )
    .await?;

    // 30 分たっていなければ、突き合わせない (t2 が履歴から消えていても、そのまま)
    history(&f, json!([m(&t1, "first")])).await;
    trigger(&f, false).await?;
    assert_eq!(
        row(&f, &backend::slack::message_id(&f.team, "C1", &t2)).await["deleted"],
        false
    );

    // 30 分以上たったら、突き合わせる: t1 の編集を反映し、履歴から消えた t2 を削除する。Zoo の投稿は消さない
    let edited = format!("{}.000000", now_secs());
    history(
        &f,
        json!([{"type":"message","ts":&t1,"user":"UA","text":"first (edited)",
        "edited":{"user":"UA","ts":&edited}}]),
    )
    .await;
    trigger(&f, true).await?;
    let first = row(&f, &backend::slack::message_id(&f.team, "C1", &t1)).await;
    assert_eq!(first["content"], "first (edited)");
    assert_eq!(first["deleted"], false);
    assert_eq!(
        row(&f, &backend::slack::message_id(&f.team, "C1", &t2)).await["deleted"],
        true
    );
    assert_eq!(row(&f, &native).await["deleted"], false);
    Ok(())
}

#[tokio::test]
async fn test_recheck_does_not_delete_when_most_posts_seem_to_be_missing() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    let all: Vec<String> = (1..=4).map(|i| ts_at(i * 10)).collect();
    history(
        &f,
        json!(all.iter().map(|t| m(t, "post")).collect::<Vec<_>>()),
    )
    .await;
    deliver(&f, &message_event(&f.team, "C1", &all[0], None)).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 4);
    // 履歴が空 (API の不具合・権限) のときは、4 件すべてが消えたように見えても、削除しない
    history(&f, json!([])).await;
    trigger(&f, true).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 4);
    Ok(())
}

#[tokio::test]
async fn test_recheck_treats_a_tombstone_as_a_delete() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    let t1 = ts_at(10);
    history(&f, json!([m(&t1, "parent")])).await;
    deliver(&f, &message_event(&f.team, "C1", &t1, None)).await?;
    history(&f, json!([{"type":"message","subtype":"tombstone","ts":&t1,"text":"This message was deleted."}])).await;
    trigger(&f, true).await?;
    assert_eq!(
        row(&f, &backend::slack::message_id(&f.team, "C1", &t1)).await["deleted"],
        true
    );
    Ok(())
}

#[tokio::test]
async fn test_recheck_does_not_delete_a_reply_whose_parent_was_never_imported() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    // 4 日前の、取り込んでいない親への新しい返信を、イベントで取り込む (親が見つからず、Zoo では親なしになる)
    let parent = format!("{}.000100", now_secs() - 4 * 86_400);
    let reply = format!("{}.000900", now_secs() - 120);
    history(&f, json!([])).await;
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
    let id = backend::slack::message_id(&f.team, "C1", &reply);
    let imported = row(&f, &id).await;
    assert_eq!(imported["content"], "late reply");
    assert_eq!(imported["deleted"], false);
    // 履歴は返信を返さないが、再確認は、返信を「消えた」とは見なさない
    trigger(&f, true).await?;
    assert_eq!(row(&f, &id).await["deleted"], false);
    Ok(())
}

#[tokio::test]
async fn test_recheck_does_not_delete_when_the_history_is_incomplete() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    let (t1, t2, t3) = (ts_at(10), ts_at(20), ts_at(30));
    history(&f, json!([m(&t1, "a"), m(&t2, "b"), m(&t3, "c")])).await;
    deliver(&f, &message_event(&f.team, "C1", &t1, None)).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 3);

    // `has_more` なのに、次のカーソルがない: 最後まで取れていないので、t2・t3 が見えなくても、削除しない
    f.slack.reset().await;
    mount_basics(&f).await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "messages": [m(&t1, "a")], "has_more": true})),
        )
        .with_priority(2)
        .mount(&f.slack)
        .await;
    trigger(&f, true).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 3);

    // 解析できない投稿 (reactions が null) がある: その投稿を「履歴にない」と誤判定しない。t3 も、この回は削除しない
    history(
        &f,
        json!([m(&t1, "a"), {"type":"message","ts":&t2,"user":"UA","text":"b","reactions":null}]),
    )
    .await;
    trigger(&f, true).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 3);
    Ok(())
}

async fn failing_recheck(f: &Fixture, response: ResponseTemplate) -> anyhow::Result<()> {
    // 再確認の履歴の取得 (最初の履歴の呼び出し) だけが失敗する
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(response)
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&f.slack)
        .await;
    sql(
        &f.state,
        "UPDATE slack_channel_syncs SET rechecked_at=now()-interval '1 hour' WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    // `trigger` は、再確認の時期を戻すので使わず、合図だけ送る
    let (status, _) = post_event(
        &f.app,
        SECRET,
        now_secs(),
        &message_event(&f.team, "C1", &f.ts_new, None),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    age_wake(&f.state, &f.team, "10 seconds").await?;
    sql(
        &f.state,
        "UPDATE slack_channel_syncs SET available_at=now()+interval '1 hour',last_synced_at=now()-interval '1 hour',not_before=NULL WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    drain(&f.state).await;
    Ok(())
}

async fn retry_in_minutes(f: &Fixture) -> f64 {
    sql(
        &f.state,
        "SELECT json_build_object('m',extract(epoch FROM (rechecked_at+interval '30 minutes'-now()))/60)::text \
         FROM slack_channel_syncs WHERE team_id=$1",
        &[&f.team],
    )
    .await
    .unwrap()[0]["m"]
        .as_f64()
        .unwrap()
}

#[tokio::test]
async fn test_a_failing_recheck_does_not_stop_the_normal_sync() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    // 再確認が 429 以外で失敗しても、通常の同期は、同じ回に、新しい投稿を取り込む
    failing_recheck(&f, ResponseTemplate::new(500)).await?;
    assert_eq!(
        count_messages(&f.state, &f.channel).await,
        1,
        "the new post is imported"
    );
    // 再確認は、5 分後にもう一度試す (30 分の間隔の 5 分前)。30 分あとではない
    let minutes = retry_in_minutes(&f).await;
    assert!((3.0..=6.0).contains(&minutes), "{minutes}");
    Ok(())
}

#[tokio::test]
async fn test_a_rate_limited_recheck_waits_for_retry_after() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET), None).await?;
    // 再確認が 429 になったら、同じ `conversations.history` を、すぐには呼ばない (Retry-After を守る)
    failing_recheck(
        &f,
        ResponseTemplate::new(429).insert_header("retry-after", "600"),
    )
    .await?;
    assert_eq!(
        count_messages(&f.state, &f.channel).await,
        0,
        "waits for Retry-After"
    );
    let rows = sql(
        &f.state,
        "SELECT json_build_object('wait',extract(epoch FROM (not_before-now())))::text FROM slack_channel_syncs WHERE team_id=$1",
        &[&f.team],
    )
    .await?;
    assert!(rows[0]["wait"].as_f64().unwrap() > 500.0, "{}", rows[0]);
    // 待ちのあとは、再確認に妨げられず、通常の同期が新しい投稿を取り込む (再確認は、待ち (10 分) より 5 分あとに回っている)
    let minutes = retry_in_minutes(&f).await;
    assert!((14.0..=16.0).contains(&minutes), "{minutes}");
    sql(
        &f.state,
        "UPDATE slack_channel_syncs SET available_at=now(),not_before=NULL WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    drain(&f.state).await;
    assert_eq!(count_messages(&f.state, &f.channel).await, 1);
    Ok(())
}
