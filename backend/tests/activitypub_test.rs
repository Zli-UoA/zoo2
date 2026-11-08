//! Isolated migrated PostgreSQL required. See docs/activitypub.md.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use base64::{Engine, engine::general_purpose::STANDARD};
use openssl::{hash::MessageDigest, pkey::PKey, rsa::Rsa, sign::Signer};
use serde_json::{Value, json};
use tower::ServiceExt;
static PROTOCOL_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn request(
    app: &Router,
    method: &str,
    user: Option<&str>,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(user) = user {
        req = req.header("X-Dev-User", user);
    }
    let res = app
        .clone()
        .oneshot(req.body(body.map_or(Body::empty(), |v| Body::from(v.to_string())))?)
        .await?;
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 2 * 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}
async fn inbox(
    app: &Router,
    user: &str,
    activity: &Value,
    key: &PKey<openssl::pkey::Private>,
    key_id: &str,
    tamper: bool,
) -> anyhow::Result<StatusCode> {
    let path = format!("/api/activitypub/users/{user}/inbox");
    let body = activity.to_string();
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    let digest = format!(
        "SHA-256={}",
        STANDARD.encode(openssl::sha::sha256(body.as_bytes()))
    );
    let signing = format!(
        "(request-target): post {path}\nhost: zoo.example.com\ndate: {date}\ndigest: {digest}"
    );
    let mut signer = Signer::new(MessageDigest::sha256(), key)?;
    signer.update(signing.as_bytes())?;
    let signature = format!(
        "keyId=\"{key_id}\",algorithm=\"rsa-sha256\",headers=\"(request-target) host date digest\",signature=\"{}\"",
        STANDARD.encode(signer.sign_to_vec()?)
    );
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("host", "zoo.example.com")
                .header("date", date)
                .header("digest", digest)
                .header("signature", signature)
                .header("content-type", "application/activity+json")
                .body(Body::from(if tamper { format!("{body} ") } else { body }))?,
        )
        .await?;
    Ok(res.status())
}
async fn value(db: &mut toasty::Db, sql: &str, args: &[&str]) -> anyhow::Result<Value> {
    let mut tx = db.transaction().await?;
    Ok(backend::apps::one(&mut tx, sql, args).await?)
}

