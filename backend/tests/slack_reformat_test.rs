//! 取り込み済みの投稿への書式の変換 (`zoo_slack_reformat`) の修正。取り込んだ投稿だけが変わり、Zoo で書いた投稿や、
//! 対象外の投稿が変わらないこと、dry-run が何も書かないこと、もう一度実行しても変わらないこと、元に戻せることを確かめる。

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{
    apps::query,
    config::Config,
    db::init_db,
    routes::create_router,
    slack::{
        message_id,
        reformat::{self, Change, Options},
    },
    state::AppState,
};

async fn sql(state: &AppState, statement: &str, params: &[&str]) -> anyhow::Result<Vec<Value>> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn post(app: &Router, user: &str, uri: &str, body: Value) -> anyhow::Result<()> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("X-Dev-User", user)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

/// 投稿を作る。`ts` があれば、取り込みと同じ ID と日時 (`ts` をミリ秒に丸めたもの) にする
async fn insert(
    state: &AppState,
    team: &str,
    user: &str,
    channel: &str,
    ts: Option<&str>,
    content: &str,
    extra: &str,
) -> anyhow::Result<String> {
    let id = match ts {
        Some(ts) => message_id(team, "C1", ts),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let created = ts.unwrap_or("1704070000.000000");
    sql(
        state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) \
         VALUES($1,$2,$3,$4,(to_timestamp($5::numeric) AT TIME ZONE 'UTC')::timestamp(3)) \
         RETURNING json_build_object('ok',true)::text",
        &[&id, user, channel, content, created],
    )
    .await?;
    if !extra.is_empty() {
        sql(
            state,
            &format!("UPDATE messages SET {extra} WHERE id=$1 RETURNING json_build_object('ok',true)::text"),
            &[&id],
        )
        .await?;
    }
    Ok(id)
}

async fn content(state: &AppState, id: &str) -> String {
    sql(
        state,
        "SELECT json_build_object('c',content)::text FROM messages WHERE id=$1",
        &[id],
    )
    .await
    .unwrap()[0]["c"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn test_reformat_converts_only_imported_messages() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());
    let team = format!("TREF{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);

    let user = uuid::Uuid::new_v4().to_string();
    post(
        &app,
        &user,
        "/api/users",
        json!({"id": user, "user_name": format!("ref_{}", uuid::Uuid::new_v4().simple()), "display_name": "ref"}),
    )
    .await?;
    let channel = uuid::Uuid::new_v4().to_string();
    post(
        &app,
        &user,
        "/api/channels",
        json!({"id": channel, "channel_name": format!("ref_{}", uuid::Uuid::new_v4().simple()), "is_private": false}),
    )
    .await?;
    sql(
        &state,
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C1',$2,'0') \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &channel],
    )
    .await?;

    // 取り込んだ投稿 (決定的な ID)
    let bold = insert(
        &state,
        &team,
        &user,
        &channel,
        Some("1704070001.000123"),
        "これは*太字*です",
        "",
    )
    .await?;
    let link = insert(
        &state,
        &team,
        &user,
        &channel,
        Some("1704070002.000456"),
        "_see https://a.test/x_y_z_",
        "",
    )
    .await?;
    // `slack_message_map` に Slack の投稿として載っている行 (ID は決定的でない)
    let mapped = insert(&state, &team, &user, &channel, None, "~gone~ 2*3*4", "").await?;
    sql(
        &state,
        "INSERT INTO slack_message_map(team_id,slack_channel_id,ts,message_id,origin) VALUES($1,'C1','1704070009.000001',$2,'slack') \
         RETURNING json_build_object('ok',true)::text",
        &[&team, &mapped],
    )
    .await?;
    // 変わらない行: Zoo で書いた投稿 (同じ作者)、変換済み、編集済み、カード付き、書式でない記号
    let native = insert(
        &state,
        &team,
        &user,
        &channel,
        None,
        "*native* ~native~",
        "",
    )
    .await?;
    let done = insert(
        &state,
        &team,
        &user,
        &channel,
        Some("1704070003.000001"),
        "**done** *x*",
        "",
    )
    .await?;
    let edited = insert(
        &state,
        &team,
        &user,
        &channel,
        Some("1704070004.000001"),
        "*edited*",
        "revision=1",
    )
    .await?;
    let card = insert(
        &state,
        &team,
        &user,
        &channel,
        Some("1704070005.000001"),
        "*card*",
        "card='x'",
    )
    .await?;
    let plain = insert(
        &state,
        &team,
        &user,
        &channel,
        Some("1704070006.000001"),
        "2*3*4 snake_case",
        "",
    )
    .await?;

    let only_this_channel = |apply| Options {
        apply,
        samples: 10,
        channel: Some(channel.clone()),
    };
    let before = [
        &bold, &link, &mapped, &native, &done, &edited, &card, &plain,
    ];
    let mut snapshot = Vec::new();
    for id in before {
        snapshot.push(content(&state, id).await);
    }

    // dry-run は何も書かない
    let dry = reformat::run(&state.db, &only_this_channel(false), |_| {
        panic!("dry-run wrote")
    })
    .await?;
    assert_eq!(dry.changed, 3);
    assert_eq!(dry.applied, 0);
    assert_eq!(dry.samples.len(), 3);
    for (id, old) in before.iter().zip(&snapshot) {
        assert_eq!(&content(&state, id).await, old);
    }

    // 書き込む
    let mut backup: Vec<Change> = Vec::new();
    let done_run = reformat::run(&state.db, &only_this_channel(true), |c| {
        backup.push(c.clone())
    })
    .await?;
    assert_eq!(
        (done_run.changed, done_run.applied, done_run.raced),
        (3, 3, 0)
    );
    assert_eq!(content(&state, &bold).await, "これは**太字**です");
    assert_eq!(
        content(&state, &link).await,
        "<i>see https://a.test/x_y_z</i>"
    );
    assert_eq!(content(&state, &mapped).await, "~~gone~~ 2*3*4");
    for (id, old) in [&native, &done, &edited, &card, &plain].iter().zip([
        &snapshot[3],
        &snapshot[4],
        &snapshot[5],
        &snapshot[6],
        &snapshot[7],
    ]) {
        assert_eq!(&content(&state, id).await, old, "{id} must not change");
    }
    // 編集の印は付けない
    let marks = sql(
        &state,
        "SELECT json_build_object('r',revision,'u',updated_at IS NULL)::text FROM messages WHERE id=$1",
        &[&bold],
    )
    .await?;
    assert_eq!(marks[0]["r"], 0);
    assert_eq!(marks[0]["u"], true);

    // もう一度実行しても、変わらない
    let again = reformat::run(&state.db, &only_this_channel(true), |_| {
        panic!("wrote twice")
    })
    .await?;
    assert_eq!((again.changed, again.applied), (0, 0));

    // 元に戻せる (新しい本文と一致する行だけ)
    sql(
        &state,
        "UPDATE messages SET content='edited by hand' WHERE id=$1 RETURNING json_build_object('ok',true)::text",
        &[&link],
    )
    .await?;
    let restored = reformat::restore(&state.db, &backup).await?;
    assert_eq!(restored, 2);
    assert_eq!(content(&state, &bold).await, snapshot[0]);
    assert_eq!(content(&state, &mapped).await, snapshot[2]);
    assert_eq!(content(&state, &link).await, "edited by hand");
    Ok(())
}
