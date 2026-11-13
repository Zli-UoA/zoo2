//! 連携した Slack アカウントで、自分が作った非公開チャンネルを API で取り込む。モックの Slack API に対して、
//! 一覧・開始・全履歴の同期・拒否する条件を確かめる。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::Engine;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};

use backend::{
    apps::query, config::Config, db::init_db, routes::create_router, slack::sync, state::AppState,
};

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Dev-User", user);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value)?)
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(builder.body(body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
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
    let (status, _) = request(
        app,
        "POST",
        "/api/users",
        &id,
        Some(json!({
            "id": id,
            "user_name": format!("{name}_{}", uuid::Uuid::new_v4().simple()),
            "display_name": name,
        })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(id)
}

async fn link(
    state: &AppState,
    key: &[u8; 32],
    user: &str,
    team: &str,
    slack_user: &str,
    token: &str,
    scopes: &str,
) -> anyhow::Result<()> {
    let enc = backend::slack::crypto::encrypt(key, token)?;
    sql(
        state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc,scopes) VALUES($1,$2,$3,$4,$5) \
         RETURNING json_build_object('ok',true)::text",
        &[user, team, slack_user, &enc, scopes],
    )
    .await?;
    Ok(())
}

fn message(ts: &str, user: &str, text: &str) -> Value {
    json!({"type": "message", "ts": ts, "user": user, "text": text})
}

/// キューを、取り出せる項目がなくなるまで回す (ほかのテストが残した項目も処理されることがある)
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

const PRIVATE_SCOPES: &str = "groups:read,groups:history,users:read";

async fn mock_channel(slack: &MockServer, id: &str, name: &str, creator: &str, private: bool) {
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", id))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {
                "id": id, "name": name, "creator": creator, "created": 1704067200,
                "is_private": private, "purpose": {"value": "secret plans"},
            },
        })))
        .mount(slack)
        .await;
}

