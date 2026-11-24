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

/// `pg_stat_activity` のロック待ちの観測に、アプリの DB プールとは別の独立した接続を使う。
/// 同期タスクがプールの接続を持ったままロック待ちに入っても、観測側が同じプールで
/// 接続を奪い合って止まらないようにする (接続・確認ともにタイムアウト付き)。
async fn observe_connect(state: &AppState) -> anyhow::Result<tokio_postgres::Client> {
    let (client, driver) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio_postgres::connect(&state.config.database_url, tokio_postgres::NoTls),
    )
    .await??;
    tokio::spawn(driver);
    Ok(client)
}

/// 誰かが `pid` の持つロックを待っている状態になるまで待つ (1 回の確認・全体にタイムアウト)。
async fn wait_blocked_on(
    observer: &tokio_postgres::Client,
    pid: i32,
    rounds: usize,
) -> anyhow::Result<bool> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for _ in 0..rounds {
            let waiting: bool = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                observer.query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                     WHERE wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)))",
                    &[&pid],
                ),
            )
            .await??
            .get(0);
            if waiting {
                return Ok(true);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok(false)
    })
    .await?
}

/// 失敗時のメッセージ用に、現在のロック待ちの一覧を取る (ベストエフォート・タイムアウト付き)。
async fn dump_lock_waiters(observer: &tokio_postgres::Client) -> String {
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        observer.query(
            "SELECT format('%s|%s|%s|%s', pid, coalesce(wait_event_type,''), \
             coalesce(wait_event,''), left(coalesce(query,''),120)) \
             FROM pg_stat_activity WHERE wait_event_type='Lock'",
            &[],
        ),
    )
    .await
    {
        Ok(Ok(rows)) => rows
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => "pg_stat_activity lock waiters unavailable".into(),
    }
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
    // 環境変数 SLACK_APP_ID に依らない (`message_event` の api_app_id と食い違うと、受信口がイベントを無視する)
    config.slack.app_id = None;
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
             last_synced_at=now()-interval '{last}',rechecked_at=now(),not_before=NULL,lease_token=NULL,lease_until=NULL,running=false \
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

/// 書式の変換を入れる前に取り込んだ bot の投稿は、書式の違いだけでは書き換えない (「編集済み」にしない)。
/// 本文が実際に変わったときは、これまでどおり書き換える
#[tokio::test]
async fn test_hints_do_not_rewrite_a_bot_post_for_a_formatting_only_difference()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    let id = backend::slack::message_id(&f.team, "C1", &ts);
    // 変換を入れる前の保存済みの本文 (記号がそのまま)
    sql(
        &f.state,
        "UPDATE messages SET content='*重要*' WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&id],
    )
    .await?;
    let before = message_row(&f, &ts).await;

    // 本文が同じ (`message_changed` はリンクの展開などで届く)。新しい変換では `**重要**` になるが、書き換えない
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","subtype":"bot_message","bot_id":"B1","ts":&ts,"user":"UA","text":"*重要*"}]),
    )
    .await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    let same = message_row(&f, &ts).await;
    assert_eq!(same["content"], "*重要*");
    assert_eq!(same["revision"], before["revision"]);
    assert_eq!(same["updated"], before["updated"]);

    // 本文が実際に変わったときは、新しい変換で書き換える
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","subtype":"bot_message","bot_id":"B1","ts":&ts,"user":"UA","text":"*重要* です"}]),
    )
    .await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    let changed = message_row(&f, &ts).await;
    assert_eq!(changed["content"], "**重要** です");
    assert_eq!(
        changed["revision"].as_i64().unwrap(),
        before["revision"].as_i64().unwrap() + 1
    );
    Ok(())
}

#[tokio::test]
async fn test_hints_do_not_rewrite_a_bot_post_imported_as_a_legacy_card() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    let id = backend::slack::message_id(&f.team, "C1", &ts);
    // 部品の導入前の取り込み (本文と `card` だけ。部品は空)
    // 当時の `encode_cards` は JSON の値を経由するので、キーは辞書順になる
    let card = r##"{"author":"","badge":"","color":"#2eb886","context":"","description":"Healthy","source":"","title":"Ready","url":"https://example.com"}"##;
    sql(
        &f.state,
        "UPDATE messages SET content='Ready\nHealthy',card=$2,components=NULL WHERE id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&id, card],
    )
    .await?;
    let before = message_row(&f, &ts).await;
    let attachment = |text: &str| {
        json!([{"type":"message","subtype":"bot_message","bot_id":"B1","ts":&ts,"text":"",
            "attachments":[{"title":"Ready","title_link":"https://example.com","text":text,"color":"good"}]}])
    };

    // Slack 側は同じ。部品の形式は違うが、書き換えない
    mock_fetch(&f, &ts, attachment("Healthy")).await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    let same = message_row(&f, &ts).await;
    assert_eq!(same["revision"], before["revision"]);
    assert_eq!(same["updated"], before["updated"]);

    // attachments が実際に変わったときは、部品で書き換える
    f.slack.reset().await;
    mount_basics(&f).await;
    mock_fetch(&f, &ts, attachment("Degraded")).await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    let changed = message_row(&f, &ts).await;
    assert_eq!(
        changed["revision"].as_i64().unwrap(),
        before["revision"].as_i64().unwrap() + 1
    );
    let row = sql(
        &f.state,
        "SELECT json_build_object('card',card,'components',components)::text FROM messages WHERE id=$1",
        &[&id],
    )
    .await?;
    assert!(row[0]["card"].is_null());
    assert!(row[0]["components"].as_str().unwrap().contains("Degraded"));
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

/// Slack のスレッドの返信 (`ts`) を、返信のイベントと取り直しの応答で、Zoo に取り込む
async fn deliver_reply(f: &Fixture, parent: &str, ts: &str) -> anyhow::Result<()> {
    deliver_reply_by(f, parent, ts, "UA").await
}

