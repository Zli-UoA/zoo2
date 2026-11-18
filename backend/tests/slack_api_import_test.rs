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
    matchers::{body_string_contains, header, method, path, query_param},
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
                 "thread_ts": "1704070100.000100", "reply_count": 1, "latest_reply": "1704070300.000100"},
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
                {"type": "message", "ts": "1704070300.000100", "user": "UU", "text": "reply",
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
    // 既存メンバー (作成者) の既読位置は、返信を含む取り込んだ履歴の最新まで進む。履歴が未読として並ばない
    let read = sql(
        &state,
        "SELECT json_build_object('read',last_read_at>=(to_timestamp(1704070300) AT TIME ZONE 'UTC')::timestamp(3))::text FROM channel_read_states WHERE channel_id=$1 AND user_id=$2",
        &[&zoo_channel, &alice],
    )
    .await?;
    assert_eq!(read[0]["read"], true, "{read:?}");
    let (_, done) = request(&app, "GET", "/api/slack/private-channels", &alice, None).await?;
    assert_eq!(done["channels"][0]["status"], "done", "{done}");
    assert_eq!(done["channels"][0]["channel_id"], zoo_channel.as_str());

    // 取り込み先を削除すると、一覧には出ない (対応は残るので、取り込み直しは 409 になる)
    sql(
        &state,
        "UPDATE channels SET deleted_at=now() WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&zoo_channel],
    )
    .await?;
    let (_, gone) = request(&app, "GET", "/api/slack/private-channels", &alice, None).await?;
    assert_eq!(gone["channels"], json!([]), "{gone}");

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

#[tokio::test]
async fn api_import_rechecks_guest_kind_after_waiting_for_user_lock() -> anyhow::Result<()> {
    let key = [9u8; 32];
    let team = format!("TLOCK{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
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
    let state = AppState::new(db, config.clone());
    let app = create_router(state.clone());
    let alice = create_user(&app, "api_import_guest_race").await?;
    link(
        &state,
        &key,
        &alice,
        &team,
        "UA",
        "xoxp-race",
        PRIVATE_SCOPES,
    )
    .await?;
    sql(
        &state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked) VALUES($1,'UA',$2,true) RETURNING json_build_object('ok',true)::text",
        &[&team, &alice],
    )
    .await?;
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", "GLOCKRACE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {"id":"GLOCKRACE", "name":"race", "creator":"UA", "created":1704067200,
                "is_private":true, "purpose":{"value":""}},
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.members"))
        .and(query_param("channel", "GLOCKRACE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "members":["UA"], "response_metadata":{"next_cursor":""},
        })))
        .mount(&slack)
        .await;

    // The ordinary guest reads in both API-import phases see the committed human
    // version. Once the import reaches its membership lock, the transition owns
    // the row and must be observed after it commits.
    let (mut transition_connection, transition_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(transition_driver);
    let transition_pid: i32 = transition_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let transition = transition_connection.transaction().await?;
    transition
        .execute(
            "UPDATE users SET guest_kind = 'multi' WHERE id = $1",
            &[&alice],
        )
        .await?;

    let import_app = app.clone();
    let import_user = alice.clone();
    let mut import = Some(tokio::spawn(async move {
        request(
            &import_app,
            "POST",
            "/api/slack/private-channels/import",
            &import_user,
            Some(json!({"slack_channel_id":"GLOCKRACE", "name":"race"})),
        )
        .await
    }));
    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if import.as_ref().unwrap().is_finished() {
                let result = import.take().unwrap().await;
                panic!("Slack API import finished before waiting for the guest-kind lock: {result:?}");
            }
            let waiting: bool = observer
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE '%FOR NO KEY UPDATE%' AND $1 = ANY(pg_blocking_pids(pid)))",
                    &[&transition_pid],
                )
                .await?
                .get(0);
            if waiting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok::<(), tokio_postgres::Error>(())
    })
    .await??;

    transition.commit().await?;
    let (status, body) =
        tokio::time::timeout(std::time::Duration::from_secs(10), import.take().unwrap())
            .await???;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let imported = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM slack_channels WHERE team_id=$1 AND slack_channel_id='GLOCKRACE'",
        &[&team],
    )
    .await?;
    assert_eq!(
        imported[0]["n"], 0,
        "guest must not create an imported channel"
    );
    Ok(())
}

