//! Slack の差分同期 (#292)。モックの Slack API に対して、カーソル・429 からの再開・重複しないこと・
//! トークンの切り替え・非公開チャンネルの扱いを確かめる。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::Engine;
use futures_util::FutureExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};

use backend::{
    apps::query, config::Config, db::init_db, routes::create_router, slack::sync, state::AppState,
};

/// `sync::run_once` は DB 上のどのチャンネルの項目でも拾い、ほかのワークスペースの項目は自分の設定で
/// 保留にしてしまう。設定 (ワークスペース・モックの Slack) の違うテスト同士が互いの項目を処理しないよう、
/// このファイルのテストは順に実行する。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    create_user_with_id(app, &id, name).await
}

async fn create_user_with_id(app: &Router, id: &str, name: &str) -> anyhow::Result<String> {
    let (status, _) = request(
        app,
        "POST",
        "/api/users",
        id,
        Some(json!({
            "id": id,
            "user_name": format!("{name}_{}", uuid::Uuid::new_v4().simple()),
            "display_name": name,
        })),
    )
    .await?;
    anyhow::ensure!(status == StatusCode::OK, "create user returned {status}");
    Ok(id.to_owned())
}

async fn create_channel(app: &Router, owner: &str, private: bool) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    create_channel_with_id(app, &id, owner, private).await
}

async fn create_channel_with_id(
    app: &Router,
    id: &str,
    owner: &str,
    private: bool,
) -> anyhow::Result<String> {
    let (status, _) = request(
        app,
        "POST",
        "/api/channels",
        owner,
        Some(json!({
            "id": id,
            "channel_name": format!("sync_{}", uuid::Uuid::new_v4().simple()),
            "is_private": private,
        })),
    )
    .await?;
    anyhow::ensure!(status == StatusCode::OK, "create channel returned {status}");
    Ok(id.to_owned())
}