/// Slack のユーザー `user` が、スレッドに返信した (`ts`) として、Zoo に取り込む
async fn deliver_reply_by(f: &Fixture, parent: &str, ts: &str, user: &str) -> anyhow::Result<()> {
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("ts", parent))
        .and(query_param("latest", ts))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
            "messages": [{"type":"message","ts":ts,"user":user,"text":"a reply","thread_ts":parent}],
            "has_more": false})))
        .with_priority(1)
        .mount(&f.slack)
        .await;
    let event = json!({"type":"event_callback","team_id":&f.team,"event":{"type":"message",
        "channel":"C1","user":user,"ts":ts,"thread_ts":parent,"text":"a reply"}});
    deliver(f, &event).await
}

/// スレッドの親が閉じているか、と、親の投稿者・返信者の参加の状態を返す。
/// `(archived, [参加行の left_at が NULL でないか])`
async fn thread_state(f: &Fixture, parent_id: &str) -> anyhow::Result<(bool, Vec<bool>)> {
    let rows = sql(
        &f.state,
        "SELECT json_build_object('archived',m.thread_archive_at IS NOT NULL,\
         'left',COALESCE((SELECT json_agg(p.left_at IS NOT NULL ORDER BY p.user_id) FROM thread_participants p \
           WHERE p.thread_id=m.id),'[]'::json))::text FROM messages m WHERE m.id=$1",
        &[parent_id],
    )
    .await?;
    let archived = rows[0]["archived"] == true;
    let left = rows[0]["left"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v == true)
        .collect();
    Ok((archived, left))
}

/// Slack の直近の返信は、zoo の返信と同じライブの投稿として、閉じたスレッドを再Openし、退出済みの投稿者を再参加させる。
/// 古い返信 (履歴) と、同じ返信の再送では、何も変えない
#[tokio::test]
async fn test_slack_live_reply_reopens_the_thread_and_rejoins_the_replier() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);
    // 最初の返信。親の投稿者と返信者が参加する (参加は、これまでどおり)
    let first = format!("{}.000200", now_secs() - 50);
    deliver_reply(&f, &parent, &first).await?;
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(!archived);
    assert!(!left.is_empty() && left.iter().all(|l| !l), "{left:?}");

    // 閉じて、全員が返信より前に退出したあとの、直近の返信は、再Openして再参加させる
    close_and_leave(&f, &parent_id, "5 minutes").await?;
    let second = format!("{}.000300", now_secs() - 40);
    deliver_reply(&f, &parent, &second).await?;
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(!archived, "a live Slack reply reopens the thread");
    assert!(left.iter().all(|l| !l), "the replier rejoins: {left:?}");

    // 同じ返信の再送 (すでにある行) は、閉じたスレッドを開かず、退出も取り消さない
    close_and_leave(&f, &parent_id, "5 minutes").await?;
    deliver_reply(&f, &parent, &second).await?;
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(archived, "a resend does not reopen the thread");
    assert!(
        left.iter().all(|l| *l),
        "a resend does not rejoin: {left:?}"
    );

    // 期間より古い返信は、履歴として取り込む。再Openも再参加もしない
    let old = format!("{}.000400", now_secs() - 3 * 86_400);
    deliver_reply(&f, &parent, &old).await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, 4);
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(archived, "an old reply is history");
    assert!(
        left.iter().all(|l| *l),
        "an old reply does not rejoin: {left:?}"
    );
    Ok(())
}

/// 閉じて、全員が `left_ago` 前に退出した状態にする
async fn close_and_leave(f: &Fixture, parent_id: &str, left_ago: &str) -> anyhow::Result<()> {
    let rows = sql(
        &f.state,
        &format!(
            "WITH a AS (UPDATE messages SET thread_archive_at=now() AT TIME ZONE 'UTC' WHERE id=$1 RETURNING id), \
                  l AS (UPDATE thread_participants SET left_at=(now() AT TIME ZONE 'UTC')-interval '{left_ago}' \
                        WHERE thread_id=$1 RETURNING thread_id) \
             SELECT json_build_object('a',(SELECT count(*) FROM a),'l',(SELECT count(*) FROM l))::text"
        ),
        &[parent_id],
    )
    .await?;
    assert_eq!(rows[0]["a"], 1, "{rows:?}");
    assert!(rows[0]["l"].as_i64().unwrap() >= 1, "{rows:?}");
    Ok(())
}

/// 返信より後に zoo で退出した人は、遅れて取り込んだ返信では再参加しない。返信より後の退出でなければ、再参加する。
/// 再Openは、返信の時刻が活動時刻を進めるので、どちらでも行う
#[tokio::test]
async fn test_slack_reply_older_than_the_leave_does_not_rejoin() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);
    deliver_reply(&f, &parent, &format!("{}.000200", now_secs() - 55)).await?;

    // 5 分前に退出した人の、10 分前の返信 (退出より前に書いた返信) を、いま取り込む
    close_and_leave(&f, &parent_id, "5 minutes").await?;
    deliver_reply(&f, &parent, &format!("{}.000300", now_secs() - 600)).await?;
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(!archived, "the reply still extends the thread's activity");
    assert!(
        left.iter().all(|l| *l),
        "a reply older than the leave does not rejoin: {left:?}"
    );

    // 5 分前に退出した人の、1 分前の返信 (退出のあとに書いた返信) は、再参加させる
    close_and_leave(&f, &parent_id, "5 minutes").await?;
    deliver_reply(&f, &parent, &format!("{}.000400", now_secs() - 60)).await?;
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(!archived);
    assert!(
        left.iter().all(|l| !l),
        "a reply after the leave rejoins: {left:?}"
    );
    Ok(())
}

