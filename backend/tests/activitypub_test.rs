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
        let (_, home) = request(&app, "GET", Some(&bob), "/api/home", None).await?;
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
        let (_, feed) = request(&app, "GET", Some(&bob), "/api/home", None).await?;
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
    toasty::sql::statement("UPDATE activitypub_settings SET ltl_channel_pattern='ltl-%'")
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
    let (_, feed) = request(&app, "GET", Some(&alice), "/api/home", None).await?;
    assert!(
        feed.as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == note_id && p["content"] == "Hello & Zoo\n")
    );
    let (_, feed) = request(&app, "GET", Some(&bob), "/api/home", None).await?;
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