async fn link(
    state: &AppState,
    key: &[u8; 32],
    user: &str,
    team: &str,
    slack_user: &str,
    token: &str,
) -> anyhow::Result<()> {
    let enc = backend::slack::crypto::encrypt(key, token)?;
    sql(
        state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,$3,$4) \
         RETURNING json_build_object('ok',true)::text",
        &[user, team, slack_user, &enc],
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

async fn due_now(state: &AppState, team: &str) {
    sql(
        state,
        "UPDATE slack_channel_syncs SET available_at=now() WHERE team_id=$1 AND lease_token IS NULL \
         RETURNING json_build_object('ok',true)::text",
        &[team],
    )
    .await
    .unwrap();
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

#[tokio::test]
#[ignore = "requires an explicitly configured disposable SLACK_SYNC_TEST_DATABASE_URL"]
async fn slack_sync_holds_mapping_until_message_save_commits() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let database_url = std::env::var("SLACK_SYNC_TEST_DATABASE_URL")?;
    let (guard, guard_driver) =
        tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(guard_driver);
    let database: String = guard
        .query_one("SELECT current_database()", &[])
        .await?
        .get(0);
    anyhow::ensure!(
        database.starts_with("zoo2_notes_api_slack_sync_test_"),
        "refusing database outside the dedicated Slack sync test namespace: {database}"
    );

    let key = [3u8; 32];
    let team = format!("TSYNC{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let slack = MockServer::start().await;
    let mut config = Config::from_env();
    config.database_url = database_url.clone();
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

    let user = uuid::Uuid::new_v4().to_string();
    let old_channel = uuid::Uuid::new_v4().to_string();
    let new_channel = uuid::Uuid::new_v4().to_string();
    let other_team = format!("TOTHER{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let other_user = uuid::Uuid::new_v4().to_string();
    let other_channel = uuid::Uuid::new_v4().to_string();
    let token_key = format!("slack-channel:{team}:C-RACE");
    let gate_key = uuid::Uuid::new_v4().as_u128() as i64;
    let trigger_suffix = uuid::Uuid::new_v4().simple().to_string();
    let function = format!("slack_sync_gate_{trigger_suffix}");
    let trigger = format!("slack_sync_gate_trg_{trigger_suffix}");
    let gate_table = format!("slack_sync_gate_seen_{trigger_suffix}");
    let mut tasks = tokio::task::JoinSet::new();
    let execution = async {
        create_user_with_id(&app, &user, "sync_mapping_race").await?;
        create_channel_with_id(&app, &old_channel, &user, false).await?;
        create_channel_with_id(&app, &new_channel, &user, false).await?;
        link(&state, &key, &user, &team, "U1", "xoxp-sync-race").await?;
        create_user_with_id(&app, &other_user, "sync_mapping_other_workspace").await?;
        create_channel_with_id(&app, &other_channel, &other_user, false).await?;
        link(
            &state,
            &key,
            &other_user,
            &other_team,
            "U1",
            "xoxp-other-workspace",
        )
        .await?;
        sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C-RACE',$2,'0') \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &old_channel],
    ).await?;
        sql(
            &state,
            "INSERT INTO slack_channel_syncs(team_id,slack_channel_id) VALUES($1,'C-RACE') \
         RETURNING json_build_object('ok',true)::text",
            &[&team],
        )
        .await?;
        sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C-OTHER',$2,'0') \
         RETURNING json_build_object('ok',true)::text",
        &[&other_team, &other_channel],
    ).await?;
        sql(
            &state,
            "INSERT INTO slack_channel_syncs(team_id,slack_channel_id) VALUES($1,'C-OTHER') \
         RETURNING json_build_object('ok',true)::text",
            &[&other_team],
        )
        .await?;
        guard.batch_execute(&format!(
        "CREATE TABLE {gate_table}(seen boolean NOT NULL); \
         CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
         IF NEW.channel_id = '{old_channel}' THEN INSERT INTO {gate_table} VALUES(true); \
         PERFORM pg_advisory_xact_lock({gate_key}); END IF; RETURN NEW; END $$; \
         CREATE TRIGGER {trigger} BEFORE INSERT ON messages FOR EACH ROW EXECUTE FUNCTION {function}()"
    )).await?;

        Mock::given(method("GET"))
            .and(path("/conversations.info"))
            .and(query_param("channel", "C-RACE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true, "channel": {"id": "C-RACE", "is_private": false},
            })))
            .mount(&slack)
            .await;
        Mock::given(method("GET"))
            .and(path("/conversations.history"))
            .and(query_param("channel", "C-RACE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true, "messages": [message("1800000000.000001", "U1", "saved during remap")],
                "has_more": false,
            })))
            .mount(&slack)
            .await;

        guard
            .query_one("SELECT pg_advisory_lock($1)", &[&gate_key])
            .await?;
        let queue_before = sql(
          &state,
          "SELECT json_build_object('available_at',available_at,'attempts',attempts,'running',running,'lease_token',lease_token,'lease_until',lease_until,'last_error',last_error)::text \
           FROM slack_channel_syncs WHERE team_id=$1 AND slack_channel_id='C-OTHER'",
          &[&other_team],
      ).await?;
        let inject_missing_lock =
            std::env::var("SLACK_SYNC_TEST_INJECT_FAILURE").as_deref() == Ok("missing-lock");
        if !inject_missing_lock {
            let sync_state = state.clone();
            let sync_team = team.clone();
            tasks.spawn(async move {
                while sync::run_once_for_team(&sync_state, &sync_team).await? {}
                Ok::<(), anyhow::Error>(())
            });
        }
        let lock_wait_limit = if inject_missing_lock {
            std::time::Duration::from_secs(1)
        } else {
            std::time::Duration::from_secs(10)
        };
        tokio::time::timeout(lock_wait_limit, async {
        loop {
            let rows = sql(
                &state,
                "SELECT json_build_object('waiting',EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' \
                 AND classid=(($1::bigint >> 32) & 4294967295)::oid AND objid=($1::bigint & 4294967295)::oid \
                 AND NOT granted))::text",
                &[&gate_key.to_string()],
            ).await?;
            if rows[0]["waiting"] == true { return anyhow::Ok(()); }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
      }).await??;

        match std::env::var("SLACK_SYNC_TEST_INJECT_FAILURE").as_deref() {
            Ok("panic") => panic!("injected panic after the sync reached the DB lock"),
            Ok("timeout") => {
                tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>())
                    .await?;
            }
            _ => {}
        }

        let remap_url = database_url.clone();
        let remap_team = team.clone();
        let remap_target = new_channel.clone();
        let remap_app = format!("slack_sync_remap_{}", uuid::Uuid::new_v4().simple());
        let remap_app_for_task = remap_app.clone();
        tasks.spawn(async move {
        let (mut client, driver) =
            tokio_postgres::connect(&remap_url, tokio_postgres::NoTls).await?;
        tokio::spawn(driver);
        client.query("SELECT set_config('application_name',$1,false)", &[&remap_app_for_task]).await?;
        let tx = client.transaction().await?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            &[&token_key],
        )
        .await?;
        tx.execute(
            "UPDATE slack_channels SET channel_id=$2 WHERE team_id=$1 AND slack_channel_id='C-RACE'",
            &[&remap_team, &remap_target],
        ).await?;
        tx.commit().await?;
        anyhow::Ok(())
    });
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let rows = sql(
                &state,
                "SELECT json_build_object('waiting',EXISTS(SELECT 1 FROM pg_stat_activity a WHERE a.application_name=$1 \
                 AND a.wait_event_type='Lock' AND cardinality(pg_blocking_pids(a.pid))>0))::text",
                &[&remap_app],
            ).await?;
            if rows[0]["waiting"] == true { return anyhow::Ok(()); }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await??;
        guard
            .query_one("SELECT pg_advisory_unlock($1)", &[&gate_key])
            .await?;
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Some(result) = tasks.join_next().await {
                result??;
            }
            anyhow::Ok(())
        })
        .await??;
        let mapping = sql(
        &state,
        "SELECT json_build_object('channel_id',channel_id)::text FROM slack_channels WHERE team_id=$1 AND slack_channel_id='C-RACE'",
        &[&team],
    ).await?;
        anyhow::ensure!(mapping[0]["channel_id"] == new_channel);
        anyhow::ensure!(count_messages(&state, &old_channel).await == 1);
        anyhow::ensure!(count_messages(&state, &new_channel).await == 0);
        let queue_after = sql(
        &state,
        "SELECT json_build_object('available_at',available_at,'attempts',attempts,'running',running,'lease_token',lease_token,'lease_until',lease_until,'last_error',last_error)::text \
         FROM slack_channel_syncs WHERE team_id=$1 AND slack_channel_id='C-OTHER'",
        &[&other_team],
    ).await?;
        anyhow::ensure!(
            queue_before == queue_after,
            "other workspace queue was changed"
        );
        anyhow::Ok(())
    };
    let outcome = std::panic::AssertUnwindSafe(execution).catch_unwind().await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), tasks.shutdown()).await;
    let cleanup = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let _ = guard.query_one("SELECT pg_advisory_unlock($1)", &[&gate_key]).await;
        let statements = [
            format!("DROP TRIGGER IF EXISTS {trigger} ON messages"),
            format!("DROP FUNCTION IF EXISTS {function}()"),
            format!("DROP TABLE IF EXISTS {gate_table}"),
            format!("DELETE FROM slack_user_links WHERE user_id IN ('{user}','{other_user}')"),
            format!("DELETE FROM slack_channels WHERE team_id IN ('{team}','{other_team}')"),
            format!("DELETE FROM channel_user WHERE channel_id IN ('{old_channel}','{new_channel}','{other_channel}')"),
            format!("DELETE FROM messages WHERE channel_id IN ('{old_channel}','{new_channel}','{other_channel}')"),
            format!("DELETE FROM channels WHERE id IN ('{old_channel}','{new_channel}','{other_channel}')"),
            format!("DELETE FROM channel_user WHERE user_id IN (SELECT user_id FROM slack_users WHERE team_id = '{team}' AND slack_user_id = 'U1' AND NOT linked)"),
            format!("DELETE FROM channel_user WHERE user_id IN ('{user}','{other_user}')"),
            format!("DELETE FROM users WHERE id IN (SELECT user_id FROM slack_users WHERE team_id = '{team}' AND slack_user_id = 'U1' AND NOT linked)"),
            format!("DELETE FROM users WHERE id IN ('{user}','{other_user}')"),
        ];
        let mut first_error = None;
        for statement in statements {
            if let Err(error) = guard.batch_execute(&statement).await {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(anyhow::anyhow!(error));
        }
        Ok::<(), anyhow::Error>(())
    }).await;
    if let Err(cleanup_error) = cleanup {
        return Err(anyhow::anyhow!(
            "fixture cleanup timed out: {cleanup_error}"
        ));
    }
    cleanup??;
    let residue: i64 = guard
        .query_one(
            "SELECT (SELECT count(*) FROM users WHERE id IN ($1,$2)) \
         +(SELECT count(*) FROM channels WHERE id IN ($3,$4,$5)) \
         +(SELECT count(*) FROM slack_channels WHERE team_id IN ($6,$7)) \
         +(SELECT count(*) FROM slack_channel_syncs WHERE team_id IN ($6,$7)) \
         +(SELECT count(*) FROM slack_users WHERE team_id = $6 AND slack_user_id = 'U1') \
         +(SELECT count(*) FROM slack_user_links WHERE user_id IN ($1,$2)) \
         +(SELECT count(*) FROM channel_user WHERE channel_id IN ($3,$4,$5)) \
         +(SELECT count(*) FROM messages WHERE channel_id IN ($3,$4,$5)) \
         +(SELECT count(*) FROM pg_trigger WHERE tgname=$8) \
         +(SELECT count(*) FROM pg_proc WHERE proname=$9) \
         +(SELECT count(*) FROM pg_class WHERE relname=$10)",
            &[
                &user,
                &other_user,
                &old_channel,
                &new_channel,
                &other_channel,
                &team,
                &other_team,
                &trigger,
                &function,
                &gate_table,
            ],
        )
        .await?
        .get(0);
    anyhow::ensure!(
        residue == 0,
        "test fixture residue remains after cleanup: {residue}"
    );
    match outcome {
        Ok(result) => result?,
        Err(_) => anyhow::bail!("slack sync race test panicked; fixture cleanup completed"),
    }
    Ok(())
}