/// Slack の返信の時刻が、DB の時計より先（時計のずれ）でも、返信より後に確定した退出は取り消さない。
/// 返信の時刻が先だと、`created_at >= トランザクションの開始時刻` が真になる。その場合も、取り込みの返信は
/// `left_at < created_at` のときだけ再参加させる（zoo の返信は、従来どおり常に再参加する）
#[tokio::test]
async fn test_slack_reply_dated_ahead_of_the_db_clock_does_not_rejoin_a_later_leave()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);
    let first = format!("{}.000200", now_secs() - 50);
    deliver_reply(&f, &parent, &first).await?;
    let ua = author_of(&f, &first).await;

    // 閉じて、UA が、これから取り込む返信（30 秒先の時刻）より後（2 分先）に退出した状態にする
    sql(&f.state, close_thread_sql(), &[&parent_id]).await?;
    sql(
        &f.state,
        "UPDATE thread_participants SET left_at=(now() AT TIME ZONE 'UTC')+interval '2 minutes' \
         WHERE thread_id=$1 AND user_id=$2 RETURNING json_build_object('ok',true)::text",
        &[&parent_id, &ua],
    )
    .await?;
    deliver_reply(&f, &parent, &format!("{}.000300", now_secs() + 30)).await?;
    assert!(
        !archived(&f, &parent_id).await,
        "the reply still extends the thread's activity"
    );
    assert_eq!(
        left_of(&f, &parent_id, &ua).await,
        Some(true),
        "a reply dated before the leave does not rejoin, even ahead of the DB clock"
    );
    Ok(())
}

/// 期間 (24 時間) の端に近い返信は、再Openしても、活動時刻がすぐ期間を外れて閉じ直されるので、履歴として扱う
#[tokio::test]
async fn test_slack_reply_near_the_end_of_the_idle_window_is_history() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);
    deliver_reply(&f, &parent, &format!("{}.000200", now_secs() - 55)).await?;

    // 24 時間より 5 分手前 (余裕の 10 分の内側) は、再Openも再参加もしない
    close_and_leave(&f, &parent_id, "1 minute").await?;
    deliver_reply(
        &f,
        &parent,
        &format!("{}.000300", now_secs() - (86_400 - 300)),
    )
    .await?;
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(archived, "a reply at the end of the window is history");
    assert!(left.iter().all(|l| *l), "{left:?}");

    // 24 時間より 20 分手前 (余裕の外側) は、直近の返信として、再Openし、再参加させる
    deliver_reply(
        &f,
        &parent,
        &format!("{}.000400", now_secs() - (86_400 - 1_200)),
    )
    .await?;
    let (archived, _) = thread_state(&f, &parent_id).await?;
    assert!(!archived, "a reply outside the margin is live");
    Ok(())
}

/// 差分同期 (ページ単位の書き込み) の経路でも、直近の返信だけが再Openし、古い返信は履歴として入るだけ
#[tokio::test]
async fn test_sync_reopens_for_a_recent_reply_but_not_for_an_old_one() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);
    deliver_reply(&f, &parent, &format!("{}.000200", now_secs() - 90)).await?;
    close_and_leave(&f, &parent_id, "1 hour").await?;
    let before = count_messages(&f.state, &f.channel).await;

    // スレッドの返信の取得に、1 回ずつ、決めた返信を返す (呼ばれた順に使う)
    let serve = |reply: String| {
        let slack = &f.slack;
        let parent = parent.clone();
        async move {
            Mock::given(method("GET"))
                .and(path("/conversations.replies"))
                .and(query_param("ts", parent.as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
                    "messages": [
                        {"type":"message","ts":&parent,"user":"UA","text":"new post","thread_ts":&parent,"reply_count":2},
                        {"type":"message","ts":&reply,"user":"UA","text":"swept reply","thread_ts":&parent}],
                    "has_more": false})))
                .up_to_n_times(1)
                .with_priority(1)
                .mount(slack)
                .await;
        }
    };
    // 直前の同期から 5 分以内でないことにして、監視中のスレッドを全部取り直す同期にする
    let sweep = || async {
        sql(
            &f.state,
            "UPDATE slack_channel_syncs SET swept_at=NULL WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
            &[&f.team],
        )
        .await?;
        sync_now(&f).await
    };

    // 3 日前の返信を、同期が取り込む: 再Openも再参加もしない
    serve(format!("{}.000500", now_secs() - 3 * 86_400)).await;
    sweep().await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, before + 1);
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(archived, "an old reply swept in is history");
    assert!(left.iter().all(|l| *l), "{left:?}");

    // 30 秒前の返信を、同期が取り込む: 再Openし、再参加させる
    serve(format!("{}.000600", now_secs() - 30)).await;
    sweep().await?;
    assert_eq!(count_messages(&f.state, &f.channel).await, before + 2);
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(!archived, "a recent reply swept in reopens the thread");
    assert!(left.iter().all(|l| !l), "{left:?}");
    Ok(())
}

/// 取り込んだ投稿の投稿者 (Zoo のユーザー ID)
async fn author_of(f: &Fixture, ts: &str) -> String {
    let id = backend::slack::message_id(&f.team, "C1", ts);
    sql(
        &f.state,
        "SELECT json_build_object('user',user_id)::text FROM messages WHERE id=$1",
        &[&id],
    )
    .await
    .unwrap()[0]["user"]
        .as_str()
        .unwrap()
        .to_string()
}

/// スレッドの親が閉じているか (参加の状態は変えない)
async fn archived(f: &Fixture, parent_id: &str) -> bool {
    sql(
        &f.state,
        "SELECT json_build_object('a',thread_archive_at IS NOT NULL)::text FROM messages WHERE id=$1",
        &[parent_id],
    )
    .await
    .unwrap()[0]["a"]
        == true
}

fn close_thread_sql() -> &'static str {
    "UPDATE messages SET thread_archive_at=now() AT TIME ZONE 'UTC' WHERE id=$1 RETURNING json_build_object('ok',true)::text"
}

/// ユーザーの参加の状態。`None` は参加行がない (未参加)、`Some(true)` は退出済み
async fn left_of(f: &Fixture, parent_id: &str, user: &str) -> Option<bool> {
    sql(
        &f.state,
        "SELECT json_build_object('left',left_at IS NOT NULL)::text FROM thread_participants WHERE thread_id=$1 AND user_id=$2",
        &[parent_id, user],
    )
    .await
    .unwrap()
    .first()
    .map(|row| row["left"] == true)
}

/// そのユーザーが `ago` 前に退出した状態にする
async fn leave(f: &Fixture, parent_id: &str, user: &str, ago: &str) {
    let rows = sql(
        &f.state,
        &format!(
            "UPDATE thread_participants SET left_at=(now() AT TIME ZONE 'UTC')-interval '{ago}' \
             WHERE thread_id=$1 AND user_id=$2 RETURNING json_build_object('ok',true)::text"
        ),
        &[parent_id, user],
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "{user} has a participation row");
}

