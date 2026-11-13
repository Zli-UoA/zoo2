use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use base64::Engine;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, header as header_is, method, path},
};

use backend::{
    apps::query, config::Config, db::init_db, models::UserRole, routes::create_router,
    state::AppState,
};
use std::time::Duration;

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    user: Option<&str>,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value, Option<String>)> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(user) = user {
        builder = builder.header("X-Dev-User", user);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value)?)
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(builder.body(body)?).await?;
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, value, location))
}

async fn sql(state: &AppState, statement: &str, params: &[&str]) -> anyhow::Result<Vec<Value>> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(&mut tx, statement, params).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn exec(state: &AppState, statement: &str, params: &[&str]) -> anyhow::Result<()> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let mut q = toasty::sql::statement(statement);
    for p in params {
        q = q.bind(*p);
    }
    q.exec(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn create_user(app: &Router, name: &str) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (status, _, _) = request(
        app,
        "POST",
        "/api/users",
        Some(&id),
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

fn query_param(url: &str, name: &str) -> String {
    reqwest::Url::parse(&format!("https://x{url}"))
        .or_else(|_| reqwest::Url::parse(url))
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// Slack の `oauth.v2.access` を、code ごとに返す Slack ユーザーを変えてモックする
async fn mock_slack(team: &str) -> MockServer {
    let server = MockServer::start().await;
    for (code, slack_user, team_id) in [
        ("code-alice", "U1", team),
        ("code-carol", "U1", team),
        ("code-other-alice", "U3", team),
        ("code-other-team", "U2", "TOTHER"),
    ] {
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .and(header_is("authorization", "Basic Y2lkOnNlYw=="))
            .and(body_string_contains(format!("code={code}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "authed_user": {"id": slack_user, "scope": "users:read", "access_token": format!("xoxp-{code}"), "token_type": "user"},
                "team": {"id": team_id, "name": "Zli"},
            })))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "user": {"name": "alice", "profile": {"display_name": "Alice S"}}
        })))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn test_slack_account_link_reassigns_placeholder() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let slack = mock_slack(&team).await;
    // 最初の失効は Slack 側の一時的なエラーで失敗し、2 回目で成功する
    Mock::given(method("POST"))
        .and(path("/auth.revoke"))
        .and(header_is("authorization", "Bearer xoxp-code-alice"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&slack)
        .await;
    Mock::given(method("POST"))
        .and(path("/auth.revoke"))
        .and(header_is("authorization", "Bearer xoxp-code-alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .with_priority(2)
        .expect(1)
        .mount(&slack)
        .await;

    // 連携済みの Slack ユーザーとの競合で拒否したときは、既存の連携の認可を消さないよう失効させない
    Mock::given(method("POST"))
        .and(path("/auth.revoke"))
        .and(header_is("authorization", "Bearer xoxp-code-carol"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(0)
        .mount(&slack)
        .await;
    // 別の Slack アカウントへの付け替えは拒否し、受け取ったトークンは使わないので失効させる
    Mock::given(method("POST"))
        .and(path("/auth.revoke"))
        .and(header_is("authorization", "Bearer xoxp-code-other-alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&slack)
        .await;
    // 別のワークスペースのトークンは使わないので失効させる
    Mock::given(method("POST"))
        .and(path("/auth.revoke"))
        .and(header_is("authorization", "Bearer xoxp-code-other-team"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&slack)
        .await;

    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.clone());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key =
        Some(base64::engine::general_purpose::STANDARD.encode([3u8; 32]));
    config.slack.api_base_url = slack.uri();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());

    let alice = create_user(&app, "link_alice").await?;
    let bob = create_user(&app, "link_bob").await?;
    let carol = create_user(&app, "link_carol").await?;

    // ZIP 取り込みで作られるのと同じ形のプレースホルダと投稿
    let placeholder = backend::slack::user_id(&team, "U1");
    // 本人のユーザー名 (link_alice_…) より短くし、書き換えで本文が伸びるようにする
    let placeholder_name = format!(
        "slack-a.{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    exec(
        &state,
        "INSERT INTO users(id,user_name,display_name,guest_kind) VALUES($1,$2,'alice (Slack)','deactivated')",
        &[&placeholder, &placeholder_name],
    )
    .await?;
    exec(
        &state,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,name) VALUES($1,'U1',$2,'alice')",
        &[&team, &placeholder],
    )
    .await?;
    let channel = uuid::Uuid::new_v4().to_string();
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/channels",
        Some(&bob),
        Some(json!({"id": channel, "channel_name": format!("link_{}", uuid::Uuid::new_v4().simple()), "is_private": false})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let message = uuid::Uuid::new_v4().to_string();
    exec(
        &state,
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP)",
        &[&placeholder, &channel],
    )
    .await?;
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'from slack','2024-01-01 00:00:00')",
        &[&message, &placeholder, &channel],
    )
    .await?;
    // 取り込んだ本文のメンションは、取り込み時点のプレースホルダの名前で保存されている
    exec(
        &state,
        // 同期カーソルは 2024-01-01 00:00:02。それより新しい投稿 (00:00:03) は連携後も未読に残る
        "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'C1',$2,'1704067202.000000')",
        &[&team, &channel],
    )
    .await?;
    let p = &placeholder_name;
    let mentions = format!("@{p} hi (@{p}), @{p}. @{p}x @{p}.more mail@{p}");
    let (mentioned, unrelated) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,$4,'2024-01-01 00:00:01')",
        &[&mentioned, &bob, &channel, &mentions],
    )
    .await?;
    // 本文とカードの両方がある投稿は、取り込みで本文を見出し「Slack」のカードにも保存している。
    // カードの説明は MFM でないので、リンクの形が本文と違う
    let carded = uuid::Uuid::new_v4().to_string();
    let card = json!({"cards": [
        {"title": "Slack", "description": format!("@{p} outage, see status (https://status.example.com)")},
        {"title": "Status", "description": format!("@{p} in an attachment")},
    ]})
    .to_string();
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at,card) VALUES($1,$2,$3,$4,'2024-01-01 00:00:01',$5)",
        &[&carded, &bob, &channel, &format!("@{p} outage, see [status](https://status.example.com)"), &card],
    )
    .await?;
    // カードの説明が書き換えで上限 (4,000 文字) を超える投稿も、本文とカードの両方を書き換える (説明は上限で切る)
    let long_card = uuid::Uuid::new_v4().to_string();
    let long_card_description = format!("{} @{p}", "x".repeat(4000 - p.chars().count() - 2));
    let long_card_json = json!({"cards": [
        {"title": "Slack", "description": long_card_description},
        {"title": "Status", "description": "attachment"},
    ]})
    .to_string();
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at,card) VALUES($1,$2,$3,$4,'2024-01-01 00:00:01',$5)",
        &[&long_card, &bob, &channel, &format!("@{p} short"), &long_card_json],
    )
    .await?;
    // 書き換えると上限 (40,000 文字) を超える本文は、切り詰めずにそのまま残す
    let long = uuid::Uuid::new_v4().to_string();
    let long_content = format!("@{p} {}", "x".repeat(40_000 - p.chars().count() - 2));
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,$4,'2024-01-01 00:00:03')",
        &[&long, &bob, &channel, &long_content],
    )
    .await?;
    // 他の人がプレースホルダの投稿へ Home で返信している
    let home_reply = uuid::Uuid::new_v4().to_string();
    exec(
        &state,
        "INSERT INTO home_posts(id,user_id,content,reply_user_id) VALUES($1,$2,'reply on Home',$3)",
        &[&home_reply, &bob, &placeholder],
    )
    .await?;
    // Slack から取り込んでいないチャンネルの本文は書き換えない
    let other = uuid::Uuid::new_v4().to_string();
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/channels",
        Some(&bob),
        Some(json!({"id": other, "channel_name": format!("other_{}", uuid::Uuid::new_v4().simple()), "is_private": false})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,$4,'2024-01-01 00:00:02')",
        &[&unrelated, &bob, &other, &format!("@{p}")],
    )
    .await?;
    // 同じ絵文字を本人とプレースホルダの両方が付けている
    for (id, user, emoji) in [
        ("r1", placeholder.as_str(), "👍"),
        ("r2", alice.as_str(), "👍"),
        ("r3", placeholder.as_str(), "🎉"),
    ] {
        exec(
            &state,
            "INSERT INTO message_reactions(id,message_id,user_id,emoji) VALUES($1,$2,$3,$4)",
            &[&format!("{id}-{message}"), &message, user, emoji],
        )
        .await?;
    }

    // 未連携の別のプレースホルダだけへのメンション。連携で書き換えないので、ロックもしない
    let bystander = uuid::Uuid::new_v4().to_string();
    let bystander_content = format!(
        "@slack-z.{} unrelated",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,$4,'2024-01-01 00:00:01')",
        &[&bystander, &bob, &channel, &bystander_content],
    )
    .await?;

    // 未連携の状態
    let (status, link, _) = request(&app, "GET", "/api/slack/link", Some(&alice), None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(link, json!({"configured": true, "link": null}));

    // 認可 URL
    let (status, started, _) =
        request(&app, "POST", "/api/slack/oauth/start", Some(&alice), None).await?;
    assert_eq!(status, StatusCode::OK);
    let url = started["authorize_url"].as_str().unwrap();
    assert!(url.starts_with("https://slack.com/oauth/v2/authorize?"));
    assert_eq!(query_param(url, "client_id"), "cid");
    assert_eq!(query_param(url, "team"), team);
    // 差分同期 (#292) とブリッジ (#293・#294) で使うスコープも、再連携を 1 回で済ませるため、先にまとめて求める
    assert_eq!(
        query_param(url, "user_scope"),
        "users:read,channels:history,groups:history,im:history,mpim:history,channels:read,groups:read,im:read,mpim:read,reactions:read,reactions:write,chat:write"
    );
    let state_param = query_param(url, "state");

    // Slack からの戻りはトークンを交換せず、フロントへ渡すだけ
    let (status, _, location) = request(
        &app,
        "GET",
        &format!("/api/slack/oauth/callback?code=code-alice&state={state_param}"),
        None,
        None,
    )
    .await?;
    assert!(status.is_redirection());
    let location = location.unwrap();
    assert!(location.starts_with("/settings?tab=slack&code=code-alice&state="));
    assert_eq!(query_param(&location, "state"), state_param);
    let (_, _, denied) = request(
        &app,
        "GET",
        "/api/slack/oauth/callback?error=access_denied&state=x",
        None,
        None,
    )
    .await?;
    assert_eq!(
        denied.as_deref(),
        Some("/settings?tab=slack&slack=error&reason=denied")
    );

    // 他人が作った state では完了できない (連携の CSRF 対策)
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/slack/oauth/complete",
        Some(&bob),
        Some(json!({"code": "code-alice", "state": state_param})),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/slack/oauth/complete",
        Some(&alice),
        Some(json!({"code": "code-alice", "state": "forged.state"})),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 本人なら連携でき、プレースホルダの投稿が付け替わる。
    // 書き換えないメンションを含む投稿のロックは待たない (チームの投稿をまとめてロックしない)
    let mut held_db = state.db.clone();
    let mut held = held_db.transaction().await?;
    query(
        &mut held,
        "SELECT json_build_object('id',id)::text FROM messages WHERE id=$1 FOR UPDATE",
        &[&bystander],
    )
    .await?;
    let linked = tokio::time::timeout(
        Duration::from_secs(10),
        request(
            &app,
            "POST",
            "/api/slack/oauth/complete",
            Some(&alice),
            Some(json!({"code": "code-alice", "state": state_param})),
        ),
    )
    .await;
    held.rollback().await?;
    let (status, linked, _) = linked.expect("the link must not wait for unrelated posts")?;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["link"]["slack_user_id"], "U1");
    assert_eq!(linked["link"]["team_allowed"], true);
    assert_eq!(linked["link"]["slack_user_name"], "Alice S");
    assert_eq!(linked["link"]["team_name"], "Zli");

    let owner = sql(
        &state,
        "SELECT json_build_object('user_id',user_id)::text FROM messages WHERE id=$1",
        &[&message],
    )
    .await?;
    assert_eq!(owner[0]["user_id"], alice.as_str());
    let reactions = sql(
        &state,
        "SELECT json_build_object('user_id',user_id,'emoji',emoji)::text FROM message_reactions WHERE message_id=$1 ORDER BY emoji",
        &[&message],
    )
    .await?;
    assert_eq!(reactions.len(), 2);
    assert!(reactions.iter().all(|r| r["user_id"] == alice.as_str()));
    let members = sql(
        &state,
        "SELECT json_build_object('user_id',user_id)::text FROM channel_user WHERE channel_id=$1 ORDER BY user_id",
        &[&channel],
    )
    .await?;
    assert!(members.iter().any(|m| m["user_id"] == alice.as_str()));
    assert!(!members.iter().any(|m| m["user_id"] == placeholder.as_str()));
    // 新しく参加したチャンネルの既読位置は、チャンネル全体の最新ではなく同期カーソルまで
    let read = sql(
        &state,
        "SELECT json_build_object('at',last_read_at)::text FROM channel_read_states WHERE user_id=$1 AND channel_id=$2",
        &[&alice, &channel],
    )
    .await?;
    assert_eq!(read[0]["at"], "2024-01-01T00:00:02");
    let content = |id: String| {
        let state = state.clone();
        async move {
            sql(
                &state,
                "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
                &[&id],
            )
            .await
            .unwrap()[0]["content"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let alice_name = sql(
        &state,
        "SELECT json_build_object('name',user_name)::text FROM users WHERE id=$1",
        &[&alice],
    )
    .await?[0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let a = &alice_name;
    assert_eq!(
        content(mentioned.clone()).await,
        format!("@{a} hi (@{a}), @{a}. @{p}x @{p}.more mail@{p}")
    );
    assert_eq!(content(unrelated.clone()).await, format!("@{p}"));
    // カードのある投稿は本文を表示しないので、本文を複製したテキストカードも書き換える。添付のカードはそのまま
    assert_eq!(
        content(carded.clone()).await,
        format!("@{a} outage, see [status](https://status.example.com)")
    );
    let stored = sql(
        &state,
        "SELECT json_build_object('card',card)::text FROM messages WHERE id=$1",
        &[&carded],
    )
    .await?;
    let stored: serde_json::Value = serde_json::from_str(stored[0]["card"].as_str().unwrap())?;
    assert_eq!(
        stored["cards"][0]["description"],
        format!("@{a} outage, see status (https://status.example.com)")
    );
    assert_eq!(
        stored["cards"][1]["description"],
        format!("@{p} in an attachment")
    );
    assert_eq!(content(long.clone()).await, long_content);
    assert_eq!(content(long_card.clone()).await, format!("@{a} short"));
    let kept = sql(
        &state,
        "SELECT json_build_object('card',card)::text FROM messages WHERE id=$1",
        &[&long_card],
    )
    .await?;
    let kept: serde_json::Value = serde_json::from_str(kept[0]["card"].as_str().unwrap())?;
    let expected: String = format!("{} @{a}", "x".repeat(4000 - p.chars().count() - 2))
        .chars()
        .take(4000)
        .collect();
    assert_eq!(kept["cards"][0]["description"], expected);
    assert_eq!(kept["cards"][1]["description"], "attachment");
    assert_eq!(content(bystander.clone()).await, bystander_content);
    let reply = sql(
        &state,
        "SELECT json_build_object('reply_user_id',reply_user_id)::text FROM home_posts WHERE id=$1",
        &[&home_reply],
    )
    .await?;
    assert_eq!(reply[0]["reply_user_id"], alice.as_str());
    let mapping = sql(
        &state,
        "SELECT json_build_object('user_id',user_id,'linked',linked)::text FROM slack_users WHERE team_id=$1 AND slack_user_id='U1'",
        &[&team],
    )
    .await?;
    assert_eq!(mapping[0], json!({"user_id": alice, "linked": true}));
    let stored = sql(
        &state,
        "SELECT json_build_object('token',access_token_enc)::text FROM slack_user_links WHERE user_id=$1",
        &[&alice],
    )
    .await?;
    let token = stored[0]["token"].as_str().unwrap();
    assert!(!token.contains("xoxp"), "the user token must be encrypted");
    assert_eq!(
        backend::slack::crypto::decrypt(&[3u8; 32], token)?,
        "xoxp-code-alice"
    );

    // 取り込み中に連携した場合、ワーカーがキャッシュしたプレースホルダへ書き込みが続くことがある。
    // 取り込みの最後に呼ぶ reconcile_team で、その分も本人へ移る。
    let late = uuid::Uuid::new_v4().to_string();
    exec(
        &state,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at) VALUES($1,$2,$3,'late import','2024-01-02 00:00:00')",
        &[&late, &placeholder, &channel],
    )
    .await?;
    let mut held_db = state.db.clone();
    let mut held = held_db.transaction().await?;
    query(
        &mut held,
        "SELECT json_build_object('id',id)::text FROM messages WHERE id=$1 FOR UPDATE",
        &[&bystander],
    )
    .await?;
    let reconciled = tokio::time::timeout(Duration::from_secs(10), async {
        let mut db = state.db.clone();
        let mut tx = db.transaction().await?;
        backend::slack::link::reconcile_team(&mut tx, &team).await?;
        tx.commit().await?;
        anyhow::Ok(())
    })
    .await;
    held.rollback().await?;
    reconciled.expect("reconcile_team must not wait for unrelated posts")?;
    let owner = sql(
        &state,
        "SELECT json_build_object('user_id',user_id)::text FROM messages WHERE id=$1",
        &[&late],
    )
    .await?;
    assert_eq!(owner[0]["user_id"], alice.as_str());

    // 同じ Slack アカウントは別の Zoo ユーザーに連携できない
    let (_, started, _) =
        request(&app, "POST", "/api/slack/oauth/start", Some(&carol), None).await?;
    let carol_state = query_param(started["authorize_url"].as_str().unwrap(), "state");
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/slack/oauth/complete",
        Some(&carol),
        Some(json!({"code": "code-carol", "state": carol_state})),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);

    // 別のワークスペースのアカウントは拒否する
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/slack/oauth/complete",
        Some(&carol),
        Some(json!({"code": "code-other-team", "state": carol_state})),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 連携中に別の Slack アカウントで連携し直すと、いまのトークンを失わないよう拒否する
    let (_, started, _) =
        request(&app, "POST", "/api/slack/oauth/start", Some(&alice), None).await?;
    let alice_state = query_param(started["authorize_url"].as_str().unwrap(), "state");
    let (status, body, _) = request(
        &app,
        "POST",
        "/api/slack/oauth/complete",
        Some(&alice),
        Some(json!({"code": "code-other-alice", "state": alice_state})),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "linked_other_account");
    let (_, still, _) = request(&app, "GET", "/api/slack/link", Some(&alice), None).await?;
    assert_eq!(still["link"]["slack_user_id"], "U1");

    // 失効に失敗したら連携 (トークン) を残し、もう一度解除して再試行できる
    let (status, _, _) = request(&app, "DELETE", "/api/slack/link", Some(&alice), None).await?;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let (_, still, _) = request(&app, "GET", "/api/slack/link", Some(&alice), None).await?;
    assert_eq!(still["link"]["slack_user_id"], "U1");

    // SLACK_TEAM_ID を後から別のワークスペースにすると、連携は解除できるよう見せたまま、取り込みには使わせない
    let mut other = state.clone();
    other.config.slack.team_id = Some(format!("T{}", uuid::Uuid::new_v4().simple()));
    let other_app = create_router(other);
    let (status, restricted, _) =
        request(&other_app, "GET", "/api/slack/link", Some(&alice), None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(restricted["link"]["slack_user_id"], "U1");
    assert_eq!(restricted["link"]["team_allowed"], false);

    // 解除するとトークンを無効にして消す。付け替えた投稿は残る。許可していないワークスペースの連携も解除できる
    let (status, unlinked, _) =
        request(&other_app, "DELETE", "/api/slack/link", Some(&alice), None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unlinked["link"], Value::Null);
    let owner = sql(
        &state,
        "SELECT json_build_object('user_id',user_id)::text FROM messages WHERE id=$1",
        &[&message],
    )
    .await?;
    assert_eq!(owner[0]["user_id"], alice.as_str());
    Ok(())
}

/// ゲストの参加チャンネルは管理者が決めるため、連携しても投稿だけを移し、参加チャンネルは移さない。
/// 1 チャンネルだけのゲストも、2 つ目のチャンネルに入れようとして連携が失敗することはない。
#[tokio::test]
async fn test_slack_link_does_not_add_guests_to_channels() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let owner = create_user(&app, "link_guest_owner").await?;

    let mut channels = Vec::new();
    for _ in 0..2 {
        let channel = uuid::Uuid::new_v4().to_string();
        let (status, _, _) = request(
            &app,
            "POST",
            "/api/channels",
            Some(&owner),
            Some(json!({"id": channel, "channel_name": format!("guest_{}", uuid::Uuid::new_v4().simple()), "is_private": false})),
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        channels.push(channel);
    }

    for (slack_user, kind) in [("UG1", "multi"), ("UG2", "single")] {
        let guest = create_user(&app, &format!("link_guest_{kind}")).await?;
        exec(
            &state,
            "UPDATE users SET guest_kind=$2 WHERE id=$1",
            &[&guest, kind],
        )
        .await?;
        let placeholder = backend::slack::user_id(&team, slack_user);
        exec(
            &state,
            "INSERT INTO users(id,user_name,display_name,guest_kind) VALUES($1,$2,'guest (Slack)','deactivated')",
            &[&placeholder, &format!("slack-g.{}", &uuid::Uuid::new_v4().simple().to_string()[..8])],
        )
        .await?;
        exec(
            &state,
            "INSERT INTO slack_users(team_id,slack_user_id,user_id,name) VALUES($1,$2,$3,'guest')",
            &[&team, slack_user, &placeholder],
        )
        .await?;
        let message = uuid::Uuid::new_v4().to_string();
        for channel in &channels {
            exec(
                &state,
                "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP)",
                &[&placeholder, channel],
            )
            .await?;
        }
        exec(
            &state,
            "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'from slack')",
            &[&message, &placeholder, &channels[0]],
        )
        .await?;

        {
            let mut db = state.db.clone();
            let mut tx = db.transaction().await?;
            backend::slack::link::reassign(&mut tx, &team, slack_user, "guest", &guest).await?;
            tx.commit().await?;
        }

        let moved = sql(
            &state,
            "SELECT json_build_object('user_id',user_id)::text FROM messages WHERE id=$1",
            &[&message],
        )
        .await?;
        assert_eq!(moved[0]["user_id"], guest.as_str(), "{kind}");
        for channel in &channels {
            let joined = sql(
                &state,
                "SELECT json_build_object('n',count(*))::text FROM channel_user WHERE user_id=$1 AND channel_id=$2",
                &[&guest, channel],
            )
            .await?;
            assert_eq!(joined[0]["n"], 0, "{kind} guest must not join {channel}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn test_slack_link_requires_configuration_and_admin_manifest() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());
    let user = create_user(&app, "link_plain").await?;
    let admin = create_user(&app, "link_admin").await?;
    {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::create!(UserRole {
            user_id: admin.as_str(),
            role_id: "role_admin",
        })
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
    }

    let (status, link, _) = request(&app, "GET", "/api/slack/link", Some(&user), None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(link["configured"], false);
    let (status, _, _) = request(&app, "POST", "/api/slack/oauth/start", Some(&user), None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) =
        request(&app, "GET", "/api/admin/slack/manifest", Some(&user), None).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, manifest, _) =
        request(&app, "GET", "/api/admin/slack/manifest", Some(&admin), None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(manifest["configured"], false);
    assert_eq!(
        manifest["manifest"]["oauth_config"]["scopes"]["user"],
        json!([
            "users:read",
            "channels:history",
            "groups:history",
            "im:history",
            "mpim:history",
            "channels:read",
            "groups:read",
            "im:read",
            "mpim:read",
            "reactions:read",
            "reactions:write",
            "chat:write"
        ])
    );
    assert_eq!(
        manifest["manifest"]["oauth_config"]["scopes"]["bot"],
        json!(["files:read", "chat:write", "chat:write.customize"])
    );
    Ok(())
}

/// 連携を設定した状態。チームと Slack のモックはテストごとに作る (DB は並行するテストで共有する)
async fn linking_state(team: &str, slack: &MockServer) -> anyhow::Result<(AppState, Router)> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.slack.client_id = Some("cid".into());
    config.slack.client_secret = Some("sec".into());
    config.slack.team_id = Some(team.to_string());
    config.slack.oauth_redirect_url = Some("https://zoo.example/api/slack/oauth/callback".into());
    config.slack.token_encryption_key =
        Some(base64::engine::general_purpose::STANDARD.encode([3u8; 32]));
    config.slack.api_base_url = slack.uri();
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db, config);
    let app = create_router(state.clone());
    Ok((state, app))
}

async fn mock_access(server: &MockServer, code: &str, team: &str, slack_user: &str, token: &str) {
    Mock::given(method("POST"))
        .and(path("/oauth.v2.access"))
        .and(body_string_contains(format!("code={code}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "authed_user": {"id": slack_user, "scope": "users:read", "access_token": token, "token_type": "user"},
            "team": {"id": team, "name": "Zli"},
        })))
        .mount(server)
        .await;
}

async fn mock_users_info(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "user": {"name": "u", "profile": {"display_name": "U"}}
        })))
        .mount(server)
        .await;
}

