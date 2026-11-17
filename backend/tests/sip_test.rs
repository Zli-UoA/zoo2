use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn request(
    app: &Router,
    actor: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("X-Dev-User", actor)
        .header("content-type", "application/json")
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(serde_json::to_vec(&body)?)
        })?;
    let response = app.clone().oneshot(req).await?;
    let status = response.status();
    if status.is_success() {
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}
#[tokio::test]
async fn sip_personal_and_shared_authorization_and_encryption() -> anyhow::Result<()> {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let gateway = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/janus"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"janus":"success","data":{"id":1}})),
        )
        .mount(&gateway)
        .await;
    Mock::given(method("POST"))
        .and(path("/janus/1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"janus":"success","data":{"id":2}})),
        )
        .mount(&gateway)
        .await;
    Mock::given(method("POST"))
        .and(path("/janus/1/2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"janus":"ack"})))
        .mount(&gateway)
        .await;
    Mock::given(method("GET"))
        .and(path("/janus/1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"janus":"keepalive"}))
                .set_delay(std::time::Duration::from_secs(1)),
        )
        .mount(&gateway)
        .await;
    let mut config = Config::from_env();
    config.phone_gateway = backend::phone_gateway::GatewayConfig {
        url: Some(format!("{}/janus", gateway.uri())),
        api_secret: Some("dummy-gateway-secret".into()),
        pbx_uri: Some("sip:pbx.example:5060".into()),
        pbx_config_secret: Some("dummy-config".into()),
        media_mode: None,
    };
    config.dev_auth_enabled = true;
    config.sip_encryption_key = Some(STANDARD.encode([3; 32]));
    config.webrtc_ice_servers_json =
        r#"[{"urls":"TURN:relay.example:3478","username":"dummy","credential":"dummy"}]"#.into();
    let mut db = init_db(&config.database_url).await?;
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let admin = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob, &admin] {
        toasty::sql::statement(
            "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'SIP test')",
        )
        .bind(id.clone())
        .exec(&mut db)
        .await?;
    }
    toasty::sql::statement(
        "INSERT INTO role(id,role_name) VALUES('role_admin','admin') ON CONFLICT DO NOTHING",
    )
    .exec(&mut db)
    .await?;
    toasty::sql::statement("INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin')")
        .bind(admin.clone())
        .exec(&mut db)
        .await?;
    let app = create_router(AppState::new(db.clone(), config.clone()));
    let mut input = json!({"name":"Home","shared":false,"websocket_path":"/sip/ws","sip_domain":"pbx.example","username":"alice","password":"dummy-sip-password","phone_number":"0312345678","member_ids":[],"phones":[{"name":"Home desk","extension":"101"}]});
    let (status, result) = request(&app, &alice, "POST", "/api/sip/lines", input.clone()).await?;
    assert_eq!(status, StatusCode::OK, "{result}");
    let personal = result["data"]["id"].as_str().unwrap().to_string();
    let path = format!("/api/sip/lines/{personal}");
    for actor in [&bob, &admin] {
        let (s, v) = request(&app, actor, "GET", "/api/sip/lines", Value::Null).await?;
        assert_eq!(s, StatusCode::OK);
        assert!(!v.to_string().contains(&personal));
        for method in ["POST", "DELETE"] {
            assert_eq!(
                request(&app, actor, method, &path, input.clone()).await?.0,
                StatusCode::NOT_FOUND
            );
        }
        assert_eq!(
            request(&app, actor, "POST", &format!("{path}/connect"), Value::Null)
                .await?
                .0,
            StatusCode::NOT_FOUND
        );
    }
    let (_, catalog) = request(&app, &alice, "GET", "/api/sip/lines", Value::Null).await?;
    assert!(!catalog.to_string().contains("dummy-sip-password"));
    assert!(!catalog.to_string().contains("password_ciphertext"));
    assert_eq!(catalog["lines"][0]["phones"][0]["extension"], "101");
    let phone_id = catalog["lines"][0]["phones"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    input["phones"][0]["id"] = json!(phone_id);
    let (_, credentials) = request(
        &app,
        &alice,
        "POST",
        &format!("{path}/connect"),
        Value::Null,
    )
    .await?;
    assert!(credentials["data"]["session_id"].is_string());
    for secret in [
        "password",
        "username",
        "sip_domain",
        "websocket_path",
        "gateway_session_id",
        "gateway_handle_id",
    ] {
        assert!(
            credentials["data"].get(secret).is_none(),
            "client must not receive {secret}"
        );
    }
    assert!(!credentials.to_string().contains("dummy-sip-password"));
    let session = credentials["data"]["session_id"].as_str().unwrap();
    let command_path = format!("/api/phone/sessions/{session}/command");
    assert_eq!(
        request(
            &app,
            &bob,
            "POST",
            &command_path,
            json!({"action":"hangup"})
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(request(&app,&alice,"POST",&command_path,json!({"action":"call","number":"sip:attacker@other","signal":{"type":"offer","sdp":"v=0"}})).await?.0,StatusCode::BAD_REQUEST);
    let connect_path = format!("{path}/connect");
    let results = futures_util::future::join_all(
        (0..5).map(|_| request(&app, &alice, "POST", &connect_path, Value::Null)),
    )
    .await;
    let mut extra_sessions = Vec::new();
    let mut rejected = 0;
    for result in results {
        let (status, value) = result?;
        if status == StatusCode::OK {
            extra_sessions.push(value["data"]["session_id"].as_str().unwrap().to_owned());
        } else {
            assert_eq!(status, StatusCode::CONFLICT, "{value}");
            rejected += 1;
        }
    }
    assert_eq!(extra_sessions.len(), 3);
    assert_eq!(rejected, 2);
    for extra in extra_sessions {
        assert_eq!(
            request(
                &app,
                &alice,
                "POST",
                &format!("/api/phone/sessions/{extra}/command"),
                json!({"action":"disconnect"})
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    let stream_path = format!("/api/phone/sessions/{session}/stream");
    let stream_request = || {
        Request::builder()
            .method("POST")
            .uri(&stream_path)
            .header("X-Dev-User", &alice)
            .body(Body::empty())
            .unwrap()
    };
    let stream = app.clone().oneshot(stream_request()).await?;
    assert_eq!(stream.status(), StatusCode::OK);
    let duplicate = app.clone().oneshot(stream_request()).await?;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    drop(stream);
    for _ in 0..100 {
        if request(
            &app,
            &alice,
            "POST",
            &command_path,
            json!({"action":"hangup"}),
        )
        .await?
        .0 == StatusCode::NOT_FOUND
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        request(
            &app,
            &alice,
            "POST",
            &command_path,
            json!({"action":"hangup"})
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    async fn pbx(
        app: &Router,
        secret: &str,
        table: &str,
        body: &str,
    ) -> anyhow::Result<(StatusCode, String)> {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/internal/pbx/{table}/single"))
                    .header("Authorization", format!("Bearer {secret}"))
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from(body.to_owned()))?,
            )
            .await?;
        let status = response.status();
        let body = to_bytes(response.into_body(), 262144).await?;
        Ok((status, String::from_utf8(body.to_vec())?))
    }
    assert_eq!(
        pbx(&app, "wrong", "auths", &format!("id=auth-{personal}"))
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, auth) = pbx(
        &app,
        "dummy-config",
        "auths",
        &format!("id=auth-{personal}"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(auth.contains("password=dummy-sip-password"));
    let (_, phone) = pbx(
        &app,
        "dummy-config",
        "endpoints",
        &format!("id=phone-{phone_id}"),
    )
    .await?;
    assert!(phone.contains("context=zoo-call"));
    assert!(!phone.contains("password"));
    let (_, dial) = pbx(
        &app,
        "dummy-config",
        "extensions",
        &format!("context=zoo-line-{personal}&exten=101&priority=1"),
    )
    .await?;
    assert!(dial.contains(&format!("phone-{phone_id}")), "{dial}");
    assert!(dial.contains("app=Gosub"), "{dial}");
    assert!(!dial.contains("PJSIP_DIAL_CONTACTS"));
    let (_, incoming) = pbx(
        &app,
        "dummy-config",
        "extensions",
        &format!("context=zoo-home-{personal}&exten=s&priority=1"),
    )
    .await?;
    assert!(
        incoming.contains("app=Gosub") && incoming.contains(&format!("line-{personal}")),
        "{incoming}"
    );
    assert!(incoming.len() < 256 && !incoming.contains("PJSIP_DIAL_CONTACTS"));

    let (_, unknown) = pbx(
        &app,
        "dummy-config",
        "endpoints",
        "id=phone-550e8400-e29b-41d4-a716-446655440000",
    )
    .await?;
    assert!(unknown.is_empty());
    let (s, authorization) = request(
        &app,
        &alice,
        "POST",
        &format!("{path}/authorize"),
        Value::Null,
    )
    .await?;
    assert_eq!(s, StatusCode::OK);
    assert!(authorization["data"]["revision"].is_string());
    assert!(!authorization.to_string().contains("dummy-sip-password"));
    let mut tx = db.transaction().await?;
    let stored = backend::apps::one(
        &mut tx,
        "SELECT json_build_object('secret',password_ciphertext)::text FROM sip_lines WHERE id=$1",
        &[&personal],
    )
    .await?;
    assert_ne!(stored["secret"], "dummy-sip-password");
    tx.rollback().await?;
    input["password"] = json!("");
    input["name"] = json!("Home renamed");
    assert_eq!(
        request(&app, &alice, "POST", &path, input.clone()).await?.0,
        StatusCode::OK
    );
    let (status, connection) = request(
        &app,
        &alice,
        "POST",
        &format!("{path}/connect"),
        Value::Null,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{connection}");
    assert!(connection["data"]["password"].is_null());
    let requests = gateway.received_requests().await.unwrap();
    let registrations: Vec<Value> = requests
        .iter()
        .filter_map(|r| r.body_json::<Value>().ok())
        .filter(|v| v["body"]["request"] == "register")
        .collect();
    assert_eq!(registrations.len(), 5);
    for registration in registrations {
        assert_eq!(registration["body"]["secret"], "dummy-sip-password");
        assert_eq!(registration["body"]["proxy"], "sip:pbx.example:5060");
    }
    // Editing and connecting used to lock user/line in opposite orders.
    for _ in 0..8 {
        let (connected, edited) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            tokio::join!(
                request(&app, &alice, "POST", &connect_path, Value::Null),
                request(&app, &alice, "POST", &path, input.clone())
            )
        })
        .await?;
        let (status, value) = connected?;
        assert!(
            matches!(
                status,
                StatusCode::OK | StatusCode::NOT_FOUND | StatusCode::CONFLICT
            ),
            "{status}: {value}"
        );
        assert_eq!(edited?.0, StatusCode::OK);
        if status == StatusCode::OK {
            request(
                &app,
                &alice,
                "POST",
                &format!(
                    "/api/phone/sessions/{}/command",
                    value["data"]["session_id"].as_str().unwrap()
                ),
                json!({"action":"disconnect"}),
            )
            .await?;
        }
    }
    let (_, updated) = request(&app, &alice, "GET", "/api/sip/lines", Value::Null).await?;
    assert_eq!(updated["lines"][0]["phones"][0]["id"], phone_id);
    input["phones"][0].as_object_mut().unwrap().remove("id");
    input["shared"] = json!(true);
    assert_eq!(
        request(&app, &alice, "POST", &path, input.clone()).await?.0,
        StatusCode::BAD_REQUEST
    );
    input["password"] = json!("dummy-shared-password");
    input["member_ids"] = json!([bob]);
    assert_eq!(
        request(&app, &alice, "POST", "/api/sip/lines", input.clone())
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    let (s, shared) = request(&app, &admin, "POST", "/api/sip/lines", input.clone()).await?;
    assert_eq!(s, StatusCode::OK, "{shared}");
    let shared = shared["data"]["id"].as_str().unwrap();
    let shared_path = format!("/api/sip/lines/{shared}");
    let (_, catalog) = request(&app, &bob, "GET", "/api/sip/lines", Value::Null).await?;
    assert_eq!(catalog["lines"][0]["can_manage"], false);
    assert_eq!(catalog["lines"][0]["can_connect"], true);
    assert_eq!(
        request(&app, &bob, "POST", &shared_path, input.clone())
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            &bob,
            "POST",
            &format!("{shared_path}/connect"),
            Value::Null
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            &admin,
            "POST",
            &format!("{shared_path}/connect"),
            Value::Null
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    // Shared lines belong to the workspace, not to the administrator who created them.
    toasty::sql::statement("UPDATE users SET is_deactivated=true WHERE id=$1")
        .bind(admin.clone())
        .exec(&mut db)
        .await?;
    let (status, shared_auth) =
        pbx(&app, "dummy-config", "auths", &format!("id=auth-{shared}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(shared_auth.contains("password=dummy-shared-password"));
    toasty::sql::statement("UPDATE users SET is_deactivated=false WHERE id=$1")
        .bind(admin.clone())
        .exec(&mut db)
        .await?;
    input["member_ids"] = json!([]);
    input["password"] = json!("");
    assert_eq!(
        request(&app, &admin, "POST", &shared_path, input.clone())
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            &bob,
            "POST",
            &format!("{shared_path}/connect"),
            Value::Null
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    toasty::sql::statement("UPDATE users SET is_deactivated=true WHERE id=$1")
        .bind(alice.clone())
        .exec(&mut db)
        .await?;
    assert!(
        pbx(
            &app,
            "dummy-config",
            "auths",
            &format!("id=auth-{personal}")
        )
        .await?
        .1
        .is_empty()
    );
    assert!(
        !request(
            &app,
            &alice,
            "POST",
            &format!("{path}/connect"),
            Value::Null
        )
        .await?
        .0
        .is_success()
    );
    toasty::sql::statement("UPDATE users SET is_deactivated=false WHERE id=$1")
        .bind(alice.clone())
        .exec(&mut db)
        .await?;
    let mut lan_config = config.clone();
    lan_config.phone_gateway.media_mode = Some("lan".into());
    lan_config.webrtc_ice_servers_json = "[]".into();
    let lan_app = create_router(AppState::new(db.clone(), lan_config));
    let (status, lan) = request(
        &lan_app,
        &alice,
        "POST",
        &format!("{path}/connect"),
        Value::Null,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{lan}");
    assert_eq!(lan["data"]["relay_only"], false);
    assert_eq!(lan["data"]["ice_servers"], json!([]));
    let mut relay_config = config.clone();
    relay_config.webrtc_ice_servers_json = "[]".into();
    let relay_app = create_router(AppState::new(db.clone(), relay_config));
    assert_eq!(
        request(
            &relay_app,
            &alice,
            "POST",
            &format!("{path}/connect"),
            Value::Null
        )
        .await?
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    for (actor, path) in [(&alice, &path), (&admin, &shared_path)] {
        assert_eq!(
            request(&app, actor, "DELETE", path, Value::Null).await?.0,
            StatusCode::OK
        );
    }
    let mut disabled = config;
    disabled.sip_encryption_key = None;
    let disabled = create_router(AppState::new(db.clone(), disabled));
    assert_eq!(
        request(&disabled, &alice, "GET", "/api/sip/lines", Value::Null)
            .await?
            .1["enabled"],
        false
    );
    assert_eq!(
        request(&disabled, &alice, "POST", "/api/sip/lines", input)
            .await?
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    toasty::sql::statement("DELETE FROM user_role WHERE user_id=$1")
        .bind(admin.clone())
        .exec(&mut db)
        .await?;
    for id in [alice, bob, admin] {
        toasty::sql::statement("DELETE FROM users WHERE id=$1")
            .bind(id)
            .exec(&mut db)
            .await?;
    }
    Ok(())
}