/// Slack の返信者も、zoo の返信と同じ参加の扱いになる:
/// - 未参加の人が返信すると、参加して返信する
/// - アーカイブ済みのスレッドへの他人の返信は、スレッドを再表示（再Open）する。ほかの人の退出は取り消さない
/// - 退出済みの人が、退出のあとに返信すると、参加し直して返信する
#[tokio::test]
async fn test_slack_repliers_follow_the_zoo_participation_rules() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);

    // UA が先に返信して、スレッドの参加者になる。UB は、まだ参加していない
    let first = format!("{}.000200", now_secs() - 300);
    deliver_reply(&f, &parent, &first).await?;
    let ua = author_of(&f, &first).await;
    assert_eq!(left_of(&f, &parent_id, &ua).await, Some(false));

    // 閉じたスレッドに、未参加の UB が返信する: スレッドが再表示され、UB は参加する
    sql(&f.state, close_thread_sql(), &[&parent_id]).await?;
    let second = format!("{}.000300", now_secs() - 120);
    deliver_reply_by(&f, &parent, &second, "UB").await?;
    let ub = author_of(&f, &second).await;
    assert_ne!(ua, ub);
    assert!(
        !archived(&f, &parent_id).await,
        "another person's reply reopens the thread"
    );
    assert_eq!(left_of(&f, &parent_id, &ub).await, Some(false), "UB joins");
    assert_eq!(left_of(&f, &parent_id, &ua).await, Some(false));

    // UA が退出したあと、閉じたスレッドへの UB の返信: 再表示されるが、UA の退出は取り消さない
    leave(&f, &parent_id, &ua, "10 minutes").await;
    sql(&f.state, close_thread_sql(), &[&parent_id]).await?;
    deliver_reply_by(&f, &parent, &format!("{}.000400", now_secs() - 90), "UB").await?;
    assert!(!archived(&f, &parent_id).await);
    assert_eq!(
        left_of(&f, &parent_id, &ua).await,
        Some(true),
        "UA stays left"
    );
    assert_eq!(left_of(&f, &parent_id, &ub).await, Some(false));

    // UB が退出し、そのあとに UB が返信する: 参加し直して返信する。UA は退出のまま
    leave(&f, &parent_id, &ub, "5 minutes").await;
    sql(&f.state, close_thread_sql(), &[&parent_id]).await?;
    deliver_reply_by(&f, &parent, &format!("{}.000500", now_secs() - 60), "UB").await?;
    assert!(!archived(&f, &parent_id).await);
    assert_eq!(
        left_of(&f, &parent_id, &ub).await,
        Some(false),
        "a reply after the leave rejoins"
    );
    assert_eq!(left_of(&f, &parent_id, &ua).await, Some(true));
    Ok(())
}

/// 直近の返信と古い返信を、同じ同期（1 つのトランザクション）で取り込む。直近の返信の人だけが再参加し、
/// 古い返信の人の退出は取り消さない
#[tokio::test]
async fn test_sync_imports_recent_and_old_replies_in_one_page() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);
    // UA と UB が、先に返信して、参加者になる
    let a = format!("{}.000200", now_secs() - 200);
    let b = format!("{}.000250", now_secs() - 190);
    deliver_reply(&f, &parent, &a).await?;
    deliver_reply_by(&f, &parent, &b, "UB").await?;
    let (ua, ub) = (author_of(&f, &a).await, author_of(&f, &b).await);
    // 閉じて、UA は 1 時間前に、UB は 4 日前（古い返信より前）に退出した状態にする。UB の退出を古い返信より前に
    // しておくので、古い返信にフラグが誤って立つと、再参加のガード（left_at < created_at）を通って再参加する
    sql(&f.state, close_thread_sql(), &[&parent_id]).await?;
    leave(&f, &parent_id, &ua, "1 hour").await;
    leave(&f, &parent_id, &ub, "4 days").await;
    let before = count_messages(&f.state, &f.channel).await;

    // 1 つのスレッドの返信に、直近の UA の返信と、3 日前の UB の返信を並べて返す。同期は `ts` の昇順に
    // 1 つのトランザクションで入れるので、実際の順序は、古い UB の返信が先で、直近の UA の返信があと
    let recent = format!("{}.000600", now_secs() - 30);
    let old = format!("{}.000500", now_secs() - 3 * 86_400);
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("ts", parent.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
            "messages": [
                {"type":"message","ts":&parent,"user":"UA","text":"new post","thread_ts":&parent,"reply_count":3},
                {"type":"message","ts":&recent,"user":"UA","text":"recent","thread_ts":&parent},
                {"type":"message","ts":&old,"user":"UB","text":"old","thread_ts":&parent}],
            "has_more": false})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&f.slack)
        .await;
    sql(
        &f.state,
        "UPDATE slack_channel_syncs SET swept_at=NULL WHERE team_id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    sync_now(&f).await?;

    assert_eq!(count_messages(&f.state, &f.channel).await, before + 2);
    assert!(
        !archived(&f, &parent_id).await,
        "the recent reply reopens the thread"
    );
    assert_eq!(
        left_of(&f, &parent_id, &ua).await,
        Some(false),
        "UA rejoins"
    );
    assert_eq!(
        left_of(&f, &parent_id, &ub).await,
        Some(true),
        "the old reply does not rejoin UB"
    );
    Ok(())
}

