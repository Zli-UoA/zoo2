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
    let manifest = json!({"scopes":apps::SCOPES,"events":apps::EVENTS,"transport":"websocket","include_bot_events":true,"user_connections":true,"functions":["echo"],"commands":["event"],"shortcuts":["event"],"message_shortcuts":["event"],"link_domains":["example.com"],"dm_enabled":true,"redirect_uris":["https://example.com/callback"]});
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
        json!({"name":ename,"url":"https://example.com/emoji.png"}),
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
    apps::events::maintain(&f.state).await?;
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
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let f = fixture().await?;
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
        .await?;
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
    .await?;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/disable", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    let mut terminal_types = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(6), async {
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
    .await?;
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
    let mut db = f.state.db.clone();
    // Prepare delivery history at the hourly boundary; source events still come from an API write.
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
    toasty::sql::statement("INSERT INTO app_deliveries(id,event_id,installation_id,event_type,payload,first_delivered_at,acked_at) SELECT gen_random_uuid()::text,gen_random_uuid()::text,$1,'message.created','{}',now(),now() FROM generate_series(1,30000)").bind(f.installation.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    let warning = apps::delivery::claim(&f.state, &f.installation)
        .await?
        .unwrap();
    assert_eq!(warning["event"]["type"], "app.rate_limited");
    assert!(
        apps::delivery::claim(&f.state, &f.installation)
            .await?
            .is_none()
    );
    let mut tx = db.transaction().await?;
    toasty::sql::statement("UPDATE app_deliveries SET first_delivered_at=date_trunc('hour',now())-interval '1 second' WHERE installation_id=$1 AND acked_at IS NOT NULL").bind(f.installation.as_str()).exec(&mut tx).await?;
    tx.commit().await?;
    let mut found = false;
    for _ in 0..1000 {
        let Some(delivery) = apps::delivery::claim(&f.state, &f.installation).await? else {
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
    apps::delivery::cleanup(&f.state).await?;
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
    let state = f.state.clone();
    let worker = tokio::spawn(async move { apps::delivery::worker(state).await });
    let result=tokio::time::timeout(std::time::Duration::from_secs(2),async{loop{let mut db=f.state.db.clone();let mut tx=db.transaction().await.unwrap();let count=apps::one(&mut tx,"SELECT json_build_object('attempted',count(*))::text FROM app_deliveries WHERE installation_id=$1 AND event_type='message.created' AND attempts>0",&[&f.installation]).await.unwrap();if count["attempted"].as_u64().unwrap()>=12 {break;}tokio::time::sleep(std::time::Duration::from_millis(20)).await;}}).await;
    worker.abort();
    result?;
    ok(
        &f.app,
        "POST",
        &format!("/api/apps/{}/actions/disable", f.id),
        &f.owner,
        Value::Null,
    )
    .await;
    let state = f.state.clone();
    let worker = tokio::spawn(async move { apps::delivery::worker(state).await });
    let result=tokio::time::timeout(std::time::Duration::from_secs(2),async{loop{let mut db=f.state.db.clone();let mut tx=db.transaction().await.unwrap();let count=apps::one(&mut tx,"SELECT json_build_object('attempted',count(*))::text FROM app_deliveries WHERE installation_id=$1 AND event_type='tokens.revoked' AND attempts>0",&[&f.installation]).await.unwrap();if count["attempted"].as_u64().unwrap()>0 {break;}tokio::time::sleep(std::time::Duration::from_millis(20)).await;}}).await;
    worker.abort();
    result?;
    Ok(())
}

#[tokio::test]
async fn websocket_slots_and_presence_requests_have_distributed_limits() -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let f = fixture().await?;
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
