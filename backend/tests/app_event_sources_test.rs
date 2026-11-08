//! Exercise mutations through the public API and inspect the resulting durable deliveries.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{apps, config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
static HTTP_WORKER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn display_cache_updates_do_not_emit_message_updates() -> anyhow::Result<()> {
    let f = fixture_with_events(json!(["message.created", "message.updated"])).await?;
    let message = ok(
        &f.app,
        "POST",
        "/api/messages",
        &f.owner,
        json!({"channel_id":f.channel,"content":"Original text"}),
    )
    .await;
    let id = message["data"]["id"].as_str().unwrap();
    ok(
        &f.app,
        "POST",
        &format!("/api/messages/{id}/reactions"),
        &f.owner,
        json!({"emoji":"👍"}),
    )
    .await;
    let before = payloads(&f, &f.channel).await?;
    assert!(
        !before
            .iter()
            .any(|event| event["type"] == "message.updated"),
        "Display cache updates must not look like edits: {before:?}"
    );
    ok(
        &f.app,
        "PATCH",
        &format!("/api/messages/{id}"),
        &f.owner,
        json!({"content":"Edited text"}),
    )
    .await;
    let after = payloads(&f, &f.channel).await?;
    assert_eq!(
        after
            .iter()
            .filter(|event| event["type"] == "message.updated")
            .count(),
        1
    );
    assert!(
        after
            .iter()
            .all(|event| !event.to_string().contains("display_data"))
    );
    Ok(())
}

#[tokio::test]
async fn app_message_shapes_exclude_display_data_with_and_without_metadata_scope()
-> anyhow::Result<()> {
    let f = fixture().await?;
    let message = ok(
        &f.app,
        "POST",
        "/api/messages",
        &f.owner,
        json!({"channel_id":f.channel,"content":"Shape projection boundary","metadata":{"event_type":"shape_scope_marker","event_payload":{}}}),
    )
    .await;
    let mid = message["data"]["id"].as_str().unwrap();
    for metadata_allowed in [false, true] {
        let mut scopes = vec!["messages:read"];
        if metadata_allowed {
            scopes.push("metadata:read");
        }
        let token = reconfigure(
            &f,
            json!({"scopes":scopes,"events":[],"transport":"websocket"}),
        )
        .await?;
        let response = f.app.clone().oneshot(
            Request::builder()
                .uri(format!("/api/apps/v1/shapes/messages?channel_id={}&offset=-1&columns=display_data,metadata&_app_metadata_allowed=true", f.channel))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())?,
        ).await?;
        let status = response.status();
        if !status.is_success() {
            let bytes = to_bytes(response.into_body(), usize::MAX).await?;
            panic!(
                "App Shape returned {status}: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        let schema: Value = serde_json::from_str(
            response
                .headers()
                .get("electric-schema")
                .unwrap()
                .to_str()?,
        )?;
        assert!(schema.get("content").is_some());
        assert!(schema.get("display_data").is_none());
        assert_eq!(schema.get("metadata").is_some(), metadata_allowed);
        let bytes = to_bytes(response.into_body(), usize::MAX).await?;
        let rows: Value = serde_json::from_slice(&bytes)?;
        let row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["value"]["id"] == mid)
            .expect("message in initial snapshot");
        assert_eq!(row["value"]["content"], "Shape projection boundary");
        assert!(row["value"].get("display_data").is_none());
        assert_eq!(row["value"].get("metadata").is_some(), metadata_allowed);
    }
    // The user-facing Shape still needs the joined projection to render.
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/shapes/messages?channel_id={}&offset=-1",
                    f.channel
                ))
                .header("x-dev-user", &f.owner)
                .body(Body::empty())?,
        )
        .await?;
    assert!(response.status().is_success());
    let schema: Value = serde_json::from_str(
        response
            .headers()
            .get("electric-schema")
            .unwrap()
            .to_str()?,
    )?;
    assert!(schema.get("display_data").is_some());
    Ok(())
}

#[tokio::test]
async fn special_message_events_exclude_the_joined_display_projection() -> anyhow::Result<()> {
    let f = fixture().await?;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let bot = apps::one(
        &mut tx,
        "SELECT json_build_object('name',user_name)::text FROM users WHERE id=$1",
        &[&f.bot],
    )
    .await?;
    drop(tx);
    let message = ok(
        &f.app,
        "POST",
        "/api/messages",
        &f.owner,
        json!({"channel_id":f.channel,"content":format!("@{} https://example.com/item",bot["name"].as_str().unwrap())}),
    ).await;
    let mid = message["data"]["id"].as_str().unwrap();
    let mut tx = db.transaction().await?;
    let cached = apps::one(
        &mut tx,
        "SELECT display_data::text FROM messages WHERE id=$1",
        &[mid],
    )
    .await?;
    assert_eq!(cached["author"]["id"], f.owner);
    drop(tx);
    // Inspect the durable payload for both special paths.
    let events = payloads(&f, &f.channel).await?;
    for kind in ["app.mentioned", "link.shared"] {
        let event = events
            .iter()
            .find(|event| event["type"] == kind)
            .expect(kind);
        assert_eq!(event["data"]["id"], mid);
        if kind == "app.mentioned" {
            assert_eq!(event["data"]["content"], message["data"]["content"]);
        } else {
            assert_eq!(event["data"]["links"], json!(["https://example.com/item"]));
            assert!(event["data"].get("content").is_none());
        }
        assert!(
            event["data"].get("display_data").is_none(),
            "{kind}: {event}"
        );
    }
    // The current event source already strips the projection. Exercise the
    // retained legacy trigger function too, using a transaction-local table
    // so the global messages triggers and other tests remain untouched.
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "CREATE TEMP TABLE legacy_special_messages (LIKE messages INCLUDING DEFAULTS) ON COMMIT DROP",
    )
    .exec(&mut tx)
    .await?;
    toasty::sql::statement("CREATE TRIGGER legacy_special_events AFTER INSERT ON legacy_special_messages FOR EACH ROW EXECUTE FUNCTION enqueue_app_special_message()")
        .exec(&mut tx).await?;
    toasty::sql::statement(
        "INSERT INTO legacy_special_messages SELECT * FROM messages WHERE id=$1",
    )
    .bind(mid)
    .exec(&mut tx)
    .await?;
    let events = apps::query(&mut tx,
        "SELECT payload::text FROM app_deliveries WHERE installation_id=$1 AND event_type IN ('app.mentioned','link.shared') AND payload::jsonb->'data'->>'id'=$2",
        &[&f.installation, mid],
    ).await?;
    assert_eq!(
        events.len(),
        4,
        "Both sources must emit both special events"
    );
    assert!(
        events
            .iter()
            .all(|event| event["data"].get("display_data").is_none())
    );
    Ok(())
}

