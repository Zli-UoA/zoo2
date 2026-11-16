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
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    config.sip_encryption_key = Some(STANDARD.encode([3; 32]));
    config.webrtc_ice_servers_json =
        r#"[{"urls":"turn:relay.example:3478","username":"dummy","credential":"dummy"}]"#.into();
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
    let (_, credentials) = request(
        &app,
        &alice,
        "POST",
        &format!("{path}/connect"),
        Value::Null,
    )
    .await?;
    assert_eq!(credentials["data"]["password"], "dummy-sip-password");
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
    assert_eq!(
        request(
            &app,
            &alice,
            "POST",
            &format!("{path}/connect"),
            Value::Null
        )
        .await?
        .1["data"]["password"],
        "dummy-sip-password"
    );
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
