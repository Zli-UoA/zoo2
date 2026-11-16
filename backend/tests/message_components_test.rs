//! メッセージ部品（Discord Components V2 形式）の App API・操作・選択肢の問い合わせ。
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{apps, config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn request_with_headers(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    r = if actor.starts_with("zoo_at_") {
        r.header("authorization", format!("Bearer {actor}"))
    } else {
        r.header("x-dev-user", actor)
    };
    let response = app
        .clone()
        .oneshot(r.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
    )
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (s, v, _) = request_with_headers(app, method, path, actor, body).await;
    (s, v)
}
async fn ok(app: &Router, method: &str, path: &str, actor: &str, body: Value) -> Value {
    let (s, v) = request(app, method, path, actor, body).await;
    assert_eq!(s, StatusCode::OK, "{method} {path}: {v}");
    v
}
#[allow(dead_code)]
struct Fixture {
    state: AppState,
    app: Router,
    owner: String,
    other: String,
    id: String,
    bot: String,
    installation: String,
    token: String,
    grant: String,
    grant_refresh: String,
    channel: String,
}
async fn fixture() -> anyhow::Result<Fixture> {
    fixture_with_events(json!(apps::EVENTS)).await
}

async fn fixture_with_events(events: Value) -> anyhow::Result<Fixture> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let state = AppState::new(init_db(&config.database_url).await?, config);
    let app = create_router(state.clone());
    let owner = Uuid::new_v4().to_string();
    let other = Uuid::new_v4().to_string();
    let admin = backend::auth::DEFAULT_DEV_USER_ID;
    for actor in [&owner, &other, &admin.to_owned()] {
        let(s,v)=request(&app,"POST","/api/users",actor,json!({"id":actor,"user_name":format!("evt_{}",Uuid::new_v4().simple()),"display_name":"Events"})).await;
        assert!(s == StatusCode::OK || s == StatusCode::CONFLICT, "{v}");
    }
    let manifest = json!({"scopes":apps::SCOPES,"events":events,"transport":"websocket","include_bot_events":true,"user_connections":true,"functions":["echo"],"commands":["event"],"shortcuts":["event"],"message_shortcuts":["event"],"link_domains":["example.com"],"dm_enabled":true,"redirect_uris":["https://example.com/callback"]});
    let created = ok(
        &app,
        "POST",
        "/api/apps",
        &owner,
        json!({"name":"Event coverage","manifest":manifest}),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_owned();
    let bot = created["bot_user_id"].as_str().unwrap().to_owned();
    ok(
        &app,
        "POST",
        &format!("/api/apps/{id}/actions/submit"),
        &owner,
        Value::Null,
    )
    .await;
    ok(
        &app,
        "POST",
        &format!("/api/apps/{id}/actions/install"),
        admin,
        json!({"revision":1}),
    )
    .await;
    let t = ok(
        &app,
        "POST",
        &format!("/api/apps/{id}/actions/token"),
        &owner,
        Value::Null,
    )
    .await;
    let token = t["access_token"].as_str().unwrap().to_owned();
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let i = apps::one(
        &mut tx,
        "SELECT json_build_object('id',id)::text FROM app_installations WHERE app_id=$1",
        &[&id],
    )
    .await?;
    let installation = i["id"].as_str().unwrap().to_owned();
    // Seed the user's explicit consent using the same credential minting code as OAuth.
    let credentials =
        apps::mint(&mut tx, &installation, Some(&owner), &json!(apps::SCOPES)).await?;
    let grant = credentials["access_token"].as_str().unwrap().to_owned();
    let grant_refresh = credentials["refresh_token"].as_str().unwrap().to_owned();
    tx.commit().await?;
    let c = ok(
        &app,
        "POST",
        "/api/channels",
        &owner,
        json!({"channel_name":format!("evt_{}",Uuid::new_v4().simple()),"is_private":false}),
    )
    .await;
    let channel = c["data"]["id"].as_str().unwrap().to_owned();
    ok(
        &app,
        "POST",
        &format!("/api/apps/{id}/channels/{channel}"),
        &owner,
        Value::Null,
    )
    .await;
    Ok(Fixture {
        state,
        app,
        owner,
        other,
        id,
        bot,
        installation,
        token,
        grant,
        grant_refresh,
        channel,
    })
}

async fn delivery(f: &Fixture, event_id: &str) -> anyhow::Result<Value> {
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    Ok(apps::one(
        &mut tx,
        "SELECT payload::jsonb::text FROM app_deliveries WHERE event_id=$1",
        &[event_id],
    )
    .await?)
}

async fn app_message(f: &Fixture) -> String {
    let post = ok(
        &f.app,
        "POST",
        "/api/apps/v1/messages",
        &f.token,
        json!({"channel_id":f.channel,"content":"Pick"}),
    )
    .await;
    post["data"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn app_messages_accept_discord_slack_and_legacy_components() -> anyhow::Result<()> {
    let f = fixture().await?;
    let mid = app_message(&f).await;
    let path = format!("/api/apps/v1/messages/{mid}/components");
    // Discord（文字列の type も受け付け、数値で返す）
    let (s, v, h) = request_with_headers(&f.app, "POST", &path, &f.token, json!({"flags":32768,"components":[
        {"type":"container","accent_color":"#2da44e","components":[{"type":"text_display","content":"-# hi"}]},
        {"type":1,"components":[{"type":3,"custom_id":"pick","options":[{"label":"A","value":"a"},{"label":"B","value":"b"}],"max_values":2}]},
        {"type":1,"components":[{"type":1001,"custom_id":"search","min_query_length":1}]}
    ]})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(h.get("deprecation").is_none());
    assert_eq!(v["components"]["components"][0]["type"], 17);
    assert_eq!(
        v["components"]["components"][0]["components"][0]["content"],
        "<small>hi</small>"
    );
    // Slack Block Kit
    let (s, v) = request(&f.app, "POST", &path, &f.token, json!({"blocks":[{"type":"actions","elements":[
        {"type":"datepicker","action_id":"when"},{"type":"button","text":{"type":"plain_text","text":"Go"},"action_id":"go","style":"primary"}]}]})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // 旧形式は受け付けるが、廃止予定であることを知らせる
    let (s, _, h) = request_with_headers(
        &f.app,
        "POST",
        &format!("/api/apps/v1/messages/{mid}/blocks"),
        &f.token,
        json!([{"type":"button","action_id":"old","label":"Old"}]),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get("deprecation").unwrap(), "true");
    // 知らない部品・配置の誤り・危ない URL は 400
    for bad in [
        json!({"components":[{"type":99}]}),
        json!({"blocks":[{"type":"unknown_block"}]}),
        json!({"components":[{"type":2,"label":"x","custom_id":"x"}]}),
        json!({"components":[{"type":12,"items":[{"media":{"url":"javascript:alert(1)"}}]}]}),
        json!({"components":[{"type":13,"file":{"url":"attachment://missing.txt"}}]}),
    ] {
        let (s, v) = request(&f.app, "POST", &path, &f.token, bad.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    // 人は自分の投稿に部品を付けられない（作成の API に components を渡しても無視する）
    let (s, human) = request(&f.app, "POST", "/api/messages", &f.owner, json!({"channel_id":f.channel,"content":"x","components":{"components":[{"type":10,"content":"forged"}]}})).await;
    assert_eq!(s, StatusCode::OK);
    assert!(human["data"]["components"].is_null());
    Ok(())
}

#[tokio::test]
async fn actions_validate_values_and_report_the_component() -> anyhow::Result<()> {
    let f = fixture().await?;
    let mid = app_message(&f).await;
    ok(&f.app, "POST", &format!("/api/apps/v1/messages/{mid}/components"), &f.token, json!({"components":[
        {"type":1,"components":[{"type":3,"custom_id":"pick","options":[{"label":"A","value":"a"},{"label":"B","value":"b"}],"max_values":2}]},
        {"type":1,"components":[{"type":2,"label":"Off","custom_id":"off","disabled":true},{"type":2,"label":"Go","custom_id":"go","value":"42"}]}
    ]})).await;
    let invoke = format!("/api/apps/{}/invoke", f.id);
    let base = json!({"kind":"action","channel_id":f.channel,"message_id":mid});
    let with = |action: &str, values: Value| {
        let mut b = base.clone();
        b["action_id"] = json!(action);
        b["values"] = values;
        b
    };
    let v = ok(
        &f.app,
        "POST",
        &invoke,
        &f.owner,
        with("pick", json!({"pick":["a","b"]})),
    )
    .await;
    let event = delivery(&f, v["event_id"].as_str().unwrap()).await?;
    assert_eq!(event["data"]["custom_id"], "pick");
    assert_eq!(event["data"]["component_type"], 3);
    assert_eq!(event["data"]["values"]["pick"], json!(["a", "b"]));
    for (action, values) in [
        ("pick", json!({"pick":["z"]})),
        ("pick", json!({"pick":["a","b","a"]})),
        ("off", json!({})),
        ("nope", json!({})),
    ] {
        let (s, v) = request(
            &f.app,
            "POST",
            &invoke,
            &f.owner,
            with(action, values.clone()),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{action} {values}: {v}");
    }
    ok(
        &f.app,
        "POST",
        &invoke,
        &f.owner,
        with("go", json!({"go":"42"})),
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn modal_submissions_check_required_and_formatted_inputs() -> anyhow::Result<()> {
    let f = fixture().await?;
    let invoke = format!("/api/apps/{}/invoke", f.id);
    let v = ok(
        &f.app,
        "POST",
        &invoke,
        &f.owner,
        json!({"kind":"shortcut","name":"event","channel_id":f.channel}),
    )
    .await;
    let trigger =
        delivery(&f, v["event_id"].as_str().unwrap()).await?["data"]["trigger_id"].clone();
    let surface = ok(&f.app, "POST", "/api/apps/v1/surfaces", &f.token, json!({"kind":"modal","user_id":f.owner,"channel_id":f.channel,"trigger_id":trigger,"components":[
        {"type":18,"label":"Name","component":{"type":4,"custom_id":"name","style":1}},
        {"type":18,"label":"Count","component":{"type":4,"custom_id":"count","format":"number","min_value":1,"max_value":10,"required":false}},
        {"type":18,"label":"Pick","component":{"type":21,"custom_id":"pick","options":[{"label":"A","value":"a"},{"label":"B","value":"b"}]}}
    ]})).await;
    let submit = |values: Value| json!({"kind":"submit","surface_id":surface["id"],"channel_id":f.channel,"values":values});
    for values in [
        json!({"count":"3","pick":"a"}),
        json!({"name":"n","count":"x","pick":"a"}),
        json!({"name":"n","count":"30","pick":"a"}),
        json!({"name":"n","pick":"z"}),
    ] {
        let (s, v) = request(&f.app, "POST", &invoke, &f.owner, submit(values.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{values}: {v}");
    }
    ok(
        &f.app,
        "POST",
        &invoke,
        &f.owner,
        submit(json!({"name":"n","count":"3","pick":"b"})),
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn external_selects_ask_the_app_for_options() -> anyhow::Result<()> {
    let f = fixture().await?;
    let mid = app_message(&f).await;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/v1/messages/{mid}/components"),
        &f.token,
        json!({"components":[
        {"type":1,"components":[{"type":1001,"custom_id":"search","min_query_length":2}]}]}),
    )
    .await;
    let invoke = format!("/api/apps/{}/invoke", f.id);
    let ask = |query: &str| json!({"kind":"options","channel_id":f.channel,"message_id":mid,"action_id":"search","values":{"query":query}});
    let (s, _) = request(&f.app, "POST", &invoke, &f.owner, ask("a")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "too short");
    let v = ok(&f.app, "POST", &invoke, &f.owner, ask("ab")).await;
    let request_id = v["request_id"].as_str().unwrap().to_string();
    let event = delivery(&f, v["event_id"].as_str().unwrap()).await?;
    assert_eq!(event["type"], "options.requested");
    assert_eq!(event["data"]["request_id"], request_id.as_str());
    // 別の人は、この問い合わせの答えを読めない
    let (s, _) = request(
        &f.app,
        "GET",
        &format!("/api/apps/{}/options/{request_id}", f.id),
        &f.other,
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Slack 形式の選択肢も受け付ける
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/v1/options/{request_id}"),
        &f.token,
        json!({"options":[
        {"label":"Alpha","value":"a"},{"text":{"type":"plain_text","text":"Beta"},"value":"b"}]}),
    )
    .await;
    let got = ok(
        &f.app,
        "GET",
        &format!("/api/apps/{}/options/{request_id}", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    assert_eq!(got["options"][1]["label"], "Beta");
    // 選んだ値は、問い合わせで返した選択肢のどれかとして送る
    ok(&f.app, "POST", &invoke, &f.owner, json!({"kind":"action","channel_id":f.channel,"message_id":mid,"action_id":"search","values":{"search":"b"}})).await;
    Ok(())
}

#[tokio::test]
async fn backfill_moves_legacy_cards_and_blocks_and_can_be_restored() -> anyhow::Result<()> {
    let f = fixture().await?;
    let mid = app_message(&f).await;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "UPDATE messages SET card=$2, app_blocks=$3, app_origin_id=$4, components=NULL WHERE id=$1",
    )
    .bind(mid.as_str())
    .bind(r#"{"title":"Deploy","color":"good"}"#)
    .bind(r#"[{"type":"button","action_id":"retry","label":"Retry"}]"#)
    .bind(f.id.as_str())
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    let mut lines = Vec::new();
    let report = backend::components::backfill::run(&f.state.db, true, |c| {
        lines.push(backend::components::backfill::to_line(c));
        Ok(())
    })
    .await?;
    assert!(report.applied >= 1, "{report:?}");
    let read = |db: &mut toasty::Db| {
        let mut db = db.clone();
        let mid = mid.clone();
        async move {
            let mut tx = db.transaction().await.unwrap();
            apps::one(&mut tx, "SELECT json_build_object('c',components::jsonb,'card',card)::text FROM messages WHERE id=$1", &[mid.as_str()]).await.unwrap()
        }
    };
    let row = read(&mut db).await;
    assert_eq!(row["c"]["flags"], 32768);
    assert_eq!(row["c"]["components"][0]["accent_color"], 0x2eb886);
    assert_eq!(
        row["c"]["components"][1]["components"][0]["custom_id"],
        "retry"
    );
    assert!(
        row["card"].is_string(),
        "the legacy column stays until migration B"
    );
    // 移した部品のボタンも押せる
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/invoke", f.id),
        &f.owner,
        json!({"kind":"action","channel_id":f.channel,"message_id":mid,"action_id":"retry"}),
    )
    .await;
    // 2 回目は何も変えない
    let again = backend::components::backfill::run(&f.state.db, true, |_| Ok(())).await?;
    assert!(!lines.is_empty());
    let mine: Vec<_> = lines
        .iter()
        .map(|l| backend::components::backfill::from_line(l).unwrap())
        .filter(|c| c.id == mid)
        .collect();
    assert_eq!(mine.len(), 1);
    let _ = again;
    backend::components::backfill::restore(&f.state.db, &mine).await?;
    assert!(read(&mut db).await["c"].is_null());
    Ok(())
}