async fn request(
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
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn ok(app: &Router, method: &str, path: &str, actor: &str, body: Value) -> Value {
    let (s, v) = request(app, method, path, actor, body).await;
    assert_eq!(s, StatusCode::OK, "{method} {path}: {v}");
    v
}
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
async fn event_types(f: &Fixture) -> anyhow::Result<std::collections::BTreeSet<String>> {
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    Ok(apps::query(
        &mut tx,
        "SELECT to_json(event_type)::text FROM app_deliveries WHERE installation_id=$1",
        &[&f.installation],
    )
    .await?
    .iter()
    .map(|v| v.as_str().unwrap().to_owned())
    .collect())
}
#[tokio::test]
async fn mutations_produce_authorized_events_and_private_state_requires_consent()
-> anyhow::Result<()> {
    let f = fixture().await?;
    let a = &f.app;
    let u = &f.owner;
    let c = &f.channel;
    let admin = backend::auth::DEFAULT_DEV_USER_ID;
    ok(a,"PATCH","/api/users/me/profile",u,json!({"display_name":"Updated","status_text":"Working","status_emoji":"💻","preferences":{"theme":"dark"},"presence":"away","dnd_until":"2099-01-01T00:00:00Z"})).await;
    ok(
        a,
        "PATCH",
        "/api/users/me/profile",
        u,
        json!({"presence":"auto","dnd_until":null}),
    )
    .await;
    ok(a, "POST", "/api/presence/heartbeat", u, json!({})).await;
    ok(
        a,
        "PATCH",
        &format!("/api/users/{}/guest", f.other),
        admin,
        json!({"kind":"multi"}),
    )
    .await;
    ok(
        a,
        "PATCH",
        &format!("/api/channels/{c}/settings"),
        u,
        json!({"topic":"Topic","purpose":"Purpose","posting_policy":"managers"}),
    )
    .await;
    ok(
        a,
        "PATCH",
        &format!("/api/channels/{c}/settings"),
        u,
        json!({"posting_policy":"members"}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/channels/{c}/view"),
        u,
        json!({"open":true}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/channels/{c}/view"),
        u,
        json!({"open":false}),
    )
    .await;
    let m=ok(a,"POST","/api/messages",u,json!({"channel_id":c,"content":"https://example.com/object","metadata":{"event_type":"issue","event_payload":{"id":1}}})).await;
    let mid = m["data"]["id"].as_str().unwrap();
    ok(
        a,
        "POST",
        "/api/messages",
        u,
        json!({"channel_id":c,"content":"reply","parent_id":mid,"broadcast":true,"kind":"emote"}),
    )
    .await;
    ok(
        a,
        "PATCH",
        &format!("/api/messages/{mid}/metadata"),
        u,
        json!({"event_type":"issue","event_payload":{"id":2}}),
    )
    .await;
    ok(
        a,
        "DELETE",
        &format!("/api/messages/{mid}/metadata"),
        u,
        Value::Null,
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/channels/{c}/pins"),
        u,
        json!({"item_type":"message","item_id":mid}),
    )
    .await;
    ok(
        a,
        "DELETE",
        &format!("/api/channels/{c}/pins/message/{mid}"),
        u,
        Value::Null,
    )
    .await;
    ok(
        a,
        "POST",
        "/api/stars",
        u,
        json!({"item_type":"message","item_id":mid}),
    )
    .await;
    ok(
        a,
        "DELETE",
        &format!("/api/stars/message/{mid}"),
        u,
        Value::Null,
    )
    .await;
    let ename = format!("evt-party-parrot-{}", Uuid::new_v4().simple());
    ok(
        a,
        "POST",
        "/api/emojis",
        u,
        json!({"name":ename,"url":format!("/api/emoji-images/{}",Uuid::new_v4())}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/messages/{mid}/reactions"),
        u,
        json!({"emoji":format!(":{ename}:")}),
    )
    .await;
    ok(
        a,
        "DELETE",
        &format!("/api/messages/{mid}/reactions?emoji=%3A{ename}%3A"),
        u,
        Value::Null,
    )
    .await;
    ok(a, "DELETE", &format!("/api/emojis/{ename}"), u, Value::Null).await;
    let group = ok(
        a,
        "POST",
        "/api/usergroups",
        admin,
        json!({"name":"On call","handle":format!("g_{}",Uuid::new_v4().simple())}),
    )
    .await;
    let gid = group["id"].as_str().unwrap();
    ok(
        a,
        "PATCH",
        &format!("/api/usergroups/{gid}"),
        admin,
        json!({"name":"Support","handle":group["handle"]}),
    )
    .await;
    ok(
        a,
        "PUT",
        &format!("/api/usergroups/{gid}/members"),
        admin,
        json!([u]),
    )
    .await;
    ok(
        a,
        "PUT",
        &format!("/api/usergroups/{gid}/members"),
        admin,
        json!([]),
    )
    .await;
    ok(a,"PATCH","/api/workspace",admin,json!({"name":format!("Workspace {}",Uuid::new_v4()),"preferences":{"locale":"ja","revision":Uuid::new_v4().to_string()},"profile_fields":[{"id":"a","label":"A","type":"text"},{"id":"b","label":"B","type":"text"}]})).await;
    ok(a,"PATCH","/api/workspace",admin,json!({"profile_fields":[{"id":"b","label":"B","type":"text"},{"id":"a","label":"A","type":"text"}]})).await;
    ok(
        a,
        "PATCH",
        "/api/workspace",
        admin,
        json!({"profile_fields":[]}),
    )
    .await;
    ok(
        a,
        "POST",
        "/api/invite-requests",
        u,
        json!({"email":"new@example.com","channel_id":c}),
    )
    .await;
    ok(
        a,
        "POST",
        "/api/reminders",
        u,
        json!({"content":"remember","due_at":"2020-01-01T00:00:00Z","channel_id":c}),
    )
    .await;
    apps::events::maintain(&f.state.db).await?;
    let session = ok(
        a,
        "POST",
        "/api/assistant-sessions",
        u,
        json!({"app_id":f.id,"user_id":u,"channel_id":c,"context":{"kind":"channel"}}),
    )
    .await;
    ok(
        a,
        "PATCH",
        &format!(
            "/api/assistant-sessions/{}",
            session["id"].as_str().unwrap()
        ),
        u,
        json!({"title":"Summary","context":{"kind":"thread"},"state":"stopped"}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{}/context", f.id),
        u,
        json!({"channel_id":c,"context":{"view":"channel"}}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{}/connection", f.id),
        u,
        json!({"channel_id":c,"connect":true}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/v1/user-connections/{u}"),
        &f.token,
        json!({"connected":true}),
    )
    .await;
    let function = ok(
        a,
        "POST",
        &format!("/api/apps/{}/functions/echo/invoke", f.id),
        u,
        json!({"channel_id":c,"inputs":{"text":"hello"}}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!(
            "/api/apps/v1/function-executions/{}/complete",
            function["id"].as_str().unwrap()
        ),
        &f.token,
        json!({"outputs":{"text":"hello"}}),
    )
    .await;
    ok(
        a,
        "DELETE",
        &format!("/api/channels/{c}/history"),
        u,
        Value::Null,
    )
    .await;
    let types = event_types(&f).await?;
    for name in [
        "user.changed",
        "user.profile_changed",
        "user.status_changed",
        "user.preferences_changed",
        "user.presence_changed",
        "user.manual_presence_changed",
        "user.dnd_changed",
        "user.dnd_changed_public",
        "user.guest_status_changed",
        "channel.created",
        "member.joined",
        "channel.updated",
        "channel.topic_changed",
        "channel.purpose_changed",
        "channel.posting_permissions_changed",
        "conversation.opened",
        "conversation.closed",
        "message.created",
        "message.updated",
        "message.deleted",
        "message.replied",
        "message.thread_broadcast",
        "message.emote",
        "message.metadata_posted",
        "message.metadata_updated",
        "message.metadata_deleted",
        "pin.added",
        "pin.removed",
        "star.added",
        "star.removed",
        "emoji.changed",
        "reaction.added",
        "reaction.removed",
        "usergroup.created",
        "usergroup.updated",
        "usergroup.members_changed",
        "usergroup.self_added",
        "usergroup.self_removed",
        "workspace.renamed",
        "workspace.preferences_changed",
        "workspace.profile_changed",
        "workspace.profile_deleted",
        "workspace.profile_reordered",
        "invite.requested",
        "reminder.created",
        "reminder.fired",
        "assistant.started",
        "assistant.title_changed",
        "assistant.context_changed",
        "assistant.stopped",
        "app.context_changed",
        "user.connection_requested",
        "function.executed",
        "link.shared",
        "channel.history_changed",
    ] {
        assert!(
            types.contains(name),
            "Missing real source event {name}; observed {types:?}"
        );
    }
    // The queued personal events become undeliverable when the user withdraws consent.
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let private=apps::query(&mut tx,"SELECT json_build_object('type',event_type,'allowed',app_delivery_allowed(d))::text FROM app_deliveries d WHERE installation_id=$1 AND subject_user_id=$2",&[&f.installation,u]).await?;
    assert!(!private.is_empty());
    assert!(private.iter().all(|v| v["allowed"] == true));
    drop(tx);
    ok(
        a,
        "DELETE",
        &format!("/api/apps/{}/authorization", f.id),
        u,
        Value::Null,
    )
    .await;
    let mut tx = db.transaction().await?;
    let private=apps::query(&mut tx,"SELECT json_build_object('allowed',app_delivery_allowed(d))::text FROM app_deliveries d WHERE installation_id=$1 AND subject_user_id=$2",&[&f.installation,u]).await?;
    assert!(private.iter().all(|v| v["allowed"] == false));
    Ok(())
}

#[tokio::test]
async fn channel_aliases_guest_restrictions_and_delivery_reauthorization() -> anyhow::Result<()> {
    let f = fixture().await?;
    let a = &f.app;
    let u = &f.owner;
    let c = &f.channel;
    let admin = backend::auth::DEFAULT_DEV_USER_ID;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    for (name, actor, identity, expected) in [
        ("channel_joined", f.other.as_str(), f.bot.as_str(), false),
        ("channel_joined", f.bot.as_str(), f.bot.as_str(), true),
        (
            "member_joined_channel",
            f.other.as_str(),
            f.bot.as_str(),
            true,
        ),
        ("group_joined", f.bot.as_str(), f.bot.as_str(), false),
    ] {
        let v=apps::one(&mut tx,"SELECT json_build_object('matches',app_event_matches($1,'member.joined',$2,$3,$4))::text",&[&json!({"events":[name]}).to_string(),c,actor,identity]).await?;
        assert_eq!(v["matches"], expected, "{name}");
    }
    drop(tx);
    ok(
        a,
        "PATCH",
        &format!("/api/users/{}/guest", f.other),
        admin,
        json!({"kind":"single"}),
    )
    .await;
    assert_eq!(
        request(
            a,
            "POST",
            &format!("/api/channels/{c}/join"),
            &f.other,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            a,
            "GET",
            &format!("/api/channels/{c}"),
            &f.other,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    ok(
        a,
        "PATCH",
        &format!("/api/channels/{c}"),
        u,
        json!({"channel_name":format!("renamed_{}",Uuid::new_v4().simple()),"is_private":true}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/channels/{c}/archive"),
        u,
        Value::Null,
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/channels/{c}/unarchive"),
        u,
        Value::Null,
    )
    .await;
    ok(
        a,
        "POST",
        "/api/apps/v1/dms",
        &f.token,
        json!({"target_user_ids":[u]}),
    )
    .await;
    ok(
        a,
        "POST",
        "/api/messages",
        u,
        json!({"channel_id":c,"content":"pending permission check"}),
    )
    .await;
    let mut tx = db.transaction().await?;
    let pending=apps::one(&mut tx,"SELECT json_build_object('id',id,'allowed',app_delivery_allowed(d))::text FROM app_deliveries d WHERE installation_id=$1 AND channel_id=$2 AND event_type='message.created' ORDER BY created_at DESC LIMIT 1",&[&f.installation,c]).await?;
    assert_eq!(pending["allowed"], true);
    drop(tx);
    ok(
        a,
        "DELETE",
        &format!("/api/apps/{}/authorization", f.id),
        u,
        Value::Null,
    )
    .await;
    ok(
        a,
        "DELETE",
        &format!("/api/apps/{}/channels/{c}", f.id),
        u,
        Value::Null,
    )
    .await;
    let mut tx = db.transaction().await?;
    let v=apps::one(&mut tx,"SELECT json_build_object('allowed',app_delivery_allowed(d))::text FROM app_deliveries d WHERE id=$1",&[pending["id"].as_str().unwrap()]).await?;
    assert_eq!(v["allowed"], false);
    drop(tx);
    let types = event_types(&f).await?;
    for name in [
        "channel.renamed",
        "channel.privacy_changed",
        "channel.archived",
        "channel.unarchived",
        "conversation.created",
        "member.left",
    ] {
        assert!(types.contains(name), "{name}");
    }
    ok(
        a,
        "PATCH",
        &format!("/api/users/{}/guest", f.other),
        admin,
        json!({"kind":"deactivated"}),
    )
    .await;
    assert_eq!(
        request(a, "GET", "/api/users/me/profile", &f.other, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // A revoked delegated identity cannot be used even if its opaque token is cached.
    ok(
        a,
        "DELETE",
        &format!("/api/apps/{}/authorization", f.id),
        u,
        Value::Null,
    )
    .await;
    assert_eq!(
        request(a, "GET", "/api/apps/v1/auth", &f.grant, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

#[tokio::test]
async fn file_lifecycle_rate_limit_and_app_administration_have_real_sources() -> anyhow::Result<()>
{
    let f = fixture().await?;
    let a = &f.app;
    let u = &f.owner;
    let c = &f.channel;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let file = Uuid::new_v4().to_string();
    // Completed object storage upload state; no network or fabricated event insertion.
    apps::one(&mut tx,"INSERT INTO app_files(id,installation_id,user_id,channel_id,object_key,name,content_type,size_bytes,completed) VALUES($1,$2,$3,$4,'private/object/'||$1,'test.txt','text/plain',5,false) RETURNING '{}'::text",&[&file,&f.installation,u,c]).await?;
    apps::one(
        &mut tx,
        "UPDATE app_files SET completed=true WHERE id=$1 RETURNING '{}'::text",
        &[&file],
    )
    .await?;
    tx.commit().await?;
    ok(
        a,
        "PATCH",
        &format!("/api/apps/v1/files/{file}"),
        &f.token,
        json!({"name":"renamed.txt","description":"A real file"}),
    )
    .await;
    let public = ok(
        a,
        "POST",
        &format!("/api/apps/v1/files/{file}/public-link"),
        &f.token,
        json!({"public":true}),
    )
    .await;
    assert!(
        public["path"]
            .as_str()
            .unwrap()
            .starts_with("/api/public-files/zoo_file_")
    );
    ok(
        a,
        "POST",
        &format!("/api/apps/v1/files/{file}/public-link"),
        &f.token,
        json!({"public":false}),
    )
    .await;
    ok(
        a,
        "DELETE",
        &format!("/api/apps/v1/files/{file}"),
        &f.token,
        Value::Null,
    )
    .await;
    assert_eq!(
        request(
            a,
            "GET",
            &format!("/api/apps/v1/files/{file}"),
            &f.token,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut tx = db.transaction().await?;
    let payloads=apps::query(&mut tx,"SELECT payload::text FROM app_deliveries WHERE installation_id=$1 AND event_type LIKE 'file.%'",&[&f.installation]).await?;
    for p in payloads {
        let text = p.to_string();
        for secret in ["object_key", "public_token_hash", "private/object/key"] {
            assert!(!text.contains(secret), "Leaked {secret}");
        }
    }
    // Cross the documented hourly limit with normal mutations, then check one warning.
    apps::one(&mut tx,"INSERT INTO app_event_limits VALUES($1,date_trunc('hour',now()),30000) ON CONFLICT(installation_id,window_start) DO UPDATE SET events=30000 RETURNING '{}'::text",&[&f.installation]).await?;
    tx.commit().await?;
    for _ in 0..2 {
        ok(
            a,
            "POST",
            "/api/messages",
            u,
            json!({"channel_id":c,"content":"rate limited"}),
        )
        .await;
    }
    let mut tx = db.transaction().await?;
    let limits=apps::one(&mut tx,"SELECT json_build_object('warnings',count(*) FILTER(WHERE event_type='app.rate_limited'),'messages',count(*) FILTER(WHERE event_type='message.created'))::text FROM app_deliveries WHERE installation_id=$1",&[&f.installation]).await?;
    assert_eq!(limits["warnings"], 1);
    assert_eq!(limits["messages"], 0);
    apps::one(
        &mut tx,
        "UPDATE app_event_limits SET events=0 WHERE installation_id=$1 RETURNING '{}'::text",
        &[&f.installation],
    )
    .await?;
    tx.commit().await?;
    let unrelated = ok(
        a,
        "POST",
        "/api/apps",
        u,
        json!({"name":"Another app","manifest":{"scopes":[],"events":[],"transport":"websocket"}}),
    )
    .await;
    let id = unrelated["id"].as_str().unwrap();
    ok(
        a,
        "POST",
        &format!("/api/apps/{id}/actions/submit"),
        u,
        Value::Null,
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{id}/actions/install"),
        backend::auth::DEFAULT_DEV_USER_ID,
        json!({"revision":1}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{id}/actions/delete"),
        u,
        Value::Null,
    )
    .await;
    let types = event_types(&f).await?;
    for name in [
        "file.created",
        "file.changed",
        "file.public",
        "file.shared",
        "file.unshared",
        "file.deleted",
        "app.rate_limited",
        "app.requested",
        "app.deleted",
        "app.installed_workspace",
        "app.uninstalled_workspace",
        "bot.added",
    ] {
        assert!(types.contains(name), "Missing {name}: {types:?}");
    }
    assert_eq!(
        request(
            a,
            "POST",
            &format!("/api/apps/{id}/actions/submit"),
            u,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}

#[tokio::test]
async fn interaction_callbacks_and_remaining_lifecycle_events() -> anyhow::Result<()> {
    let f = fixture().await?;
    let a = &f.app;
    let u = &f.owner;
    let c = &f.channel;
    let created = Uuid::new_v4().to_string();
    ok(a,"POST","/api/users",&created,json!({"id":created,"user_name":format!("new_{}",Uuid::new_v4().simple()),"display_name":"New member"})).await;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let bot = apps::one(
        &mut tx,
        "SELECT json_build_object('name',user_name)::text FROM users WHERE id=$1",
        &[&f.bot],
    )
    .await?;
    drop(tx);
    let m=ok(a,"POST","/api/messages",u,json!({"channel_id":c,"content":format!("@{} https://example.com/object",bot["name"].as_str().unwrap())})).await;
    let mid = m["data"]["id"].as_str().unwrap();
    ok(
        a,
        "POST",
        &format!("/api/channels/{c}/typing"),
        u,
        json!({"session_id":Uuid::new_v4(),"active":true}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/channels/{c}/read-state"),
        u,
        json!({"message_id":mid}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/v1/messages/{mid}/unfurls"),
        &f.token,
        json!({"url":"https://example.com/object","card":{"title":"Object"}}),
    )
    .await;
    for kind in ["details", "comments"] {
        ok(
            a,
            "POST",
            &format!("/api/apps/{}/entities/requests", f.id),
            u,
            json!({"kind":kind,"url":"https://example.com/object","message_id":mid,"channel_id":c}),
        )
        .await;
    }
    ok(
        a,
        "POST",
        &format!("/api/apps/{}/invoke", f.id),
        u,
        json!({"kind":"command","name":"event","channel_id":c}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{}/invoke", f.id),
        u,
        json!({"kind":"shortcut","name":"event","channel_id":c}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{}/invoke", f.id),
        u,
        json!({"kind":"home"}),
    )
    .await;
    let post = ok(
        a,
        "POST",
        "/api/apps/v1/messages",
        &f.token,
        json!({"channel_id":c,"content":"Action"}),
    )
    .await;
    let action_mid = post["data"]["id"].as_str().unwrap();
    ok(
        a,
        "POST",
        &format!("/api/apps/v1/messages/{action_mid}/blocks"),
        &f.token,
        json!([{"type":"button","action_id":"inspect","label":"Inspect"}]),
    )
    .await;
    let invocation = ok(
        a,
        "POST",
        &format!("/api/apps/{}/invoke", f.id),
        u,
        json!({"kind":"action","channel_id":c,"message_id":action_mid,"action_id":"inspect"}),
    )
    .await;
    let mut tx = db.transaction().await?;
    let callback = apps::one(
        &mut tx,
        "SELECT payload::jsonb::text FROM app_deliveries WHERE event_id=$1",
        &[invocation["event_id"].as_str().unwrap()],
    )
    .await?;
    drop(tx);
    let surface=ok(a,"POST","/api/apps/v1/surfaces",&f.token,json!({"kind":"modal","user_id":u,"channel_id":c,"trigger_id":callback["data"]["trigger_id"],"blocks":[{"type":"input","action_id":"answer","label":"Answer"}]})).await;
    ok(a,"POST",&format!("/api/apps/{}/invoke",f.id),u,json!({"kind":"submit","surface_id":surface["id"],"channel_id":c,"values":{"answer":"yes"}})).await;
    // Approval of a changed command registry is the source of commands.changed.
    let mut tx = db.transaction().await?;
    let original = apps::one(
        &mut tx,
        "SELECT json_build_object('manifest',manifest::jsonb)::text FROM apps WHERE id=$1",
        &[&f.id],
    )
    .await?;
    drop(tx);
    let mut manifest = original["manifest"].clone();
    manifest["commands"] = json!(["event", "another"]);
    let _changed = ok(
        a,
        "PATCH",
        &format!("/api/apps/{}", f.id),
        u,
        json!({"name":"Changed bot","manifest":manifest}),
    )
    .await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{}/actions/submit", f.id),
        u,
        Value::Null,
    )
    .await;
    ok(a,"POST",&format!("/api/apps/{}/actions/install",f.id),backend::auth::DEFAULT_DEV_USER_ID,json!({"revision":ok(a,"GET",&format!("/api/apps/{}",f.id),u,Value::Null).await["revision"]})).await;
    ok(a, "DELETE", &format!("/api/channels/{c}"), u, Value::Null).await;
    ok(
        a,
        "POST",
        &format!("/api/apps/{}/actions/disable", f.id),
        u,
        Value::Null,
    )
    .await;
    let types = event_types(&f).await?;
    for name in [
        "user.joined",
        "user.typing",
        "conversation.marked",
        "app.mentioned",
        "entity.details_requested",
        "entity.comments_requested",
        "command.invoked",
        "shortcut.invoked",
        "home.opened",
        "action.invoked",
        "view.submitted",
        "bot.changed",
        "commands.changed",
        "channel.deleted",
        "tokens.revoked",
        "app.installed",
        "app.uninstalled",
    ] {
        assert!(types.contains(name), "Missing {name}: {types:?}");
    }
    Ok(())
}

#[tokio::test]
async fn websocket_control_frames_and_scoped_presence_commands() -> anyhow::Result<()> {
    use anyhow::Context;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    // Avoid unrelated events from other parallel fixtures occupying the socket.
    let f = fixture_with_events(json!([
        "app.uninstalled",
        "tokens.revoked",
        "app.rate_limited"
    ]))
    .await?;
    let ticket = ok(
        &f.app,
        "POST",
        "/api/apps/v1/socket-ticket",
        &f.token,
        Value::Null,
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = f.app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!(
        "ws://{address}/api/apps/v1/socket?ticket={}",
        ticket["ticket"].as_str().unwrap()
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await?;
    let hello: Value = serde_json::from_str(ws.next().await.unwrap()?.to_text()?)?;
    assert_eq!(hello["type"], "hello");
    for kind in ["presence_query", "presence_sub"] {
        ws.send(Message::Text(
            json!({"type":kind,"id":kind,"users":[f.owner]})
                .to_string()
                .into(),
        ))
        .await?;
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                let frame = ws.next().await.unwrap().unwrap();
                let v: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                if v["request_id"] == kind {
                    break v;
                }
            }
        })
        .await
        .with_context(|| format!("waiting for {kind} response"))?;
        assert_eq!(snapshot["type"], "presence_snapshot");
        assert_eq!(snapshot["users"][0]["user_id"], f.owner);
    }
    // Presence is refreshed when a real lease changes.
    ok(
        &f.app,
        "POST",
        "/api/presence/heartbeat",
        &f.owner,
        json!({}),
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let frame = ws.next().await.unwrap().unwrap();
            let v: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            if v["type"] == "presence_snapshot" && v["users"][0]["presence"] == "active" {
                break;
            }
        }
    })
    .await
    .context("waiting for active presence subscription update")?;
    exhaust_event_quota(&f).await?;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/disable", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    let mut terminal_types = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let Some(Ok(frame)) = ws.next().await else {
                panic!("No goodbye")
            };
            if let Ok(text) = frame.to_text() {
                let v: Value = serde_json::from_str(text).unwrap();
                if v["type"] == "goodbye" {
                    break;
                }
                if v["delivery_id"].is_string() {
                    if matches!(
                        v["event"]["type"].as_str(),
                        Some("app.uninstalled" | "tokens.revoked" | "app.deleted")
                    ) {
                        terminal_types.push(v["event"]["type"].as_str().unwrap().to_owned());
                    }
                    ws.send(Message::Text(
                        json!({"ack":v["delivery_id"],"ack_token":v["ack_token"]})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                }
            }
        }
    })
    .await
    .context("waiting for terminal events and goodbye")?;
    assert_eq!(
        terminal_types.first().map(String::as_str),
        Some("app.uninstalled")
    );
    assert!(terminal_types.iter().any(|s| s == "tokens.revoked"));
    server.abort();
    Ok(())
}

#[tokio::test]
async fn dispatcher_enforces_quota_without_charging_retries() -> anyhow::Result<()> {
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created","app.rate_limited"],"transport":"websocket"})).await?;
    let mut db = f.state.db.clone();
    // Pin synthetic, acknowledged history inside the quota's lower-bounded window.
    // A transaction's now() is fixed at BEGIN: seeding 30,000 rows can cross the
    // wall-clock hour before claim() starts, which otherwise resets the quota.
    // Moving these rows to the previous window below still verifies expiry.
    let mut tx = db.transaction().await?;
    toasty::sql::statement("UPDATE app_deliveries SET acked_at=now() WHERE installation_id=$1")
        .bind(f.installation.as_str())
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    ok(
        &f.app,
        "POST",
        "/api/messages",
        &f.owner,
        json!({"channel_id":f.channel,"content":"queued before the boundary"}),
    )
    .await;
    let mut tx = db.transaction().await?;
    toasty::sql::statement("INSERT INTO app_deliveries(id,event_id,installation_id,event_type,payload,first_delivered_at,acked_at) SELECT gen_random_uuid()::text,gen_random_uuid()::text,$1,'message.created','{}',now()+interval '1 hour',now() FROM generate_series(1,30000)").bind(f.installation.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    let warning = apps::delivery::claim(&f.state.db, &f.installation)
        .await?
        .unwrap();
    assert_eq!(warning["event"]["type"], "app.rate_limited");
    assert!(
        apps::delivery::claim(&f.state.db, &f.installation)
            .await?
            .is_none()
    );
    let mut tx = db.transaction().await?;
    toasty::sql::statement("UPDATE app_deliveries SET first_delivered_at=date_trunc('hour',now())-interval '1 second' WHERE installation_id=$1 AND acked_at IS NOT NULL").bind(f.installation.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    let mut found = false;
    for _ in 0..1000 {
        let Some(delivery) = apps::delivery::claim(&f.state.db, &f.installation).await? else {
            break;
        };
        if delivery["event"]["data"]["content"] == "queued before the boundary" {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "Queued source event should resume in the next window"
    );
    Ok(())
}

#[tokio::test]
async fn deployed_registry_and_contracts_match_the_reference_snapshot() -> anyhow::Result<()> {
    let config = Config::from_env();
    let mut db = init_db(&config.database_url).await?;
    let mut tx = db.transaction().await?;
    let mut actual=apps::query(&mut tx,"SELECT json_build_object('name',name,'scope',required_scope,'audience',audience)::text FROM app_event_types",&[]).await?;
    let mut expected = apps::events::coverage()["native_events"]
        .as_array()
        .unwrap()
        .clone();
    actual.sort_by_key(Value::to_string);
    expected.sort_by_key(Value::to_string);
    assert_eq!(actual, expected);
    let mut actual=apps::query(&mut tx,"SELECT json_build_object('alias',alias,'event',event,'conversation_kind',conversation_kind)::text FROM app_event_aliases",&[]).await?;
    let mut expected = apps::events::coverage()["aliases"]
        .as_array()
        .unwrap()
        .clone();
    actual.sort_by_key(Value::to_string);
    expected.sort_by_key(Value::to_string);
    assert_eq!(actual, expected);
    let schema: Value = serde_json::from_str(include_str!("../../docs/app-events.schema.json"))?;
    assert_eq!(
        schema["properties"]["event"]["properties"]["type"]["enum"],
        json!(apps::EVENTS)
    );
    Ok(())
}

async fn reconfigure(f: &Fixture, manifest: Value) -> anyhow::Result<String> {
    ok(
        &f.app,
        "PATCH",
        &format!("/api/apps/{}", f.id),
        &f.owner,
        json!({"name":"Review fixture","manifest":manifest}),
    )
    .await;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/submit", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    let detail = ok(
        &f.app,
        "GET",
        &format!("/api/apps/{}", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/install", f.id),
        backend::auth::DEFAULT_DEV_USER_ID,
        json!({"revision":detail["revision"]}),
    )
    .await;
    Ok(ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/token", f.id),
        &f.owner,
        Value::Null,
    )
    .await["access_token"]
        .as_str()
        .unwrap()
        .to_owned())
}
async fn payloads(f: &Fixture, channel: &str) -> anyhow::Result<Vec<Value>> {
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    Ok(apps::query(&mut tx,"SELECT payload::text FROM app_deliveries WHERE installation_id=$1 AND channel_id=$2 AND acked_at IS NULL AND expires_at>now()",&[&f.installation,channel]).await?)
}

#[tokio::test]
async fn delegated_consent_never_subscribes_the_app_to_unjoined_private_conversations()
-> anyhow::Result<()> {
    let f = fixture().await?;
    let private = ok(
        &f.app,
        "POST",
        "/api/channels",
        &f.owner,
        json!({"channel_name":format!("private_{}",Uuid::new_v4().simple()),"is_private":true}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let dm = ok(
        &f.app,
        "POST",
        "/api/dms",
        &f.owner,
        json!({"target_user_ids":[f.other]}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for channel in [&private, &dm] {
        ok(
            &f.app,
            "POST",
            "/api/messages",
            &f.owner,
            json!({"channel_id":channel,"content":"not for this app"}),
        )
        .await;
        assert!(
            payloads(&f, channel).await?.is_empty(),
            "Unjoined private conversation leaked"
        );
    }
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/channels/{private}", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    ok(
        &f.app,
        "POST",
        "/api/messages",
        &f.owner,
        json!({"channel_id":private,"content":"joined"}),
    )
    .await;
    assert!(
        payloads(&f, &private)
            .await?
            .iter()
            .any(|p| p["type"] == "message.created")
    );
    ok(
        &f.app,
        "DELETE",
        &format!("/api/apps/{}/channels/{private}", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let allowed=apps::one(&mut tx,"SELECT json_build_object('allowed',app_delivery_allowed(d))::text FROM app_deliveries d WHERE installation_id=$1 AND channel_id=$2 AND event_type='message.created' LIMIT 1",&[&f.installation,&private]).await?;
    assert_eq!(
        allowed["allowed"], false,
        "Delegated consent must not replace Bot membership at dispatch"
    );
    Ok(())
}

#[tokio::test]
async fn link_callbacks_and_message_streams_obey_their_distinct_read_scopes() -> anyhow::Result<()>
{
    let f = fixture().await?;
    let link_token=reconfigure(&f,json!({"scopes":["links:write"],"events":["link.shared"],"transport":"websocket","link_domains":["example.com"]})).await?;
    let m=ok(&f.app,"POST","/api/messages",&f.owner,json!({"channel_id":f.channel,"content":"secret surrounding text https://example.com/item https://unregistered.example/private"})).await;
    let data = payloads(&f, &f.channel).await?;
    let link = data.iter().find(|p| p["type"] == "link.shared").unwrap();
    assert_eq!(link["data"]["id"], m["data"]["id"]);
    assert_eq!(link["data"]["links"], json!(["https://example.com/item"]));
    assert!(link["data"]["content"].is_null());
    assert!(!link.to_string().contains("secret surrounding"));
    assert_eq!(
        request(
            &f.app,
            "GET",
            &format!(
                "/api/apps/v1/shapes/messages?channel_id={}&offset=-1",
                f.channel
            ),
            &link_token,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let token=reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created","message.updated","message.replied"],"transport":"websocket"})).await?;
    let m=ok(&f.app,"POST","/api/messages",&f.owner,json!({"channel_id":f.channel,"content":"visible text","metadata":{"event_type":"secret_review_metadata","event_payload":{"ticket":"confidential"}}})).await;
    let mid = m["data"]["id"].as_str().unwrap();
    ok(
        &f.app,
        "PATCH",
        &format!("/api/messages/{mid}"),
        &f.owner,
        json!({"content":"visible edit"}),
    )
    .await;
    ok(&f.app,"POST","/api/messages",&f.owner,json!({"channel_id":f.channel,"content":"reply","parent_id":mid,"metadata":{"event_type":"secret_review_metadata","event_payload":{}}})).await;
    for p in payloads(&f, &f.channel).await? {
        assert!(
            !p.to_string().contains("secret_review_metadata"),
            "Metadata escaped through current/previous/reply data"
        );
    }
    // A real Electric response must use a shape whose selected columns exclude metadata.
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/apps/v1/shapes/messages?channel_id={}&offset=-1",
                    f.channel
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert!(response.status().is_success());
    let schema = response
        .headers()
        .get("electric-schema")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(!schema.contains("metadata"));
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    assert!(!String::from_utf8_lossy(&bytes).contains("secret_review_metadata"));
    Ok(())
}

#[tokio::test]
async fn token_refresh_and_consent_revocation_serialize_on_the_installation() -> anyhow::Result<()>
{
    for authorization_code in [false, true] {
        let f = fixture().await?;
        let grant_body = if authorization_code {
            let code = apps::secret("zoo_code_");
            let verifier = "a".repeat(43);
            use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
            use sha2::{Digest, Sha256};
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
            let mut db = f.state.db.clone();
            let mut tx = db.transaction().await?;
            apps::one(&mut tx,"INSERT INTO app_oauth_codes(code_hash,installation_id,user_id,redirect_uri,code_challenge,scopes,expires_at) SELECT $1,id,$3,'https://example.com/callback',$4,scopes,now()+interval '5 minutes' FROM app_installations WHERE id=$2 RETURNING '{}'::text",&[&apps::hash(&code),&f.installation,&f.owner,&challenge]).await?;
            tx.commit().await?;
            json!({"grant_type":"authorization_code","code":code,"redirect_uri":"https://example.com/callback","code_verifier":verifier})
        } else {
            json!({"grant_type":"refresh_token","refresh_token":f.grant_refresh})
        };
        let mut db = f.state.db.clone();
        let mut barrier = db.transaction().await?;
        apps::lock_installation(&mut barrier, &f.installation).await?;
        let router = f.app.clone();
        let mut exchange = tokio::spawn(async move {
            request(&router, "POST", "/api/apps/oauth/token", "", grant_body).await
        });
        let router = f.app.clone();
        let owner = f.owner.clone();
        let path = format!("/api/apps/{}/authorization", f.id);
        let mut revoke =
            tokio::spawn(
                async move { request(&router, "DELETE", &path, &owner, Value::Null).await },
            );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut exchange)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut revoke)
                .await
                .is_err()
        );
        barrier.commit().await?;
        let refreshed = exchange.await?;
        assert_eq!(revoke.await?.0, StatusCode::OK);
        assert!(
            refreshed.0 == StatusCode::OK || refreshed.0 == StatusCode::UNAUTHORIZED,
            "{refreshed:?}"
        );
        if refreshed.0 == StatusCode::OK {
            let token = refreshed.1["access_token"].as_str().unwrap();
            assert_eq!(
                request(&f.app, "GET", "/api/apps/v1/auth", token, Value::Null)
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
            let rt = refreshed.1["refresh_token"].as_str().unwrap();
            assert_eq!(
                request(
                    &f.app,
                    "POST",
                    "/api/apps/oauth/token",
                    "",
                    json!({"grant_type":"refresh_token","refresh_token":rt})
                )
                .await
                .0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            request(&f.app, "GET", "/api/apps/v1/auth", &f.grant, Value::Null)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    Ok(())
}

#[tokio::test]
async fn delegated_resources_remain_bound_to_the_user_and_the_issuing_app() -> anyhow::Result<()> {
    let f = fixture().await?;
    let owner_invite = ok(
        &f.app,
        "POST",
        "/api/invite-requests",
        &f.owner,
        json!({"email":"owner-review@example.com"}),
    )
    .await;
    let other_invite = ok(
        &f.app,
        "POST",
        "/api/invite-requests",
        &f.other,
        json!({"email":"other-review@example.com"}),
    )
    .await;
    let invites = ok(
        &f.app,
        "GET",
        "/api/apps/v1/invite-requests",
        &f.grant,
        Value::Null,
    )
    .await;
    assert!(
        invites
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == owner_invite["id"])
    );
    assert!(
        !invites
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == other_invite["id"])
    );
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/connection", f.id),
        &f.other,
        json!({"connect":true}),
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("/api/apps/v1/user-connections/{}", f.other),
            &f.grant,
            json!({"connected":true})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ok(
            &f.app,
            "GET",
            &format!("/api/apps/{}/connection", f.id),
            &f.other,
            Value::Null
        )
        .await["state"],
        "requested"
    );
    let created=ok(&f.app,"POST","/api/apps",&f.owner,json!({"name":"Other review assistant","manifest":{"scopes":["assistant:write"],"events":["assistant.started"],"transport":"websocket"}})).await;
    let id = created["id"].as_str().unwrap();
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{id}/actions/submit"),
        &f.owner,
        Value::Null,
    )
    .await;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{id}/actions/install"),
        backend::auth::DEFAULT_DEV_USER_ID,
        json!({"revision":1}),
    )
    .await;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{id}/channels/{}", f.channel),
        &f.owner,
        Value::Null,
    )
    .await;
    let session = ok(
        &f.app,
        "POST",
        "/api/assistant-sessions",
        &f.owner,
        json!({"app_id":id,"user_id":f.owner,"channel_id":f.channel}),
    )
    .await;
    let session_id = session["id"].as_str().unwrap();
    let rows = ok(
        &f.app,
        "GET",
        "/api/apps/v1/assistant-sessions",
        &f.grant,
        Value::Null,
    )
    .await;
    assert!(
        !rows
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == session["id"])
    );
    assert_eq!(
        request(
            &f.app,
            "PATCH",
            &format!("/api/apps/v1/assistant-sessions/{session_id}"),
            &f.grant,
            json!({"title":"tampered"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    ok(
        &f.app,
        "PATCH",
        &format!("/api/assistant-sessions/{session_id}"),
        &f.owner,
        json!({"title":"owner still controls it"}),
    )
    .await;
    Ok(())
}

#[tokio::test]
async fn group_dm_creation_uses_the_committed_participant_set() -> anyhow::Result<()> {
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read","channels:read","dm:write"],"events":["im_created","message.mpim"],"transport":"websocket","dm_enabled":true})).await?;
    let channel = ok(
        &f.app,
        "POST",
        "/api/dms",
        &f.owner,
        json!({"target_user_ids":[f.bot,f.other]}),
    )
    .await["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        !payloads(&f, &channel)
            .await?
            .iter()
            .any(|p| p["type"] == "conversation.created"),
        "Group DM was transiently classified as a two-person DM"
    );
    ok(
        &f.app,
        "POST",
        "/api/messages",
        &f.owner,
        json!({"channel_id":channel,"content":"group DM"}),
    )
    .await;
    assert!(
        payloads(&f, &channel)
            .await?
            .iter()
            .any(|p| p["type"] == "message.created")
    );
    Ok(())
}

#[tokio::test]
async fn reapproval_invalidates_private_ui_response_capabilities_and_unapproved_interactions()
-> anyhow::Result<()> {
    let f = fixture().await?;
    let invocation = ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/invoke", f.id),
        &f.owner,
        json!({"kind":"command","name":"event","channel_id":f.channel}),
    )
    .await;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let event = apps::one(
        &mut tx,
        "SELECT payload::text FROM app_deliveries WHERE event_id=$1",
        &[invocation["event_id"].as_str().unwrap()],
    )
    .await?;
    drop(tx);
    let surface=ok(&f.app,"POST","/api/apps/v1/surfaces",&f.token,json!({"kind":"modal","user_id":f.owner,"channel_id":f.channel,"trigger_id":event["data"]["trigger_id"],"blocks":[{"type":"input","action_id":"draft","label":"Draft"}]})).await;
    assert!(
        ok(
            &f.app,
            "GET",
            &format!("/api/apps/{}/surfaces", f.id),
            &f.owner,
            Value::Null
        )
        .await
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == surface["id"])
    );
    let narrow_token = reconfigure(
        &f,
        json!({"scopes":["messages:read"],"events":["message.created"],"transport":"websocket"}),
    )
    .await?;
    assert!(
        ok(
            &f.app,
            "GET",
            &format!("/api/apps/{}/surfaces", f.id),
            &f.owner,
            Value::Null
        )
        .await
        .as_array()
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            event["data"]["response_path"].as_str().unwrap(),
            &f.owner,
            json!([{"type":"text","text":"stale"}])
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            &format!("/api/apps/{}/invoke", f.id),
            &f.owner,
            json!({"kind":"home"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // A credential with more scopes than the approved installation is rejected, even if active.
    let mut tx = db.transaction().await?;
    apps::one(&mut tx,"UPDATE app_credentials SET scopes='[\"messages:read\",\"messages:write\"]' WHERE installation_id=$1 AND revoked_at IS NULL RETURNING '{}'::text",&[&f.installation]).await?;
    tx.commit().await?;
    assert_eq!(
        request(
            &f.app,
            "GET",
            "/api/apps/v1/auth",
            &narrow_token,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let mut tx = db.transaction().await?;
    assert!(
        apps::mint(
            &mut tx,
            &f.installation,
            Some(&f.owner),
            &json!(["messages:write"])
        )
        .await
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn public_file_capabilities_expire_when_the_app_is_disabled() -> anyhow::Result<()> {
    let f = fixture().await?;
    let id = Uuid::new_v4().to_string();
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    apps::one(&mut tx,"INSERT INTO app_files(id,installation_id,user_id,channel_id,object_key,name,content_type,size_bytes,completed) VALUES($1,$2,$3,$4,$1,'review.txt','text/plain',1,false) RETURNING '{}'::text",&[&id,&f.installation,&f.owner,&f.channel]).await?;
    apps::one(
        &mut tx,
        "UPDATE app_files SET completed=true WHERE id=$1 RETURNING '{}'::text",
        &[&id],
    )
    .await?;
    tx.commit().await?;
    let public = ok(
        &f.app,
        "POST",
        &format!("/api/apps/v1/files/{id}/public-link"),
        &f.token,
        json!({"public":true}),
    )
    .await;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/disable", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "GET",
            public["path"].as_str().unwrap(),
            &f.owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut tx = db.transaction().await?;
    assert_eq!(apps::one(&mut tx,"SELECT json_build_object('revoked',public_token_hash IS NULL)::text FROM app_files WHERE id=$1",&[&id]).await?["revoked"],true);
    Ok(())
}

#[tokio::test]
async fn retention_uses_bounded_batches_and_keeps_fresh_deliveries() -> anyhow::Result<()> {
    let f = fixture().await?;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("INSERT INTO app_deliveries(id,event_id,installation_id,event_type,payload,created_at) SELECT gen_random_uuid()::text,gen_random_uuid()::text,$1,'message.created','{}',now()-interval '31 days' FROM generate_series(1,2100)").bind(f.installation.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    let before = event_types(&f).await?.len();
    apps::delivery::cleanup(&f.state.db).await?;
    let mut tx = db.transaction().await?;
    let count=apps::one(&mut tx,"SELECT json_build_object('old',count(*) FILTER(WHERE created_at<now()-interval '30 days'),'fresh',count(*) FILTER(WHERE created_at>=now()-interval '30 days'))::text FROM app_deliveries WHERE installation_id=$1",&[&f.installation]).await?;
    assert_eq!(count["old"], 0);
    assert!(count["fresh"].as_u64().unwrap() > 0);
    assert!(before > 0);
    Ok(())
}

#[tokio::test]
async fn http_dispatch_batches_messages_and_keeps_revocation_only_apps_eligible()
-> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    // A non-public destination is rejected before network I/O. Attempts still exercise the real batch scheduler.
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created","tokens.revoked"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    for _ in 0..12 {
        ok(
            &f.app,
            "POST",
            "/api/messages",
            &f.owner,
            json!({"channel_id":f.channel,"content":"HTTP batch review"}),
        )
        .await;
    }
    // Exercise a complete batch directly. Counting twelve attempts after one pass
    // detects a one-event-per-tick regression without relying on CI finishing in 2s.
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        apps::delivery::http_pass(&f.state.db),
    )
    .await??;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let count = apps::one(&mut tx, "SELECT json_build_object('attempted',count(*))::text FROM app_deliveries WHERE installation_id=$1 AND event_type='message.created' AND attempts>0", &[&f.installation]).await?;
    assert_eq!(count["attempted"], 12);
    drop(tx);
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/disable", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        apps::delivery::http_pass(&f.state.db),
    )
    .await??;
    let mut tx = db.transaction().await?;
    let count = apps::one(&mut tx, "SELECT json_build_object('attempted',count(*))::text FROM app_deliveries WHERE installation_id=$1 AND event_type='tokens.revoked' AND attempts>0", &[&f.installation]).await?;
    assert_eq!(
        count["attempted"], 1,
        "disabled Apps must still dispatch revocation events"
    );
    Ok(())
}

#[tokio::test]
async fn websocket_slots_and_presence_requests_have_distributed_limits() -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    // This test exercises control requests, not the event delivery queue.
    let f = fixture_with_events(json!([])).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = f.app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut sockets = Vec::new();
    for _ in 0..4 {
        let ticket = ok(
            &f.app,
            "POST",
            "/api/apps/v1/socket-ticket",
            &f.token,
            Value::Null,
        )
        .await;
        let (mut socket, _) = tokio_tungstenite::connect_async(format!(
            "ws://{address}/api/apps/v1/socket?ticket={}",
            ticket["ticket"].as_str().unwrap()
        ))
        .await?;
        let frame = socket.next().await.unwrap()?;
        assert_eq!(
            serde_json::from_str::<Value>(frame.to_text()?)?["type"],
            "hello"
        );
        sockets.push(socket);
    }
    let ticket = ok(
        &f.app,
        "POST",
        "/api/apps/v1/socket-ticket",
        &f.token,
        Value::Null,
    )
    .await;
    let denied = tokio_tungstenite::connect_async(format!(
        "ws://{address}/api/apps/v1/socket?ticket={}",
        ticket["ticket"].as_str().unwrap()
    ))
    .await;
    assert!(
        matches!(denied,Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status()==StatusCode::TOO_MANY_REQUESTS)
    );
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    apps::one(&mut tx,"INSERT INTO app_socket_presence_limits VALUES($1,date_trunc('minute',now()),30) ON CONFLICT(installation_id,window_start) DO UPDATE SET requests=30 RETURNING '{}'::text",&[&f.installation]).await?;
    tx.commit().await?;
    sockets[0]
        .send(Message::Text(
            json!({"type":"presence_query","users":[f.owner]})
                .to_string()
                .into(),
        ))
        .await?;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let frame = sockets[0].next().await.unwrap().unwrap();
            let v: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            if v["type"] == "error" {
                assert_eq!(v["error"], "presence_rate_limited");
                break;
            }
        }
    })
    .await?;
    server.abort();
    Ok(())
}

#[tokio::test]
async fn reapproval_cannot_leave_a_concurrently_refreshed_overscoped_token() -> anyhow::Result<()> {
    let f = fixture().await?;
    let mut db = f.state.db.clone();
    let mut barrier = db.transaction().await?;
    apps::lock_installation(&mut barrier, &f.installation).await?;
    let router = f.app.clone();
    let refresh = f.grant_refresh.clone();
    let mut exchange = tokio::spawn(async move {
        request(
            &router,
            "POST",
            "/api/apps/oauth/token",
            "",
            json!({"grant_type":"refresh_token","refresh_token":refresh}),
        )
        .await
    });
    let narrow = reconfigure(
        &f,
        json!({"scopes":["messages:read"],"events":["message.created"],"transport":"websocket"}),
    );
    tokio::pin!(narrow);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut exchange)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut narrow)
            .await
            .is_err()
    );
    barrier.commit().await?;
    let (refreshed, current) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(exchange, narrow)
    })
    .await?;
    let refreshed = refreshed?;
    let current = current?;
    assert!(matches!(
        refreshed.0,
        StatusCode::OK | StatusCode::UNAUTHORIZED
    ));
    if refreshed.0 == StatusCode::OK {
        assert_eq!(
            request(
                &f.app,
                "GET",
                "/api/apps/v1/auth",
                refreshed.1["access_token"].as_str().unwrap(),
                Value::Null
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        request(&f.app, "GET", "/api/apps/v1/auth", &current, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let mut tx = db.transaction().await?;
    assert_eq!(apps::one(&mut tx,"SELECT json_build_object('count',count(*))::text FROM app_credentials t JOIN app_installations i ON i.id=t.installation_id WHERE i.id=$1 AND t.revoked_at IS NULL AND NOT(t.scopes::jsonb <@ i.scopes::jsonb)",&[&f.installation]).await?["count"],0);
    Ok(())
}

#[tokio::test]
async fn concurrent_bot_profile_changes_do_not_lock_other_event_recipients() -> anyhow::Result<()> {
    let first = fixture().await?;
    let second = fixture().await?;
    let mut db_a = first.state.db.clone();
    let mut db_b = second.state.db.clone();
    let mut a = db_a.transaction().await?;
    let mut b = db_b.transaction().await?;
    apps::one(
        &mut a,
        "SELECT '{}'::text FROM users WHERE id=$1 FOR UPDATE",
        &[&first.bot],
    )
    .await?;
    apps::one(
        &mut b,
        "SELECT '{}'::text FROM users WHERE id=$1 FOR UPDATE",
        &[&second.bot],
    )
    .await?;
    let (left,right)=tokio::time::timeout(std::time::Duration::from_secs(5),async {
        tokio::join!(
            async {apps::one(&mut a,"UPDATE users SET display_name='Concurrent first' WHERE id=$1 RETURNING '{}'::text",&[&first.bot]).await},
            async {apps::one(&mut b,"UPDATE users SET display_name='Concurrent second' WHERE id=$1 RETURNING '{}'::text",&[&second.bot]).await}
        )
    }).await?;
    left?;
    right?;
    a.commit().await?;
    b.commit().await?;
    Ok(())
}

async fn exhaust_event_quota(f: &Fixture) -> anyhow::Result<()> {
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    apps::one(&mut tx,"INSERT INTO app_event_limits VALUES($1,date_trunc('hour',now()),30000) ON CONFLICT(installation_id,window_start) DO UPDATE SET events=30000 RETURNING '{}'::text",&[&f.installation]).await?;
    toasty::sql::statement("INSERT INTO app_deliveries(id,event_id,installation_id,event_type,payload,first_delivered_at,acked_at) SELECT gen_random_uuid()::text,gen_random_uuid()::text,$1,'message.created','{}',now(),now() FROM generate_series(1,30000)").bind(f.installation.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn terminal_events_survive_source_and_http_dispatch_quotas() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created","app.uninstalled","tokens.revoked"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    let observer = fixture().await?;
    reconfigure(
        &observer,
        json!({"scopes":["apps:read"],"events":["app.deleted"],"transport":"websocket"}),
    )
    .await?;
    exhaust_event_quota(&observer).await?;
    exhaust_event_quota(&f).await?;
    ok(
        &f.app,
        "POST",
        "/api/messages",
        &f.owner,
        json!({"channel_id":f.channel,"content":"must be limited"}),
    )
    .await;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/delete", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    let types = event_types(&f).await?;
    for event in ["app.uninstalled", "tokens.revoked"] {
        assert!(types.iter().any(|t| t == event), "Missing {event} at quota");
    }
    let candidates = apps::delivery::http_candidates(&f.state.db).await?;
    assert!(candidates.iter().any(|i| i["id"] == f.installation));
    let mut delivered = Vec::new();
    while let Some(event) = apps::delivery::claim(&f.state.db, &f.installation).await? {
        delivered.push(event["event"]["type"].as_str().unwrap().to_owned());
    }
    for event in ["app.uninstalled", "tokens.revoked"] {
        assert!(
            delivered.iter().any(|t| t == event),
            "Cannot dispatch {event} at quota"
        );
    }
    assert!(!delivered.iter().any(|t| t == "message.created"));
    let deletion = apps::delivery::claim(&observer.state.db, &observer.installation)
        .await?
        .expect("Workspace deletion event must survive the observer quota");
    assert_eq!(deletion["event"]["type"], "app.deleted");
    Ok(())
}

async fn seed_http_candidates(f: &Fixture, count: i64, joined: bool) -> anyhow::Result<String> {
    let prefix = format!("review-http-{}-", Uuid::new_v4());
    let manifest=json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"}).to_string();
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    apps::one(&mut tx,"WITH rows AS(INSERT INTO users(id,user_name,display_name,is_bot) SELECT $1||n,$1||n,'HTTP candidate',true FROM generate_series(1,$2::integer) n RETURNING id) SELECT '{}'::text FROM rows LIMIT 1",&[&prefix,&count.to_string()]).await?;
    apps::one(&mut tx,"WITH rows AS(INSERT INTO apps(id,bot_user_id,owner_id,name,manifest,status) SELECT $1||n,$1||n,$3,'HTTP candidate',$4,'installed' FROM generate_series(1,$2::integer) n RETURNING id) SELECT '{}'::text FROM rows LIMIT 1",&[&prefix,&count.to_string(),&f.owner,&manifest]).await?;
    apps::one(&mut tx,"WITH rows AS(INSERT INTO app_installations(id,app_id,scopes,manifest,revision) SELECT $1||n,$1||n,'[\"messages:read\"]',$3,1 FROM generate_series(1,$2::integer) n RETURNING id) SELECT '{}'::text FROM rows LIMIT 1",&[&prefix,&count.to_string(),&manifest]).await?;
    if joined {
        apps::one(&mut tx,"WITH rows AS(INSERT INTO channel_user(channel_id,user_id,joined_at) SELECT $3,$1||n,now() FROM generate_series(1,$2::integer) n RETURNING user_id) SELECT '{}'::text FROM rows LIMIT 1",&[&prefix,&count.to_string(),&f.channel]).await?;
    }
    apps::one(&mut tx,"WITH rows AS(INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload,required_scope,access_user_id) SELECT gen_random_uuid()::text,gen_random_uuid()::text,$1||n,$3,'message.created','{}','messages:read',$1||n FROM generate_series(1,$2::integer) n RETURNING id) SELECT '{}'::text FROM rows LIMIT 1",&[&prefix,&count.to_string(),&f.channel]).await?;
    tx.commit().await?;
    Ok(prefix)
}

#[tokio::test]
async fn http_selection_skips_inaccessible_events_and_rotates_beyond_one_page() -> anyhow::Result<()>
{
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    let blocked = seed_http_candidates(&f, 101, false).await?;
    let eligible = seed_http_candidates(&f, 101, true).await?;
    let leased = seed_http_candidates(&f, 1, true).await?;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    apps::one(&mut tx,"UPDATE app_deliveries SET lease_until=now()+interval '1 hour' WHERE installation_id=$1 RETURNING '{}'::text",&[&format!("{leased}1")]).await?;
    tx.commit().await?;
    for _ in 0..2 {
        let page = apps::delivery::http_candidates(&f.state.db).await?;
        assert!(page.len() <= 100);
        assert!(
            page.iter()
                .all(|i| !i["id"].as_str().unwrap().starts_with(&blocked)
                    && !i["id"].as_str().unwrap().starts_with(&leased))
        );
    }
    let mut tx = db.transaction().await?;
    let visited=apps::one(&mut tx,"SELECT json_build_object('count',count(*))::text FROM app_installations WHERE starts_with(id,$1) AND last_http_dispatch_at IS NOT NULL",&[&eligible]).await?;
    assert_eq!(
        visited["count"], 101,
        "Every eligible App must get a turn beyond LIMIT 100"
    );
    for prefix in [&blocked, &eligible, &leased] {
        for statement in [
            "DELETE FROM app_deliveries WHERE starts_with(installation_id,$1)",
            "DELETE FROM app_installations WHERE starts_with(id,$1)",
            "DELETE FROM app_versions WHERE starts_with(app_id,$1)",
            "DELETE FROM apps WHERE starts_with(id,$1)",
            "DELETE FROM channel_user WHERE starts_with(user_id,$1)",
            "DELETE FROM users WHERE starts_with(id,$1)",
        ] {
            toasty::sql::statement(statement)
                .bind(prefix.as_str())
                .exec(&mut tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn custom_emoji_storage_metadata_aliases_and_permissions() -> anyhow::Result<()> {
    let f = fixture().await?;
    backend::storage::ensure_bucket_exists(&f.state.s3_client, &f.state.config.s3.bucket).await;
    let name = format!("custom_{}", Uuid::new_v4().simple());
    let alias = format!("alias_{}", Uuid::new_v4().simple());
    let payload = json!({"name":name,"category":"Greetings","aliases":[alias,"挨拶"],"content_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII="});
    let emoji = ok(
        &f.app,
        "POST",
        "/api/emojis/upload",
        &f.owner,
        payload.clone(),
    )
    .await;
    assert_eq!(emoji["category"], "Greetings");
    assert_eq!(emoji["width"], 3);
    assert_eq!(emoji["height"], 1);
    // Existing uploads acquire dimensions without being re-uploaded.
    let mut db = f.state.db.clone();
    toasty::sql::statement("UPDATE workspace_emojis SET width=NULL,height=NULL WHERE name=$1")
        .bind(name.as_str())
        .exec(&mut db)
        .await?;
    backend::storage::image_dimensions::backfill_emojis(&f.state).await?;
    let restored = ok(
        &f.app,
        "GET",
        &format!("/api/emojis/{name}"),
        &f.owner,
        json!(null),
    )
    .await;
    assert_eq!(restored["width"], 3);
    assert_eq!(restored["height"], 1);
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/emojis",
            &f.owner,
            json!({"name":name,"url":"https://example.com/external.png"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(emoji["aliases"], json!([alias, "挨拶"]));
    let url = emoji["url"].as_str().unwrap();
    let id = url.strip_prefix("/api/emoji-images/").unwrap();
    assert!(Uuid::parse_str(id).is_ok());
    let stored = f
        .state
        .s3_client
        .get_object()
        .bucket(&f.state.config.s3.bucket)
        .key(format!("custom-emojis/{id}"))
        .send()
        .await?;
    assert_eq!(stored.content_type(), Some("image/png"));
    let bytes = stored.body.collect().await?.into_bytes();
    assert_eq!(&bytes[16..24], &[0, 0, 0, 3, 0, 0, 0, 1]); // 3:1 is preserved.
    let image = f
        .app
        .clone()
        .oneshot(Request::builder().uri(url).body(Body::empty())?)
        .await?;
    assert_eq!(image.status(), StatusCode::OK);
    assert_eq!(image.headers()["content-type"], "image/png");
    assert_eq!(to_bytes(image.into_body(), usize::MAX).await?, bytes);
    // The public route must serve a hot image without another object-store read.
    f.state
        .s3_client
        .delete_object()
        .bucket(&f.state.config.s3.bucket)
        .key(format!("custom-emojis/{id}"))
        .send()
        .await?;
    let cached = f
        .app
        .clone()
        .oneshot(Request::builder().uri(url).body(Body::empty())?)
        .await?;
    assert_eq!(cached.status(), StatusCode::OK);
    assert_eq!(to_bytes(cached.into_body(), usize::MAX).await?, bytes);
    f.state
        .s3_client
        .put_object()
        .bucket(&f.state.config.s3.bucket)
        .key(format!("custom-emojis/{id}"))
        .content_type("image/png")
        .body(bytes.to_vec().into())
        .send()
        .await?;
    let missing = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/emoji-images/{}", Uuid::new_v4()))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/emojis/upload",
            &f.other,
            payload.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut invalid = payload.clone();
    invalid["data"] = json!("aGVsbG8=");
    assert_eq!(
        request(&f.app, "POST", "/api/emojis/upload", &f.owner, invalid)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut collision = payload.clone();
    collision["name"] = json!(format!("other_{}", Uuid::new_v4().simple()));
    assert_eq!(
        request(&f.app, "POST", "/api/emojis/upload", &f.owner, collision)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let message = ok(&f.app,"POST","/api/messages",&f.owner,json!({"id":Uuid::new_v4().to_string(),"channel_id":f.channel,"content":format!(":{alias}:")})).await;
    let mid = message["data"]["id"].as_str().unwrap();
    let reaction = ok(
        &f.app,
        "POST",
        &format!("/api/messages/{mid}/reactions"),
        &f.owner,
        json!({"emoji":format!(":{alias}:")}),
    )
    .await;
    assert_eq!(reaction["data"]["emoji"], format!(":{name}:"));
    let renamed = format!("renamed_{}", Uuid::new_v4().simple());
    let updated = ok(
        &f.app,
        "POST",
        "/api/emojis",
        &f.owner,
        json!({"name":renamed,"previous_name":name,"url":url,"category":"Zoo","aliases":[alias]}),
    )
    .await;
    assert_eq!(updated["name"], renamed);
    assert_eq!(updated["url"], url);
    assert_eq!(updated["category"], "Zoo");
    assert!(
        updated["aliases"]
            .as_array()
            .unwrap()
            .contains(&json!(name))
    );
    assert_eq!(
        ok(
            &f.app,
            "GET",
            &format!("/api/emojis/{name}"),
            &f.owner,
            Value::Null
        )
        .await["name"],
        renamed
    );
    // Both an old chip and the renamed picker entry must join the existing group.
    for spelling in [&name, &renamed] {
        let joined = ok(
            &f.app,
            "POST",
            &format!("/api/messages/{mid}/reactions"),
            &f.other,
            json!({"emoji":format!(":{spelling}:")}),
        )
        .await;
        assert_eq!(joined["data"]["emoji"], format!(":{name}:"));
    }
    let joined = ok(
        &f.app,
        "POST",
        &format!("/api/messages/{mid}/reactions"),
        &f.owner,
        json!({"emoji":format!(":{renamed}:")}),
    )
    .await;
    assert!(
        joined["txid"].is_null(),
        "renamed selection must not add another chip"
    );
    ok(
        &f.app,
        "DELETE",
        &format!("/api/messages/{mid}/reactions?emoji=%3A{renamed}%3A"),
        &f.owner,
        Value::Null,
    )
    .await;

    assert_eq!(
        request(
            &f.app,
            "DELETE",
            &format!("/api/emojis/{renamed}"),
            &f.other,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut db = f.state.db.clone();
    toasty::sql::statement("UPDATE users SET guest_kind='multi' WHERE id=$1")
        .bind(f.owner.as_str())
        .exec(&mut db)
        .await?;
    assert_eq!(
        request(
            &f.app,
            "DELETE",
            &format!("/api/emojis/{renamed}"),
            &f.owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    toasty::sql::statement("UPDATE users SET guest_kind='none' WHERE id=$1")
        .bind(f.owner.as_str())
        .exec(&mut db)
        .await?;
    ok(
        &f.app,
        "DELETE",
        &format!("/api/emojis/{renamed}"),
        &f.owner,
        Value::Null,
    )
    .await;
    assert_eq!(
        request(
            &f.app,
            "GET",
            &format!("/api/emojis/{renamed}"),
            &f.owner,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    f.state
        .s3_client
        .delete_object()
        .bucket(&f.state.config.s3.bucket)
        .key(format!("custom-emojis/{id}"))
        .send()
        .await?;
    Ok(())
}

#[tokio::test]
async fn upload_shortcode_remains_readable_and_removable() -> anyhow::Result<()> {
    let f = fixture().await?;
    backend::storage::ensure_bucket_exists(&f.state.s3_client, &f.state.config.s3.bucket).await;
    for prefix in ["/api/emojis", "/api/apps/v1/emojis"] {
        ok(&f.app, "POST", &format!("{prefix}/upload"), &f.owner, json!({"name":"upload","content_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII="})).await;
        let emoji = ok(
            &f.app,
            "GET",
            &format!("{prefix}/upload"),
            &f.owner,
            Value::Null,
        )
        .await;
        assert_eq!(emoji["name"], "upload");
        ok(
            &f.app,
            "DELETE",
            &format!("{prefix}/upload"),
            &f.owner,
            Value::Null,
        )
        .await;
    }
    Ok(())
}

#[tokio::test]
async fn delayed_emoji_storage_does_not_block_message_writes() -> anyhow::Result<()> {
    use base64::Engine;
    let f = fixture_with_events(json!([])).await?;
    let storage = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(10)),
        )
        .mount(&storage)
        .await;
    let mut state = f.state.clone();
    let mut config = state.config.s3.clone();
    config.endpoint = storage.uri();
    state.s3_client = backend::storage::create_s3_client(&config);
    let app = create_router(state);
    let name = format!("gate_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let body = json!({"name":name,"category":"","aliases":[],"content_type":"image/gif","data":base64::engine::general_purpose::STANDARD.encode(b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff\x21\xf9\x04\x01\x00\x00\x00\x00\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02\x44\x01\x00\x3b")});
    let uploading = tokio::spawn({
        let app = app.clone();
        let owner = f.owner.clone();
        async move { request(&app, "POST", "/api/emojis/upload", &owner, body).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while storage.received_requests().await.unwrap().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ok(
            &app,
            "POST",
            "/api/messages",
            &f.owner,
            json!({"channel_id":f.channel,"content":"Not blocked by emoji storage"}),
        ),
    )
    .await?;
    let (status, _) = uploading.await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn emoji_backfill_commits_successes_before_the_startup_deadline() -> anyhow::Result<()> {
    use base64::Engine;
    let f = fixture_with_events(json!([])).await?;
    let storage = wiremock::MockServer::start().await;
    let fast = Uuid::new_v4().to_string();
    let slow: Vec<String> = (0..17).map(|_| Uuid::new_v4().to_string()).collect();
    let bytes=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAMAAAABCAYAAAAb4BS0AAAADklEQVR4nGP438DwH4YBMWYHe7X3S+QAAAAASUVORK5CYII=")?;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path_regex(format!(
            "/custom-emojis/{fast}$"
        )))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&storage)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path_regex(format!(
            "/custom-emojis/({})$",
            slow.join("|")
        )))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_bytes(vec![0])
                .set_delay(std::time::Duration::from_secs(11)),
        )
        .mount(&storage)
        .await;
    let mut state = f.state.clone();
    let mut config = state.config.s3.clone();
    config.endpoint = storage.uri();
    state.s3_client = backend::storage::create_s3_client(&config);
    let mut db = state.db.clone();
    for id in std::iter::once(&fast).chain(slow.iter()) {
        toasty::sql::statement(
            "INSERT INTO workspace_emojis(name,url,created_by) VALUES($1,$2,$3)",
        )
        .bind(format!("budget_{id}"))
        .bind(format!("/api/emoji-images/{id}"))
        .bind(f.owner.as_str())
        .exec(&mut db)
        .await?;
    }
    // Seventeen slow reads need three waves at concurrency eight. The former
    // collect-all implementation cannot commit the fast result within 25s.
    tokio::time::timeout(
        std::time::Duration::from_secs(25),
        backend::storage::image_dimensions::backfill_emojis(&state),
    )
    .await??;
    let mut tx = db.transaction().await?;
    let restored=apps::one(&mut tx,"SELECT json_build_object('width',width,'height',height)::text FROM workspace_emojis WHERE name=$1",&[&format!("budget_{fast}")]).await?;
    assert_eq!(restored, json!({"width":3,"height":1}));
    toasty::sql::statement(
        "DELETE FROM workspace_emojis WHERE created_by=$1 AND name LIKE 'budget_%'",
    )
    .bind(f.owner)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn disabled_http_worker_leaves_deliveries_queued_until_another_worker_runs()
-> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    for _ in 0..3 {
        ok(
            &f.app,
            "POST",
            "/api/messages",
            &f.owner,
            json!({"channel_id":f.channel,"content":"queued while disabled"}),
        )
        .await;
    }
    // HTTP 配信を無効にしたプロセスのワーカーは、ポーリング間隔を十分超えて待っても送らない。
    let worker = tokio::spawn(apps::delivery::worker_with(f.state.clone(), false));
    tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
    let count = |f: &Fixture| {
        let mut db = f.state.db.clone();
        let installation = f.installation.clone();
        async move {
            let mut tx = db.transaction().await?;
            let row = apps::one(&mut tx, "SELECT json_build_object('queued',count(*),'attempted',count(*) FILTER(WHERE attempts>0),'acked',count(*) FILTER(WHERE acked_at IS NOT NULL))::text FROM app_deliveries WHERE installation_id=$1 AND event_type='message.created'", &[&installation]).await?;
            anyhow::Ok(row)
        }
    };
    let row = count(&f).await?;
    assert!(row["queued"].as_u64().unwrap() >= 3);
    assert_eq!(
        row["attempted"], 0,
        "disabled worker must not attempt delivery"
    );
    assert_eq!(row["acked"], 0);
    // 別のワーカー（dispatcher 相当）が動けば、溜まっていた配信が失われず再開する。
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        apps::delivery::http_pass(&f.state.db),
    )
    .await??;
    let row = count(&f).await?;
    assert_eq!(row["attempted"], row["queued"], "queued deliveries resume");
    worker.abort();
    Ok(())
}

async fn message_delivery_attempts(f: &Fixture) -> anyhow::Result<Vec<u64>> {
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = apps::query(&mut tx, "SELECT json_build_object('attempts',attempts)::text FROM app_deliveries WHERE installation_id=$1 AND event_type='message.created' ORDER BY id", &[&f.installation]).await?;
    Ok(rows
        .iter()
        .map(|r| r["attempts"].as_u64().unwrap())
        .collect())
}

async fn post_messages(f: &Fixture, n: usize) {
    for _ in 0..n {
        ok(
            &f.app,
            "POST",
            "/api/messages",
            &f.owner,
            json!({"channel_id":f.channel,"content":"concurrent worker check"}),
        )
        .await;
    }
}

#[tokio::test]
async fn concurrent_workers_never_claim_or_send_a_delivery_twice() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    post_messages(&f, 24).await;
    // 別プロセス（backend のワーカーと dispatcher）が同じ行を取り合う状況を、claim の並行実行で作る。
    let claimers: Vec<_> = (0..8)
        .map(|_| {
            let db = f.state.db.clone();
            let installation = f.installation.clone();
            tokio::spawn(async move {
                let mut ids = Vec::new();
                while let Some(event) = apps::delivery::claim(&db, &installation).await? {
                    ids.push(event["delivery_id"].as_str().unwrap().to_string());
                }
                anyhow::Ok(ids)
            })
        })
        .collect();
    let mut all = Vec::new();
    for claimer in claimers {
        all.extend(claimer.await??);
    }
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), all.len(), "a delivery was leased twice");
    assert!(all.len() >= 24);
    // 配信パスそのものを複数同時に回しても、リース中の行は再送されない。
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    post_messages(&f, 24).await;
    let started = std::time::Instant::now();
    let passes: Vec<_> = (0..6)
        .map(|_| {
            let db = f.state.db.clone();
            tokio::spawn(async move { apps::delivery::http_pass(&db).await })
        })
        .collect();
    for pass in passes {
        tokio::time::timeout(std::time::Duration::from_secs(120), pass).await???;
    }
    // 失敗した配信はリース（HTTP は 5 秒）が切れるまで再取得されない。したがって試行回数は、
    // 経過時間から許される回数（最初の 1 回 + 5 秒ごとに 1 回）を超えない。
    // 同時に回した 6 本が同じ行を取り合って重複送信していれば、この上限を超える。
    let allowed = 1 + (started.elapsed().as_secs() / 5);
    let attempts = message_delivery_attempts(&f).await?;
    assert!(attempts.len() >= 24);
    assert!(
        attempts.iter().all(|a| (1..=allowed).contains(a)),
        "a delivery was sent more often than its lease allows ({allowed} max): {attempts:?}"
    );
    Ok(())
}

#[tokio::test]
async fn http_loop_resumes_queued_deliveries_and_stops_on_shutdown() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    // dispatcher が居ない間に溜まった配信。
    post_messages(&f, 5).await;
    assert!(message_delivery_attempts(&f).await?.iter().all(|a| *a == 0));
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let dispatcher = tokio::spawn(apps::delivery::http_loop(f.state.db.clone(), shutdown));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if message_delivery_attempts(&f).await?.iter().all(|a| *a >= 1) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "queued deliveries were not resumed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    stop.send(true)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), dispatcher).await??;
    // 停止後に届いた配信は取らない（溜まるだけ）。
    post_messages(&f, 2).await;
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let attempts = message_delivery_attempts(&f).await?;
    assert_eq!(attempts.iter().filter(|a| **a == 0).count(), 2);
    Ok(())
}

#[tokio::test]
async fn lease_is_measured_from_the_claim_not_from_the_start_of_a_blocked_transaction()
-> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    post_messages(&f, 1).await;
    // 別の配信プロセスが advisory lock を持っている間、claim はロック待ちで止まる。
    let mut holder_db = f.state.db.clone();
    let mut holder = holder_db.transaction().await?;
    apps::one(&mut holder,"SELECT json_build_object('locked',pg_advisory_xact_lock(hashtextextended($1,0)) IS NULL)::text",&[&format!("app-dispatch:{}",f.installation)]).await?;
    let db = f.state.db.clone();
    let installation = f.installation.clone();
    let claim = tokio::spawn(async move { apps::delivery::claim_http(&db, &installation).await });
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    holder.commit().await?;
    let claimed = claim.await??.expect("a delivery is claimable");
    // now()（トランザクション開始）を起点にすると、残りは約 2.5 秒しかなくなり、送信中に再取得される。
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let row = apps::one(&mut tx,"SELECT json_build_object('left',extract(epoch from lease_until-clock_timestamp()))::text FROM app_deliveries WHERE id=$1",&[claimed["delivery_id"].as_str().unwrap()]).await?;
    let left = row["left"].as_f64().unwrap();
    assert!(
        left > 4.0,
        "the lease must last ~5s after the claim, got {left}s"
    );
    Ok(())
}

async fn listen_app_delivery() -> anyhow::Result<backend::db::PgNotificationListener> {
    let listener =
        backend::db::connect_raw_listener(&backend::config::database_url_from_env()).await?;
    listener.client.batch_execute("LISTEN app_delivery").await?;
    Ok(listener)
}

async fn next_notification(
    listener: &mut backend::db::PgNotificationListener,
    wait: std::time::Duration,
) -> bool {
    matches!(
        tokio::time::timeout(wait, listener.notifications.recv()).await,
        Ok(Some(n)) if n.channel() == "app_delivery"
    )
}

#[tokio::test]
async fn new_deliveries_and_manual_retries_notify_the_dispatcher() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    let mut listener = listen_app_delivery().await?;
    while next_notification(&mut listener, std::time::Duration::from_millis(300)).await {}
    post_messages(&f, 1).await;
    assert!(
        next_notification(&mut listener, std::time::Duration::from_secs(5)).await,
        "an INSERT into app_deliveries must NOTIFY app_delivery"
    );
    // 手動再送は UPDATE なので、トリガーではなく再送ハンドラの pg_notify で起こす。
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let row = apps::one(&mut tx, "SELECT json_build_object('id',id)::text FROM app_deliveries WHERE installation_id=$1 AND event_type='message.created' AND acked_at IS NULL LIMIT 1", &[&f.installation]).await?;
    drop(tx);
    while next_notification(&mut listener, std::time::Duration::from_millis(300)).await {}
    ok(
        &f.app,
        "POST",
        &format!(
            "/api/apps/{}/deliveries/{}/retry",
            f.id,
            row["id"].as_str().unwrap()
        ),
        &f.owner,
        Value::Null,
    )
    .await;
    assert!(
        next_notification(&mut listener, std::time::Duration::from_secs(5)).await,
        "a manual retry must NOTIFY app_delivery"
    );
    Ok(())
}

#[tokio::test]
async fn a_notification_wakes_the_dispatcher_without_waiting_for_the_poll() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    let wake = std::sync::Arc::new(tokio::sync::Notify::new());
    let ready = std::sync::Arc::new(tokio::sync::Notify::new());
    let listener = apps::delivery::listen_for_deliveries_with(
        backend::config::database_url_from_env(),
        wake.clone(),
        Some(ready.clone()),
    );
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    // ポーリングは 1 時間後なので、配信されるなら通知で起きたとき以外にない。
    let dispatcher = tokio::spawn(apps::delivery::http_loop_with(
        f.state.db.clone(),
        shutdown,
        wake,
        std::time::Duration::from_secs(3600),
    ));
    // LISTEN が成立するまで待つ。成立直後の「接続したので 1 回起こす」pass が終わってから投稿し、
    // 配信が始まるのが通知の転送によるものだけになるようにする。
    tokio::time::timeout(std::time::Duration::from_secs(30), ready.notified()).await?;
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    post_messages(&f, 3).await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let attempts = message_delivery_attempts(&f).await?;
        if attempts.len() >= 3 && attempts.iter().all(|a| *a >= 1) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the dispatcher was not woken by NOTIFY: {attempts:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    stop.send(true)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), dispatcher).await??;
    listener.abort();
    Ok(())
}

#[tokio::test]
async fn open_circuit_defers_deliveries_without_consuming_attempts() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    let host = format!("down-{}.example.com", Uuid::new_v4().simple());
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":format!("https://{host}/events")})).await?;
    post_messages(&f, 3).await;
    apps::circuit::global().force_open(&host, std::time::Duration::from_secs(120));
    assert!(
        apps::dispatch_metrics::global()
            .render()
            .contains(&format!("zoo_dispatch_circuit_open{{host=\"{host}\"}} 1"))
    );
    apps::delivery::http_pass(&f.state.db).await?;
    // 取っていない（attempts は 0 のまま）。配信は開放時刻まで先送りされ、候補から外れる。
    assert!(message_delivery_attempts(&f).await?.iter().all(|a| *a == 0));
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = apps::query(&mut tx, "SELECT json_build_object('wait',extract(epoch from available_at-now()))::text FROM app_deliveries WHERE installation_id=$1 AND event_type='message.created'", &[&f.installation]).await?;
    tx.commit().await?;
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter().all(|r| r["wait"].as_f64().unwrap() > 60.0),
        "{rows:?}"
    );
    let candidates = apps::delivery::http_candidates(&f.state.db).await?;
    assert!(
        !candidates
            .iter()
            .any(|c| c["id"] == f.installation.as_str()),
        "deferred deliveries must not be candidates"
    );
    apps::circuit::global().reset(&host);
    Ok(())
}

#[tokio::test]
async fn a_pass_started_after_the_stop_signal_takes_no_new_leases() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    post_messages(&f, 3).await;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    stop.send(true)?;
    apps::delivery::http_pass_until(&f.state.db, &stopped).await?;
    assert!(message_delivery_attempts(&f).await?.iter().all(|a| *a == 0));
    Ok(())
}

#[tokio::test]
async fn a_backlog_is_drained_without_waiting_for_the_next_poll() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    post_messages(&f, 3).await;
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    // 取れた分があれば true（続けて pass を回す合図）。取れなければ false。
    assert!(apps::delivery::http_pass_until(&f.state.db, &stopped).await?);
    // 取った配信はバックオフ中なので、すぐ次の pass では取るものがない。
    let mut more = true;
    for _ in 0..20 {
        more = apps::delivery::http_pass_until(&f.state.db, &stopped).await?;
        if !more {
            break;
        }
    }
    assert!(!more, "a pass with nothing left to claim must report false");
    Ok(())
}

#[tokio::test]
async fn the_loop_keeps_draining_a_backlog_without_waiting_for_the_poll() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    // 1 回の pass が 1 インストールに送るのは最大 32 件。それを超える分も、poll（1 時間）を待たずに進む。
    post_messages(&f, 40).await;
    let wake = std::sync::Arc::new(tokio::sync::Notify::new());
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let dispatcher = tokio::spawn(apps::delivery::http_loop_with(
        f.state.db.clone(),
        shutdown,
        wake,
        std::time::Duration::from_secs(3600),
    ));
    // interval の最初の tick で 1 回目の pass が走る。残りは「取れたので続けて回す」で進まなければならない。
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let attempts = message_delivery_attempts(&f).await?;
        if attempts.len() >= 40 && attempts.iter().all(|a| *a >= 1) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the backlog stalled after one pass: {attempts:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    stop.send(true)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), dispatcher).await??;
    Ok(())
}

#[tokio::test]
async fn websocket_deliveries_do_not_wake_the_http_dispatcher() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    // 既定の fixture は WebSocket。HTTP の配信先が無い INSERT では通知しない。
    let f = fixture().await?;
    let mut listener = listen_app_delivery().await?;
    while next_notification(&mut listener, std::time::Duration::from_millis(300)).await {}
    post_messages(&f, 1).await;
    assert!(
        !next_notification(&mut listener, std::time::Duration::from_millis(1500)).await,
        "a WebSocket-only delivery must not NOTIFY the HTTP dispatcher"
    );
    // 同じ App を HTTP にすると通知される。
    reconfigure(&f,json!({"scopes":["messages:read"],"events":["message.created"],"transport":"http","event_url":"https://127.0.0.1:1/events"})).await?;
    while next_notification(&mut listener, std::time::Duration::from_millis(300)).await {}
    post_messages(&f, 1).await;
    assert!(
        next_notification(&mut listener, std::time::Duration::from_secs(5)).await,
        "an HTTP delivery must NOTIFY"
    );
    Ok(())
}

#[tokio::test]
async fn deferring_a_large_backlog_is_batched_and_keeps_attempts() -> anyhow::Result<()> {
    let _guard = HTTP_WORKER_TEST_LOCK.lock().await;
    let f = fixture().await?;
    let mut db = f.state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("INSERT INTO app_deliveries(id,event_id,installation_id,event_type,payload,available_at) SELECT gen_random_uuid()::text,gen_random_uuid()::text,$1,'message.created','{}',now()-interval '1 minute' FROM generate_series(1,2500)").bind(f.installation.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    apps::delivery::defer_install(
        &f.state.db,
        &f.installation,
        std::time::Duration::from_secs(120),
    )
    .await?;
    let mut tx = db.transaction().await?;
    let row = apps::one(&mut tx,"SELECT json_build_object('due',count(*) FILTER(WHERE available_at<=now()),'deferred',count(*) FILTER(WHERE available_at>now()+interval '60 seconds'),'attempted',count(*) FILTER(WHERE attempts>0))::text FROM app_deliveries WHERE installation_id=$1 AND event_type='message.created' AND acked_at IS NULL",&[&f.installation]).await?;
    assert_eq!(row["due"], 0, "{row}");
    assert!(row["deferred"].as_u64().unwrap() >= 2500, "{row}");
    assert_eq!(row["attempted"], 0, "{row}");
    Ok(())
}