/// API の最初の全履歴の同期（カーソル 0・未同期）は、履歴の取り込み。直近の返信でも、再Openも再参加もしない。
/// 同じ返信を、通常の差分同期で取り込むと、再Openする（`test_sync_reopens_for_a_recent_reply_but_not_for_an_old_one`）
#[tokio::test]
async fn test_api_initial_history_sync_does_not_reopen_threads() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let parent_id = backend::slack::message_id(&f.team, "C1", &parent);
    deliver_reply(&f, &parent, &format!("{}.000200", now_secs() - 90)).await?;
    close_and_leave(&f, &parent_id, "1 hour").await?;
    let before = count_messages(&f.state, &f.channel).await;

    let recent = format!("{}.000600", now_secs() - 30);
    // Slack の時計が DB より進んでいて、時刻が先の返信 (`created_at >= トランザクションの開始時刻`) も、履歴の取り込みでは
    // ライブにしない
    let ahead = format!("{}.000700", now_secs() + 30);
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("ts", parent.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true,
            "messages": [
                {"type":"message","ts":&parent,"user":"UA","text":"new post","thread_ts":&parent,"reply_count":3},
                {"type":"message","ts":&recent,"user":"UA","text":"history reply","thread_ts":&parent},
                {"type":"message","ts":&ahead,"user":"UA","text":"history reply ahead of the DB clock","thread_ts":&parent}],
            "has_more": false})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&f.slack)
        .await;
    // API から取り込みを始めたチャンネルの、最初の同期にする（カーソルが 0 で、まだ一度も同期が終わっていない）
    sql(
        &f.state,
        "UPDATE slack_channels SET last_ts='0' WHERE team_id=$1 AND slack_channel_id='C1' RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    sql(
        &f.state,
        "UPDATE slack_channel_syncs SET last_synced_at=NULL,swept_at=NULL,available_at=now()-interval '1 minute',\
         not_before=NULL,lease_token=NULL,lease_until=NULL,running=false WHERE team_id=$1 \
         RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    drain(&f.state).await;

    assert_eq!(count_messages(&f.state, &f.channel).await, before + 2);
    let (archived, left) = thread_state(&f, &parent_id).await?;
    assert!(
        archived,
        "a recent reply, even one dated ahead of the DB clock, in the initial history is history"
    );
    assert!(
        left.iter().all(|l| *l),
        "the initial history does not rejoin: {left:?}"
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

#[tokio::test]
async fn test_missing_reactions_scope_does_not_stop_the_channel_sync() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    // users が一部 (count のほうが多い) で、補完の reactions.get が missing_scope になる
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","ts":&ts,"user":"UA","text":"new post",
        "reactions":[{"name":"thumbsup","users":["UB"],"count":3}]}]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/reactions.get"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "missing_scope"})),
        )
        .mount(&f.slack)
        .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &ts, "UA", "thumbsup", false),
    )
    .await?;
    // 確かめられないので、ヒントは残る。同期は止まらず、最後まで終わる (終わった時刻が入り、エラーがない)
    assert_eq!(hints(&f).await.len(), 1);
    let rows = sql(
        &f.state,
        "SELECT json_build_object('recent',last_synced_at>now()-interval '1 minute','error',last_error)::text \
         FROM slack_channel_syncs WHERE team_id=$1",
        &[&f.team],
    )
    .await?;
    assert_eq!(rows[0]["recent"], true, "{}", rows[0]);
    assert!(rows[0]["error"].is_null(), "{}", rows[0]);
    Ok(())
}

#[tokio::test]
async fn test_hints_reply_with_fallback_user_waits_for_users_before_locking_channel()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let reply = format!("{}.000900", now_secs() - 30);

    // 暗号化変換と同じ順序 (users -> channels FOR UPDATE) で競合させるため、チャンネルを非公開にしておく
    sql(
        &f.state,
        "UPDATE channels SET is_private=true WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.channel],
    )
    .await?;

    // 既存の未連携ユーザー (profile_source='fallback') を用意する。reply の username で ensure_user が users を更新する
    let ub_id = backend::slack::user_id(&f.team, "UB");
    sql(
        &f.state,
        "INSERT INTO users(id,user_name,display_name) VALUES($1,$2,'Old Fallback') \
         RETURNING json_build_object('ok',true)::text",
        &[
            &ub_id,
            &format!("slack_ub_{}", uuid::Uuid::new_v4().simple()),
        ],
    )
    .await?;
    sql(
        &f.state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name,profile_source) \
         VALUES($1,'UB',$2,false,'UB','','fallback') RETURNING json_build_object('ok',true)::text",
        &[&f.team, &ub_id],
    )
    .await?;

    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("ts", parent.as_str()))
        .and(query_param("latest", reply.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [{
                "type": "message",
                "ts": &reply,
                "user": "UB",
                "username": "Fallback Bot",
                "text": "reply mentioning <@UC>",
                "thread_ts": &parent,
                "reactions": [{"name": "thumbsup", "users": ["UD"], "count": 1}]
            }],
            "has_more": false
        })))
        .with_priority(1)
        .mount(&f.slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "user_not_found"})),
        )
        .mount(&f.slack)
        .await;

    let event = json!({
        "type": "event_callback",
        "team_id": &f.team,
        "event": {
            "type": "message",
            "channel": "C1",
            "user": "UB",
            "ts": &reply,
            "thread_ts": &parent,
            "text": "reply"
        }
    });
    let (status, _) = post_event(&f.app, SECRET, now_secs(), &event).await?;
    assert_eq!(status, StatusCode::OK);
    sql(
        &f.state,
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) VALUES($1,'C1') \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now() RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    age_wake(&f.state, &f.team, "10 seconds").await?;
    set_sync_state(&f.state, &f.team, "1 hour", "1 hour").await?;

    // e2ee_convert と同じく先に users をロックする
    let mut conv_db = f.state.db.clone();
    let mut conv_tx = conv_db.transaction().await?;
    let conv_pid: i32 = query(
        &mut conv_tx,
        "SELECT json_build_object('pid',pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    // ロック待ちの観測はアプリの DB プールと別の独立した接続で行う (同期タスクが
    // プールの接続を持ったまま待っても、観測側が止まらないように)。
    let observer = observe_connect(&f.state).await?;
    backend::routes::e2ee_channels::lock_membership_users(&mut conv_tx, &[ub_id.as_str()]).await?;

    let sync_state = f.state.clone();
    let sync_team = f.team.clone();
    let sync_task =
        tokio::spawn(async move { sync::run_once_for_team(&sync_state, &sync_team).await });

    // hints::apply が channels を取る前に users のロック待ちに入ることを確認する
    let blocked = wait_blocked_on(&observer, conv_pid, 200).await?;
    assert!(
        blocked,
        "hints::apply did not block on users before locking channel; lock waiters: {}",
        dump_lock_waiters(&observer).await
    );

    // まだ channels はロックされていないため、users -> channels FOR UPDATE が 40P01 にならず完了する。
    // `FOR UPDATE NOWAIT` なので、順序が逆 (channels を先に取る) になっていたら、ここで
    // 即座に lock_not_available になり、デッドロックを待たずに失敗する
    toasty::sql::query("SELECT 1 FROM channels WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(f.channel.as_str())
        .exec(&mut conv_tx)
        .await?;
    toasty::sql::statement("UPDATE channels SET encrypted=true WHERE id=$1")
        .bind(f.channel.as_str())
        .exec(&mut conv_tx)
        .await?;
    conv_tx.commit().await?;

    let ran = tokio::time::timeout(std::time::Duration::from_secs(5), sync_task).await???;
    assert!(ran);
    assert_eq!(count_messages(&f.state, &f.channel).await, 1);

    // 暗号化により平文拒否となり、attempts が 1 になってロールバックされる (デッドロックではない)
    let hint_rows = hints(&f).await;
    assert_eq!(hint_rows.len(), 1);
    assert_eq!(hint_rows[0]["kind"], "reply");
    assert_eq!(hint_rows[0]["attempts"], 1);
    let slack_users = sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM slack_users WHERE team_id=$1 AND slack_user_id IN ('UC','UD')",
        &[&f.team],
    )
    .await?;
    assert_eq!(slack_users[0]["n"], 0);
    Ok(())
}