#[tokio::test]
async fn channel_crossposting_visibility_and_ltl() -> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut cfg = Config::from_env();
    cfg.dev_auth_enabled = true;
    let mut db = init_db(&cfg.database_url).await?;
    let app = create_router(AppState::new(db.clone(), cfg));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    let channel = uuid::Uuid::new_v4().to_string();
    let private = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob] {
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    for (id, hidden) in [(&channel, false), (&private, true)] {
        toasty::sql::statement("INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,$4::boolean)").bind(id.as_str()).bind(format!("ltl-{id}")).bind(alice.as_str()).bind(hidden.to_string()).exec(&mut db).await?;
        toasty::sql::statement("INSERT INTO channel_user(channel_id,user_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP)")
            .bind(id.as_str())
            .bind(alice.as_str())
            .exec(&mut db)
            .await?;
    }
    // Posting membership and private-channel boundaries are checked on the server.
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&bob),
            "/api/home",
            Some(json!({"content":"no access","channel_id":channel}))
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&alice),
            "/api/home",
            Some(
                json!({"content":"private","channel_id":private,"apub":true,"visibility":"public"})
            )
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    // Federation-off preserves each selected audience, without a public ActivityPub object.
    for audience in ["public", "unlisted", "followers"] {
        let (status, p) = request(
            &app,
            "POST",
            Some(&alice),
            "/api/home",
            Some(json!({"content":format!("local-{audience}"),"apub":false,"visibility":audience})),
        )
        .await?;
        assert_eq!(status, StatusCode::CREATED, "{p}");
        let id = p["id"].as_str().unwrap();
        assert_eq!(
            request(
                &app,
                "GET",
                None,
                &format!("/api/activitypub/notes/{id}"),
                None
            )
            .await?
            .0,
            StatusCode::NOT_FOUND
        );
        let (_, home) = request(&app, "GET", Some(&bob), "/api/home?timeline=social", None).await?;
        assert_eq!(
            home.as_array().unwrap().iter().any(|p| p["id"] == id),
            audience == "public"
        );
        let (_, ltl) = request(&app, "GET", Some(&alice), "/api/home?timeline=local", None).await?;
        assert_eq!(
            ltl.as_array().unwrap().iter().any(|p| p["id"] == id),
            audience == "public"
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&alice),
            "/api/home",
            Some(json!({"content":"invalid local federation","apub":true,"visibility":"local"}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let(status,post)=request(&app,"POST",Some(&alice),"/api/home",Some(json!({"content":"one original","channel_id":channel,"apub":true,"visibility":"public"}))).await?;
    assert_eq!(status, StatusCode::CREATED, "{post}");
    let id = post["id"].as_str().unwrap();
    let m = value(
        &mut db,
        "SELECT row_to_json(m)::text FROM messages m WHERE id=$1",
        &[id],
    )
    .await?;
    assert_eq!(m["home_post_id"], id);
    assert_eq!(m["apub"], true);
    assert_eq!(m["apub_visibility"], "public");
    let (status, note) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/notes/{id}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(note["content"], "one original");
    // Channel edits update the same public object; Home edits update the channel row.
    assert_eq!(
        request(
            &app,
            "PATCH",
            Some(&alice),
            &format!("/api/messages/{id}"),
            Some(json!({"content":"edited in channel"}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "GET",
            None,
            &format!("/api/activitypub/notes/{id}"),
            None
        )
        .await?
        .1["content"],
        "edited in channel"
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            Some(&alice),
            &format!("/api/home/{id}"),
            Some(json!({"content":"edited in Home"}))
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        value(
            &mut db,
            "SELECT row_to_json(m)::text FROM messages m WHERE id=$1",
            &[id]
        )
        .await?["content"],
        "edited in Home"
    );
    assert_eq!(
        request(&app, "DELETE", Some(&bob), &format!("/api/home/{id}"), None)
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    // Followers-only and local content must not leak through public routes or other users' Home.
    for visibility in ["local", "unlisted", "followers"] {
        let(_,p)=request(&app,"POST",Some(&alice),"/api/home",Some(json!({"content":format!("audience-{visibility}"),"apub":visibility!="local","visibility":visibility}))).await?;
        let pid = p["id"].as_str().unwrap();
        let status = request(
            &app,
            "GET",
            None,
            &format!("/api/activitypub/notes/{pid}"),
            None,
        )
        .await?
        .0;
        assert_eq!(
            status,
            if visibility == "unlisted" {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        );
        let (_, feed) = request(&app, "GET", Some(&bob), "/api/home?timeline=social", None).await?;
        assert_eq!(
            feed.as_array().unwrap().iter().any(|p| p["id"] == pid),
            visibility == "local"
        );
    }
    let (_, outbox) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{alice}/outbox?page=true"),
        None,
    )
    .await?;
    assert_eq!(outbox["orderedItems"].as_array().unwrap().len(), 1);
    assert_eq!(
        request(
            &app,
            "PUT",
            Some(&bob),
            "/api/home/settings",
            Some(json!({"ltl_channel_pattern":"%"}))
        )
        .await?
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            Some(&alice),
            &format!("/api/channels/{channel}"),
            Some(json!({"is_private":true}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    // Pattern streams new public root messages locally, without enabling federation.
    toasty::sql::statement("UPDATE activitypub_settings SET ltl_channel_pattern='^ltl-.*$'")
        .exec(&mut db)
        .await?;
    let message = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&alice),
            "/api/messages",
            Some(json!({"id":message,"channel_id":channel,"content":"local channel stream"}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let streamed = value(
        &mut db,
        "SELECT row_to_json(p)::text FROM home_posts p WHERE id=$1",
        &[&message],
    )
    .await?;
    assert_eq!(streamed["apub"], false);
    let long_id = uuid::Uuid::new_v4().to_string();
    let (status, error) = request(
        &app,
        "POST",
        Some(&alice),
        "/api/messages",
        Some(json!({"id":long_id,"channel_id":channel,"content":"長".repeat(40000)})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{error}");
    assert_eq!(value(&mut db,"SELECT json_build_object('length',char_length(content))::text FROM home_posts WHERE id=$1",&[&long_id]).await?["length"],40000);
    let private_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&alice),
            "/api/messages",
            Some(json!({"id":private_id,"channel_id":private,"content":"never in LTL"}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        value(
            &mut db,
            "SELECT json_build_object('count',count(*))::text FROM home_posts WHERE id=$1",
            &[&private_id]
        )
        .await?["count"],
        0
    );
    let reply_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(request(&app,"POST",Some(&alice),"/api/messages",Some(json!({"id":reply_id,"channel_id":channel,"parent_id":message,"content":"thread reply in local timeline"}))).await?.0,StatusCode::OK);
    assert_eq!(
        value(
            &mut db,
            "SELECT row_to_json(p)::text FROM home_posts p WHERE id=$1",
            &[&reply_id]
        )
        .await?["apub"],
        false
    );
    // Deletion propagates and yields an ActivityStreams tombstone.
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&alice),
            &format!("/api/messages/{id}"),
            None
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "GET",
            None,
            &format!("/api/activitypub/notes/{id}"),
            None
        )
        .await?
        .0,
        StatusCode::GONE
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&alice),
            &format!("/api/home/{message}"),
            None
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        !value(
            &mut db,
            "SELECT row_to_json(m)::text FROM messages m WHERE id=$1",
            &[&message]
        )
        .await?["deleted_at"]
            .is_null()
    );
    toasty::sql::statement("UPDATE activitypub_settings SET ltl_channel_pattern=''")
        .exec(&mut db)
        .await?;
    toasty::sql::statement("DELETE FROM messages WHERE id=$1")
        .bind(reply_id)
        .exec(&mut db)
        .await?;
    for c in [&channel, &private] {
        toasty::sql::statement("DELETE FROM messages WHERE channel_id=$1")
            .bind(c.as_str())
            .exec(&mut db)
            .await?;
        toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1")
            .bind(c.as_str())
            .exec(&mut db)
            .await?;
        toasty::sql::statement("DELETE FROM channels WHERE id=$1")
            .bind(c.as_str())
            .exec(&mut db)
            .await?;
    }
    for u in [&alice, &bob] {
        toasty::sql::statement("DELETE FROM users WHERE id=$1")
            .bind(u.as_str())
            .exec(&mut db)
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn signed_federation_follow_create_update_delete_replay() -> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut cfg = Config::from_env();
    cfg.dev_auth_enabled = true;
    let mut db = init_db(&cfg.database_url).await?;
    let state = AppState::new(db.clone(), cfg);
    let app = create_router(state.clone());
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob] {
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    let remote = format!("https://remote.example/users/{}", uuid::Uuid::new_v4());
    let key_id = format!("{remote}#main-key");
    let key = PKey::from_rsa(Rsa::generate(2048)?)?;
    let pem = String::from_utf8(key.public_key_to_pem()?)?;
    toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$2,$3,'remote','Remote',$4,$5)").bind(remote.as_str()).bind(format!("{remote}/inbox")).bind(format!("{remote}/followers")).bind(key_id.as_str()).bind(pem).exec(&mut db).await?;
    let encoded =
        percent_encoding::utf8_percent_encode(&remote, percent_encoding::NON_ALPHANUMERIC);
    let (status, found) = request(
        &app,
        "GET",
        Some(&alice),
        &format!("/api/activitypub/search?q={encoded}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found[0]["id"], remote);
    assert_eq!(found[0]["display_name"], "Remote");
    assert_eq!(found[0]["remote"], true);
    assert!(found[0].get("public_key").is_none());
    assert!(found[0].get("inbox").is_none());
    let local = format!("https://zoo.example.com/api/activitypub/users/{alice}");
    let activity = |kind: &str, object: Value| json!({"@context":"https://www.w3.org/ns/activitystreams","id":format!("{remote}/activities/{}",uuid::Uuid::new_v4()),"actor":remote,"type":kind,"object":object});
    let follow = activity("Follow", json!(local));
    assert_eq!(
        inbox(&app, &alice, &follow, &key, &key_id, true).await?,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        inbox(&app, &alice, &follow, &key, &key_id, false).await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        inbox(&app, &alice, &follow, &key, &key_id, false).await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_deliveries WHERE user_id=$1",&[&alice]).await?["count"],0);
    let (status, _) = request(
        &app,
        "POST",
        Some(&alice),
        "/api/activitypub/follows/respond",
        Some(json!({"actor":remote,"activity_id":follow["id"],"accept":true})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_deliveries WHERE user_id=$1",&[&alice]).await?["count"],1);
    let (_, local_post) = request(
        &app,
        "POST",
        Some(&alice),
        "/api/home",
        Some(json!({"content":"never federate this","apub":false,"visibility":"public"})),
    )
    .await?;
    assert!(local_post["id"].is_string());
    assert_eq!(value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_deliveries WHERE user_id=$1",&[&alice]).await?["count"],1);
    // Publishing queues a signed Create exactly once for the accepted remote follower.
    let (_, post) = request(
        &app,
        "POST",
        Some(&alice),
        "/api/home",
        Some(json!({"content":"federated","apub":true,"visibility":"public"})),
    )
    .await?;
    let queued=value(&mut db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence DESC LIMIT 1",&[&alice]).await?;
    assert_eq!(
        serde_json::from_str::<Value>(queued["payload"].as_str().unwrap())?["type"],
        "Create"
    );
    assert_eq!(
        request(
            &app,
            "PATCH",
            Some(&alice),
            &format!("/api/home/{}", post["id"].as_str().unwrap()),
            Some(json!({"content":"federated edit"}))
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&alice),
            &format!("/api/home/{}", post["id"].as_str().unwrap()),
            None
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    let kinds=value(&mut db,"SELECT json_build_object('kinds',json_agg(payload::json->>'type' ORDER BY sequence))::text FROM apub_deliveries WHERE user_id=$1",&[&alice]).await?;
    assert_eq!(
        kinds["kinds"],
        json!(["Accept", "Create", "Update", "Delete"])
    );
    // Follow a cached remote actor through the public API, then receive Accept.
    assert_eq!(
        request(
            &app,
            "POST",
            Some(&alice),
            "/api/activitypub/follows",
            Some(json!({"actor":remote}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let outbound = value(
        &mut db,
        "SELECT row_to_json(f)::text FROM apub_follows f WHERE user_id=$1 AND direction='outgoing'",
        &[&alice],
    )
    .await?;
    assert_eq!(
        inbox(
            &app,
            &alice,
            &activity("Accept", outbound["activity_id"].clone()),
            &key,
            &key_id,
            false
        )
        .await?,
        StatusCode::ACCEPTED
    );
    let note_id = format!("{remote}/notes/1");
    let note = json!({"type":"Note","id":note_id,"attributedTo":remote,"content":"<p>Hello &amp; <b>Zoo</b></p>","published":"2026-01-01T00:00:00Z","to":[format!("{remote}/followers")]});
    let create = activity("Create", note.clone());
    assert_eq!(
        inbox(&app, &alice, &create, &key, &key_id, false).await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        inbox(&app, &bob, &create, &key, &key_id, false).await?,
        StatusCode::FORBIDDEN
    );
    let (_, local_feed) =
        request(&app, "GET", Some(&alice), "/api/home?timeline=local", None).await?;
    assert!(
        !local_feed
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == note_id)
    );
    let (_, social_feed) =
        request(&app, "GET", Some(&alice), "/api/home?timeline=social", None).await?;
    assert!(
        social_feed
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == note_id)
    );
    let (_, feed) = request(&app, "GET", Some(&alice), "/api/home", None).await?;
    assert!(
        feed.as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == note_id && p["content"] == "Hello & Zoo\n")
    );
    let (_, feed) = request(&app, "GET", Some(&bob), "/api/home?timeline=social", None).await?;
    assert!(!feed.as_array().unwrap().iter().any(|p| p["id"] == note_id));
    let mut update = note.clone();
    update["content"] = json!("Edited");
    update["updated"] = json!("2026-01-02T00:00:00Z");
    assert_eq!(
        inbox(
            &app,
            &alice,
            &activity("Update", update),
            &key,
            &key_id,
            false
        )
        .await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        value(
            &mut db,
            "SELECT row_to_json(p)::text FROM apub_remote_posts p WHERE id=$1",
            &[&note_id]
        )
        .await?["content"],
        "Edited"
    );
    let mut forged = note.clone();
    forged["attributedTo"] = json!("https://other.example/users/x");
    assert_eq!(
        inbox(
            &app,
            &alice,
            &activity("Update", forged),
            &key,
            &key_id,
            false
        )
        .await?,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        inbox(
            &app,
            &alice,
            &activity("Delete", json!(note_id)),
            &key,
            &key_id,
            false
        )
        .await?,
        StatusCode::ACCEPTED
    );
    // Even a new Create activity ID cannot resurrect a deleted remote object.
    assert_eq!(
        inbox(
            &app,
            &alice,
            &activity("Create", note),
            &key,
            &key_id,
            false
        )
        .await?,
        StatusCode::ACCEPTED
    );
    assert!(
        !value(
            &mut db,
            "SELECT row_to_json(p)::text FROM apub_remote_posts p WHERE id=$1",
            &[&note_id]
        )
        .await?["deleted_at"]
            .is_null()
    );
    assert_eq!(
        request(
            &app,
            "DELETE",
            Some(&alice),
            "/api/activitypub/follows",
            Some(json!({"actor":remote}))
        )
        .await?
        .0,
        StatusCode::NO_CONTENT
    );
    // A blocked destination is terminal and cannot stall subsequent delivery records.
    toasty::sql::statement("UPDATE apub_actors SET blocked=true WHERE id=$1")
        .bind(remote.as_str())
        .exec(&mut db)
        .await?;
    assert!(backend::activitypub::deliver_next(&state).await?);
    let failed=value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_deliveries WHERE user_id=$1 AND failed AND attempts=1",&[&alice]).await?;
    assert_eq!(failed["count"], 1);
    for u in [&alice, &bob] {
        toasty::sql::statement("DELETE FROM users WHERE id=$1")
            .bind(u.as_str())
            .exec(&mut db)
            .await?;
    }
    toasty::sql::statement("DELETE FROM apub_remote_posts WHERE actor_id=$1")
        .bind(remote.as_str())
        .exec(&mut db)
        .await?;
    toasty::sql::statement("DELETE FROM apub_actors WHERE id=$1")
        .bind(remote)
        .exec(&mut db)
        .await?;
    Ok(())
}

struct ProtocolFixture {
    app: Router,
    db: toasty::Db,
    user: String,
    other: String,
    remote: String,
    key: PKey<openssl::pkey::Private>,
    key_id: String,
}
async fn protocol_fixture() -> anyhow::Result<ProtocolFixture> {
    let mut cfg = Config::from_env();
    cfg.dev_auth_enabled = true;
    let mut db = init_db(&cfg.database_url).await?;
    let user = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    for id in [&user, &other] {
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    let remote = format!("https://remote.example/users/{}", uuid::Uuid::new_v4());
    let key_id = format!("{remote}#key");
    let key = PKey::from_rsa(Rsa::generate(2048)?)?;
    toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$2,$3,'peer','Peer',$4,$5)")
        .bind(remote.as_str()).bind(format!("{remote}/inbox")).bind(format!("{remote}/followers")).bind(key_id.as_str()).bind(String::from_utf8(key.public_key_to_pem()?)?).exec(&mut db).await?;
    let app = create_router(AppState::new(db.clone(), cfg));
    Ok(ProtocolFixture {
        app,
        db,
        user,
        other,
        remote,
        key,
        key_id,
    })
}
fn peer_activity(f: &ProtocolFixture, kind: &str, object: Value) -> Value {
    json!({"@context":"https://www.w3.org/ns/activitystreams","id":format!("{}/activities/{}",f.remote,uuid::Uuid::new_v4()),"type":kind,"actor":f.remote,"object":object})
}
async fn protocol_post(
    f: &ProtocolFixture,
    visibility: &str,
    apub: bool,
) -> anyhow::Result<String> {
    let (status, body) = request(
        &f.app,
        "POST",
        Some(&f.user),
        "/api/home",
        Some(json!({"content":"protocol test","visibility":visibility,"apub":apub})),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    Ok(body["id"].as_str().unwrap().to_string())
}
async fn receive_peer(f: &ProtocolFixture, activity: &Value) -> anyhow::Result<StatusCode> {
    inbox(&f.app, &f.user, activity, &f.key, &f.key_id, false).await
}

#[tokio::test]
async fn protocol_manual_follow_requires_owner_consent_and_preserves_original_activity()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let local = format!("https://zoo.example.com/api/activitypub/users/{}", f.user);
    let (_, actor) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/users/{}", f.user),
        None,
    )
    .await?;
    assert_eq!(actor["manuallyApprovesFollowers"], true);
    assert_eq!(actor["publicKey"]["owner"], local);
    let follow = peer_activity(&f, "Follow", json!(local));
    for _ in 0..2 {
        assert_eq!(receive_peer(&f, &follow).await?, StatusCode::ACCEPTED);
    }
    let (_, notifications) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(notifications["unread_count"], 1);
    assert_eq!(notifications["items"][0]["pending"], true);
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_deliveries WHERE user_id=$1",
            &[&f.user]
        )
        .await?["n"],
        0
    );
    toasty::sql::statement("INSERT INTO apub_notifications(id,user_id,actor_id,activity_id,kind) SELECT gen_random_uuid()::text,$1,$2,gen_random_uuid()::text,'like' FROM generate_series(1,101)").bind(f.user.as_str()).bind(f.remote.as_str()).exec(&mut f.db).await?;
    let (_, page) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(page["items"].as_array().unwrap().len(), 100);
    assert_eq!(
        page["items"][0]["pending"], false,
        "Newer reactions sort before older pending requests"
    );
    let cursor = page["next_cursor"].as_str().unwrap();
    let (_, older) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/activitypub/notifications?before={cursor}"),
        None,
    )
    .await?;
    assert!(
        older["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["activity_id"] == follow["id"] && item["pending"] == true),
        "Pending requests remain actionable in chronological pagination"
    );
    let private = protocol_post(&f, "followers", true).await?;
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_deliveries WHERE user_id=$1",
            &[&f.user]
        )
        .await?["n"],
        0,
        "unapproved follower received private post"
    );
    let decision = json!({"actor":f.remote,"activity_id":follow["id"],"accept":true});
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.other),
            "/api/activitypub/follows/respond",
            Some(decision.clone())
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    let (first, second) = tokio::join!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/follows/respond",
            Some(decision.clone())
        ),
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/follows/respond",
            Some(decision.clone())
        )
    );
    let statuses = [first?.0, second?.0];
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1);
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::NOT_FOUND)
            .count(),
        1
    );
    let queued=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence LIMIT 1",&[&f.user]).await?;
    let accept: Value = serde_json::from_str(queued["payload"].as_str().unwrap())?;
    assert_eq!(accept["type"], "Accept");
    assert_eq!(accept["object"], follow);
    assert_eq!(accept["actor"], local);
    assert_eq!(accept["to"], json!([f.remote]));
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/follows/respond",
            Some(decision)
        )
        .await?
        .0,
        StatusCode::NOT_FOUND,
        "duplicate approval must not queue twice"
    );
    let (_, followers) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/users/{}/followers", f.user),
        None,
    )
    .await?;
    assert_eq!(followers["totalItems"], 1);
    let newer = protocol_post(&f, "followers", true).await?;
    let snapshot=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence DESC LIMIT 1",&[&f.user]).await?;
    let created: Value = serde_json::from_str(snapshot["payload"].as_str().unwrap())?;
    assert_eq!(
        backend::activitypub::note_value(&created["zoo_post"])?["id"],
        format!("https://zoo.example.com/api/activitypub/notes/{newer}")
    );
    assert_eq!(
        backend::activitypub::note_value(&created["zoo_post"])?["to"],
        json!([format!("{local}/followers")])
    );
    assert!(!created.to_string().contains("activitystreams#Public"));
    let undo = peer_activity(&f, "Undo", follow["id"].clone());
    assert_eq!(receive_peer(&f, &undo).await?, StatusCode::ACCEPTED);
    let (_, followers) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/users/{}/followers", f.user),
        None,
    )
    .await?;
    assert_eq!(followers["totalItems"], 0);
    let request2 = peer_activity(&f, "Follow", json!(local));
    assert_eq!(receive_peer(&f, &request2).await?, StatusCode::ACCEPTED);
    let reject = json!({"actor":f.remote,"activity_id":request2["id"],"accept":false});
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/follows/respond",
            Some(reject)
        )
        .await?
        .0,
        StatusCode::OK
    );
    let rejected=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence DESC LIMIT 1",&[&f.user]).await?;
    let rejection: Value = serde_json::from_str(rejected["payload"].as_str().unwrap())?;
    assert_eq!(rejection["type"], "Reject");
    assert_eq!(rejection["object"], request2);
    // Undo arriving before Follow must not resurrect a cancelled relationship.
    let future = peer_activity(&f, "Follow", json!(local));
    let cancel = peer_activity(&f, "Undo", future["id"].clone());
    assert_eq!(receive_peer(&f, &cancel).await?, StatusCode::ACCEPTED);
    assert_eq!(receive_peer(&f, &future).await?, StatusCode::ACCEPTED);
    assert_eq!(value(&mut f.db,"SELECT json_build_object('n',count(*))::text FROM apub_follows WHERE user_id=$1 AND direction='incoming'",&[&f.user]).await?["n"],0);
    assert_eq!(
        request(
            &f.app,
            "GET",
            None,
            &format!("/api/activitypub/notes/{private}"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    toasty::sql::statement("DELETE FROM users WHERE id=$1 OR id=$2")
        .bind(f.user.as_str())
        .bind(f.other.as_str())
        .exec(&mut f.db)
        .await?;
    Ok(())
}

#[tokio::test]
async fn protocol_like_undo_enforces_actor_ownership_audience_and_notification_isolation()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let post = protocol_post(&f, "public", true).await?;
    let note = format!("https://zoo.example.com/api/activitypub/notes/{post}");
    let (_, wire_note) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/notes/{post}"),
        None,
    )
    .await?;
    assert_eq!(wire_note["type"], "Note");
    assert_eq!(wire_note["likes"], format!("{note}/likes"));
    wire_note["published"]
        .as_str()
        .unwrap()
        .parse::<jiff::Timestamp>()?;
    let like = peer_activity(&f, "Like", json!(note));
    for _ in 0..2 {
        assert_eq!(receive_peer(&f, &like).await?, StatusCode::ACCEPTED);
    }
    let (_, feed) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    assert_eq!(
        feed.as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == post)
            .unwrap()["likes"],
        1
    );
    let (_, collection) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/notes/{post}/likes"),
        None,
    )
    .await?;
    assert_eq!(collection["type"], "Collection");
    assert_eq!(collection["totalItems"], 1);
    let (_, notifications) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(notifications["items"].as_array().unwrap().len(), 1);
    assert_eq!(notifications["items"][0]["kind"], "like");
    let notification = notifications["items"][0]["id"].clone();
    request(
        &f.app,
        "POST",
        Some(&f.other),
        "/api/activitypub/notifications/read",
        Some(json!({"id":notification})),
    )
    .await?;
    let (_, notifications) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(notifications["unread_count"], 1);
    let (_, others) = request(
        &f.app,
        "GET",
        Some(&f.other),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(others["items"], json!([]));
    let mut forged = like.clone();
    forged["actor"] = json!("https://other.example/actor");
    let bad = peer_activity(&f, "Undo", forged);
    assert_eq!(receive_peer(&f, &bad).await?, StatusCode::BAD_REQUEST);
    let undo = peer_activity(&f, "Undo", like["id"].clone());
    assert_eq!(receive_peer(&f, &undo).await?, StatusCode::ACCEPTED);
    assert_eq!(receive_peer(&f, &like).await?, StatusCode::ACCEPTED);
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_reactions WHERE post_id=$1",
            &[&post]
        )
        .await?["n"],
        0
    );
    let first_like = peer_activity(&f, "Like", json!(note));
    let replacement = peer_activity(&f, "Like", json!(note));
    assert_eq!(receive_peer(&f, &first_like).await?, StatusCode::ACCEPTED);
    assert_eq!(receive_peer(&f, &replacement).await?, StatusCode::ACCEPTED);
    let old_undo = peer_activity(&f, "Undo", first_like);
    assert_eq!(receive_peer(&f, &old_undo).await?, StatusCode::ACCEPTED);
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_reactions WHERE post_id=$1",
            &[&post]
        )
        .await?["n"],
        1,
        "old Undo removed the replacement Like"
    );
    let latest_undo = peer_activity(&f, "Undo", replacement);
    assert_eq!(receive_peer(&f, &latest_undo).await?, StatusCode::ACCEPTED);
    let future = peer_activity(&f, "Like", json!(note));
    let cancel = peer_activity(&f, "Undo", future.clone());
    assert_eq!(receive_peer(&f, &cancel).await?, StatusCode::ACCEPTED);
    assert_eq!(receive_peer(&f, &future).await?, StatusCode::ACCEPTED);
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_reactions WHERE post_id=$1",
            &[&post]
        )
        .await?["n"],
        0
    );
    for (visibility, apub) in [("public", false), ("followers", true)] {
        let private = protocol_post(&f, visibility, apub).await?;
        let unauthorized = peer_activity(
            &f,
            "Like",
            json!(format!(
                "https://zoo.example.com/api/activitypub/notes/{private}"
            )),
        );
        assert_eq!(
            receive_peer(&f, &unauthorized).await?,
            StatusCode::NOT_FOUND
        );
        if visibility == "followers" {
            assert_eq!(
                request(
                    &f.app,
                    "POST",
                    Some(&f.other),
                    "/api/activitypub/reactions",
                    Some(json!({"post_id":private}))
                )
                .await?
                .0,
                StatusCode::NOT_FOUND
            );
        }
    }
    let body = json!({"post_id":post});
    for _ in 0..2 {
        assert_eq!(
            request(
                &f.app,
                "POST",
                Some(&f.other),
                "/api/activitypub/reactions",
                Some(body.clone())
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_reactions WHERE post_id=$1",
            &[&post]
        )
        .await?["n"],
        1
    );
    for _ in 0..2 {
        assert_eq!(
            request(
                &f.app,
                "DELETE",
                Some(&f.other),
                "/api/activitypub/reactions",
                Some(body.clone())
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_reactions WHERE post_id=$1",
            &[&post]
        )
        .await?["n"],
        0
    );
    toasty::sql::statement("DELETE FROM users WHERE id=$1 OR id=$2")
        .bind(f.user.as_str())
        .bind(f.other.as_str())
        .exec(&mut f.db)
        .await?;
    Ok(())
}

#[tokio::test]
async fn protocol_outgoing_like_and_undo_use_stable_object_actor_and_private_recipient()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let follow_id = format!(
        "https://zoo.example.com/api/activitypub/activities/{}",
        uuid::Uuid::new_v4()
    );
    toasty::sql::statement("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id) VALUES($1,$2,'outgoing',$3)").bind(f.user.as_str()).bind(f.remote.as_str()).bind(follow_id.as_str()).exec(&mut f.db).await?;
    let wrong = peer_activity(
        &f,
        "Accept",
        json!({"type":"Follow","id":follow_id,"actor":"https://impostor.example/actor","object":f.remote}),
    );
    assert_eq!(receive_peer(&f, &wrong).await?, StatusCode::BAD_REQUEST);
    let accepted = peer_activity(&f, "Accept", json!(follow_id));
    for _ in 0..2 {
        assert_eq!(receive_peer(&f, &accepted).await?, StatusCode::ACCEPTED);
    }
    let (_, notifications) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(notifications["items"].as_array().unwrap().len(), 1);
    assert_eq!(notifications["items"][0]["kind"], "follow_accepted");
    let post = format!("{}/notes/{}", f.remote, uuid::Uuid::new_v4());
    let note = json!({"type":"Note","id":post,"attributedTo":f.remote,"content":"<p>Private &amp; safe</p>","published":"2026-01-01T00:00:00Z","to":[format!("{}/followers",f.remote)]});
    let create = peer_activity(&f, "Create", note);
    assert_eq!(receive_peer(&f, &create).await?, StatusCode::ACCEPTED);
    let body = json!({"post_id":post});
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.other),
            "/api/activitypub/reactions",
            Some(body.clone())
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    for _ in 0..2 {
        assert_eq!(
            request(
                &f.app,
                "POST",
                Some(&f.user),
                "/api/activitypub/reactions",
                Some(body.clone())
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    let queued=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence LIMIT 1",&[&f.user]).await?;
    let like: Value = serde_json::from_str(queued["payload"].as_str().unwrap())?;
    assert_eq!(like["type"], "Like");
    assert_eq!(like["object"], post);
    assert_eq!(
        like["actor"],
        format!("https://zoo.example.com/api/activitypub/users/{}", f.user)
    );
    assert_eq!(like["to"], json!([f.remote]));
    assert!(!like.to_string().contains("activitystreams#Public"));
    // Retract remains available after access is lost, and only emits a single Undo.
    toasty::sql::statement("DELETE FROM apub_follows WHERE user_id=$1 AND direction='outgoing'")
        .bind(f.user.as_str())
        .exec(&mut f.db)
        .await?;
    for _ in 0..2 {
        assert_eq!(
            request(
                &f.app,
                "DELETE",
                Some(&f.user),
                "/api/activitypub/reactions",
                Some(body.clone())
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    let queued=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence DESC LIMIT 1",&[&f.user]).await?;
    let undo: Value = serde_json::from_str(queued["payload"].as_str().unwrap())?;
    assert_eq!(undo["type"], "Undo");
    assert_eq!(undo["object"], like);
    assert_eq!(undo["actor"], like["actor"]);
    assert_eq!(undo["to"], like["to"]);
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_deliveries WHERE user_id=$1",
            &[&f.user]
        )
        .await?["n"],
        2
    );
    toasty::sql::statement("DELETE FROM users WHERE id=$1 OR id=$2")
        .bind(f.user.as_str())
        .bind(f.other.as_str())
        .exec(&mut f.db)
        .await?;
    Ok(())
}

async fn follow_peer(f: &mut ProtocolFixture) -> anyhow::Result<()> {
    toasty::sql::statement("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,accepted) VALUES($1,$2,'outgoing',$3,true) ON CONFLICT(user_id,actor_id,direction) DO UPDATE SET accepted=true")
        .bind(f.user.as_str()).bind(f.remote.as_str()).bind(format!("https://zoo.example.com/activities/{}",uuid::Uuid::new_v4())).exec(&mut f.db).await?;
    Ok(())
}
async fn finish_protocol(f: &mut ProtocolFixture) -> anyhow::Result<()> {
    toasty::sql::statement("DELETE FROM workspace_emojis WHERE created_by=$1 OR created_by=$2")
        .bind(f.user.as_str())
        .bind(f.other.as_str())
        .exec(&mut f.db)
        .await?;
    toasty::sql::statement("DELETE FROM users WHERE id=$1 OR id=$2")
        .bind(f.user.as_str())
        .bind(f.other.as_str())
        .exec(&mut f.db)
        .await?;
    Ok(())
}
#[tokio::test]
async fn misskey_custom_reactions_preserve_tags_replace_with_undo_and_proxy_only_known_images()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    follow_peer(&mut f).await?;
    let post = format!("{}/notes/custom", f.remote);
    let tag = json!({"id":format!("{}/emojis/blobcat",f.remote),"type":"Emoji","name":":blobcat:","icon":{"type":"Image","url":"https://cdn.remote.example/blobcat.png"}});
    let note = json!({"id":post,"type":"Note","attributedTo":f.remote,"content":"<p>Hello <img src=\"https://cdn.remote.example/blobcat.png\" alt=\":blobcat:\"></p>","published":"2026-01-01T00:00:00Z","to":["https://www.w3.org/ns/activitystreams#Public"],"tag":[tag]});
    let create = peer_activity(&f, "Create", note);
    assert_eq!(receive_peer(&f, &create).await?, StatusCode::ACCEPTED);
    let (_, feed) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    let remote = feed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == post)
        .unwrap();
    assert!(remote["content"].as_str().unwrap().contains(":blobcat:"));
    let emoji = remote["emojis"][0].clone();
    let remote_emoji = emoji.clone();
    assert_eq!(emoji["name"], ":blobcat:");
    assert_eq!(emoji["image_id"].as_str().unwrap().len(), 64);
    toasty::sql::statement("INSERT INTO apub_emoji_catalog(origin,software,emojis) VALUES('https://remote.example','misskey',$1::jsonb) ON CONFLICT(origin) DO UPDATE SET emojis=EXCLUDED.emojis,software='misskey',fetched_at=now()")
        .bind(json!([emoji]).to_string()).exec(&mut f.db).await?;
    let image_url = format!("/api/emoji-images/{}", uuid::Uuid::new_v4());
    toasty::sql::statement("INSERT INTO workspace_emojis(name,url,created_by,category,aliases) VALUES('blobcat',$1,$2,'cats','[\"cat_alias\"]'::jsonb)").bind(image_url.as_str()).bind(f.user.as_str()).exec(&mut f.db).await?;
    let encoded = percent_encoding::utf8_percent_encode(&post, percent_encoding::NON_ALPHANUMERIC);
    let (status, options) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/activitypub/reaction-options?post_id={encoded}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let emoji = options["emojis"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == ":blobcat:")
        .unwrap()
        .clone();
    assert_eq!(emoji["local"], true);
    assert_eq!(emoji["category"], "cats");
    assert_eq!(emoji["aliases"], json!(["cat_alias"]));
    assert_ne!(emoji["url"], tag["icon"]["url"]);
    assert_eq!(
        request(
            &f.app,
            "GET",
            Some(&f.other),
            &format!("/api/activitypub/reaction-options?post_id={encoded}"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    let body = json!({"post_id":post,"reaction":":blobcat:","emoji_id":emoji["image_id"]});
    for _ in 0..2 {
        assert_eq!(
            request(
                &f.app,
                "POST",
                Some(&f.user),
                "/api/activitypub/reactions",
                Some(body.clone())
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    let row=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence LIMIT 1",&[&f.user]).await?;
    let like: Value = serde_json::from_str(row["payload"].as_str().unwrap())?;
    assert_eq!(like["type"], "Like");
    assert_eq!(like["_misskey_reaction"], ":blobcat:");
    assert_eq!(like["content"], ":blobcat:");
    assert_eq!(like["tag"][0]["icon"]["url"], emoji["url"]);
    assert_eq!(like["object"], post);
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/reactions",
            Some(json!({"post_id":post,"reaction":"👍"}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let mut tx = f.db.transaction().await?;
    let queued = backend::apps::query(
        &mut tx,
        "SELECT payload FROM apub_deliveries WHERE user_id=$1 ORDER BY sequence",
        &[&f.user],
    )
    .await?;
    drop(tx);
    assert_eq!(queued.len(), 3);
    assert_eq!(queued[1]["type"], "Undo");
    assert_eq!(queued[1]["object"], like);
    assert_eq!(queued[2]["_misskey_reaction"], "👍");
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/reactions",
            Some(json!({"post_id":post,"reaction":":blobcat:","emoji_id":"unknown"}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let local = protocol_post(&f, "public", true).await?;
    let mut incoming = peer_activity(
        &f,
        "Like",
        json!(format!(
            "https://zoo.example.com/api/activitypub/notes/{local}"
        )),
    );
    incoming["_misskey_reaction"] = json!(":blobcat:");
    incoming["content"] = json!(":blobcat:");
    incoming["tag"] = json!([tag]);
    assert_eq!(receive_peer(&f, &incoming).await?, StatusCode::ACCEPTED);
    let (_, feed) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    let local_row = feed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == local)
        .unwrap();
    assert_eq!(local_row["reactions"][0]["reaction"], ":blobcat:");
    assert_eq!(
        local_row["reactions"][0]["emoji"]["image_id"],
        remote_emoji["image_id"]
    );
    // A received descriptor is reusable without being imported into the registry.
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/reactions",
            Some(
                json!({"post_id":post,"reaction":":blobcat:","emoji_id":remote_emoji["image_id"]})
            )
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, received_options) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/activitypub/reaction-options?post_id={encoded}"),
        None,
    )
    .await?;
    assert!(
        received_options["emojis"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["image_id"] == remote_emoji["image_id"])
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/reactions",
            Some(
                json!({"post_id":local,"reaction":":blobcat:","emoji_id":remote_emoji["image_id"]})
            )
        )
        .await?
        .0,
        StatusCode::OK
    );
    let undo = peer_activity(&f, "Undo", incoming["id"].clone());
    assert_eq!(receive_peer(&f, &undo).await?, StatusCode::ACCEPTED);
    toasty::sql::statement("INSERT INTO apub_emoji_images(id,url) VALUES('private-test','https://127.0.0.1/secret') ON CONFLICT DO NOTHING").exec(&mut f.db).await?;
    assert_eq!(
        request(
            &f.app,
            "GET",
            None,
            "/api/activitypub/emoji-images/private-test",
            None
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &f.app,
            "GET",
            None,
            "/api/activitypub/emoji-images/unregistered?url=https://example.com",
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn reply_preserves_parent_recipient_and_audience_including_nonfollowed_author()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let parent = protocol_post(&f, "public", true).await?;
    let local_actor = format!("https://zoo.example.com/api/activitypub/users/{}", f.user);
    let remote_reply = format!("{}/notes/reply", f.remote);
    let note = json!({"id":remote_reply,"type":"Note","attributedTo":f.remote,"inReplyTo":format!("https://zoo.example.com/api/activitypub/notes/{parent}"),"content":"<p>Reply from an actor not followed</p>","published":"2026-01-01T00:00:00Z","to":["https://www.w3.org/ns/activitystreams#Public"],"cc":[local_actor]});
    assert_eq!(
        receive_peer(&f, &peer_activity(&f, "Create", note)).await?,
        StatusCode::ACCEPTED
    );
    let (_, feed) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    assert!(
        feed.as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == remote_reply)
    );
    assert_eq!(
        feed.as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == parent)
            .unwrap()["replies"],
        1
    );
    let (_, other) = request(
        &f.app,
        "GET",
        Some(&f.other),
        "/api/home?timeline=social",
        None,
    )
    .await?;
    assert!(
        !other
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == remote_reply)
    );
    let(status,response)=request(&f.app,"POST",Some(&f.user),"/api/home",Some(json!({"content":"Reply back","visibility":"public","apub":true,"in_reply_to":remote_reply}))).await?;
    assert_eq!(status, StatusCode::CREATED);
    let reply = response["id"].as_str().unwrap();
    let (_, wire) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/notes/{reply}"),
        None,
    )
    .await?;
    assert_eq!(wire["inReplyTo"], remote_reply);
    assert!(wire["cc"].as_array().unwrap().contains(&json!(f.remote)));
    assert_eq!(wire["tag"][0]["href"], f.remote);
    let queued=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence LIMIT 1",&[&f.user]).await?;
    assert_eq!(queued["actor_id"], f.remote);
    request(
        &f.app,
        "PATCH",
        Some(&f.user),
        &format!("/api/home/{reply}"),
        Some(json!({"content":"Edited reply"})),
    )
    .await?;
    request(
        &f.app,
        "DELETE",
        Some(&f.user),
        &format!("/api/home/{reply}"),
        None,
    )
    .await?;
    let mut tx = f.db.transaction().await?;
    let deliveries = backend::apps::query(
        &mut tx,
        "SELECT payload FROM apub_deliveries WHERE user_id=$1 ORDER BY sequence",
        &[&f.user],
    )
    .await?;
    drop(tx);
    assert_eq!(
        deliveries
            .iter()
            .map(|d| d["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Create", "Update", "Delete"]
    );
    let private = protocol_post(&f, "followers", true).await?;
    assert_eq!(request(&f.app,"POST",Some(&f.user),"/api/home",Some(json!({"content":"Public leak","visibility":"public","apub":true,"in_reply_to":private}))).await?.0,StatusCode::BAD_REQUEST);
    let plain = protocol_post(&f, "public", false).await?;
    assert_eq!(request(&f.app,"POST",Some(&f.user),"/api/home",Some(json!({"content":"Federated local-only reply","visibility":"public","apub":true,"in_reply_to":plain}))).await?.0,StatusCode::BAD_REQUEST);
    let(status,other_reply)=request(&f.app,"POST",Some(&f.other),"/api/home",Some(json!({"content":"Local response","visibility":"unlisted","apub":false,"in_reply_to":plain}))).await?;
    assert_eq!(status, StatusCode::CREATED);
    let (_, feed) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    assert!(
        feed.as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == other_reply["id"])
    );
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn reposts_create_distinct_feed_entries_cursor_and_announces_but_never_private_objects()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let parent = protocol_post(&f, "public", false).await?;
    let body = json!({"post_id":parent});
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.other),
            "/api/activitypub/reposts",
            Some(body.clone())
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, feed) = request(&f.app, "GET", Some(&f.other), "/api/home", None).await?;
    let share = feed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == parent)
        .unwrap();
    assert!(share["entry_id"].as_str().unwrap().starts_with("repost:"));
    assert!(share["reposted_by"].is_string());
    assert_eq!(share["reposted"], true);
    let (_, first) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/home?timeline=local&limit=1",
        None,
    )
    .await?;
    let cursor = percent_encoding::utf8_percent_encode(
        first[0]["entry_id"].as_str().unwrap(),
        percent_encoding::NON_ALPHANUMERIC,
    );
    let (_, second) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/home?timeline=local&limit=1&before={cursor}"),
        None,
    )
    .await?;
    assert_ne!(first[0]["entry_id"], second[0]["entry_id"]);
    for _ in 0..2 {
        assert_eq!(
            request(
                &f.app,
                "DELETE",
                Some(&f.other),
                "/api/activitypub/reposts",
                Some(body.clone())
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    let private = protocol_post(&f, "followers", true).await?;
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/reposts",
            Some(json!({"post_id":private}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    follow_peer(&mut f).await?;
    let remote_post = format!("{}/notes/public-repost", f.remote);
    let create = peer_activity(
        &f,
        "Create",
        json!({"type":"Note","id":remote_post,"attributedTo":f.remote,"content":"<p>Public original</p>","published":"2026-01-01T00:00:00Z","to":["https://www.w3.org/ns/activitystreams#Public"]}),
    );
    assert_eq!(receive_peer(&f, &create).await?, StatusCode::ACCEPTED);
    let body = json!({"post_id":remote_post});
    for _ in 0..2 {
        assert_eq!(
            request(
                &f.app,
                "POST",
                Some(&f.user),
                "/api/activitypub/reposts",
                Some(body.clone())
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    let remote_author =
        percent_encoding::utf8_percent_encode(&f.remote, percent_encoding::NON_ALPHANUMERIC);
    let (_, remote_profile) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/home?timeline=social&author={remote_author}"),
        None,
    )
    .await?;
    let originals: Vec<_> = remote_profile
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["id"] == remote_post)
        .collect();
    assert_eq!(
        originals.len(),
        1,
        "remote profile duplicated the locally reposted original"
    );
    assert_eq!(originals[0]["entry_id"], remote_post);
    assert!(
        remote_profile
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("reposted_by").is_none())
    );
    let row=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence LIMIT 1",&[&f.user]).await?;
    let announce: Value = serde_json::from_str(row["payload"].as_str().unwrap())?;
    assert_eq!(announce["type"], "Announce");
    assert_eq!(announce["object"], remote_post);
    assert_eq!(
        announce["to"],
        json!(["https://www.w3.org/ns/activitystreams#Public"])
    );
    let (_, social) = request(
        &f.app,
        "GET",
        Some(&f.other),
        "/api/home?timeline=social",
        None,
    )
    .await?;
    assert!(
        social
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == remote_post && p["reposted_by"].is_string())
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.other),
            "/api/activitypub/reactions",
            Some(json!({"post_id":remote_post}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &f.app,
            "DELETE",
            Some(&f.user),
            "/api/activitypub/reposts",
            Some(body)
        )
        .await?
        .0,
        StatusCode::OK
    );
    let row=value(&mut f.db,"SELECT row_to_json(d)::text FROM apub_deliveries d WHERE user_id=$1 ORDER BY sequence DESC LIMIT 1",&[&f.user]).await?;
    let undo: Value = serde_json::from_str(row["payload"].as_str().unwrap())?;
    assert_eq!(undo["type"], "Undo");
    assert_eq!(undo["object"], announce);
    assert_eq!(undo["to"], announce["to"]);
    let own = protocol_post(&f, "public", true).await?;
    let mut incoming = peer_activity(
        &f,
        "Announce",
        json!(format!(
            "https://zoo.example.com/api/activitypub/notes/{own}"
        )),
    );
    incoming["to"] = json!(["https://www.w3.org/ns/activitystreams#Public"]);
    assert_eq!(receive_peer(&f, &incoming).await?, StatusCode::ACCEPTED);
    let (_, feed) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    assert_eq!(
        feed.as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == own)
            .unwrap()["reposts"],
        1
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.other),
            "/api/activitypub/reposts",
            Some(json!({"post_id":own}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, author_posts) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/home?timeline=social&author={}", f.user),
        None,
    )
    .await?;
    let originals: Vec<_> = author_posts
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["id"] == own)
        .collect();
    assert_eq!(
        originals.len(),
        1,
        "profile duplicated an original for local/remote reposters"
    );
    assert_eq!(originals[0]["entry_id"], own);
    assert!(
        author_posts
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("reposted_by").is_none())
    );
    assert_eq!(
        request(
            &f.app,
            "DELETE",
            Some(&f.other),
            "/api/activitypub/reposts",
            Some(json!({"post_id":own}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let cancel = peer_activity(&f, "Undo", incoming["id"].clone());
    assert_eq!(receive_peer(&f, &cancel).await?, StatusCode::ACCEPTED);
    assert_eq!(receive_peer(&f, &incoming).await?, StatusCode::ACCEPTED);
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_reposts WHERE post_id=$1",
            &[&own]
        )
        .await?["n"],
        0
    );
    toasty::sql::statement("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,accepted) VALUES($1,$2,'outgoing',$3,true)").bind(f.other.as_str()).bind(f.remote.as_str()).bind(format!("https://zoo.example.com/follows/{}",uuid::Uuid::new_v4())).exec(&mut f.db).await?;
    assert_eq!(
        inbox(&f.app, &f.other, &incoming, &f.key, &f.key_id, false).await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_reposts WHERE post_id=$1",
            &[&own]
        )
        .await?["n"],
        0,
        "delivery to another inbox resurrected an undone Announce"
    );
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn remote_reply_notifications_include_content_only_for_recipient_until_deleted()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let parent = protocol_post(&f, "public", true).await?;
    let remote_reply = format!("{}/notes/notification-reply", f.remote);
    let note = json!({"id":remote_reply,"type":"Note","attributedTo":f.remote,"inReplyTo":format!("https://zoo.example.com/api/activitypub/notes/{parent}"),"content":"<p>Reply notification content</p>","published":"2026-01-01T00:00:00Z","to":["https://www.w3.org/ns/activitystreams#Public"]});
    assert_eq!(
        receive_peer(&f, &peer_activity(&f, "Create", note)).await?,
        StatusCode::ACCEPTED
    );
    let (status, notices) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(notices["items"][0]["kind"], "reply");
    assert_eq!(notices["items"][0]["post_id"], remote_reply);
    let actual = notices["items"][0]["content"].clone();
    assert_eq!(actual, "Reply notification content\n");
    let (_, other) = request(
        &f.app,
        "GET",
        Some(&f.other),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert!(other["items"].as_array().unwrap().is_empty());
    assert_eq!(
        receive_peer(
            &f,
            &peer_activity(&f, "Delete", json!({"id":remote_reply,"type":"Tombstone"}))
        )
        .await?,
        StatusCode::ACCEPTED
    );
    let (_, notices) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(notices["items"][0]["kind"], "reply");
    assert!(notices["items"][0]["content"].is_null());
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn same_origin_actor_cannot_reserve_another_actors_activity_or_deleted_note()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let parent = protocol_post(&f, "public", true).await?;
    let victim = format!("https://remote.example/users/{}", uuid::Uuid::new_v4());
    let key_id = format!("{victim}#key");
    toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'victim','Victim',$2,$3)").bind(victim.as_str()).bind(key_id.as_str()).bind(String::from_utf8(f.key.public_key_to_pem()?)?).exec(&mut f.db).await?;
    let activity_id = format!("{victim}/activities/create");
    let note_id = format!("{victim}/notes/real");
    let mut forged = peer_activity(&f, "Add", json!("ignored"));
    forged["id"] = json!(activity_id);
    assert_eq!(receive_peer(&f, &forged).await?, StatusCode::ACCEPTED);
    assert_eq!(
        receive_peer(
            &f,
            &peer_activity(&f, "Delete", json!({"id":note_id,"type":"Tombstone"}))
        )
        .await?,
        StatusCode::ACCEPTED
    );
    let note = json!({"id":note_id,"type":"Note","attributedTo":victim,"content":"<p>Real author</p>","published":"2026-01-01T00:00:00Z","inReplyTo":format!("https://zoo.example.com/api/activitypub/notes/{parent}"),"to":["https://www.w3.org/ns/activitystreams#Public"]});
    let activity = json!({"id":activity_id,"type":"Create","actor":victim,"object":note});
    assert_eq!(
        inbox(&f.app, &f.user, &activity, &f.key, &key_id, false).await?,
        StatusCode::ACCEPTED
    );
    let stored = value(
        &mut f.db,
        "SELECT row_to_json(p)::text FROM apub_remote_posts p WHERE id=$1",
        &[&note_id],
    )
    .await?;
    assert_eq!(stored["actor_id"], victim);
    assert!(stored["deleted_at"].is_null());
    let deletion = json!({"id":format!("{victim}/activities/delete"),"type":"Delete","actor":victim,"object":note_id});
    assert_eq!(
        inbox(&f.app, &f.user, &deletion, &f.key, &key_id, false).await?,
        StatusCode::ACCEPTED
    );
    let mut recreate = activity.clone();
    recreate["id"] = json!(format!("{victim}/activities/recreate"));
    assert_eq!(
        inbox(&f.app, &f.user, &recreate, &f.key, &key_id, false).await?,
        StatusCode::ACCEPTED
    );
    assert!(
        !value(
            &mut f.db,
            "SELECT row_to_json(p)::text FROM apub_remote_posts p WHERE id=$1",
            &[&note_id]
        )
        .await?["deleted_at"]
            .is_null()
    );
    let early = format!("{}/notes/early", f.remote);
    assert_eq!(
        receive_peer(&f, &peer_activity(&f, "Delete", json!(early))).await?,
        StatusCode::ACCEPTED
    );
    let mut early_note = note;
    early_note["id"] = json!(early);
    early_note["attributedTo"] = json!(f.remote);
    assert_eq!(
        receive_peer(&f, &peer_activity(&f, "Create", early_note)).await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        value(
            &mut f.db,
            "SELECT json_build_object('n',count(*))::text FROM apub_remote_posts WHERE id=$1",
            &[&early]
        )
        .await?["n"],
        0
    );
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn more_than_one_page_of_pending_follows_remains_actionable_after_mark_all_read()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) SELECT $1||'/pending/'||n,$1,$1,'pending','Pending',$1,$2 FROM generate_series(1,105) n").bind(f.remote.as_str()).bind(String::from_utf8(f.key.public_key_to_pem()?)?).exec(&mut f.db).await?;
    toasty::sql::statement("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id) SELECT $1,$2||'/pending/'||n,'incoming',$2||'/follow/'||n FROM generate_series(1,105) n").bind(f.user.as_str()).bind(f.remote.as_str()).exec(&mut f.db).await?;
    toasty::sql::statement("INSERT INTO apub_notifications(id,user_id,actor_id,activity_id,kind) SELECT gen_random_uuid()::text,user_id,actor_id,activity_id,'follow_request' FROM apub_follows WHERE user_id=$1").bind(f.user.as_str()).exec(&mut f.db).await?;
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/notifications/read",
            Some(json!({}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, first) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(first["items"].as_array().unwrap().len(), 100);
    let cursor = first["next_cursor"].as_str().unwrap();
    let path = format!("/api/activitypub/notifications?before={cursor}");
    let (_, second) = request(&f.app, "GET", Some(&f.user), &path, None).await?;
    assert_eq!(second["items"].as_array().unwrap().len(), 5);
    assert!(second["next_cursor"].is_null());
    for item in second["items"].as_array().unwrap() {
        assert_eq!(item["pending"], true);
        assert!(
            !first["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == item["id"])
        );
    }
    let target = &second["items"][4];
    assert_eq!(request(&f.app,"POST",Some(&f.user),"/api/activitypub/follows/respond",Some(json!({"actor":target["actor_id"],"activity_id":target["activity_id"],"accept":true}))).await?.0,StatusCode::OK);
    let (_, other) = request(&f.app, "GET", Some(&f.other), &path, None).await?;
    assert!(other["items"].as_array().unwrap().is_empty());
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn timeline_sort_keys_preserve_microseconds_inside_a_single_millisecond() -> anyhow::Result<()>
{
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let older = protocol_post(&f, "public", false).await?;
    let newer = protocol_post(&f, "public", false).await?;
    for (id, time) in [
        (&older, "2026-01-01T00:00:00.000001Z"),
        (&newer, "2026-01-01T00:00:00.000002Z"),
    ] {
        toasty::sql::statement("UPDATE home_posts SET created_at=$2::timestamptz WHERE id=$1")
            .bind(id.as_str())
            .bind(time)
            .exec(&mut f.db)
            .await?;
    }
    let (status, posts) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(posts[0]["id"], newer);
    assert_eq!(posts[0]["sort_key"], "1767225600000002");
    assert_eq!(posts[1]["id"], older);
    assert_eq!(posts[1]["sort_key"], "1767225600000001");
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn apub_profile_is_independent_updates_followers_and_detail_respects_audience()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut cfg = Config::from_env();
    cfg.dev_auth_enabled = true;
    let mut db = init_db(&cfg.database_url).await?;
    let app = create_router(AppState::new(db.clone(), cfg.clone()));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob] {
        toasty::sql::statement(
            "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'Chat name')",
        )
        .bind(id.as_str())
        .exec(&mut db)
        .await?;
    }
    let remote = format!("https://peer.example/users/{alice}");
    toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key,avatar,summary) VALUES($1,$1,$1,'peer','Peer',$1,'test',$2::jsonb,'Remote bio')")
        .bind(remote.as_str()).bind(json!({"image_id":"b".repeat(64),"url":"https://cdn.peer.example/avatar.png"}).to_string()).exec(&mut db).await?;
    toasty::sql::statement("INSERT INTO apub_follows(user_id,actor_id,direction,accepted,activity_id) VALUES($1,$2,'incoming',true,$3)").bind(alice.as_str()).bind(remote.as_str()).bind(uuid::Uuid::new_v4().to_string()).exec(&mut db).await?;
    toasty::sql::statement("INSERT INTO user_role(user_id,role_id) VALUES($1,'role_admin')")
        .bind(alice.as_str())
        .exec(&mut db)
        .await?;
    let pattern = "^(public|ltl)-[a-z]+$";
    assert_eq!(
        request(
            &app,
            "PUT",
            Some(&alice),
            "/api/home/settings",
            Some(json!({"ltl_channel_pattern":pattern}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            Some(&alice),
            "/api/home/settings",
            Some(json!({"ltl_channel_pattern":"["}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let setting = request(&app, "GET", Some(&alice), "/api/home/settings", None)
        .await?
        .1;
    assert_eq!(setting["ltl_channel_pattern"], pattern);
    let matches=value(&mut db,"SELECT json_build_object('yes',channel_matches_ltl('public-news',$1),'no',channel_matches_ltl('private-news',$1),'literal',channel_matches_ltl('public-news','public-%'),'invalid',channel_matches_ltl('public-news','['),'disabled',channel_matches_ltl('public-news',''))::text",&[pattern]).await?;
    assert_eq!(
        matches,
        json!({"yes":true,"no":false,"literal":false,"invalid":false,"disabled":false})
    );
    toasty::sql::statement("UPDATE activitypub_settings SET ltl_channel_pattern=''")
        .exec(&mut db)
        .await?;
    let path = "/api/activitypub/profile";
    assert_eq!(
        request(&app, "GET", Some(&alice), path, None).await?.1["display_name"],
        ""
    );
    let (status,profile) = request(&app,"PUT",Some(&alice),path,Some(json!({"display_name":"  Federation name  ","summary":"Bio <safe>\nnext line","user_id":bob}))).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(profile["display_name"], "Federation name");
    assert_eq!(
        request(&app, "GET", Some(&bob), path, None).await?.1["display_name"],
        ""
    );
    assert_eq!(
        value(
            &mut db,
            "SELECT json_build_object('name',display_name)::text FROM users WHERE id=$1",
            &[&alice]
        )
        .await?["name"],
        "Chat name"
    );
    let actor = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{alice}"),
        None,
    )
    .await?
    .1;
    assert_eq!(actor["name"], "Federation name");
    assert_eq!(actor["preferredUsername"], alice);
    assert_eq!(actor["summary"], "Bio &lt;safe&gt;<br>next line");
    assert!(actor["url"].as_str().unwrap().contains("/public/users/"));
    let public_profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/activitypub/users/{alice}"))
                .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(public_profile.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        public_profile.headers()["location"],
        actor["url"].as_str().unwrap()
    );
    assert_eq!(public_profile.headers()["vary"], "Accept");
    let (_, public_data) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{alice}/profile"),
        None,
    )
    .await?;
    assert!(public_data["account"]["avatar_url"].is_null());
    let avatar_url = "https://avater.zliusercontent.com/test-avatar.png";
    toasty::sql::statement("UPDATE users SET avatar_url=$2 WHERE id=$1")
        .bind(alice.as_str())
        .bind(avatar_url)
        .exec(&mut db)
        .await?;
    let (_, updated_profile) = request(
        &app,
        "GET",
        None,
        &format!("/api/activitypub/users/{alice}/profile"),
        None,
    )
    .await?;
    assert_eq!(updated_profile["account"]["avatar_url"], avatar_url);
    let (_, local_account) = request(
        &app,
        "GET",
        Some(&alice),
        &format!("/api/activitypub/accounts?actor={alice}"),
        None,
    )
    .await?;
    assert_eq!(local_account["avatar_url"], avatar_url);
    assert_eq!(public_data["account"]["display_name"], "Federation name");
    assert_eq!(public_data["account"]["summary"], "Bio <safe>\nnext line");
    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/activitypub/users/nonexistent-public-profile")
                .header("accept", "text/html")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let (_, webfinger) = request(
        &app,
        "GET",
        None,
        &format!("/.well-known/webfinger?resource=acct:{alice}@zoo.example.com"),
        None,
    )
    .await?;
    assert!(
        webfinger["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |link| link["rel"] == "http://webfinger.net/rel/profile-page"
                    && link["type"] == "text/html"
                    && link["href"] == actor["url"]
            )
    );
    assert_eq!(profile["summary"], "Bio <safe>\nnext line");
    let state = AppState::new(db.clone(), cfg.clone());
    while backend::activitypub::queue_next_actor_update(&state).await? {}
    let delivery = value(&mut db,"SELECT payload::jsonb::text FROM apub_deliveries WHERE user_id=$1 AND actor_id=$2 ORDER BY created_at DESC LIMIT 1",&[&alice,&remote]).await?;
    assert_eq!(delivery["type"], "Update");
    assert_eq!(delivery["object"]["name"], "Federation name");
    assert_eq!(delivery["object"]["icon"]["url"], avatar_url);
    assert_eq!(delivery["object"]["summary"], actor["summary"]);
    assert_eq!(delivery["object"]["id"], actor["id"]);
    assert_eq!(delivery["object"]["publicKey"], actor["publicKey"]);
    assert_eq!(
        delivery["to"][0],
        format!("{}/followers", actor["id"].as_str().unwrap())
    );
    let (_, post) = request(
        &app,
        "POST",
        Some(&alice),
        "/api/home",
        Some(json!({"content":"Profile test","apub":true,"visibility":"public"})),
    )
    .await?;
    let id = post["id"].as_str().unwrap();
    let detail = request(&app, "GET", Some(&bob), &format!("/api/home/{id}"), None).await?;
    assert_eq!(detail.0, StatusCode::OK);
    assert_eq!(detail.1["display_name"], "Federation name");
    let own = request(
        &app,
        "GET",
        Some(&alice),
        &format!("/api/home?timeline=social&author={alice}"),
        None,
    )
    .await?
    .1;
    assert!(
        own.as_array()
            .unwrap()
            .iter()
            .all(|p| p["user_id"] == alice)
    );
    let (_, private) = request(
        &app,
        "POST",
        Some(&alice),
        "/api/home",
        Some(json!({"content":"private","apub":true,"visibility":"followers"})),
    )
    .await?;
    let private_id = private["id"].as_str().unwrap();
    assert_eq!(
        request(
            &app,
            "GET",
            Some(&bob),
            &format!("/api/home/{private_id}"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "GET",
            Some(&alice),
            &format!("/api/home/{private_id}"),
            None
        )
        .await?
        .0,
        StatusCode::OK
    );
    let account = request(
        &app,
        "GET",
        Some(&alice),
        &format!(
            "/api/activitypub/accounts?actor={}",
            percent_encoding::utf8_percent_encode(&remote, percent_encoding::NON_ALPHANUMERIC)
        ),
        None,
    )
    .await?
    .1;
    assert_eq!(account["avatar"]["image_id"], "b".repeat(64));
    assert_eq!(account["summary"], "Remote bio");
    for body in [
        json!({"display_name":"a".repeat(101)}),
        json!({"display_name":"bad\nname"}),
        json!({"display_name":"ok","summary":"a".repeat(1001)}),
    ] {
        assert_eq!(
            request(&app, "PUT", Some(&alice), path, Some(body))
                .await?
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    request(
        &app,
        "PUT",
        Some(&alice),
        path,
        Some(json!({"display_name":"","summary":""})),
    )
    .await?;
    assert_eq!(
        request(
            &app,
            "GET",
            None,
            &format!("/api/activitypub/users/{alice}"),
            None
        )
        .await?
        .1["name"],
        "Chat name"
    );
    cfg.dev_auth_enabled = false;
    let protected = create_router(AppState::new(db.clone(), cfg));
    assert_eq!(
        request(
            &protected,
            "PUT",
            None,
            path,
            Some(json!({"display_name":"intruder"}))
        )
        .await?
        .0,
        StatusCode::UNAUTHORIZED
    );
    toasty::sql::statement("DELETE FROM user_role WHERE user_id=$1")
        .bind(alice.as_str())
        .exec(&mut db)
        .await?;
    for id in [&alice, &bob] {
        toasty::sql::statement("DELETE FROM home_posts WHERE user_id=$1")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
        toasty::sql::statement("DELETE FROM users WHERE id=$1")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
    }
    toasty::sql::statement("DELETE FROM apub_actors WHERE id=$1")
        .bind(remote.as_str())
        .exec(&mut db)
        .await?;
    Ok(())
}

#[tokio::test]
async fn misskey_mfm_sources_survive_create_update_and_local_note_export() -> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    follow_peer(&mut f).await?;
    let id = format!("{}/notes/mfm", f.remote);
    let mfm = "**太字** $[ruby 漢字 かんじ] :blobcat:\n`literal`";
    let mut note = json!({"id":id,"type":"Note","attributedTo":f.remote,"content":"<p>HTML fallback</p>","source":{"mediaType":"text/x.misskeymarkdown","content":mfm},"published":"2026-01-01T00:00:00Z","to":["https://www.w3.org/ns/activitystreams#Public"]});
    assert_eq!(
        receive_peer(&f, &peer_activity(&f, "Create", note.clone())).await?,
        StatusCode::ACCEPTED
    );
    let path = format!(
        "/api/home/{}",
        percent_encoding::utf8_percent_encode(&id, percent_encoding::NON_ALPHANUMERIC)
    );
    assert_eq!(
        request(&f.app, "GET", Some(&f.user), &path, None).await?.1["content"],
        mfm
    );
    note.as_object_mut().unwrap().remove("source");
    note["_misskey_content"] = json!("~~更新~~ $[fg.color=ff0000 赤]");
    note["updated"] = json!("2026-01-02T00:00:00Z");
    assert_eq!(
        receive_peer(&f, &peer_activity(&f, "Update", note.clone())).await?,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        request(&f.app, "GET", Some(&f.user), &path, None).await?.1["content"],
        note["_misskey_content"]
    );
    let (_, local) = request(
        &f.app,
        "POST",
        Some(&f.user),
        "/api/home",
        Some(json!({"content":mfm,"apub":true,"visibility":"public"})),
    )
    .await?;
    let (_, export) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/notes/{}", local["id"].as_str().unwrap()),
        None,
    )
    .await?;
    assert_eq!(export["source"]["mediaType"], "text/x.misskeymarkdown");
    assert_eq!(export["source"]["content"], mfm);
    assert_eq!(export["_misskey_content"], mfm);
    assert!(export["content"].as_str().unwrap().contains("<br>"));
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn anonymous_profile_only_lists_public_federated_posts_with_stable_pagination()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let cfg = Config::from_env();
    let mut db = init_db(&cfg.database_url).await?;
    let app = create_router(AppState::new(db.clone(), cfg));
    let alice = uuid::Uuid::new_v4().to_string();
    let bob = uuid::Uuid::new_v4().to_string();
    for id in [&alice, &bob] {
        toasty::sql::statement(
            "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'Public profile')",
        )
        .bind(id.as_str())
        .exec(&mut db)
        .await?;
    }
    // Equal timestamps exercise the ID tie breaker across the page boundary.
    for i in 0..21 {
        toasty::sql::statement("INSERT INTO home_posts(id,user_id,content,apub,visibility,created_at) VALUES($1,$2,$3,true,'public','2026-11-02T12:34:56Z')")
            .bind(format!("{alice}-{i:02}" )).bind(alice.as_str()).bind(format!("visible-public-{i:02}")).exec(&mut db).await?;
    }
    for (name, author, apub, visibility, deleted) in [
        ("unlisted", &alice, "true", "unlisted", "false"),
        ("followers", &alice, "true", "followers", "false"),
        ("nonfederated", &alice, "false", "public", "false"),
        ("deleted", &alice, "true", "public", "true"),
        ("otherauthor", &bob, "true", "public", "false"),
    ] {
        toasty::sql::statement("INSERT INTO home_posts(id,user_id,content,apub,visibility,deleted_at) VALUES($1,$2,$3,$4::boolean,$5,CASE WHEN $6::boolean THEN now() END)")
            .bind(format!("{alice}-{name}")).bind(author.as_str()).bind(format!("hidden-{name}")).bind(apub).bind(visibility).bind(deleted).exec(&mut db).await?;
    }
    let profile_path = format!("/api/activitypub/users/{alice}/profile");
    let (status, data) = request(&app, "GET", None, &profile_path, None).await?;
    assert_eq!(status, StatusCode::OK);
    let posts = data["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 20);
    assert_eq!(posts[0]["content"], "visible-public-20");
    assert_eq!(posts[19]["content"], "visible-public-01");
    assert!(!data.to_string().contains("hidden-"));
    let cursor = format!("{alice}-01");
    assert_eq!(data["next_cursor"], cursor);
    let encoded =
        percent_encoding::utf8_percent_encode(&cursor, percent_encoding::NON_ALPHANUMERIC);
    let (_, older) = request(
        &app,
        "GET",
        None,
        &format!("{profile_path}?before={encoded}"),
        None,
    )
    .await?;
    let posts = older["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0]["content"], "visible-public-00");
    assert!(older["next_cursor"].is_null());
    let (_, invalid_cursor) = request(
        &app,
        "GET",
        None,
        &format!("{profile_path}?before={alice}-followers"),
        None,
    )
    .await?;
    assert!(invalid_cursor["posts"].as_array().unwrap().is_empty());
    let (status, _) = request(
        &app,
        "GET",
        None,
        "/api/activitypub/users/nonexistent-public-profile/profile",
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn outbound_notes_include_misskey_emoji_tags_for_names_and_aliases() -> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let image = format!("/api/emoji-images/{}", uuid::Uuid::new_v4());
    toasty::sql::statement("INSERT INTO workspace_emojis(name,url,created_by,category,aliases) VALUES('zoo_wave',$1,$2,'Greetings','[\"zoo_hello\"]'::jsonb)").bind(image.as_str()).bind(f.user.as_str()).exec(&mut f.db).await?;
    let id = protocol_post(&f, "public", true).await?;
    toasty::sql::statement(
        "UPDATE home_posts SET content='Hello :zoo_wave: :zoo_hello:' WHERE id=$1",
    )
    .bind(id.as_str())
    .exec(&mut f.db)
    .await?;
    let (status, note) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/notes/{id}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(note["source"]["mediaType"], "text/x.misskeymarkdown");
    for name in [":zoo_wave:", ":zoo_hello:"] {
        let tag = note["tag"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tag| tag["name"] == name)
            .unwrap();
        assert_eq!(tag["type"], "Emoji");
        assert_eq!(tag["icon"]["type"], "Image");
        assert_eq!(
            tag["icon"]["url"],
            format!("https://zoo.example.com{image}")
        );
    }
    finish_protocol(&mut f).await
}

#[tokio::test]
async fn channel_linked_posts_share_local_and_remote_reactions() -> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let channel = uuid::Uuid::new_v4().to_string();
    let (status, _) = request(
        &f.app,
        "POST",
        Some(&f.user),
        "/api/channels",
        Some(json!({"id":channel,"channel_name":format!("shared-{}",channel),"is_private":false})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let (status,post) = request(&f.app,"POST",Some(&f.user),"/api/home",Some(json!({"content":"shared reactions","apub":true,"visibility":"public","channel_id":channel}))).await?;
    assert_eq!(status, StatusCode::CREATED);
    let id = post["id"].as_str().unwrap();
    let native = format!("/api/messages/{id}/reactions");
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            &native,
            Some(json!({"emoji":"👍"}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    for reaction in ["🎉", "👍", "👍"] {
        assert_eq!(
            request(
                &f.app,
                "POST",
                Some(&f.user),
                "/api/activitypub/reactions",
                Some(json!({"post_id":id,"reaction":reaction}))
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.other),
            &native,
            Some(json!({"emoji":"👍"}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, home) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/home/{id}"),
        None,
    )
    .await?;
    assert_eq!(home["likes"], 3);
    let groups = home["reactions"].as_array().unwrap();
    assert_eq!(
        groups.iter().find(|r| r["reaction"] == "👍").unwrap()["count"],
        2
    );
    assert!(groups.iter().all(|r| r["selected"] == true));
    assert_eq!(
        request(
            &f.app,
            "DELETE",
            Some(&f.user),
            "/api/activitypub/reactions",
            Some(json!({"post_id":id,"reaction":"👍"}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let count=value(&mut f.db,"SELECT json_build_object('count',count(*))::text FROM message_reactions WHERE message_id=$1",&[id]).await?;
    assert_eq!(count["count"], 2);
    let object = format!("https://zoo.example.com/api/activitypub/notes/{id}");
    let mut incoming = peer_activity(&f, "Like", json!(object));
    incoming["_misskey_reaction"] = json!("👍");
    assert_eq!(receive_peer(&f, &incoming).await?, StatusCode::ACCEPTED);
    let (status, remote) = request(
        &f.app,
        "GET",
        Some(&f.other),
        &format!("/api/channels/{channel}/federated-reactions"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(remote.as_array().unwrap().len(), 1);
    assert_eq!(remote[0]["message_id"], id);
    assert_eq!(remote[0]["emoji"], "👍");
    assert_eq!(remote[0]["user_id"], f.remote);
    let (_, home) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/home/{id}"),
        None,
    )
    .await?;
    assert_eq!(home["likes"], 3);
    assert_eq!(
        home["reactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["reaction"] == "👍")
            .unwrap()["count"],
        2
    );
    let (_, likes) = request(
        &f.app,
        "GET",
        None,
        &format!("/api/activitypub/notes/{id}/likes"),
        None,
    )
    .await?;
    assert_eq!(likes["totalItems"], 3);
    let undo = peer_activity(&f, "Undo", incoming);
    assert_eq!(receive_peer(&f, &undo).await?, StatusCode::ACCEPTED);
    assert_eq!(
        request(
            &f.app,
            "GET",
            Some(&f.user),
            &format!("/api/channels/{channel}/federated-reactions"),
            None
        )
        .await?
        .1,
        json!([])
    );
    let mut custom = peer_activity(&f, "Like", json!(object));
    custom["_misskey_reaction"] = json!(":remote_cat:");
    custom["tag"] = json!([{"type":"Emoji","name":":remote_cat:","icon":{"url":"https://cdn.remote.example/cat.png"}}]);
    assert_eq!(receive_peer(&f, &custom).await?, StatusCode::ACCEPTED);
    let (_, overlay) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/channels/{channel}/federated-reactions"),
        None,
    )
    .await?;
    assert!(overlay[0]["emoji"].as_str().unwrap().starts_with("apub:"));
    let (_, feed) = request(&f.app, "GET", Some(&f.user), "/api/home", None).await?;
    let linked = feed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap();
    let foreign = linked["reactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["reaction"] == ":remote_cat:")
        .unwrap();
    assert_eq!(foreign["can_react"], false);
    assert_eq!(request(&f.app,"POST",Some(&f.user),"/api/activitypub/reactions",Some(json!({"post_id":id,"reaction":":remote_cat:","emoji_id":foreign["emoji"]["image_id"]}))).await?.0,StatusCode::BAD_REQUEST);

    assert!(
        overlay[0]["emoji"]
            .as_str()
            .unwrap()
            .ends_with(":remote_cat:")
    );
    toasty::sql::statement("UPDATE users SET guest_kind='multi' WHERE id=$1")
        .bind(f.other.as_str())
        .exec(&mut f.db)
        .await?;
    assert_eq!(
        request(
            &f.app,
            "GET",
            Some(&f.other),
            &format!("/api/channels/{channel}/federated-reactions"),
            None
        )
        .await?
        .0,
        StatusCode::NOT_FOUND
    );
    toasty::sql::statement("UPDATE users SET guest_kind='none' WHERE id=$1")
        .bind(f.other.as_str())
        .exec(&mut f.db)
        .await?;
    toasty::sql::statement("UPDATE apub_actors SET blocked=true WHERE id=$1")
        .bind(f.remote.as_str())
        .exec(&mut f.db)
        .await?;
    assert_eq!(
        request(
            &f.app,
            "GET",
            Some(&f.user),
            &format!("/api/channels/{channel}/federated-reactions"),
            None
        )
        .await?
        .1,
        json!([])
    );
    toasty::sql::statement("UPDATE apub_actors SET blocked=false WHERE id=$1")
        .bind(f.remote.as_str())
        .exec(&mut f.db)
        .await?;
    assert_eq!(
        receive_peer(&f, &peer_activity(&f, "Undo", custom)).await?,
        StatusCode::ACCEPTED
    );
    // Legacy Home writes during a rolling deployment mirror into the native table.
    let activity = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO apub_reactions(post_id,actor_id,user_id,activity_id,activity_json,reaction) VALUES($1,$2,$3,$4,'{}','😊')").bind(id).bind(format!("local:{}",f.user)).bind(f.user.as_str()).bind(activity.as_str()).exec(&mut f.db).await?;
    let (_, home) = request(
        &f.app,
        "GET",
        Some(&f.user),
        &format!("/api/home/{id}"),
        None,
    )
    .await?;
    assert_eq!(
        home["likes"], 3,
        "Legacy and native copies must not be counted twice"
    );
    toasty::sql::statement("DELETE FROM apub_reactions WHERE activity_id=$1")
        .bind(activity)
        .exec(&mut f.db)
        .await?;
    // Both entry points obey the channel's archive restriction.
    toasty::sql::statement("UPDATE channels SET archive_at=CURRENT_TIMESTAMP WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut f.db)
        .await?;
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/reactions",
            Some(json!({"post_id":id,"reaction":"😊"}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    toasty::sql::statement("DELETE FROM messages WHERE channel_id=$1")
        .bind(channel.as_str())
        .exec(&mut f.db)
        .await?;
    toasty::sql::statement("DELETE FROM home_posts WHERE channel_id=$1")
        .bind(channel.as_str())
        .exec(&mut f.db)
        .await?;
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id=$1")
        .bind(channel.as_str())
        .exec(&mut f.db)
        .await?;
    toasty::sql::statement("DELETE FROM channels WHERE id=$1")
        .bind(channel.as_str())
        .exec(&mut f.db)
        .await?;
    finish_protocol(&mut f).await
}

#[tokio::test]
async fn notification_batch_reads_only_selected_owned_ids_and_preserves_cursor_order()
-> anyhow::Result<()> {
    let _serial = PROTOCOL_SERIAL.lock().await;
    let mut f = protocol_fixture().await?;
    let one = uuid::Uuid::new_v4().to_string();
    let two = uuid::Uuid::new_v4().to_string();
    let foreign = uuid::Uuid::new_v4().to_string();
    for (id, user) in [(&one, &f.user), (&two, &f.user), (&foreign, &f.other)] {
        toasty::sql::statement("INSERT INTO apub_notifications(id,user_id,actor_id,activity_id,kind) VALUES($1,$2,$3,$1,'like')")
            .bind(id.as_str()).bind(user.as_str()).bind(f.remote.as_str()).exec(&mut f.db).await?;
    }
    let (_, before) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/notifications/read",
            Some(json!({"ids":[one,foreign]}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/notifications/read",
            Some(json!({"ids":[]}))
        )
        .await?
        .0,
        StatusCode::OK
    );
    let (_, after) = request(
        &f.app,
        "GET",
        Some(&f.user),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(
        before["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| &v["id"])
            .collect::<Vec<_>>(),
        after["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| &v["id"])
            .collect::<Vec<_>>()
    );
    assert_eq!(after["unread_count"], 1);
    assert_eq!(
        after["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == one)
            .unwrap()["is_read"],
        true
    );
    assert_eq!(
        after["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == two)
            .unwrap()["is_read"],
        false
    );
    let (_, other) = request(
        &f.app,
        "GET",
        Some(&f.other),
        "/api/activitypub/notifications",
        None,
    )
    .await?;
    assert_eq!(other["items"][0]["is_read"], false);
    assert_eq!(
        request(
            &f.app,
            "POST",
            Some(&f.user),
            "/api/activitypub/notifications/read",
            Some(json!({"id":one,"ids":[two]}))
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    finish_protocol(&mut f).await?;
    Ok(())
}

#[tokio::test]
async fn avatar_changes_queue_current_actor_for_accepted_followers_atomically() -> anyhow::Result<()>
{
    let _serial = PROTOCOL_SERIAL.lock().await;
    let cfg = Config::from_env();
    let mut db = init_db(&cfg.database_url).await?;
    let state = AppState::new(db.clone(), cfg);
    let user = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement(
        "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'Chat name')",
    )
    .bind(user.as_str())
    .exec(&mut db)
    .await?;
    let mut recipients = Vec::new();
    for (kind, direction, accepted, blocked) in [
        ("accepted", "incoming", true, false),
        ("second", "incoming", true, false),
        ("pending", "incoming", false, false),
        ("outgoing", "outgoing", true, false),
        ("blocked", "incoming", true, true),
    ] {
        let remote = format!("https://peer.example/{kind}/{user}");
        toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key,blocked) VALUES($1,$1,$1,$1,$1,$1,'test',$2)")
            .bind(remote.as_str()).bind(blocked).exec(&mut db).await?;
        toasty::sql::statement(
            "INSERT INTO apub_follows(user_id,actor_id,direction,accepted,activity_id) VALUES($1,$2,$3,$4,$2)",
        )
        .bind(user.as_str())
        .bind(remote.as_str())
        .bind(direction)
        .bind(accepted)
        .exec(&mut db)
        .await?;
        if accepted && direction == "incoming" && !blocked {
            recipients.push(remote);
        }
    }
    let first = "https://avater.zliusercontent.com/avatars/first.png";
    let latest = "https://avater.zliusercontent.com/avatars/latest.png";
    // Rolled-back avatar changes also roll back their durable update intent.
    let mut tx = db.transaction().await?;
    toasty::sql::statement("UPDATE users SET avatar_url=$2 WHERE id=$1")
        .bind(user.as_str())
        .bind(first)
        .exec(&mut tx)
        .await?;
    tx.rollback().await?;
    assert_eq!(value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_actor_updates WHERE user_id=$1",&[&user]).await?["count"],0);
    for url in [first, latest] {
        toasty::sql::statement("UPDATE users SET avatar_url=$2 WHERE id=$1")
            .bind(user.as_str())
            .bind(url)
            .exec(&mut db)
            .await?;
    }
    assert_eq!(value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_actor_updates WHERE user_id=$1",&[&user]).await?["count"],1);
    // The same pending row also coalesces a concurrent name/bio change.
    toasty::sql::statement("INSERT INTO apub_profiles(user_id,display_name,summary) VALUES($1,'Federation name','Current bio')").bind(user.as_str()).exec(&mut db).await?;
    while backend::activitypub::queue_next_actor_update(&state).await? {}
    let deliveries = toasty::sql::query(
        "SELECT actor_id,payload FROM apub_deliveries WHERE user_id=$1 ORDER BY sequence",
    )
    .bind(user.as_str())
    .exec(&mut db)
    .await?;
    assert_eq!(deliveries.len(), 2);
    let mut activity_id = None;
    for row in deliveries {
        let toasty::stmt::Value::Record(row) = row else {
            panic!("expected record")
        };
        let toasty::stmt::Value::String(actor) = &row.fields[0] else {
            panic!("expected actor")
        };
        assert!(recipients.contains(actor));
        let toasty::stmt::Value::String(payload) = &row.fields[1] else {
            panic!("expected payload")
        };
        let payload: Value = serde_json::from_str(payload)?;
        assert_eq!(payload["type"], "Update");
        assert_eq!(payload["object"]["type"], "Person");
        assert_eq!(payload["object"]["name"], "Federation name");
        assert_eq!(payload["object"]["summary"], "Current bio");
        assert_eq!(
            payload["object"]["icon"],
            json!({"type":"Image","url":latest})
        );
        assert_eq!(payload["actor"], payload["object"]["id"]);
        assert_eq!(
            payload["to"],
            json!([format!("{}/followers", payload["actor"].as_str().unwrap())])
        );
        if let Some(id) = &activity_id {
            assert_eq!(&payload["id"], id);
        }
        activity_id = Some(payload["id"].clone());
    }
    // The same URL produces no second Update. Removal explicitly clears the remote icon.
    toasty::sql::statement("UPDATE users SET avatar_url=$2 WHERE id=$1")
        .bind(user.as_str())
        .bind(latest)
        .exec(&mut db)
        .await?;
    assert_eq!(value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_actor_updates WHERE user_id=$1",&[&user]).await?["count"],0);
    toasty::sql::statement("UPDATE users SET avatar_url=NULL WHERE id=$1")
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
    while backend::activitypub::queue_next_actor_update(&state).await? {}
    let removal = value(&mut db,"SELECT payload::jsonb::text FROM apub_deliveries WHERE user_id=$1 ORDER BY sequence DESC LIMIT 1",&[&user]).await?;
    assert!(removal["object"].get("icon").unwrap().is_null());
    toasty::sql::statement("UPDATE users SET avatar_url=$2 WHERE id=$1")
        .bind(user.as_str())
        .bind(first)
        .exec(&mut db)
        .await?;
    let (left, right) = tokio::join!(
        backend::activitypub::queue_next_actor_update(&state),
        backend::activitypub::queue_next_actor_update(&state)
    );
    left?;
    right?;
    assert_eq!(value(&mut db,"SELECT json_build_object('count',count(*))::text FROM apub_deliveries WHERE user_id=$1",&[&user]).await?["count"],6);
    toasty::sql::statement("UPDATE users SET avatar_url=NULL WHERE id=$1")
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
    while backend::activitypub::queue_next_actor_update(&state).await? {}
    let actor = request(
        &create_router(state),
        "GET",
        None,
        &format!("/api/activitypub/users/{user}"),
        None,
    )
    .await?
    .1;
    assert!(actor.get("icon").unwrap().is_null());
    toasty::sql::statement("DELETE FROM users WHERE id=$1")
        .bind(user)
        .exec(&mut db)
        .await?;
    Ok(())
}
