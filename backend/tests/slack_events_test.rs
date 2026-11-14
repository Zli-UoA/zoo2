//! Slack の Events API の受信口 (#293)。署名の検証と、合図で差分同期を早める仕組み
//! (デバウンス・最短間隔・待ち `not_before` を前倒ししないこと) を、モックの Slack API で確かめる。
//! 時間のかかる待ちは、DB の日時を書き換えて進める (sleep しない)。

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

#[tokio::test]
async fn test_events_endpoint_checks_the_signature() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET)).await?;
    let event = message_event(&f.team, "C1", &f.ts_new, None);
    let now = now_secs();

    // 署名が違う・古い要求は 401 で、何も書かない
    let (status, _) = post_event(&f.app, "wrong-secret", now, &event).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = post_event(&f.app, SECRET, now - 600, &event).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/slack/events")
                .body(Body::from(serde_json::to_vec(&event)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 0);

    // url_verification は、署名が正しければ challenge を返す
    let (status, text) = post_event(
        &f.app,
        SECRET,
        now,
        &json!({"type": "url_verification", "challenge": "abc123"}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text, "abc123");

    // 正しい署名の投稿は、合図が 1 行になる。再送 (同じ内容) でも 1 行のまま
    let (status, _) = post_event(&f.app, SECRET, now, &event).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_event(&f.app, SECRET, now, &event).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 1);
    Ok(())
}

#[tokio::test]
async fn test_events_endpoint_is_disabled_without_a_signing_secret() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(None).await?;
    let (status, _) = post_event(
        &f.app,
        SECRET,
        now_secs(),
        &message_event(&f.team, "C1", &f.ts_new, None),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 0);
    Ok(())
}

#[tokio::test]
async fn test_events_ignore_unregistered_and_non_post_events() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET)).await?;
    let now = now_secs();
    // 対応付けのないチャンネル・許可していないワークスペース・投稿でないイベント・入退室・編集と削除は、200 で何も書かない
    let other_channel = message_event(&f.team, "C999", &f.ts_new, None);
    let other_team = message_event("TOTHER", "C1", &f.ts_new, None);
    let mut reaction = message_event(&f.team, "C1", &f.ts_new, None);
    reaction["event"]["type"] = json!("reaction_added");
    let join = message_event(&f.team, "C1", &f.ts_new, Some("channel_join"));
    let changed = message_event(&f.team, "C1", &f.ts_new, Some("message_changed"));
    let deleted = message_event(&f.team, "C1", &f.ts_new, Some("message_deleted"));
    for event in [other_channel, other_team, reaction, join, changed, deleted] {
        let (status, _) = post_event(&f.app, SECRET, now, &event).await?;
        assert_eq!(status, StatusCode::OK, "{event}");
    }
    let rows = sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM slack_channel_wakes WHERE team_id=$1",
        &[&f.team],
    )
    .await?;
    assert_eq!(rows[0]["n"], 0);
    // 壊れた JSON は、署名が正しくても 200 で受け流す (再送しても直らない)
    let bytes = b"not json".to_vec();
    let ts = now.to_string();
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/slack/events")
                .header("x-slack-request-timestamp", &ts)
                .header("x-slack-signature", sign(SECRET, &ts, &bytes))
                .body(Body::from(bytes))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn test_wake_starts_a_sync_after_the_debounce_and_the_minimum_interval() -> anyhow::Result<()>
{
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET)).await?;
    // 合図がなければ、定期の実行 (1 時間後) まで同期しない
    drain(&f.state).await;
    assert_eq!(count_messages(&f.state, &f.channel).await, 0);

    let (status, _) = post_event(
        &f.app,
        SECRET,
        now_secs(),
        &message_event(&f.team, "C1", &f.ts_new, None),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    // デバウンス (最初の合図から 3 秒) の間は、まだ同期しない。合図は残る
    drain(&f.state).await;
    assert_eq!(count_messages(&f.state, &f.channel).await, 0);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 1);

    // 前回の同期から 30 秒たっていなければ、デバウンスが過ぎても同期しない
    age_wake(&f.state, &f.team, "10 seconds").await?;
    set_sync_state(&f.state, &f.team, "1 hour", "5 seconds").await?;
    drain(&f.state).await;
    assert_eq!(count_messages(&f.state, &f.channel).await, 0);

    // 最短間隔も過ぎたら、定期の実行を待たずに同期して、合図を消す
    set_sync_state(&f.state, &f.team, "1 hour", "1 hour").await?;
    drain(&f.state).await;
    assert_eq!(count_messages(&f.state, &f.channel).await, 1);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 0);
    // 反映したのは、差分同期と同じ (決定的 ID)
    let id = backend::slack::message_id(&f.team, "C1", &f.ts_new);
    let rows = sql(
        &f.state,
        "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
        &[&id],
    )
    .await?;
    assert_eq!(rows[0]["content"], "new post");

    // 同期のあとに届いた合図は、また残る (次の同期は、最短間隔のあと)
    let (status, _) = post_event(
        &f.app,
        SECRET,
        now_secs(),
        &message_event(&f.team, "C1", &f.ts_new, None),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 1);
    age_wake(&f.state, &f.team, "10 seconds").await?;
    drain(&f.state).await;
    let rows = sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM slack_channel_wakes WHERE team_id=$1",
        &[&f.team],
    )
    .await?;
    assert_eq!(rows[0]["n"], 1, "the minimum interval has not passed yet");
    // モックの Slack が受けた履歴の要求は、最初の同期の 1 回だけ
    let history = f
        .slack
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/conversations.history")
        .count();
    assert_eq!(history, 1);
    Ok(())
}