#[tokio::test]
async fn test_hints_edit_reaction_and_zoo_origin_preresolve_unlinked_users() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();

    // 1. 無視されるヒント (edited なしの changed / 存在しない投稿への reaction / 存在する投稿の deleted) は
    //    プレースホルダユーザーを作らない
    let missing_ts = format!("{}.000321", now_secs() - 86_400);
    mock_fetch(
        &f,
        &ts,
        json!([{"type":"message","ts":&ts,"user":"UA","text":"unfurled <@UNOTCREATED>"}]),
    )
    .await;
    mock_fetch(
        &f,
        &missing_ts,
        json!([{"type":"message","ts":&missing_ts,"user":"UA","text":"old <@UNOTCREATED2>",
        "reactions":[{"name":"thumbsup","users":["UNOTCREATED3"],"count":1}]}]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "user_not_found"})),
        )
        .mount(&f.slack)
        .await;
    deliver(&f, &changed_event(&f.team, "C1", &ts)).await?;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &missing_ts, "UNOTCREATED3", "thumbsup", true),
    )
    .await?;
    let ignored_users = sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM slack_users WHERE team_id=$1 AND slack_user_id IN ('UNOTCREATED','UNOTCREATED2','UNOTCREATED3')",
        &[&f.team],
    )
    .await?;
    assert_eq!(ignored_users[0]["n"], 0);

    // 2. zoo_origin の投稿に対するリアクションは未連携のリアクション者を事前解決して登録できる
    let zoo_ts = format!("{}.000400", now_secs() - 20);
    let zoo_user = sql(
        &f.state,
        "SELECT json_build_object('id',user_id)::text FROM slack_user_links WHERE team_id=$1",
        &[&f.team],
    )
    .await?[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let zoo_msg_id = uuid::Uuid::new_v4().to_string();
    sql(
        &f.state,
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'from zoo') RETURNING json_build_object('ok',true)::text",
        &[&zoo_msg_id, &zoo_user, &f.channel],
    )
    .await?;
    sql(
        &f.state,
        "INSERT INTO slack_message_map(team_id,slack_channel_id,ts,message_id,origin) VALUES($1,'C1',$2,$3,'zoo')          RETURNING json_build_object('ok',true)::text",
        &[&f.team, &zoo_ts, &zoo_msg_id],
    )
    .await?;
    mock_fetch(
        &f,
        &zoo_ts,
        json!([{"type":"message","ts":&zoo_ts,"user":"UA","text":"from zoo",
        "reactions":[{"name":"thumbsup","users":["UZOO"],"count":1}]}]),
    )
    .await;
    deliver(
        &f,
        &reaction_event(&f.team, "C1", &zoo_ts, "UZOO", "thumbsup", true),
    )
    .await?;
    let zoo_reacts = sql(
        &f.state,
        "SELECT json_build_object('n',count(*))::text FROM message_reactions WHERE message_id=$1",
        &[&zoo_msg_id],
    )
    .await?;
    assert_eq!(zoo_reacts[0]["n"], 1);

    // 3. changed (edited.ts あり) のメンション <@UB> と reactions.get で補完したリアクション <UC> も
    //    channels ロック前に users ロックを取るため、users -> channels と競合してもデッドロックしない
    f.slack.reset().await;
    mount_basics(&f).await;
    sql(
        &f.state,
        "UPDATE channels SET is_private=true WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.channel],
    )
    .await?;
    let uc_id = backend::slack::user_id(&f.team, "UC");
    sql(
        &f.state,
        "INSERT INTO users(id,user_name,display_name) VALUES($1,$2,'Old UC') \
         RETURNING json_build_object('ok',true)::text",
        &[
            &uc_id,
            &format!("slack_uc_{}", uuid::Uuid::new_v4().simple()),
        ],
    )
    .await?;
    sql(
        &f.state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name,profile_source) \
         VALUES($1,'UC',$2,false,'UC','','fallback') RETURNING json_build_object('ok',true)::text",
        &[&f.team, &uc_id],
    )
    .await?;

    let edited_ts = format!("{}.000000", now_secs() + 10);
    mock_fetch(
        &f,
        &ts,
        json!([{
            "type": "message",
            "ts": &ts,
            "user": "UA",
            "text": "edited mentioning <@UB>",
            "edited": {"user": "UA", "ts": &edited_ts},
            "reactions": [{"name": "thumbsup", "users": ["UA"], "count": 2}]
        }]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/reactions.get"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "message": {
                "reactions": [{"name": "thumbsup", "users": ["UA", "UC"], "count": 2}]
            }
        })))
        .mount(&f.slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "user_not_found"})),
        )
        .mount(&f.slack)
        .await;

    for ev in [
        changed_event(&f.team, "C1", &ts),
        reaction_event(&f.team, "C1", &ts, "UC", "thumbsup", true),
    ] {
        let (status, _) = post_event(&f.app, SECRET, now_secs(), &ev).await?;
        assert_eq!(status, StatusCode::OK);
    }
    sql(
        &f.state,
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) VALUES($1,'C1') \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now() RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    age_wake(&f.state, &f.team, "10 seconds").await?;
    set_sync_state(&f.state, &f.team, "1 hour", "1 hour").await?;

    let mut conv_db = f.state.db.clone();
    let mut conv_tx = conv_db.transaction().await?;
    let conv_pid: i32 = query(
        &mut conv_tx,
        "SELECT json_build_object('pid',pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    // ロック待ちの観測も独立した接続で行う (上のテストと同じ理由)。
    let observer = observe_connect(&f.state).await?;
    backend::routes::e2ee_channels::lock_membership_users(&mut conv_tx, &[uc_id.as_str()]).await?;

    let sync_state = f.state.clone();
    let sync_team = f.team.clone();
    let sync_task =
        tokio::spawn(async move { sync::run_once_for_team(&sync_state, &sync_team).await });

    let blocked = wait_blocked_on(&observer, conv_pid, 200).await?;
    assert!(
        blocked,
        "hints::apply did not block on truncated reaction user before locking channel; lock waiters: {}",
        dump_lock_waiters(&observer).await
    );

    // users のロックを取ったあとに channels を取る順序なら、この NOWAIT の確認は即座に通る。
    // 逆 (channels を先に取る) になっていたら lock_not_available で即座に失敗する
    toasty::sql::query("SELECT 1 FROM channels WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(f.channel.as_str())
        .exec(&mut conv_tx)
        .await?;
    toasty::sql::statement("UPDATE channels SET encrypted=true WHERE id=$1")
        .bind(f.channel.as_str())
        .exec(&mut conv_tx)
        .await?;
    conv_tx.commit().await?;

    let ran = tokio::time::timeout(std::time::Duration::from_secs(5), sync_task).await???;
    assert!(ran);
    assert_eq!(message_row(&f, &ts).await["content"], "new post");
    Ok(())
}

#[tokio::test]
async fn test_hints_rejects_remapped_slack_channel_target() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();
    let owner = create_user(&f.app, "hints_remap_owner").await?;
    let new_channel = create_channel(&f.app, &owner).await?;

    f.slack.reset().await;
    mount_basics(&f).await;
    let edited_ts = format!("{}.000000", now_secs() + 10);
    mock_fetch(
        &f,
        &ts,
        json!([{
            "type": "message",
            "ts": &ts,
            "user": "UA",
            "text": "edited after remap",
            "edited": {"user": "UA", "ts": &edited_ts}
        }]),
    )
    .await;

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
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) VALUES($1,'C1') \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now() RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    age_wake(&f.state, &f.team, "10 seconds").await?;
    set_sync_state(&f.state, &f.team, "1 hour", "1 hour").await?;

    let mut remap_db = f.state.db.clone();
    let mut remap_tx = remap_db.transaction().await?;
    let remap_pid: i32 = query(
        &mut remap_tx,
        "SELECT json_build_object('pid',pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    // 観測は独立した接続で行う (アプリのプールを使わない)。
    let observer = observe_connect(&f.state).await?;
    toasty::sql::statement("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("slack-channel:{}:C1", f.team))
        .exec(&mut remap_tx)
        .await?;

    let sync_state = f.state.clone();
    let sync_team = f.team.clone();
    let sync_task =
        tokio::spawn(async move { sync::run_once_for_team(&sync_state, &sync_team).await });

    let blocked = wait_blocked_on(&observer, remap_pid, 200).await?;
    assert!(
        blocked,
        "hints::apply did not wait on Slack-channel advisory lock before updating; lock waiters: {}",
        dump_lock_waiters(&observer).await
    );

    toasty::sql::statement(
        "UPDATE slack_channels SET channel_id=$2 WHERE team_id=$1 AND slack_channel_id='C1'",
    )
    .bind(f.team.as_str())
    .bind(new_channel.as_str())
    .exec(&mut remap_tx)
    .await?;
    remap_tx.commit().await?;

    let ran = tokio::time::timeout(std::time::Duration::from_secs(5), sync_task).await???;
    assert!(ran);
    assert_eq!(message_row(&f, &ts).await["content"], "new post");
    let sync_rows = sql(
        &f.state,
        "SELECT json_build_object('last_error',last_error)::text FROM slack_channel_syncs WHERE team_id=$1 AND slack_channel_id='C1'",
        &[&f.team],
    )
    .await?;
    assert!(
        sync_rows[0]["last_error"]
            .as_str()
            .is_some_and(|e| e.contains("Slack sync target changed or no longer exists")),
        "expected target change error, got {:?}",
        sync_rows
    );
    Ok(())
}

/// 非公開の Slack チャンネルの Zoo の取り込み先が、書き込みの前に公開へ変わったら、
/// hints の反映を拒否する (claim の時点では非公開だったので、同期そのものは始まる)
#[tokio::test]
async fn test_hints_rejects_a_private_slack_channel_whose_zoo_target_became_public()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let ts = f.ts_new.clone();

    // claim の時点では Zoo の取り込み先は非公開。conversations.info も非公開を返す
    sql(
        &f.state,
        "UPDATE channels SET is_private=true WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&f.channel],
    )
    .await?;
    f.slack.reset().await;
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {"id": "C1", "is_private": true},
        })))
        .mount(&f.slack)
        .await;
    let edited_ts = format!("{}.000000", now_secs() + 10);
    mock_fetch(
        &f,
        &ts,
        json!([{
            "type": "message",
            "ts": &ts,
            "user": "UA",
            "text": "edited after the zoo channel became public",
            "edited": {"user": "UA", "ts": &edited_ts}
        }]),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [],
            "has_more": false
        })))
        .mount(&f.slack)
        .await;

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
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) VALUES($1,'C1') \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now() RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    age_wake(&f.state, &f.team, "10 seconds").await?;
    set_sync_state(&f.state, &f.team, "1 hour", "1 hour").await?;

    // hints::apply の Slack-channel advisory lock を先に取り、書き込みの直前で止める
    let mut remap_db = f.state.db.clone();
    let mut remap_tx = remap_db.transaction().await?;
    let remap_pid: i32 = query(
        &mut remap_tx,
        "SELECT json_build_object('pid',pg_backend_pid())::text",
        &[],
    )
    .await?[0]["pid"]
        .as_i64()
        .unwrap() as i32;
    // 観測は独立した接続で行う (アプリのプールを使わない)。
    let observer = observe_connect(&f.state).await?;
    toasty::sql::statement("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("slack-channel:{}:C1", f.team))
        .exec(&mut remap_tx)
        .await?;

    let sync_state = f.state.clone();
    let sync_team = f.team.clone();
    let sync_task =
        tokio::spawn(async move { sync::run_once_for_team(&sync_state, &sync_team).await });

    let blocked = wait_blocked_on(&observer, remap_pid, 200).await?;
    assert!(
        blocked,
        "hints::apply did not wait on the Slack-channel advisory lock; lock waiters: {}",
        dump_lock_waiters(&observer).await
    );

    // 書き込みの前に、Zoo の取り込み先を公開へ変える
    toasty::sql::statement("UPDATE channels SET is_private=false WHERE id=$1")
        .bind(f.channel.as_str())
        .exec(&mut remap_tx)
        .await?;
    remap_tx.commit().await?;

    let ran = tokio::time::timeout(std::time::Duration::from_secs(5), sync_task).await???;
    assert!(ran);
    assert_eq!(message_row(&f, &ts).await["content"], "new post");
    let hint_rows = sql(
        &f.state,
        "SELECT json_build_object('attempts',attempts)::text FROM slack_message_hints \
         WHERE team_id=$1 AND slack_channel_id='C1' AND message_ts=$2",
        &[&f.team, &ts],
    )
    .await?;
    assert_eq!(
        hint_rows[0]["attempts"], 0,
        "hints::apply must fail without bumping attempts or consuming the hint: {hint_rows:?}"
    );
    let sync_rows = sql(
        &f.state,
        "SELECT json_build_object('last_error',last_error)::text FROM slack_channel_syncs WHERE team_id=$1 AND slack_channel_id='C1'",
        &[&f.team],
    )
    .await?;
    assert!(
        sync_rows[0]["last_error"].as_str().is_some_and(|e| e
            .contains("Slack sync target changed or no longer protects private channel content")),
        "expected private target error, got {:?}",
        sync_rows
    );
    Ok(())
}