#[tokio::test]
async fn test_slack_sync_resumes_after_429_without_duplicates() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let key = [3u8; 32];
    let team = format!("TSYNC{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
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

    // 返信の取り直しは最近のスレッドだけが対象なので、時刻は現在から相対にする
    let t0 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        - 7200;
    let cursor0 = format!("{t0}.000000");
    let ts = |offset: u64| format!("{}.000100", t0 + offset);

    let alice = create_user(&app, "sync_alice").await?;
    let bob = create_user(&app, "sync_bob").await?;
    link(&state, &key, &alice, &team, "UA", "xoxp-alice").await?;
    link(&state, &key, &bob, &team, "UB", "xoxp-bob").await?;

    // C1: 公開チャンネル。同期カーソルは 1704067200.000000 (エクスポートの最後)
    let c1 = create_channel(&app, &alice, false).await?;
    sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C1',$2,$3) \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &c1, &cursor0],
    )
    .await?;

    for channel in ["C1", "C2", "C3"] {
        Mock::given(method("GET"))
            .and(path("/conversations.info"))
            .and(query_param("channel", channel))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                // C2 は非公開の Slack チャンネル。公開の zoo チャンネルには入れない
                "channel": {"id": channel, "is_private": channel == "C2"},
            })))
            .mount(&slack)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "user": {"id": "U9", "name": "carol", "real_name": "Carol", "profile": {"display_name": "carol"}},
        })))
        .mount(&slack)
        .await;

    // 最初の呼び出しは 429。Retry-After の間は進まず、保存済みのカーソルは動かない
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&slack)
        .await;
    // 1 ページ目 (新しい順)。2 ページ目へ続く
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        // 最初の同期は、カーソル (2 時間前) より、スレッドの監視範囲 (3 日) だけ前から取り直す
        .and(query_param("limit", "200"))
        .and(header("authorization", "Bearer xoxp-alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {"type": "message", "ts": &ts(300), "user": "U9", "text": "thread parent <@UA>",
                 "thread_ts": &ts(300), "reply_count": 1, "latest_reply": &ts(310)},
                message(&ts(250), "UA", "second"),
                // 編集・削除の記録や、参加の通知は取り込まない
                {"type": "message", "subtype": "channel_join", "ts": &ts(240), "user": "UA", "text": "joined"},
            ],
            "has_more": true,
            "response_metadata": {"next_cursor": "page2"},
        })))
        .with_priority(5)
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        .and(query_param("cursor", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [message(&ts(210), "UB", "first")],
            "has_more": false,
        })))
        .with_priority(4)
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .and(query_param("channel", "C1"))
        .and(query_param("ts", ts(300)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [
                {"type": "message", "ts": &ts(300), "user": "U9", "text": "thread parent <@UA>",
                 "thread_ts": &ts(300), "reply_count": 1},
                {"type": "message", "ts": &ts(310), "user": "UB", "text": "reply",
                 "thread_ts": &ts(300)},
            ],
            "has_more": false,
        })))
        .mount(&slack)
        .await;

    // 返信のない投稿のスレッドも、新しい返信がないか確かめる。返信がなければ、親だけが返る
    Mock::given(method("GET"))
        .and(path("/conversations.replies"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "messages": [], "has_more": false})),
        )
        .with_priority(9)
        .mount(&slack)
        .await;

    // 連携した直後と同じく、チャンネルをキューに入れて同期する
    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        sync::enqueue_team(&mut tx, &team).await?;
        tx.commit().await?;
    }
    drain(&state).await;
    // 429: 何も取り込まず、カーソルも動かさず、試行回数にも数えない。次の実行は Retry-After の後
    assert_eq!(count_messages(&state, &c1).await, 0);
    let row = &sql(
        &state,
        "SELECT json_build_object('wait',extract(epoch FROM (t.available_at-now())),'attempts',t.attempts,\
         'last_ts',sc.last_ts)::text FROM slack_channel_syncs t JOIN slack_channels sc \
         ON sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id WHERE t.team_id=$1",
        &[&team],
    )
    .await?[0];
    assert!(row["wait"].as_f64().unwrap() > 60.0, "{row}");
    assert_eq!(row["attempts"], 0);
    assert_eq!(row["last_ts"], cursor0.as_str());

    // 待ち時間が過ぎたら、最後まで同期する (2 ページ + スレッドの返信)
    due_now(&state, &team).await;
    drain(&state).await;
    assert_eq!(count_messages(&state, &c1).await, 4);
    let reply = backend::slack::message_id(&team, "C1", &ts(310));
    let parent = backend::slack::message_id(&team, "C1", &ts(300));
    let rows = sql(
        &state,
        "SELECT json_build_object('parent',parent_id)::text FROM messages WHERE id=$1",
        &[&reply],
    )
    .await?;
    assert_eq!(rows[0]["parent"], parent.as_str());
    // 投稿者は、ZIP になかった人もプレースホルダとして作られ、名前は users.info の表示名になる
    let author = sql(
        &state,
        "SELECT json_build_object('name',u.display_name)::text FROM messages m JOIN users u ON u.id=m.user_id WHERE m.id=$1",
        &[&parent],
    )
    .await?;
    assert_eq!(author[0]["name"], "carol");
    // 新しい順に返る途中では動かさず、最後まで終わってから最新の投稿まで進める
    let row = &sql(
        &state,
        "SELECT json_build_object('last_ts',sc.last_ts,'running',t.running,'synced',t.last_synced_at IS NOT NULL)::text \
         FROM slack_channel_syncs t JOIN slack_channels sc ON sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id \
         WHERE t.team_id=$1",
        &[&team],
    )
    .await?[0];
    assert_eq!(row["last_ts"], ts(300).as_str());
    assert_eq!(row["running"], false);
    assert_eq!(row["synced"], true);

    // 次の同期は、新しいカーソルより後だけを求め、同じ投稿を返されても重複しない
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C1"))
        .and(query_param("oldest", ts(300)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [message(&ts(250), "UA", "second")],
            "has_more": false,
        })))
        .with_priority(1)
        .expect(1..)
        .mount(&slack)
        .await;
    due_now(&state, &team).await;
    drain(&state).await;
    assert_eq!(count_messages(&state, &c1).await, 4);

    // C3: 1 人目のトークンでは読めないチャンネルは、別の連携済みメンバーのトークンで取る
    let c3 = create_channel(&app, &alice, false).await?;
    sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C3',$2,$3) \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &c3, &cursor0],
    )
    .await?;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C3"))
        .and(header("authorization", "Bearer xoxp-alice"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "channel_not_found"})),
        )
        .with_priority(1)
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C3"))
        .and(header("authorization", "Bearer xoxp-bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [message(&ts(400), "UB", "from bob's token")],
            "has_more": false,
        })))
        .with_priority(2)
        .mount(&slack)
        .await;

    // C2: 非公開の Slack チャンネルが公開の zoo チャンネルに対応付いていたら、取り込まない
    let c2 = create_channel(&app, &alice, false).await?;
    sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C2',$2,$3) \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &c2, &cursor0],
    )
    .await?;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "messages": [message(&ts(500), "UA", "private")],
            "has_more": false,
        })))
        .expect(0)
        .mount(&slack)
        .await;

    // 対応付けただけでは同期されない (ワーカーが定期的に入れる。ここでは穴埋めの API で入れる)
    for slack_channel in ["C2", "C3"] {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        sync::request_channel(&mut tx, &team, slack_channel).await?;
        tx.commit().await?;
    }
    drain(&state).await;
    assert_eq!(count_messages(&state, &c3).await, 1);
    assert_eq!(count_messages(&state, &c2).await, 0);
    let row = &sql(
        &state,
        "SELECT json_build_object('error',last_error)::text FROM slack_channel_syncs WHERE team_id=$1 AND slack_channel_id='C2'",
        &[&team],
    )
    .await?[0];
    assert_eq!(row["error"], "private_slack_channel_in_public_channel");
    Ok(())
}

