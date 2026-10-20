use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &Router,
    actor: &str,
    tenant: Option<&str>,
    method: &str,
    path: &str,
    payload: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("X-Dev-User", actor);
    if let Some(tenant) = tenant {
        req = req.header("X-Tenant-ID", tenant);
    }
    let body = if payload.is_null() {
        Body::empty()
    } else {
        req = req.header("content-type", "application/json");
        Body::from(payload.to_string())
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn ok(
    app: &Router,
    actor: &str,
    tenant: Option<&str>,
    method: &str,
    path: &str,
    payload: Value,
) -> Value {
    let (status, body) = call(app, actor, tenant, method, path, payload).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
    body
}

#[tokio::test]
async fn workspaces_isolate_api_roles_and_electric_shapes() -> anyhow::Result<()> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));
    let alice = Uuid::new_v4().to_string();
    let bob = Uuid::new_v4().to_string();
    let charlie = Uuid::new_v4().to_string();
    let bob_name = format!("bob_{}", Uuid::new_v4().simple());
    let charlie_name = format!("charlie_{}", Uuid::new_v4().simple());
    let alice_name = format!("alice_{}", Uuid::new_v4().simple());
    for (id, name) in [
        (&alice, &alice_name),
        (&bob, &bob_name),
        (&charlie, &charlie_name),
    ] {
        ok(
            &app,
            id,
            None,
            "POST",
            "/api/users",
            json!({"id":id,"user_name":name,"display_name":name}),
        )
        .await;
    }
    let a = ok(
        &app,
        &alice,
        None,
        "POST",
        "/api/tenants",
        json!({"name":"Workspace A"}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let b = ok(
        &app,
        &bob,
        None,
        "POST",
        "/api/tenants",
        json!({"name":"Workspace B"}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &app,
        &alice,
        Some(&a),
        "POST",
        "/api/tenants/members",
        json!({"user_name":charlie_name}),
    )
    .await;
    ok(
        &app,
        &bob,
        Some(&b),
        "POST",
        "/api/tenants/members",
        json!({"user_name":alice_name}),
    )
    .await;
    // Adding a member twice is an idempotent operation.
    let duplicate = ok(
        &app,
        &bob,
        Some(&b),
        "POST",
        "/api/tenants/members",
        json!({"user_name":alice_name}),
    )
    .await;
    assert!(duplicate["txid"].is_null());
    let (status, _) = call(
        &app,
        &charlie,
        Some(&a),
        "POST",
        "/api/tenants/members",
        json!({"user_name":bob_name}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &app,
        &alice,
        Some(&b),
        "POST",
        "/api/tenants/members",
        json!({"user_name":charlie_name}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "admin in A must not be admin in B"
    );
    let list = ok(&app, &charlie, None, "GET", "/api/tenants", Value::Null).await;
    assert!(!list.as_array().unwrap().iter().any(|t| t["id"] == b));

    let channel_name = format!("same_{}", Uuid::new_v4().simple());
    let ca = ok(
        &app,
        &alice,
        Some(&a),
        "POST",
        "/api/channels",
        json!({"channel_name":channel_name}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let cb = ok(
        &app,
        &bob,
        Some(&b),
        "POST",
        "/api/channels",
        json!({"channel_name":channel_name}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = call(
        &app,
        &alice,
        Some(&a),
        "POST",
        "/api/channels",
        json!({"channel_name":channel_name}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "names are unique within a tenant"
    );
    let ma = ok(
        &app,
        &alice,
        Some(&a),
        "POST",
        "/api/messages",
        json!({"channel_id":ca,"content":"A only"}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mb = ok(
        &app,
        &bob,
        Some(&b),
        "POST",
        "/api/messages",
        json!({"channel_id":cb,"content":"B only"}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &app,
        &alice,
        Some(&a),
        "POST",
        &format!("/api/messages/{ma}/reactions"),
        json!({"emoji":"👍"}),
    )
    .await;
    let channels = ok(&app, &alice, Some(&a), "GET", "/api/channels", Value::Null).await;
    assert_eq!(channels.as_array().unwrap().len(), 1);
    assert_eq!(channels[0]["id"], ca);
    let users = ok(&app, &alice, Some(&a), "GET", "/api/users", Value::Null).await;
    assert_eq!(users.as_array().unwrap().len(), 2);
    assert!(!users.to_string().contains(&bob));
    // Alice belongs to both workspaces: selecting A must still hide every resource in B.
    for (method, path, payload) in [
        ("GET", format!("/api/channels/{cb}"), Value::Null),
        (
            "PATCH",
            format!("/api/channels/{cb}"),
            json!({"description":"changed"}),
        ),
        ("DELETE", format!("/api/channels/{cb}"), Value::Null),
        ("POST", format!("/api/channels/{cb}/join"), Value::Null),
        ("POST", format!("/api/channels/{cb}/leave"), Value::Null),
        ("POST", format!("/api/channels/{cb}/archive"), Value::Null),
        ("POST", format!("/api/channels/{cb}/unarchive"), Value::Null),
        (
            "POST",
            format!("/api/channels/{cb}/users"),
            json!({"user_id":charlie}),
        ),
        (
            "DELETE",
            format!("/api/channels/{cb}/users/{bob}"),
            Value::Null,
        ),
        ("GET", format!("/api/messages?channel_id={cb}"), Value::Null),
        (
            "POST",
            "/api/messages".into(),
            json!({"channel_id":cb,"content":"blocked"}),
        ),
        (
            "PATCH",
            format!("/api/messages/{mb}"),
            json!({"content":"blocked"}),
        ),
        ("DELETE", format!("/api/messages/{mb}"), Value::Null),
        (
            "POST",
            format!("/api/messages/{mb}/reactions"),
            json!({"emoji":"👍"}),
        ),
        (
            "DELETE",
            format!("/api/messages/{mb}/reactions?emoji=%F0%9F%91%8D"),
            Value::Null,
        ),
        ("GET", format!("/api/users/{bob}"), Value::Null),
        (
            "POST",
            format!("/api/channels/{ca}/users"),
            json!({"user_id":bob}),
        ),
        (
            "POST",
            "/api/messages".into(),
            json!({"channel_id":ca,"content":"blocked","parent_id":mb}),
        ),
    ] {
        let (status, body) = call(&app, &alice, Some(&a), method, &path, payload).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {body}");
    }
    let (status, _) = call(
        &app,
        &alice,
        Some(&b),
        "DELETE",
        &format!("/api/messages/{mb}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&app, &bob, Some(&a), "GET", "/api/channels", Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        &alice,
        Some("invalid"),
        "GET",
        "/api/channels",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    for shape in ["messages", "message_reactions", "channel_users"] {
        let (status, _) = call(
            &app,
            &alice,
            Some(&a),
            "GET",
            &format!("/api/shapes/{shape}?channel_id={cb}&offset=-1"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    for (shape, query, present, absent) in [
        ("channels", "".to_string(), ca.clone(), cb.clone()),
        ("users", "".to_string(), charlie.clone(), bob.clone()),
        ("channel_users", "".to_string(), ca.clone(), cb.clone()),
        (
            "messages",
            format!("&channel_id={ca}"),
            ma.clone(),
            mb.clone(),
        ),
        (
            "message_reactions",
            format!("&channel_id={ca}"),
            ma.clone(),
            mb.clone(),
        ),
    ] {
        let req = Request::builder()
            .uri(format!(
                "/api/shapes/{shape}?offset=-1{query}&where=true&table=users&params%5B2%5D={b}"
            ))
            .header("X-Dev-User", &alice)
            .header("X-Tenant-ID", &a)
            .body(Body::empty())?;
        let response = app.clone().oneshot(req).await?;
        assert_eq!(response.status(), StatusCode::OK, "shape {shape}");
        assert_eq!(response.headers()["cache-control"], "private, no-cache");
        assert!(response.headers()["vary"].to_str()?.contains("X-Tenant-ID"));
        let text = String::from_utf8(to_bytes(response.into_body(), usize::MAX).await?.to_vec())?;
        assert!(text.contains(&present), "shape {shape}: {text}");
        assert!(!text.contains(&absent), "shape {shape}: {text}");
    }
    // Composite foreign keys reject writes that bypass the API's tenant checks.
    let mut db = db;
    let result = toasty::create!(backend::models::Message {
        tenant_id: a.as_str(),
        id: Uuid::new_v4().to_string(),
        user_id: alice.as_str(),
        channel_id: cb.as_str(),
        content: "invalid",
        created_at: backend::models::now_utc_millis(),
        updated_at: None,
        deleted_at: None,
        parent_id: None,
    })
    .exec(&mut db)
    .await;
    assert!(result.is_err());
    // Newly provisioned accounts do not implicitly join the legacy workspace.
    let newcomer = Uuid::new_v4().to_string();
    toasty::create!(backend::models::User {
        id: newcomer.as_str(),
        user_name: format!("new_{}", Uuid::new_v4().simple()),
        display_name: "New account",
        created_at: backend::models::now_utc_millis(),
        updated_at: None,
    })
    .exec(&mut db)
    .await?;
    let tenants = ok(&app, &newcomer, None, "GET", "/api/tenants", Value::Null).await;
    assert_eq!(tenants, json!([]));
    let (status, _) = call(&app, &newcomer, None, "GET", "/api/channels", Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let profile = ok(&app, &newcomer, None, "GET", "/api/users/me", Value::Null).await;
    assert_eq!(profile["id"], newcomer);
    ok(
        &app,
        &newcomer,
        None,
        "POST",
        "/api/tenants",
        json!({"name":"First workspace"}),
    )
    .await;

    Ok(())
}