/// CodeRabbit #1: 同期の開始時は公開だった Slack チャンネルが、ヒントの取り直しの前に非公開へ変わったら、
/// 返信を公開の取り込み先へ保存しない。ヒントは消費せず、試行回数も増やさず、残りのヒントも処理しない。
#[tokio::test]
async fn test_hints_stop_without_bumping_attempts_when_the_slack_channel_became_private()
-> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let f = imported(true).await?;
    let parent = f.ts_new.clone();
    let first = format!("{}.000900", now_secs() - 40);
    let second = format!("{}.000800", now_secs() - 30);

    f.slack.reset().await;
    // 1 回目 (同期の開始) は公開、2 回目 (ヒントの取り直しの前) から非公開
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {"id": "C1", "is_private": false},
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&f.slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {"id": "C1", "is_private": true},
        })))
        .with_priority(2)
        .expect(1)
        .mount(&f.slack)
        .await;
    // 返信の取り直しには、新しい返信を返す
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("ts", parent.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [{"type":"message","ts":&first,"user":"UA","text":"late reply","thread_ts":&parent}],
            "has_more": false,
        })))
        .with_priority(1)
        .mount(&f.slack)
        .await;
    // ヒントの処理は 1 件目で止まるので、通常の同期の履歴もここまで来ない
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [],
            "has_more": false,
        })))
        .mount(&f.slack)
        .await;

    for reply in [&first, &second] {
        let event = json!({"type":"event_callback","team_id":&f.team,"event":{"type":"message",
            "channel":"C1","user":"UA","ts":reply,"thread_ts":&parent,"text":"late reply"}});
        let (status, _) = post_event(&f.app, SECRET, now_secs(), &event).await?;
        assert_eq!(status, StatusCode::OK);
    }
    sql(
        &f.state,
        "INSERT INTO slack_channel_wakes(team_id,slack_channel_id) VALUES($1,'C1') \
         ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET last_at=now() RETURNING json_build_object('ok',true)::text",
        &[&f.team],
    )
    .await?;
    age_wake(&f.state, &f.team, "10 seconds").await?;
    set_sync_state(&f.state, &f.team, "1 hour", "1 hour").await?;
    // 1 回だけ回す (reset_run は available_at を今にするので、drain すると 2 回目が
    // private_slack_channel_in_public_channel で止まり、1 回目の last_error を上書きする)
    assert!(sync::run_once_for_team(&f.state, &f.team).await?);

    // (a) 返信は、公開の Zoo チャンネルに入らない (取り込み済みの 1 件だけ)
    assert!(message_row(&f, &first).await.is_null());
    assert!(message_row(&f, &second).await.is_null());
    assert_eq!(count_messages(&f.state, &f.channel).await, 1);
    // (b) ヒントは残り、試行回数は 0。2 件目の取り直しもしない
    let rows = hints(&f).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(
        rows.iter()
            .all(|r| r["kind"] == "reply" && r["attempts"] == 0),
        "hints must stay unconsumed without bumping attempts: {rows:?}"
    );
    let reply_calls = f
        .slack
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/conversations.replies")
        .count();
    assert_eq!(reply_calls, 1, "the remaining hint must not be fetched");
    // (c) 最後のエラーに、非公開の内容を保護しない取り込み先のエラーが残る
    let sync_rows = sql(
        &f.state,
        "SELECT json_build_object('last_error',last_error)::text FROM slack_channel_syncs WHERE team_id=$1 AND slack_channel_id='C1'",
        &[&f.team],
    )
    .await?;
    assert!(
        sync_rows[0]["last_error"].as_str().is_some_and(|e| e
            .contains("Slack sync target changed or no longer protects private channel content")),
        "expected private target error, got {:?}",
        sync_rows
    );
    Ok(())
}
