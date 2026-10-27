use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
async fn req(
    app: &Router,
    method: &str,
    path: &str,
    actor: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if actor.starts_with("zoo_at_") {
        r = r.header("authorization", format!("Bearer {actor}"));
    } else {
        r = r.header("x-dev-user", actor);
    }
    let response = app
        .clone()
        .oneshot(r.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
async fn app_approval_auth_channel_events_and_revocation() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let owner = Uuid::new_v4().to_string();
    let stranger = Uuid::new_v4().to_string();
    let admin = backend::auth::DEFAULT_DEV_USER_ID;
    for actor in [&owner, &stranger, &admin.to_string()] {
        let status=req(&app,"POST","/api/users",actor,json!({"id":actor,"user_name":format!("appstest_{}",Uuid::new_v4().simple()),"display_name":"App tester"})).await.0;
        assert!(status == StatusCode::OK || (actor == admin && status == StatusCode::CONFLICT));
    }
    let manifest = json!({"scopes":["messages:read","messages:write","dm:write","ui:write"],"events":["message.created"],"transport":"websocket","dm_enabled":true,"redirect_uris":["https://example.com/callback"]});
    let (status, created) = req(
        &app,
        "POST",
        "/api/apps",
        &owner,
        json!({"name":"Echo","description":"Echo app","manifest":manifest}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap();
    let bot = created["bot_user_id"].as_str().unwrap();
    let action = |a: &str| format!("/api/apps/{id}/actions/{a}");
    assert_eq!(
        req(&app, "POST", &action("token"), &owner, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        req(&app, "POST", &action("submit"), &owner, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        req(&app, "POST", &action("install"), &owner, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &action("install"),
            admin,
            json!({"revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(&app, "POST", &action("token"), &stranger, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, token) = req(&app, "POST", &action("token"), &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{token}");
    let access = token["access_token"].as_str().unwrap();
    let channel = Uuid::new_v4().to_string();
    assert_eq!(req(&app,"POST","/api/channels",&owner,json!({"id":channel,"channel_name":format!("appstest_{}",Uuid::new_v4().simple()),"is_private":true})).await.0,StatusCode::OK);
    let message = Uuid::new_v4().to_string();
    let body = json!({"id":message,"channel_id":channel,"content":"hello"});
    assert_eq!(
        req(&app, "POST", "/api/apps/v1/messages", access, body.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/channels/{channel}"),
            &stranger,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/channels/{channel}"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, posted) = req(&app, "POST", "/api/apps/v1/messages", access, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{posted}");
    assert_eq!(posted["data"]["user_id"], bot);
    assert_eq!(
        req(&app, "POST", "/api/apps/v1/messages", access, body)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "POST",
            "/api/messages",
            &owner,
            json!({"channel_id":channel,"content":"human event"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut database = db.clone();
    let mut tx = database.transaction().await?;
    let installs = backend::apps::query(
        &mut tx,
        "SELECT json_build_object('id',id)::text FROM app_installations WHERE app_id=$1",
        &[id],
    )
    .await?;
    let installation = installs[0]["id"].as_str().unwrap();
    drop(tx);
    let event = backend::apps::delivery::claim(
        &AppState::new(db.clone(), {
            let mut c = Config::from_env();
            c.dev_auth_enabled = true;
            c
        }),
        installation,
    )
    .await?
    .unwrap();
    assert_eq!(event["event"]["type"], "message.created");
    assert_eq!(event["event"]["actor_id"], owner);
    let (status, users) = req(&app, "GET", "/api/users", &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!users.as_array().unwrap().iter().any(|u| u["id"] == bot));
    assert_eq!(
        req(
            &app,
            "POST",
            "/api/dms",
            &owner,
            json!({"target_user_ids":[bot]})
        )
        .await
        .0,
        StatusCode::OK
    );
    // A refresh token is single-use, and rotation revokes its access token.
    let (status, rotated) = req(
        &app,
        "POST",
        "/api/apps/oauth/token",
        &owner,
        json!({"grant_type":"refresh_token","refresh_token":token["refresh_token"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert_eq!(
        req(&app, "GET", "/api/apps/v1/auth", access, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        req(
            &app,
            "POST",
            "/api/apps/oauth/token",
            &owner,
            json!({"grant_type":"refresh_token","refresh_token":token["refresh_token"]})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        req(&app, "POST", &action("disable"), &owner, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "GET",
            "/api/apps/v1/auth",
            rotated["access_token"].as_str().unwrap(),
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

async fn fixture() -> anyhow::Result<(AppState, Router, String, String, String, String, String)> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let state = AppState::new(init_db(&config.database_url).await?, config);
    let router = create_router(state.clone());
    let owner = Uuid::new_v4().to_string();
    let stranger = Uuid::new_v4().to_string();
    let admin = backend::auth::DEFAULT_DEV_USER_ID;
    for actor in [&owner, &stranger, &admin.to_string()] {
        let status=req(&router,"POST","/api/users",actor,json!({"id":actor,"user_name":format!("appfix_{}",Uuid::new_v4().simple()),"display_name":"Developer"})).await.0;
        assert!(status == StatusCode::OK || actor == admin && status == StatusCode::CONFLICT);
    }
    let manifest = json!({"scopes":["messages:read","messages:write","ui:write","commands:read"],"events":["message.created","app.mentioned","app.uninstalled"],"transport":"websocket","event_url":"https://callbacks.example.test/private-path","commands":["echo"],"message_shortcuts":["inspect"],"redirect_uris":["https://example.com/callback"]});
    let (status, app) = req(
        &router,
        "POST",
        "/api/apps",
        &owner,
        json!({"name":"Fixture","manifest":manifest}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{app}");
    let id = app["id"].as_str().unwrap().to_owned();
    assert_eq!(
        req(
            &router,
            "POST",
            &format!("/api/apps/{id}/actions/submit"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &router,
            "POST",
            &format!("/api/apps/{id}/actions/install"),
            admin,
            json!({"revision":999})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        req(
            &router,
            "POST",
            &format!("/api/apps/{id}/actions/install"),
            admin,
            json!({"revision":1})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, token) = req(
        &router,
        "POST",
        &format!("/api/apps/{id}/actions/token"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{token}");
    let channel = Uuid::new_v4().to_string();
    assert_eq!(req(&router,"POST","/api/channels",&owner,json!({"id":channel,"channel_name":format!("appfix_{}",Uuid::new_v4().simple()),"is_private":true})).await.0,StatusCode::OK);
    assert_eq!(
        req(
            &router,
            "POST",
            &format!("/api/apps/{id}/channels/{channel}"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    Ok((
        state,
        router,
        owner,
        stranger,
        id,
        token["access_token"].as_str().unwrap().to_owned(),
        channel,
    ))
}
#[tokio::test]
async fn oauth_pkce_replay_and_user_consent() -> anyhow::Result<()> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};
    let (_state, app, owner, stranger, id, _access, channel) = fixture().await?;
    let (status, public) = req(
        &app,
        "GET",
        &format!("/api/apps/{id}"),
        &stranger,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(public["manifest"].get("event_url").is_none());
    let (_, private) = req(&app, "GET", &format!("/api/apps/{id}"), &owner, Value::Null).await;
    assert_eq!(
        private["manifest"]["event_url"],
        "https://callbacks.example.test/private-path"
    );

    let verifier = "a".repeat(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let authorize = json!({"app_id":id,"redirect_uri":"https://example.com/callback","code_challenge":challenge,"scopes":["messages:write"],"state":"opaque-state"});
    let mut invalid = authorize.clone();
    invalid["redirect_uri"] = json!("https://attacker.example/callback");
    assert_eq!(
        req(&app, "POST", "/api/apps/oauth/authorize", &owner, invalid)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (status, grant) = req(&app, "POST", "/api/apps/oauth/authorize", &owner, authorize).await;
    assert_eq!(status, StatusCode::OK, "{grant}");
    let url = reqwest::Url::parse(grant["redirect_uri"].as_str().unwrap())?;
    let code = url
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let exchange = json!({"grant_type":"authorization_code","code":code,"redirect_uri":"https://example.com/callback","code_verifier":verifier});
    let mut invalid = exchange.clone();
    invalid["code_verifier"] = json!("b".repeat(64));
    assert_eq!(
        req(&app, "POST", "/api/apps/oauth/token", &stranger, invalid)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, tokens) = req(
        &app,
        "POST",
        "/api/apps/oauth/token",
        &stranger,
        exchange.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(
        req(&app, "POST", "/api/apps/oauth/token", &stranger, exchange)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let access = tokens["access_token"].as_str().unwrap();
    let (status, message) = req(
        &app,
        "POST",
        "/api/apps/v1/messages",
        access,
        json!({"channel_id":channel,"content":"delegated"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{message}");
    assert_eq!(message["data"]["user_id"], owner);
    assert_eq!(
        req(
            &app,
            "POST",
            "/api/apps/v1/surfaces",
            access,
            json!({"kind":"home","user_id":owner,"blocks":[]})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req(
            &app,
            "DELETE",
            &format!("/api/apps/{id}/authorization"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(&app, "GET", "/api/apps/v1/auth", access, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}
#[tokio::test]
async fn private_ui_trigger_binding_and_transactional_events() -> anyhow::Result<()> {
    let (state, app, owner, stranger, id, access, channel) = fixture().await?;
    let (status, invocation) = req(
        &app,
        "POST",
        "/api/apps/commands/echo",
        &owner,
        json!({"channel_id":channel,"text":"hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{invocation}");
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let row = backend::apps::one(
        &mut tx,
        "SELECT payload::jsonb::text FROM app_deliveries WHERE event_id=$1",
        &[invocation["event_id"].as_str().unwrap()],
    )
    .await?;
    drop(tx);
    let publish = json!({"kind":"modal","user_id":owner,"channel_id":channel,"trigger_id":row["data"]["trigger_id"],"blocks":[{"type":"input","action_id":"answer","label":"Answer","required":true}]});
    let mut wrong = publish.clone();
    wrong["channel_id"] = Value::Null;
    assert_eq!(
        req(&app, "POST", "/api/apps/v1/surfaces", &access, wrong)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, surface) = req(
        &app,
        "POST",
        "/api/apps/v1/surfaces",
        &access,
        publish.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{surface}");
    assert_eq!(
        req(&app, "POST", "/api/apps/v1/surfaces", &access, publish)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, private) = req(&app, "GET", "/api/app-surfaces", &stranger, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !private
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == surface["id"])
    );
    let submit = json!({"kind":"submit","surface_id":surface["id"],"channel_id":channel,"values":{"answer":"yes"}});
    let mut missing = submit.clone();
    missing["values"] = json!({});
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/invoke"),
            &owner,
            missing
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/invoke"),
            &owner,
            submit.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/invoke"),
            &owner,
            submit
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // Async response tokens can create only one private response for the original user/channel.
    let path = row["data"]["response_path"].as_str().unwrap();
    let response = json!([{"type":"text","text":"Only you"}]);
    assert_eq!(
        req(&app, "POST", path, &stranger, response.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(
        req(&app, "POST", path, &stranger, response).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, post) = req(
        &app,
        "POST",
        "/api/apps/v1/messages",
        &access,
        json!({"channel_id":channel,"content":"interactive message"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{post}");
    let message_id = post["data"]["id"].as_str().unwrap();
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/v1/messages/{message_id}/blocks"),
            &access,
            json!([{"type":"button","action_id":"inspect","label":"Inspect"}])
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(req(&app,"POST",&format!("/api/apps/{id}/invoke"),&owner,json!({"kind":"action","channel_id":channel,"message_id":message_id,"action_id":"inspect"})).await.0,StatusCode::OK);
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/invoke"),
            &owner,
            json!({"kind":"shortcut","channel_id":channel,"message_id":message_id,"name":"inspect"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/invoke"),
            &stranger,
            json!({"kind":"shortcut","channel_id":channel,"message_id":message_id,"name":"inspect"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let rolled_back = Uuid::new_v4().to_string();
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "INSERT INTO messages(id,user_id,channel_id,content) VALUES($1,$2,$3,'rolled back')",
    )
    .bind(rolled_back.as_str())
    .bind(owner.as_str())
    .bind(channel.as_str())
    .exec(&mut tx)
    .await?;
    tx.rollback().await?;
    let mut tx = db.transaction().await?;
    let v=backend::apps::one(&mut tx,"SELECT json_build_object('count',count(*))::text FROM app_deliveries WHERE payload::jsonb->'data'->>'id'=$1",&[&rolled_back]).await?;
    assert_eq!(v["count"], 0);
    Ok(())
}
#[tokio::test]
async fn websocket_delivery_ack_and_single_use_ticket() -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    let (state, app, owner, _stranger, id, access, channel) = fixture().await?;
    assert_eq!(
        req(
            &app,
            "POST",
            "/api/messages",
            &owner,
            json!({"channel_id":channel,"content":"socket message"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, ticket) = req(
        &app,
        "POST",
        "/api/apps/v1/socket-ticket",
        &access,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ticket}");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!(
        "ws://{address}/api/apps/v1/socket?ticket={}",
        ticket["ticket"].as_str().unwrap()
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await?;
    assert!(tokio_tungstenite::connect_async(&url).await.is_err());
    let hello = ws.next().await.unwrap()?;
    assert_eq!(
        serde_json::from_str::<Value>(hello.to_text()?)?["type"],
        "hello"
    );
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await?
        .unwrap()?;
    let event: Value = serde_json::from_str(message.to_text()?)?;
    assert_eq!(event["event"]["data"]["content"], "socket message");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"ack":event["delivery_id"],"ack_token":event["ack_token"]})
            .to_string()
            .into(),
    ))
    .await?;
    tokio::time::timeout(std::time::Duration::from_secs(2),async{
 loop{let mut db=state.db.clone();let mut tx=db.transaction().await.unwrap();let v=backend::apps::one(&mut tx,"SELECT json_build_object('acked',acked_at IS NOT NULL)::text FROM app_deliveries WHERE id=$1",&[event["delivery_id"].as_str().unwrap()]).await.unwrap();if v["acked"]==true{break;}tokio::time::sleep(std::time::Duration::from_millis(20)).await;}
 }).await?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    backend::apps::one(
        &mut tx,
        "UPDATE apps SET status='disabled' WHERE id=$1 RETURNING '{}'::text",
        &[&id],
    )
    .await?;
    tx.commit().await?;
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await?
        .unwrap()?;
    let terminal: Value = serde_json::from_str(message.to_text()?)?;
    assert_eq!(terminal["event"]["type"], "app.uninstalled");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"ack":terminal["delivery_id"],"ack_token":terminal["ack_token"]})
            .to_string()
            .into(),
    ))
    .await?;
    server.abort();
    Ok(())
}

#[tokio::test]
async fn reapproval_invalidates_old_codes_tokens_and_queued_events() -> anyhow::Result<()> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};
    let (state, app, owner, _stranger, id, access, channel) = fixture().await?;
    let verifier = "c".repeat(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let(status,grant)=req(&app,"POST","/api/apps/oauth/authorize",&owner,json!({"app_id":id,"redirect_uri":"https://example.com/callback","code_challenge":challenge,"scopes":["messages:write"],"state":"revision-test"})).await;
    assert_eq!(status, StatusCode::OK, "{grant}");
    let url = reqwest::Url::parse(grant["redirect_uri"].as_str().unwrap())?;
    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .into_owned();
    assert_eq!(
        req(
            &app,
            "POST",
            "/api/messages",
            &owner,
            json!({"channel_id":channel,"content":"old authorization"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "PATCH",
            &format!("/api/apps/{id}"),
            &owner,
            json!({"name":"Restricted app","manifest":{"scopes":[],"events":[]}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/actions/submit"),
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/apps/{id}/actions/install"),
            backend::auth::DEFAULT_DEV_USER_ID,
            json!({"revision":2})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        req(&app, "GET", "/api/apps/v1/auth", &access, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(req(&app,"POST","/api/apps/oauth/token",&owner,json!({"grant_type":"authorization_code","code":code,"redirect_uri":"https://example.com/callback","code_verifier":verifier})).await.0,StatusCode::UNAUTHORIZED);
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let row = backend::apps::one(
        &mut tx,
        "SELECT json_build_object('id',id)::text FROM app_installations WHERE app_id=$1",
        &[&id],
    )
    .await?;
    drop(tx);
    assert!(
        backend::apps::delivery::claim(&state, row["id"].as_str().unwrap())
            .await?
            .is_none()
    );
    Ok(())
}