#[tokio::test]
async fn test_user_imports_own_private_channel_through_the_api() -> anyhow::Result<()> {
    let key = [5u8; 32];
    let team = format!("TAPI{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let suffix = uuid::Uuid::new_v4().simple().to_string()[..10].to_string();
    let slack = MockServer::start().await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key = Some(base64::engine::general_purpose::STANDARD.encode(key));
    config.slack.api_base_url = slack.uri();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());

    let alice = create_user(&app, "api_alice").await?;
    let bob = create_user(&app, "api_bob").await?;
    let carol = create_user(&app, "api_carol").await?;
    link(
        &state,
        &key,
        &alice,
        &team,
        "UA",
        "xoxp-alice",
        PRIVATE_SCOPES,
    )
    .await?;
    // 連携すると、Slack ユーザーと zoo ユーザーの対応ができる (`link::complete` と同じ)
    sql(
        &state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked) VALUES($1,'UA',$2,true) \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &alice],
    )
    .await?;
    // 履歴スコープのない、以前の版で連携した人
    link(&state, &key, &bob, &team, "UB", "xoxp-bob", "users:read").await?;

    let name = format!("secret-{suffix}");
    mock_channel(&slack, "G1", &name, "UA", true).await;
    mock_channel(&slack, "G2", "bobs", "UB", true).await;
    mock_channel(&slack, "C1", "public", "UA", false).await;
    Mock::given(method("GET"))
        .and(path("/users.conversations"))
        .and(header("authorization", "Bearer xoxp-alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channels": [
                {"id": "G1", "name": name, "creator": "UA", "is_private": true, "num_members": 2},
                // 他人が作った非公開チャンネル、DM は出さない
                {"id": "G2", "name": "bobs", "creator": "UB", "is_private": true},
                {"id": "D1", "name": "", "creator": "UA", "is_im": true},
            ],
            "response_metadata": {"next_cursor": ""},
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.members"))
        .and(query_param("channel", "G1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "members": ["UA", "UU"], "response_metadata": {"next_cursor": ""},
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "user": {"id": "UU", "name": "uma", "real_name": "Uma", "profile": {"display_name": "uma"}},
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "G1"))
        .and(query_param("oldest", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                message("1704070200.000200", "UU", "second"),
                // 3 日より古いスレッドの親。最初の同期は、返信も取る
                {"type": "message", "ts": "1704070100.000100", "user": "UA", "text": "first",
                 "thread_ts": "1704070100.000100", "reply_count": 1, "latest_reply": "1704070150.000100"},
            ],
            "has_more": false,
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("channel", "G1"))
        .and(query_param("ts", "1704070100.000100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {"type": "message", "ts": "1704070100.000100", "user": "UA", "text": "first",
                 "thread_ts": "1704070100.000100", "reply_count": 1},
                {"type": "message", "ts": "1704070150.000100", "user": "UU", "text": "reply",
                 "thread_ts": "1704070100.000100"},
            ],
            "has_more": false,
        })))
        .mount(&slack)
        .await;

    // 連携していない人には、取り込めるものがない
    let (status, none) = request(&app, "GET", "/api/slack/private-channels", &carol, None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(none["reason"], "not_linked", "{none}");
    // 履歴スコープのない連携は、連携し直すまで使えない
    let (_, old) = request(&app, "GET", "/api/slack/private-channels", &bob, None).await?;
    assert_eq!(old["reason"], "missing_scope", "{old}");
    let (status, _) = request(
        &app,
        "POST",
        "/api/slack/private-channels/import",
        &bob,
        Some(json!({"slack_channel_id": "G2", "name": "bobs"})),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 一覧に出るのは、自分が作った非公開チャンネルだけ
    let (status, listed) =
        request(&app, "GET", "/api/slack/private-channels", &alice, None).await?;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let ids: Vec<&str> = listed["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["slack_channel_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["G1"], "{listed}");
    assert_eq!(listed["channels"][0]["status"], "new");

    // 公開チャンネルと他人のチャンネルは、指定しても取り込めない
    for (id, expect) in [("C1", StatusCode::FORBIDDEN), ("G2", StatusCode::FORBIDDEN)] {
        let (status, body) = request(
            &app,
            "POST",
            "/api/slack/private-channels/import",
            &alice,
            Some(json!({"slack_channel_id": id, "name": format!("x-{suffix}")})),
        )
        .await?;
        assert_eq!(status, expect, "{id}: {body}");
    }

    let (status, started) = request(
        &app,
        "POST",
        "/api/slack/private-channels/import",
        &alice,
        Some(json!({"slack_channel_id": "G1", "name": name})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{started}");
    let zoo_channel = started["channel_id"].as_str().unwrap().to_string();
    assert_eq!(zoo_channel, backend::slack::channel_id(&team, "G1"));
    let row = sql(
        &state,
        "SELECT json_build_object('private',is_private,'creator',created_user_id,'imported',imported_from_slack,\
         'members',(SELECT count(*) FROM channel_user WHERE channel_id=$1),\
         'cursor',(SELECT last_ts FROM slack_channels WHERE team_id=$2 AND slack_channel_id='G1'))::text \
         FROM channels WHERE id=$1",
        &[&zoo_channel, &team],
    )
    .await?;
    assert_eq!(row[0]["private"], true);
    assert_eq!(row[0]["creator"], alice.as_str());
    assert_eq!(row[0]["imported"], true);
    // 作成者の alice と、未連携のメンバー UU のプレースホルダ
    assert_eq!(row[0]["members"], 2);
    assert_eq!(row[0]["cursor"], "0");
    let (_, queued) = request(&app, "GET", "/api/slack/private-channels", &alice, None).await?;
    assert_eq!(queued["channels"][0]["status"], "queued", "{queued}");

    // 同期のワーカーが、全履歴を取る。もう一度回しても重複しない
    drain(&state).await;
    assert_eq!(count_messages(&state, &zoo_channel).await, 3);
    sql(
        &state,
        "UPDATE slack_channel_syncs SET available_at=now() WHERE team_id=$1 AND lease_token IS NULL \
         RETURNING json_build_object('ok',true)::text",
        &[&team],
    )
    .await?;
    drain(&state).await;
    assert_eq!(count_messages(&state, &zoo_channel).await, 3);
    let authors = sql(
        &state,
        "SELECT json_build_object('user',user_id)::text FROM messages WHERE channel_id=$1 AND content='first'",
        &[&zoo_channel],
    )
    .await?;
    assert_eq!(authors[0]["user"], alice.as_str());
    // 既存メンバー (作成者) の既読位置は、取り込んだ履歴の最新まで進む。履歴が未読として並ばない
    let read = sql(
        &state,
        "SELECT json_build_object('read',last_read_at IS NOT NULL)::text FROM channel_read_states WHERE channel_id=$1 AND user_id=$2",
        &[&zoo_channel, &alice],
    )
    .await?;
    assert_eq!(read[0]["read"], true, "{read:?}");
    let (_, done) = request(&app, "GET", "/api/slack/private-channels", &alice, None).await?;
    assert_eq!(done["channels"][0]["status"], "done", "{done}");
    assert_eq!(done["channels"][0]["channel_id"], zoo_channel.as_str());

    // 同じチャンネルは、もう一度取り込めない
    let (status, _) = request(
        &app,
        "POST",
        "/api/slack/private-channels/import",
        &alice,
        Some(json!({"slack_channel_id": "G1", "name": format!("again-{suffix}")})),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    Ok(())
}