async fn mock_revoke(server: &MockServer, token: &str, times: u64) {
    Mock::given(method("POST"))
        .and(path("/auth.revoke"))
        .and(header_is("authorization", format!("Bearer {token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(times)
        .mount(server)
        .await;
}

async fn revokes(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == "/auth.revoke")
        .count()
}

async fn oauth_state(app: &Router, user: &str) -> anyhow::Result<String> {
    let (status, started, _) =
        request(app, "POST", "/api/slack/oauth/start", Some(user), None).await?;
    assert_eq!(status, StatusCode::OK, "{started}");
    Ok(query_param(
        started["authorize_url"].as_str().unwrap(),
        "state",
    ))
}

async fn complete(
    app: &Router,
    user: &str,
    code: &str,
) -> anyhow::Result<(StatusCode, Value, Option<String>)> {
    let state = oauth_state(app, user).await?;
    request(
        app,
        "POST",
        "/api/slack/oauth/complete",
        Some(user),
        Some(json!({"code": code, "state": state})),
    )
    .await
}

/// テストの側で、同じ Slack アカウントの連携・解除が取るロックを持つ
async fn hold_account_lock<'a>(
    db: &'a mut toasty::Db,
    team: &str,
    slack_user: &str,
) -> anyhow::Result<toasty::Transaction<'a>> {
    let mut tx = db.transaction().await?;
    query(
        &mut tx,
        "SELECT json_build_object('locked',pg_advisory_xact_lock(hashtextextended($1,0)) IS NULL)::text",
        &[&format!("slack-account:{team}:{slack_user}")],
    )
    .await?;
    Ok(tx)
}