#[tokio::test]
async fn test_wake_does_not_override_a_backoff() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET)).await?;
    // 429 のあとは、Retry-After の間は、合図があっても同期しない
    f.slack.reset().await;
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {"id": "C1", "is_private": false},
        })))
        .mount(&f.slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "600"))
        .mount(&f.slack)
        .await;
    sql(
        &f.state,
        "UPDATE slack_channel_syncs SET available_at=now() WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    drain(&f.state).await;
    let rows = sql(
        &f.state,
        "SELECT json_build_object('wait',extract(epoch FROM (not_before-now())))::text \
         FROM slack_channel_syncs WHERE team_id=$1",
        &[&f.team],
    )
    .await?;
    assert!(rows[0]["wait"].as_f64().unwrap() > 500.0, "{}", rows[0]);

    // 合図が届いて、デバウンスと最短間隔が過ぎても、待ち (not_before) の間は同期しない
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
        "UPDATE slack_channel_syncs SET last_synced_at=now()-interval '1 hour' WHERE team_id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    let before = f.slack.received_requests().await.unwrap().len();
    drain(&f.state).await;
    assert_eq!(f.slack.received_requests().await.unwrap().len(), before);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 1);
    Ok(())
}

async fn history_calls(slack: &MockServer) -> usize {
    slack
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/conversations.history")
        .count()
}

#[tokio::test]
async fn test_wake_is_kept_while_a_run_resumes_and_starts_one_more_sync() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = fixture(Some(SECRET)).await?;
    // 途中まで進んだ同期 (`running`) の続き。再開では合図を消さない
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
        "UPDATE slack_channel_syncs t SET running=true,phase='history',run_oldest=sc.last_ts,available_at=now(),\
         last_synced_at=now()-interval '1 hour',lease_token=NULL,lease_until=NULL,not_before=NULL \
         FROM slack_channels sc WHERE t.team_id=$1 AND sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    drain(&f.state).await;
    assert_eq!(
        history_calls(&f.slack).await,
        1,
        "the resumed run reads the history"
    );
    assert_eq!(count_messages(&f.state, &f.channel).await, 1);
    // 再開の途中では消えなかった合図が、終わった同期のあとに残っている
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 1);

    // 終わった同期から 30 秒たてば、その合図でもう 1 回同期する
    sql(
        &f.state,
        "UPDATE slack_channel_syncs SET last_synced_at=now()-interval '1 minute',available_at=now()+interval '1 hour' \
         WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    drain(&f.state).await;
    assert_eq!(history_calls(&f.slack).await, 2);
    assert_eq!(wakes(&f.state, &f.team, "C1").await, 0);
    Ok(())
}
