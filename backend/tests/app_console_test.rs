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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("x-dev-user", actor)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn setup() -> anyhow::Result<(Router, String, String, &'static str)> {
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db, config));
    let owner = Uuid::new_v4().to_string();
    let stranger = Uuid::new_v4().to_string();
    let admin = backend::auth::DEFAULT_DEV_USER_ID;
    for actor in [owner.as_str(), stranger.as_str(), admin] {
        let status=req(&app,"POST","/api/users",actor,json!({"id":actor,"user_name":format!("console_{}",Uuid::new_v4().simple()),"display_name":"Console tester"})).await.0;
        assert!(status == StatusCode::OK || (actor == admin && status == StatusCode::CONFLICT));
    }
    Ok((app, owner, stranger, admin))
}

async fn channel(app: &Router, actor: &str) -> String {
    let id = Uuid::new_v4().to_string();
    let (status, body) = req(app,"POST","/api/channels",actor,json!({"id":id,"channel_name":format!("console_{}",Uuid::new_v4().simple()),"is_private":false})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    id
}

#[tokio::test]
async fn detail_separates_approved_installation_and_rejects_stale_saves() -> anyhow::Result<()> {
    let (app, owner, stranger, admin) = setup().await?;
    let manifest = json!({"scopes":["messages:read"],"events":["app.mentioned"],"transport":"http","event_url":"https://example.com/approved","commands":["deploy"]});
    let (status, created) = req(
        &app,
        "POST",
        "/api/apps",
        &owner,
        json!({"name":"Console","description":"Console app","manifest":manifest}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap();
    let path = format!("/api/apps/{id}");
    let action = |a: &str| format!("/api/apps/{id}/actions/{a}");

    let (_, draft) = req(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(draft["installation"], Value::Null, "{draft}");
    // The console reads the bot username from the detail, not from the realtime user list.
    assert!(
        draft["bot_username"]
            .as_str()
            .is_some_and(|name| !name.is_empty()),
        "{draft}"
    );
    assert_eq!(
        req(&app, "POST", &action("submit"), &owner, Value::Null)
            .await
            .0,
        StatusCode::OK
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

    let (status, approved) = req(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(
        approved["installation"]["event_url"],
        "https://example.com/approved"
    );
    assert_eq!(approved["installation"]["revision"], 1);
    assert_eq!(
        approved["installation"]["endpoint_verified_at"],
        Value::Null
    );
    let loaded_at = approved["updated_at"].as_str().unwrap().to_string();
    let (_, public) = req(&app, "GET", &path, &stranger, Value::Null).await;
    assert!(public.get("installation").is_none(), "{public}");

    // A full save with a new URL creates a draft revision; the approved URL stays until re-approval.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let mut edited = approved["manifest"].clone();
    edited["event_url"] = json!("https://example.com/draft");
    let input = |manifest: &Value, expected: &str| json!({"name":"Console","description":"Console app","manifest":manifest,"expected_updated_at":expected});
    let (status, saved) = req(&app, "PATCH", &path, &owner, input(&edited, &loaded_at)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["status"], "draft");
    assert_eq!(saved["revision"], 2);
    assert_ne!(saved["updated_at"], json!(loaded_at));
    let (_, after) = req(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(after["manifest"]["event_url"], "https://example.com/draft");
    assert_eq!(after["manifest"]["commands"][0], "deploy");
    assert_eq!(
        after["installation"]["event_url"],
        "https://example.com/approved"
    );
    assert_eq!(after["installation"]["revision"], 1);

    // Saving from the stale copy must not overwrite the newer settings.
    let (status, conflict) = req(
        &app,
        "PATCH",
        &path,
        &owner,
        input(&approved["manifest"], &loaded_at),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(
        req(&app, "PATCH", &path, &owner, input(&edited, "not a time"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (_, unchanged) = req(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(
        unchanged["manifest"]["event_url"],
        "https://example.com/draft"
    );

    // A rename is visible in the detail right away.
    let mut renamed = input(&edited, unchanged["updated_at"].as_str().unwrap());
    renamed["bot_username"] = json!("console_renamed");
    let (status, saved) = req(&app, "PATCH", &path, &owner, renamed).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, after) = req(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(after["bot_username"], "console_renamed");

    // Each signing secret view is recorded.
    assert_eq!(
        req(&app, "POST", &action("connection"), &owner, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let (_, audit) = req(
        &app,
        "GET",
        &format!("/api/apps/{id}/records/audit"),
        &owner,
        Value::Null,
    )
    .await;
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "connection" && row["actor_id"] == json!(owner)),
        "{audit}"
    );
    Ok(())
}

#[tokio::test]
async fn app_webhooks_are_limited_to_managed_channels() -> anyhow::Result<()> {
    let (app, owner, stranger, admin) = setup().await?;
    let mine = channel(&app, &owner).await;
    let theirs = channel(&app, admin).await;
    let (status, first) = req(
        &app,
        "POST",
        &format!("/api/channels/{mine}/webhooks"),
        &owner,
        json!({"name":"Deploys"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let notifier = first["data"]["bot_user_id"].as_str().unwrap().to_string();
    let list = format!("/api/apps/{notifier}/webhooks");

    // An admin can attach the same app in a channel the developer does not manage.
    let (status, other) = req(
        &app,
        "POST",
        &format!("/api/channels/{theirs}/webhooks"),
        admin,
        json!({"name":"Alerts","app_id":notifier}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{other}");
    assert_eq!(other["data"]["bot_user_id"], json!(notifier));
    let (status, second) = req(
        &app,
        "POST",
        &format!("/api/channels/{mine}/webhooks"),
        &owner,
        json!({"name":"Builds","app_id":notifier}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["data"]["bot_user_id"], json!(notifier));

    let (status, owned) = req(&app, "GET", &list, &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{owned}");
    let hooks = owned["webhooks"].as_array().unwrap();
    assert_eq!(hooks.len(), 2, "{owned}");
    assert!(
        hooks
            .iter()
            .all(|h| h["channel_id"] == json!(mine)
                && h["path"].as_str().unwrap().contains("token="))
    );
    assert_eq!(owned["hidden_count"], 1);
    let (_, all) = req(&app, "GET", &list, admin, Value::Null).await;
    assert_eq!(all["webhooks"].as_array().unwrap().len(), 3);
    assert_eq!(all["hidden_count"], 0);

    assert_eq!(
        req(&app, "GET", &list, &stranger, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/channels/{theirs}/webhooks"),
            &owner,
            json!({"name":"Nope","app_id":notifier})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, regular) = req(
        &app,
        "POST",
        "/api/apps",
        &owner,
        json!({"name":"Regular","manifest":{"scopes":[],"events":[]}}),
    )
    .await;
    let regular = regular["id"].as_str().unwrap();
    assert_eq!(
        req(
            &app,
            "POST",
            &format!("/api/channels/{mine}/webhooks"),
            &owner,
            json!({"name":"Nope","app_id":regular})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}