#[tokio::test]
async fn api_import_racing_slack_link_adds_the_linked_user_to_the_channel() -> anyhow::Result<()> {
    let key = [10u8; 32];
    let team = format!("TLINK{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
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
    let state = AppState::new(db, config.clone());
    let app = create_router(state.clone());
    let alice = create_user(&app, "api_link_race_alice").await?;
    let bob = create_user(&app, "api_link_race_bob").await?;
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
    sql(
        &state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked) VALUES($1,'UA',$2,true) RETURNING json_build_object('ok',true)::text",
        &[&team, &alice],
    )
    .await?;
    let placeholder = backend::slack::user_id(&team, "UB");
    sql(
        &state,
        "INSERT INTO users(id,user_name,display_name,guest_kind) VALUES($1,$2,'Slack placeholder','deactivated') RETURNING json_build_object('ok',true)::text",
        &[&placeholder, &format!("api_link_race_{}", uuid::Uuid::new_v4().simple())],
    )
    .await?;
    sql(
        &state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked) VALUES($1,'UB',$2,false) RETURNING json_build_object('ok',true)::text",
        &[&team, &placeholder],
    )
    .await?;

    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .and(query_param("channel", "GLINKRACE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "channel": {"id":"GLINKRACE", "name":"link-race", "creator":"UA", "created":1704067200,
                "is_private":true, "purpose":{"value":""}},
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.members"))
        .and(query_param("channel", "GLINKRACE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "members":["UA", "UB"], "response_metadata":{"next_cursor":""},
        })))
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .and(header("authorization", "Basic Y2lkOnNlYw=="))
        .and(body_string_contains("code=code-bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok":true,
            "authed_user":{"id":"UB", "scope":"users:read", "access_token":"xoxp-bob", "token_type":"user"},
            "team":{"id":team, "name":"Zli"},
        })))
        .mount(&slack)
        .await;

    let (status, started) = request(&app, "POST", "/api/slack/oauth/start", &bob, None).await?;
    assert_eq!(status, StatusCode::OK);
    let authorize_url = reqwest::Url::parse(started["authorize_url"].as_str().unwrap())?;
    let oauth_state = authorize_url
        .query_pairs()
        .find(|(name, _)| name == "state")
        .map(|(_, value)| value.into_owned())
        .unwrap();
    let channel_name = format!("link_race_{}", uuid::Uuid::new_v4().simple());

    // Queue link first on the placeholder row. Without a shared Slack-link lock, the API
    // resolves the old placeholder behind this link and can insert that stale ID after
    // reassign has already moved the placeholder's existing memberships.
    let (mut blocker_connection, blocker_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(blocker_driver);
    let blocker_pid: i32 = blocker_connection
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    let blocker = blocker_connection.transaction().await?;
    blocker
        .query_one(
            "SELECT id FROM users WHERE id=$1 FOR UPDATE",
            &[&placeholder],
        )
        .await?;

    let link_app = app.clone();
    let link_user = bob.clone();
    let link_task = tokio::spawn(async move {
        request(
            &link_app,
            "POST",
            "/api/slack/oauth/complete",
            &link_user,
            Some(json!({"code":"code-bob", "state":oauth_state})),
        )
        .await
    });
    let (observer, observer_driver) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(observer_driver);
    let link_pid = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let row = observer
                .query_opt(
                    "SELECT pid FROM pg_stat_activity WHERE wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%FOR NO KEY UPDATE%' LIMIT 1",
                    &[&blocker_pid],
                )
                .await?;
            if let Some(row) = row {
                break Ok::<i32, anyhow::Error>(row.get(0));
            }
            if link_task.is_finished() {
                anyhow::bail!("Slack link finished before waiting on the placeholder row");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await??;

    let import_app = app.clone();
    let import_user = alice.clone();
    let import_task = tokio::spawn(async move {
        request(
            &import_app,
            "POST",
            "/api/slack/private-channels/import",
            &import_user,
            Some(json!({"slack_channel_id":"GLINKRACE", "name":channel_name})),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if link_task.is_finished() || import_task.is_finished() {
                anyhow::bail!("an API operation finished before the placeholder lock was released");
            }
            let import_waiting: bool = observer
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid<>$3 AND wait_event_type='Lock' AND ($1=ANY(pg_blocking_pids(pid)) OR $2=ANY(pg_blocking_pids(pid))))",
                    &[&blocker_pid, &link_pid, &link_pid],
                )
                .await?
                .get(0);
            if import_waiting {
                break Ok::<(), anyhow::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await??;

    blocker.commit().await?;
    let (link_status, link_body) =
        tokio::time::timeout(std::time::Duration::from_secs(10), link_task).await???;
    assert_eq!(link_status, StatusCode::OK, "{link_body}");
    let (import_status, imported) =
        tokio::time::timeout(std::time::Duration::from_secs(10), import_task).await???;
    assert_eq!(import_status, StatusCode::OK, "{imported}");
    let channel = imported["channel_id"].as_str().unwrap();
    let memberships = sql(
        &state,
        "SELECT json_build_object('user_id',user_id)::text FROM channel_user WHERE channel_id=$1 AND user_id IN ($2,$3) ORDER BY user_id",
        &[channel, &bob, &placeholder],
    )
    .await?;
    assert_eq!(
        memberships.len(),
        1,
        "the channel must have one of the linked identities"
    );
    assert_eq!(memberships[0]["user_id"], bob);
    Ok(())
}

/// API 取り込みは、ZIP 取り込みと同じ `(team, Slack チャンネル)` 単位の advisory lock を、
/// `slack_channels` の確認よりも前 (プレースホルダ作成よりも前) に取る。並行する ZIP
/// 取り込みが対応を確定させたあとに解放されると、API 側は 409 になり、確定済みの対応を
/// 上書きもプレースホルダの作成もしない。
#[tokio::test]
async fn api_import_waits_for_the_zip_target_lock_before_checking_the_mapping() -> anyhow::Result<()>
{
    let key = [11u8; 32];
    let team = format!(
        "TZIPLOCK{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
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
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());
    let alice = create_user(&app, "api_zip_lock_alice").await?;
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
    sql(
        &state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked) VALUES($1,'UA',$2,true) \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &alice],
    )
    .await?;
    mock_channel(&slack, "GZIPRACE", "zip-race", "UA", true).await;
    Mock::given(method("GET"))
        .and(path("/conversations.members"))
        .and(query_param("channel", "GZIPRACE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "members": ["UA", "UU"], "response_metadata": {"next_cursor": ""},
        })))
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "user": {"id": "UU", "name": "uma", "real_name": "Uma", "profile": {"display_name": "uma"}},
        })))
        .mount(&slack)
        .await;

    // A zoo channel that the racing ZIP import already committed as the target
    // mapping for this Slack channel while holding the target lock.
    let zip_target = uuid::Uuid::new_v4().to_string();
    let result: anyhow::Result<()> = async {
        sql(
            &state,
            "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,true) \
             RETURNING json_build_object('ok',true)::text",
            &[&zip_target, &format!("zip_target_{}", uuid::Uuid::new_v4().simple()), &alice],
        )
        .await?;
        // Hold the exact advisory lock the ZIP importer's mapping transaction takes,
        // and leave the mapping uncommitted until the API import is observed waiting.
        let (mut holder, holder_driver) =
            tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(holder_driver);
        let holder_pid: i32 = holder
            .query_one("SELECT pg_backend_pid()", &[])
            .await?
            .get(0);
        let holder_tx = holder.transaction().await?;
        holder_tx
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
                &[&format!("slack-channel:{team}:GZIPRACE")],
            )
            .await?;

        let import_app = app.clone();
        let import_user = alice.clone();
        let import_task = tokio::spawn(async move {
            request(
                &import_app,
                "POST",
                "/api/slack/private-channels/import",
                &import_user,
                Some(json!({"slack_channel_id": "GZIPRACE", "name": "zip-race"})),
            )
            .await
        });
        // 観測や統合に失敗したとき、取り込みが裏で動いたまま後片付けが走らない
        // よう abort 用のハンドルを手元に残す。
        let import_task_abort = import_task.abort_handle();
        let inner: anyhow::Result<()> = async {
            // The import must block on the target advisory lock itself (not on a
            // users row or the mapping row): that is what serializes API and ZIP
            // initial imports on the same target lock.
            let (observer, observer_driver) =
                tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
            tokio::spawn(observer_driver);
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    if import_task.is_finished() {
                        anyhow::bail!(
                            "the API import finished before waiting for the ZIP target lock"
                        );
                    }
                    let waiting: bool = observer
                        .query_one(
                            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                             AND query LIKE '%pg_advisory_xact_lock%')",
                            &[&holder_pid],
                        )
                        .await?
                        .get(0);
                    if waiting {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Ok::<(), anyhow::Error>(())
            })
            .await??;

            // While the import waits, the ZIP path commits its mapping under the same
            // lock (the mapping insert is what the ZIP worker's first transaction
            // does between the lock and its commit).
            holder_tx
                .execute(
                    "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) \
                     VALUES($1,'GZIPRACE',$2,'0')",
                    &[&team, &zip_target],
                )
                .await?;
            holder_tx.commit().await?;

            let (status, body) =
                tokio::time::timeout(std::time::Duration::from_secs(10), import_task).await???;
            anyhow::ensure!(
                status == StatusCode::CONFLICT,
                "the racing API import must be refused with 409, got {status}: {body}"
            );
            // The racing mapping stays exactly as the ZIP path committed it.
            let mapping = sql(
                &state,
                "SELECT json_build_object('channel_id',channel_id,'last_ts',last_ts)::text \
                 FROM slack_channels WHERE team_id=$1 AND slack_channel_id='GZIPRACE'",
                &[&team],
            )
            .await?;
            anyhow::ensure!(
                mapping.first().is_some_and(|row| row["channel_id"] == zip_target.as_str()),
                "the API import must not overwrite the committed ZIP mapping: {mapping:?}"
            );
            anyhow::ensure!(
                mapping.first().is_some_and(|row| row["last_ts"] == "0"),
                "the mapping must keep its sync cursor"
            );
            // The import refused before resolving members or creating a channel: no
            // placeholder for UU and no deterministic import channel exist.
            let member_placeholder = backend::slack::user_id(&team, "UU");
            let import_channel = backend::slack::channel_id(&team, "GZIPRACE");
            let leaked = sql(
                &state,
                "SELECT json_build_object('n', (SELECT count(*) FROM users WHERE id=$1) \
                 + (SELECT count(*) FROM channels WHERE id=$2))::text",
                &[&member_placeholder, &import_channel],
            )
            .await?;
            anyhow::ensure!(
                leaked.first().is_some_and(|row| row["n"].as_i64() == Some(0)),
                "the refused import must not create placeholders or a channel: {leaked:?}"
            );
            Ok(())
        }
        .await;
        // タイムアウトや失敗のときは後片付けの前に必ず止める (成功時は完了済み)。
        if inner.is_err() {
            import_task_abort.abort();
        }
        inner
    }
    .await;
    // Clean up only the rows this test created, even after a failure: the
    // mapping, the ZIP target channel, the Slack link rows, and the placeholder
    // (if a failed import left one behind). Cleanup failures are collected so
    // they cannot mask the actual test result.
    let member_placeholder = backend::slack::user_id(&team, "UU");
    let import_channel = backend::slack::channel_id(&team, "GZIPRACE");
    let mut cleanup_errors: Vec<String> = Vec::new();
    for (statement, params) in [
        (
            "DELETE FROM slack_channels WHERE team_id=$1",
            vec![team.as_str()],
        ),
        (
            "DELETE FROM channel_user WHERE channel_id IN ($1,$2)",
            vec![zip_target.as_str(), import_channel.as_str()],
        ),
        (
            // create_user joins the workspace default channels, and
            // channel_user.user_id is ON DELETE RESTRICT: drop both users'
            // memberships (including the default channels) before users.
            "DELETE FROM channel_user WHERE user_id IN ($1,$2)",
            vec![member_placeholder.as_str(), alice.as_str()],
        ),
        (
            "DELETE FROM channels WHERE id IN ($1,$2)",
            vec![zip_target.as_str(), import_channel.as_str()],
        ),
        (
            "DELETE FROM slack_users WHERE team_id=$1",
            vec![team.as_str()],
        ),
        (
            "DELETE FROM users WHERE id IN ($1,$2)",
            vec![member_placeholder.as_str(), alice.as_str()],
        ),
    ] {
        if let Err(error) = sql(&state, statement, &params).await {
            cleanup_errors.push(format!("{statement}: {error}"));
        }
    }
    if cleanup_errors.is_empty() {
        return result;
    }
    let cleanup_error = anyhow::anyhow!("cleanup failed: {}", cleanup_errors.join("; "));
    match result {
        Ok(()) => Err(cleanup_error),
        Err(error) => Err(error.context(cleanup_error)),
    }
}
