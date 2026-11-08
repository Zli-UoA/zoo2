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

    // Credential records report how long the refresh token works, separately from the access token.
    let (status, issued) = req(&app, "POST", &action("token"), &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let (_, records) = req(
        &app,
        "GET",
        &format!("/api/apps/{id}/records/credentials"),
        &owner,
        Value::Null,
    )
    .await;
    let record = &records[0];
    assert!(record["expires_at"].is_string(), "{records}");
    assert!(record["refresh_expires_at"].is_string(), "{records}");
    assert!(
        record["refresh_expires_at"].as_str() > record["expires_at"].as_str(),
        "{records}"
    );
    let (_, approved) = req(&app, "GET", &path, &owner, Value::Null).await;
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
    // Unique per run, since the test database is shared between runs.
    let new_name = format!("console_{}", &id[..8]);
    renamed["bot_username"] = json!(new_name);
    let (status, saved) = req(&app, "PATCH", &path, &owner, renamed).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, after) = req(&app, "GET", &path, &owner, Value::Null).await;
    assert_eq!(after["bot_username"], json!(new_name));

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

#[tokio::test]
async fn app_metrics_summarize_one_apps_deliveries_for_its_developer_and_admins()
-> anyhow::Result<()> {
    let (app, owner, stranger, admin) = setup().await?;
    let manifest = json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://example.com/metrics"});
    let (status, created) = req(
        &app,
        "POST",
        "/api/apps",
        &owner,
        json!({"name":"Metrics","description":"Metrics app","manifest":manifest}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap();
    for (actor, path, body) in [
        (
            owner.as_str(),
            format!("/api/apps/{id}/actions/submit"),
            Value::Null,
        ),
        (
            admin,
            format!("/api/apps/{id}/actions/install"),
            json!({"revision":1}),
        ),
    ] {
        assert_eq!(
            req(&app, "POST", &path, actor, body).await.0,
            StatusCode::OK
        );
    }
    let metrics = format!("/api/apps/{id}/metrics");

    // 配信が無い App は 0 件で、成功率は null（0 除算にしない）。
    let (status, empty) = req(&app, "GET", &metrics, &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["window_hours"], 24);
    assert_eq!(empty["totals"]["delivered"], 0);
    assert_eq!(empty["totals"]["success_rate"], Value::Null);
    assert_eq!(empty["hourly"].as_array().unwrap().len(), 24, "{empty}");

    let config = Config::from_env();
    let db = init_db(&config.database_url).await?;
    let mut db = db.clone();
    let mut tx = db.transaction().await?;
    let installation = backend::apps::one(
        &mut tx,
        "SELECT json_build_object('id',id)::text FROM app_installations WHERE app_id=$1",
        &[id],
    )
    .await?["id"]
        .as_str()
        .unwrap()
        .to_string();
    // delivered ×3（作成から 2 秒で ACK）、failed ×1（期限切れ）、pending ×2、
    // 窓の外 ×1 と app.rate_limited ×1 は数えない。
    for (kind, created, acked, expires, event_type) in [
        (
            "d",
            "10 minutes",
            Some("2 seconds"),
            "1 day",
            "message.created",
        ),
        (
            "d",
            "10 minutes",
            Some("2 seconds"),
            "1 day",
            "message.created",
        ),
        (
            "d",
            "10 minutes",
            Some("2 seconds"),
            "1 day",
            "message.created",
        ),
        ("f", "2 hours", None, "-1 minute", "message.created"),
        ("p", "5 minutes", None, "1 day", "message.created"),
        ("p", "5 minutes", None, "1 day", "message.created"),
        ("o", "3 days", Some("2 seconds"), "1 day", "message.created"),
        // 窓より前に作られたが、手動再送などで期限内のまま残っている滞留。
        ("q", "30 hours", None, "1 hour", "message.created"),
        ("r", "5 minutes", None, "1 day", "app.rate_limited"),
    ] {
        let acked_sql = acked
            .map(|a| format!("now()-interval '{created}'+interval '{a}'"))
            .unwrap_or_else(|| "NULL".into());
        toasty::sql::statement(format!("INSERT INTO app_deliveries(id,event_id,installation_id,event_type,payload,created_at,acked_at,expires_at,attempts) VALUES(gen_random_uuid()::text,gen_random_uuid()::text,$1,$2,'{{}}',now()-interval '{created}',{acked_sql},now()+interval '{expires}',1) /* {kind} */"))
            .bind(installation.as_str())
            .bind(event_type)
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;

    let (status, body) = req(&app, "GET", &metrics, &owner, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let totals = &body["totals"];
    assert_eq!(totals["delivered"], 3, "{body}");
    assert_eq!(totals["failed"], 1, "{body}");
    // 滞留は窓と無関係に数える（窓の外で作られた 1 件を含む）。
    assert_eq!(totals["pending"], 3, "{body}");
    assert_eq!(totals["success_rate"], 0.75, "{body}");
    assert!((totals["p50_seconds"].as_f64().unwrap() - 2.0).abs() < 0.01);
    assert!((totals["p95_seconds"].as_f64().unwrap() - 2.0).abs() < 0.01);
    assert!(totals["oldest_pending_seconds"].as_f64().unwrap() >= 30.0 * 3600.0 - 10.0);
    let hourly = body["hourly"].as_array().unwrap();
    let sum = |key: &str| -> u64 { hourly.iter().map(|h| h[key].as_u64().unwrap()).sum() };
    assert_eq!((sum("delivered"), sum("failed"), sum("pending")), (3, 1, 2));

    // 窓を 1 時間に絞っても、滞留（pending・最古）は変わらない。時間ごとの内訳は窓の中だけ。
    let (_, narrow) = req(
        &app,
        "GET",
        &format!("{metrics}?hours=1"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(narrow["totals"]["pending"], 3, "{narrow}");
    assert!(narrow["totals"]["oldest_pending_seconds"].as_f64().unwrap() >= 30.0 * 3600.0 - 10.0);
    assert_eq!(narrow["hourly"].as_array().unwrap().len(), 1, "{narrow}");

    // 窓を広げると窓の外だった 1 件も入る。
    let (_, wide) = req(
        &app,
        "GET",
        &format!("{metrics}?hours=96"),
        &owner,
        Value::Null,
    )
    .await;
    assert_eq!(wide["totals"]["delivered"], 4, "{wide}");

    // 管理者は他人の App も見られる。他の開発者は見られない。
    assert_eq!(
        req(&app, "GET", &metrics, admin, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        req(&app, "GET", &metrics, &stranger, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    for bad in ["hours=0", "hours=721", "hours=abc", "unknown=1"] {
        let (status, _) = req(
            &app,
            "GET",
            &format!("{metrics}?{bad}"),
            &owner,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(
        req(
            &app,
            "GET",
            "/api/apps/missing/metrics",
            &owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}