/// 同期した投稿の添付は、同期の中では取得せず、ファイル名を本文に補って行 (`job_id` なし) を残す。
/// 行単位の取得が、あとから添付として取得し、補った名前を本文から外す。
#[tokio::test]
async fn test_slack_sync_files_are_fetched_later_by_the_file_worker() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let key = [3u8; 32];
    let team = format!("TSYNC{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let slack = MockServer::start().await;
    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/files/F1/a.txt"))
        .and(header("authorization", "Bearer xoxb-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_bytes(b"hello".to_vec()),
        )
        .mount(&files)
        .await;
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key = Some(base64::engine::general_purpose::STANDARD.encode(key));
    config.slack.api_base_url = slack.uri();
    config.slack.import_bot_token = Some("xoxb-test".into());
    config.slack.files_base_url = format!("{}/files/", files.uri());
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    backend::storage::ensure_bucket_exists(&state.s3_client, &state.config.s3.bucket).await;
    let app = create_router(state.clone());

    let t0 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        - 7200;
    let ts = format!("{}.000100", t0 + 100);
    let alice = create_user(&app, "files_alice").await?;
    link(&state, &key, &alice, &team, "UA", "xoxp-alice").await?;
    let c1 = create_channel(&app, &alice, false).await?;
    sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C1',$2,$3) \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &c1, &format!("{t0}.000000")],
    )
    .await?;
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "channel": {"id": "C1", "is_private": false}})),
        )
        .mount(&slack)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            // 本文がなく、ファイルだけの投稿
            "messages": [{"type": "message", "ts": &ts, "user": "UA", "text": "", "files": [{
                "id": "F1", "name": "a.txt", "mimetype": "text/plain", "size": 5,
                "url_private_download": format!("{}/files/F1/a.txt", files.uri()),
            }]}],
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

    {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        sync::enqueue_team(&mut tx, &team).await?;
        tx.commit().await?;
    }
    drain(&state).await;

    let message = backend::slack::message_id(&team, "C1", &ts);
    let content = |state: AppState, message: String| async move {
        sql(
            &state,
            "SELECT json_build_object('c',content)::text FROM messages WHERE id=$1",
            &[&message],
        )
        .await
        .unwrap()[0]["c"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // 同期は取得せず、ファイル名を本文に補う。行は、ジョブに紐づかない pending
    assert_eq!(content(state.clone(), message.clone()).await, "a.txt");
    let row = &sql(
        &state,
        "SELECT json_build_object('status',status,'job',job_id,'in_content',in_content)::text \
         FROM slack_files WHERE team_id=$1 AND message_id=$2",
        &[&team, &message],
    )
    .await?[0];
    assert_eq!(row["status"], "pending");
    assert!(row["job"].is_null());
    assert_eq!(row["in_content"], true);

    // 行単位の取得が添付にして、補った名前を本文から外す
    assert!(backend::slack::worker::orphan_files_once(&state, &team).await?);
    assert_eq!(content(state.clone(), message.clone()).await, "");
    let attached = sql(
        &state,
        "SELECT json_build_object('n',count(*))::text FROM attachments WHERE message_id=$1 AND file_name='a.txt'",
        &[&message],
    )
    .await?;
    assert_eq!(attached[0]["n"], 1);
    Ok(())
}