/// 別のワークスペースのアカウントを拒否するときも、そのアカウントに連携済みなら同じトークンを失効させない
#[tokio::test]
async fn test_slack_link_keeps_token_of_existing_link_on_other_team() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let other = format!("TO{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-shared-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-x", &other, "U2", &token).await;
    mock_users_info(&slack).await;
    mock_revoke(&slack, &token, 0).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let carol = create_user(&app, "link_team_carol").await?;
    let dave = create_user(&app, "link_team_dave").await?;
    // dave は SLACK_TEAM_ID を設定する前に、別のワークスペースの U2 と連携していた
    exec(
        &state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U2','x')",
        &[&dave, &other],
    )
    .await?;
    let (status, body, _) = complete(&app, &carol, "code-x").await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "team_mismatch");
    Ok(())
}

/// 交換から保存までの間に失効したトークンは保存しない
#[tokio::test]
async fn test_slack_link_rejects_revoked_token() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-revoked-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-r", &team, "U1", &token).await;
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .and(header_is("authorization", format!("Bearer {token}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "token_revoked"})),
        )
        .mount(&slack)
        .await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_revoked").await?;
    let (status, body, _) = complete(&app, &alice, "code-r").await?;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    let links = sql(
        &state,
        "SELECT json_build_object('u',user_id)::text FROM slack_user_links WHERE user_id=$1",
        &[&alice],
    )
    .await?;
    assert!(links.is_empty());
    let mapping = sql(
        &state,
        "SELECT json_build_object('u',user_id)::text FROM slack_users WHERE team_id=$1 AND linked",
        &[&team],
    )
    .await?;
    assert!(mapping.is_empty());
    Ok(())
}

/// トークンが有効かは、同じアカウントのロックを取ってから確かめる (ロック待ちの間の失効を見逃さない)
#[tokio::test]
async fn test_slack_link_checks_token_after_account_lock() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-late-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-l", &team, "U1", &token).await;
    mock_users_info(&slack).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_late").await?;
    let oauth = oauth_state(&app, &alice).await?;

    let mut db = state.db.clone();
    let held = hold_account_lock(&mut db, &team, "U1").await?;
    let task = tokio::spawn({
        let (app, alice) = (app.clone(), alice.clone());
        async move {
            request(
                &app,
                "POST",
                "/api/slack/oauth/complete",
                Some(&alice),
                Some(json!({"code": "code-l", "state": oauth})),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    // ロックを持っている間に、同じトークンが失効した
    Mock::given(method("GET"))
        .and(path("/users.info"))
        .and(header_is("authorization", format!("Bearer {token}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "token_revoked"})),
        )
        .with_priority(1)
        .mount(&slack)
        .await;
    held.commit().await?;
    let (status, body, _) = task.await??;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    let links = sql(
        &state,
        "SELECT json_build_object('u',user_id)::text FROM slack_user_links WHERE user_id=$1",
        &[&alice],
    )
    .await?;
    assert!(links.is_empty());
    Ok(())
}

/// 同じ Slack アカウントを同時に連携したとき、後の連携は先の連携を待ち、先の連携のトークンを失効させない
#[tokio::test]
async fn test_slack_link_waits_for_inflight_link_of_same_account() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-race-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-y", &team, "U1", &token).await;
    mock_users_info(&slack).await;
    mock_revoke(&slack, &token, 0).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_race_alice").await?;
    let bob = create_user(&app, "link_race_bob").await?;
    // bob は同じワークスペースの別のアカウント (U9) と連携済み
    exec(
        &state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U9','x')",
        &[&bob, &team],
    )
    .await?;
    let oauth = oauth_state(&app, &bob).await?;

    // alice の連携が U1 のロックを持ったまま、まだコミットしていない
    let mut db = state.db.clone();
    let mut held = hold_account_lock(&mut db, &team, "U1").await?;
    let mut insert = toasty::sql::statement(
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U1','x')",
    );
    insert = insert.bind(alice.as_str()).bind(team.as_str());
    insert.exec(&mut held).await?;
    let task = tokio::spawn({
        let (app, bob) = (app.clone(), bob.clone());
        async move {
            request(
                &app,
                "POST",
                "/api/slack/oauth/complete",
                Some(&bob),
                Some(json!({"code": "code-y", "state": oauth})),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    held.commit().await?;
    let (status, body, _) = task.await??;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "already_linked");
    assert_eq!(revokes(&slack).await, 0);
    Ok(())
}

/// 解除の失効も、同じアカウントのロックを取ってから行う
#[tokio::test]
async fn test_slack_unlink_takes_account_lock_before_revoking() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-unlink-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-u", &team, "U1", &token).await;
    mock_users_info(&slack).await;
    mock_revoke(&slack, &token, 1).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_unlink").await?;
    let (status, body, _) = complete(&app, &alice, "code-u").await?;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut db = state.db.clone();
    let held = hold_account_lock(&mut db, &team, "U1").await?;
    let task = tokio::spawn({
        let (app, alice) = (app.clone(), alice.clone());
        async move { request(&app, "DELETE", "/api/slack/link", Some(&alice), None).await }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        revokes(&slack).await,
        0,
        "unlink must wait for the account lock"
    );
    held.commit().await?;
    let (status, body, _) = task.await??;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["link"], Value::Null);
    assert_eq!(revokes(&slack).await, 1);
    Ok(())
}

/// 別々のワークスペースが同じ Zoo チャンネルに取り込んでいても、付け替えは 1 つずつ行う
#[tokio::test]
async fn test_slack_link_serializes_across_workspaces_sharing_a_channel() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config.clone());
    let app = create_router(state.clone());
    let alice = create_user(&app, "link_shared_alice").await?;
    let bob = create_user(&app, "link_shared_bob").await?;
    let channel = uuid::Uuid::new_v4().to_string();
    let (status, _, _) = request(
        &app,
        "POST",
        "/api/channels",
        Some(&alice),
        Some(json!({"id": channel, "channel_name": format!("shared_{}", uuid::Uuid::new_v4().simple()), "is_private": false})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let mut messages = Vec::new();
    for label in ["a", "b"] {
        let team = format!("T{}", uuid::Uuid::new_v4().simple());
        let placeholder = backend::slack::user_id(&team, "U1");
        exec(
            &state,
            "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id) VALUES($1,'C1',$2)",
            &[&team, &channel],
        )
        .await?;
        exec(
            &state,
            "INSERT INTO users(id,user_name,display_name,guest_kind) VALUES($1,$2,'x (Slack)','deactivated')",
            &[&placeholder, &format!("slack-{label}.{}", &uuid::Uuid::new_v4().simple().to_string()[..8])],
        )
        .await?;
        exec(
            &state,
            "INSERT INTO slack_users(team_id,slack_user_id,user_id,name) VALUES($1,'U1',$2,'x')",
            &[&team, &placeholder],
        )
        .await?;
        let message = uuid::Uuid::new_v4().to_string();
        exec(
            &state,
            "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,$4)",
            &[&message, &placeholder, &channel, &format!("from {label}")],
        )
        .await?;
        messages.push((team, message));
    }

    let mut db1 = state.db.clone();
    let mut tx1 = db1.transaction().await?;
    backend::slack::link::reassign(&mut tx1, &messages[0].0, "U1", "a", &alice).await?;
    let mut handle = tokio::spawn({
        let (mut db, team, bob) = (state.db.clone(), messages[1].0.clone(), bob.clone());
        async move {
            let mut tx = db.transaction().await?;
            backend::slack::link::reassign(&mut tx, &team, "U1", "b", &bob).await?;
            tx.commit().await?;
            anyhow::Ok(())
        }
    });
    assert!(
        tokio::time::timeout(Duration::from_secs(2), &mut handle)
            .await
            .is_err(),
        "workspace B must wait for workspace A's reassignment"
    );
    tx1.commit().await?;
    handle.await??;
    for ((_, message), owner) in messages.iter().zip([&alice, &bob]) {
        let rows = sql(
            &state,
            "SELECT json_build_object('user_id',user_id)::text FROM messages WHERE id=$1",
            &[message],
        )
        .await?;
        assert_eq!(rows[0]["user_id"], owner.as_str());
    }
    Ok(())
}

/// 鍵を変えて復号できないトークンは失効できないので、解除は進めて Slack 側で失効させてもらう。
/// 連携の設定を外している間は失効できないので、解除せずに連携を残す。
#[tokio::test]
async fn test_slack_unlink_undecryptable_or_unconfigured() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth.revoke"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(0)
        .mount(&slack)
        .await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_old_key").await?;
    let old = backend::slack::crypto::encrypt(&[4u8; 32], "xoxp-old-key")?;
    exec(
        &state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U9',$3)",
        &[&alice, &team, &old],
    )
    .await?;
    let (status, body, _) = request(&app, "DELETE", "/api/slack/link", Some(&alice), None).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["link"], Value::Null);

    let mut off = state.clone();
    off.config.slack.client_id = None;
    off.config.slack.client_secret = None;
    off.config.slack.oauth_redirect_url = None;
    off.config.slack.token_encryption_key = None;
    let off = create_router(off);
    let bob = create_user(&app, "link_unconfigured").await?;
    let token = backend::slack::crypto::encrypt(&[3u8; 32], "xoxp-unconfigured")?;
    exec(
        &state,
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U8',$3)",
        &[&bob, &team, &token],
    )
    .await?;
    let (status, _, _) = request(&off, "DELETE", "/api/slack/link", Some(&bob), None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let kept = sql(
        &state,
        "SELECT json_build_object('u',user_id)::text FROM slack_user_links WHERE user_id=$1",
        &[&bob],
    )
    .await?;
    assert_eq!(kept.len(), 1, "the token stays until it can be revoked");
    // 連携していない人の解除は、設定がなくてもそのまま終わる
    let carol = create_user(&app, "link_none").await?;
    let (status, body, _) = request(&off, "DELETE", "/api/slack/link", Some(&carol), None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"configured": false, "link": null}));
    Ok(())
}

/// 別のワークスペースのトークンを片付けるときも、同じアカウントの連携が保存中なら、それを待ってから確かめる
#[tokio::test]
async fn test_slack_link_discard_waits_for_inflight_link() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let other = format!("TO{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-discard-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-d", &other, "U2", &token).await;
    mock_users_info(&slack).await;
    mock_revoke(&slack, &token, 0).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let carol = create_user(&app, "link_discard_carol").await?;
    let dave = create_user(&app, "link_discard_dave").await?;
    let oauth = oauth_state(&app, &carol).await?;

    // dave の連携 (別のワークスペースの U2) を保存している途中
    let mut db = state.db.clone();
    let mut held = hold_account_lock(&mut db, &other, "U2").await?;
    toasty::sql::statement(
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U2','x')",
    )
    .bind(dave.as_str())
    .bind(other.as_str())
    .exec(&mut held)
    .await?;
    let task = tokio::spawn({
        let (app, carol) = (app.clone(), carol.clone());
        async move {
            request(
                &app,
                "POST",
                "/api/slack/oauth/complete",
                Some(&carol),
                Some(json!({"code": "code-d", "state": oauth})),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        revokes(&slack).await,
        0,
        "discard must wait for the account lock"
    );
    held.commit().await?;
    let (status, body, _) = task.await??;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "team_mismatch");
    assert_eq!(revokes(&slack).await, 0);
    Ok(())
}

/// 同じ Zoo ユーザーが別々の Slack アカウントで同時に連携しても、先に保存した連携を上書きしない
#[tokio::test]
async fn test_slack_link_refuses_concurrent_switch_to_another_account() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-switch-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-s", &team, "U2", &token).await;
    mock_users_info(&slack).await;
    // 保存しなかった U2 のトークンは失効させる
    mock_revoke(&slack, &token, 1).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_switch").await?;
    let oauth = oauth_state(&app, &alice).await?;

    // alice の U1 との連携を保存している途中 (まだ行は見えない)
    let mut db = state.db.clone();
    let mut held = db.transaction().await?;
    toasty::sql::statement(
        "INSERT INTO slack_user_links(user_id,team_id,slack_user_id,access_token_enc) VALUES($1,$2,'U1','x')",
    )
    .bind(alice.as_str())
    .bind(team.as_str())
    .exec(&mut held)
    .await?;
    let task = tokio::spawn({
        let (app, alice) = (app.clone(), alice.clone());
        async move {
            request(
                &app,
                "POST",
                "/api/slack/oauth/complete",
                Some(&alice),
                Some(json!({"code": "code-s", "state": oauth})),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    held.commit().await?;
    let (status, body, _) = task.await??;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "linked_other_account");
    let link = sql(
        &state,
        "SELECT json_build_object('slack_user_id',slack_user_id,'token',access_token_enc)::text \
         FROM slack_user_links WHERE user_id=$1",
        &[&alice],
    )
    .await?;
    assert_eq!(link[0], json!({"slack_user_id": "U1", "token": "x"}));
    Ok(())
}

/// 取り込みのワーカーが同じ Slack ユーザーのプレースホルダの対応をコミットする前に連携しても、
/// 一意制約で失敗せず、その対応を連携済みにする
#[tokio::test]
async fn test_slack_link_during_import_reuses_the_uncommitted_mapping() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-import-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-i", &team, "U1", &token).await;
    mock_users_info(&slack).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_import_alice").await?;
    let oauth = oauth_state(&app, &alice).await?;
    let placeholder = backend::slack::user_id(&team, "U1");
    exec(
        &state,
        "INSERT INTO users(id,user_name,display_name,guest_kind) VALUES($1,$2,'alice (Slack)','deactivated')",
        &[&placeholder, &format!("slack-i.{}", &uuid::Uuid::new_v4().simple().to_string()[..8])],
    )
    .await?;

    // ワーカーが対応を作っている途中 (まだコミットしていない)
    let mut db = state.db.clone();
    let mut worker = db.transaction().await?;
    toasty::sql::statement(
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,name) VALUES($1,'U1',$2,'alice')",
    )
    .bind(team.as_str())
    .bind(placeholder.as_str())
    .exec(&mut worker)
    .await?;
    let task = tokio::spawn({
        let (app, alice) = (app.clone(), alice.clone());
        async move {
            request(
                &app,
                "POST",
                "/api/slack/oauth/complete",
                Some(&alice),
                Some(json!({"code": "code-i", "state": oauth})),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    worker.commit().await?;
    let (status, body, _) = tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .expect("the link must finish once the worker commits")??;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mapping = sql(
        &state,
        "SELECT json_build_object('user_id',user_id,'linked',linked)::text FROM slack_users WHERE team_id=$1 AND slack_user_id='U1'",
        &[&team],
    )
    .await?;
    assert_eq!(mapping[0], json!({"user_id": alice, "linked": true}));
    Ok(())
}

/// トークンを交換した後、連携を保存できなかった失敗でも、受け取ったトークンを失効させる
#[tokio::test]
async fn test_slack_link_discards_the_token_when_storing_fails() -> anyhow::Result<()> {
    let team = format!("T{}", uuid::Uuid::new_v4().simple());
    let token = format!("xoxp-store-{}", uuid::Uuid::new_v4().simple());
    let slack = MockServer::start().await;
    mock_access(&slack, "code-s2", &team, "U1", &token).await;
    mock_users_info(&slack).await;
    mock_revoke(&slack, &token, 1).await;
    let (state, app) = linking_state(&team, &slack).await?;
    let alice = create_user(&app, "link_store_alice").await?;
    let oauth = oauth_state(&app, &alice).await?;
    // 認可を始めた後で、人間のアカウントではなくなった (保存の前の確認で失敗する)
    exec(
        &state,
        "UPDATE users SET is_bot=true WHERE id=$1",
        &[&alice],
    )
    .await?;
    let (status, body, _) = request(
        &app,
        "POST",
        "/api/slack/oauth/complete",
        Some(&alice),
        Some(json!({"code": "code-s2", "state": oauth})),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let links = sql(
        &state,
        "SELECT json_build_object('u',user_id)::text FROM slack_user_links WHERE user_id=$1",
        &[&alice],
    )
    .await?;
    assert!(links.is_empty());
    Ok(())
}
