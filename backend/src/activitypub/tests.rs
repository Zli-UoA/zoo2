use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
#[ignore = "requires migrated PostgreSQL and ACTIVITYPUB_BASE_URL"]
async fn durable_signed_delivery_retries_and_preserves_order() -> anyhow::Result<()> {
    let config = crate::config::Config::from_env();
    let mut db = crate::db::init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let user = uuid::Uuid::new_v4().to_string();
    let remote = format!("https://remote.example/users/{}", uuid::Uuid::new_v4());
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
    let key = keys(&state, &user).await?;
    let local = actor_id(&user)?;
    toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'remote','Remote',$1,$2)").bind(remote.as_str()).bind(string(&key,"public_key")?).exec(&mut db).await?;
    let mut tx = db.transaction().await?;
    for i in 0..2 {
        queue(&mut tx,&user,&remote,&json!({"id":format!("{local}/activities/{i}"),"type":"Follow","actor":local,"object":remote})).await?;
    }
    tx.commit().await?;
    let server = MockServer::start().await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let actor = json!({"key_id":format!("{local}#main-key"),"public_key":key["public_key"]});
    Mock::given(method("POST"))
        .and(path("/inbox"))
        .respond_with(move |request: &wiremock::Request| {
            if transport::verify_headers(&request.headers, "/inbox", &request.body, &actor).is_err()
            {
                return ResponseTemplate::new(401);
            }
            ResponseTemplate::new(if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                503
            } else {
                202
            })
        })
        .expect(2)
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let url = reqwest::Url::parse(&format!("{}/inbox", server.uri()))?;
    let first = claim_delivery(&state).await?.unwrap();
    assert_eq!(first["attempts"], 1);
    assert!(
        claim_delivery(&state).await?.is_none(),
        "A leased predecessor must prevent concurrent delivery to this recipient"
    );
    let payload: Value = serde_json::from_str(string(&first, "payload")?)?;
    let response = transport::request_with_client(
        &state,
        Some(&user),
        reqwest::Method::POST,
        &url,
        Some(&payload),
        &client,
    )
    .await;
    assert_eq!(response.as_ref().unwrap().status(), 503);
    complete_delivery(&state, &first, response).await?;
    assert!(
        claim_delivery(&state).await?.is_none(),
        "Backoff must also hold subsequent activities"
    );
    toasty::sql::statement("UPDATE apub_deliveries SET next_attempt_at=now() WHERE id=$1")
        .bind(string(&first, "id")?)
        .exec(&mut db)
        .await?;
    let retry = claim_delivery(&state).await?.unwrap();
    assert_eq!(retry["id"], first["id"]);
    assert_eq!(retry["attempts"], 2);
    let response = transport::request_with_client(
        &state,
        Some(&user),
        reqwest::Method::POST,
        &url,
        Some(&payload),
        &client,
    )
    .await;
    assert_eq!(response.as_ref().unwrap().status(), 202);
    complete_delivery(&state, &retry, response).await?;
    let next = claim_delivery(&state).await?.unwrap();
    assert_ne!(next["id"], first["id"]);
    let mut tx = db.transaction().await?;
    let saved = one(
        &mut tx,
        "SELECT row_to_json(d)::text FROM apub_deliveries d WHERE id=$1",
        &[string(&first, "id")?],
    )
    .await?;
    drop(tx);
    assert!(!saved["delivered_at"].is_null());
    assert_eq!(saved["failed"], false);
    toasty::sql::statement("DELETE FROM users WHERE id=$1")
        .bind(user)
        .exec(&mut db)
        .await?;
    toasty::sql::statement("DELETE FROM apub_actors WHERE id=$1")
        .bind(remote)
        .exec(&mut db)
        .await?;
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    Ok(())
}
#[test]
#[ignore = "requires ACTIVITYPUB_BASE_URL"]
fn addressing_respects_each_visibility() -> anyhow::Result<()> {
    let mut p = json!({"id":"post","user_id":"alice","content":"<script>x</script>","visibility":"public","created_at":"2026-01-01T00:00:00Z","updated_at":null});
    let public = note_value(&p)?;
    assert_eq!(public["to"], json!([PUBLIC]));
    assert!(!public["content"].as_str().unwrap().contains("<script>"));
    p["visibility"] = json!("unlisted");
    let unlisted = note_value(&p)?;
    assert_eq!(unlisted["cc"], json!([PUBLIC]));
    assert_ne!(unlisted["to"], json!([PUBLIC]));
    p["visibility"] = json!("followers");
    let followers = note_value(&p)?;
    assert_eq!(followers["cc"], json!([]));
    assert_ne!(followers["to"], json!([PUBLIC]));
    p["visibility"] = json!("local");
    assert!(note_value(&p).is_err());
    Ok(())
}

#[test]
#[ignore = "requires ACTIVITYPUB_BASE_URL"]
fn protocol_wire_snapshot_matrix_preserves_addressing_identity_and_tombstones() -> anyhow::Result<()>
{
    let base = base_url()?;
    for visibility in ["public", "unlisted", "followers"] {
        let post = json!({"id":"protocol-post","user_id":"protocol-author","content":"<script>unsafe</script>\nA & B","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","visibility":visibility});
        let note = note_value(&post)?;
        assert_eq!(note["type"], "Note");
        assert_eq!(
            note["attributedTo"],
            format!("{base}/api/activitypub/users/protocol-author")
        );
        assert!(!note["content"].as_str().unwrap().contains("<script>"));
        assert!(note["content"].as_str().unwrap().contains("<br>"));
        note["published"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()?;
        for kind in ["Create", "Update", "Delete"] {
            let payload = json!({"type":kind,"activity_id":"stable-activity","zoo_post":post});
            let activity = delivery_activity(&payload)?;
            assert_eq!(
                delivery_activity(&payload)?,
                activity,
                "retry changed activity identity"
            );
            assert_eq!(activity["@context"], CONTEXT);
            assert_eq!(activity["type"], kind);
            assert_eq!(
                activity["id"],
                format!("{base}/api/activitypub/activities/stable-activity")
            );
            assert_eq!(activity["actor"], note["attributedTo"]);
            assert_eq!(activity["to"], note["to"]);
            assert_eq!(activity["cc"], note["cc"]);
            assert_eq!(activity["object"]["id"], note["id"]);
            if kind == "Delete" {
                assert_eq!(activity["object"]["type"], "Tombstone");
                assert!(activity["object"].get("content").is_none());
            } else {
                assert_eq!(activity["object"], note);
            }
            if visibility == "followers" {
                assert!(!activity.to_string().contains(PUBLIC));
            }
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires migrated PostgreSQL and ACTIVITYPUB_BASE_URL"]
async fn protocol_signed_interaction_wire_roundtrip() -> anyhow::Result<()> {
    let config = crate::config::Config::from_env();
    let mut db = crate::db::init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let user = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
    let key = keys(&state, &user).await?;
    let local = actor_id(&user)?;
    let remote = "https://peer.example/users/remote";
    let follow = json!({"id":"https://peer.example/follows/1","type":"Follow","actor":remote,"object":local});
    let like = json!({"@context":CONTEXT,"id":format!("{local}/likes/1"),"type":"Like","actor":local,"object":"https://peer.example/notes/1","to":[remote]});
    let mut activities = vec![
        json!({"@context":CONTEXT,"id":format!("{local}/accept/1"),"type":"Accept","actor":local,"object":follow,"to":[remote]}),
        json!({"@context":CONTEXT,"id":format!("{local}/reject/1"),"type":"Reject","actor":local,"object":follow,"to":[remote]}),
        like.clone(),
        json!({"@context":CONTEXT,"id":format!("{local}/undo/1"),"type":"Undo","actor":local,"object":like,"to":[remote]}),
    ];
    let custom = json!({"@context":[CONTEXT,{"Emoji":"http://joinmastodon.org/ns#Emoji","_misskey_reaction":"https://misskey-hub.net/ns#_misskey_reaction"}],"id":format!("{local}/custom-like/1"),"type":"Like","actor":local,"object":"https://peer.example/notes/1","content":":blobcat:","_misskey_reaction":":blobcat:","tag":[{"type":"Emoji","id":"https://peer.example/emojis/blobcat","name":":blobcat:","icon":{"type":"Image","url":"https://cdn.peer.example/blobcat.png"}}],"to":[remote]});
    activities.push(custom.clone());
    activities.push(json!({"@context":CONTEXT,"id":format!("{local}/custom-undo/1"),"type":"Undo","actor":local,"object":custom,"to":[remote]}));
    let server = MockServer::start().await;
    let verifier = json!({"key_id":format!("{local}#main-key"),"public_key":key["public_key"]});
    let expected = activities.clone();
    Mock::given(method("POST"))
        .and(path("/inbox"))
        .respond_with(move |request: &wiremock::Request| {
            assert_eq!(request.headers.get("content-type").unwrap(), CONTENT_TYPE);
            transport::verify_headers(&request.headers, "/inbox", &request.body, &verifier)
                .unwrap();
            let activity: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(
                expected.contains(&activity),
                "wire activity differs from the durable original"
            );
            ResponseTemplate::new(202)
        })
        .expect(6)
        .mount(&server)
        .await;
    let url = reqwest::Url::parse(&format!("{}/inbox", server.uri()))?;
    for activity in activities {
        let response = transport::request_with_client(
            &state,
            Some(&user),
            reqwest::Method::POST,
            &url,
            Some(&activity),
            &reqwest::Client::new(),
        )
        .await?;
        assert_eq!(response.status(), 202);
    }
    toasty::sql::statement("DELETE FROM users WHERE id=$1")
        .bind(user)
        .exec(&mut db)
        .await?;
    Ok(())
}

#[test]
fn misskey_tags_keep_shortcodes_and_reject_unsafe_or_invalid_image_metadata() {
    let tag = json!({"type":"Emoji","name":":blobcat:","id":"https://misskey.example/emojis/blobcat","icon":{"url":"https://cdn.example/blobcat.png"}});
    let parsed = emoji::tags(&json!([tag]));
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["image_id"].as_str().unwrap().len(), 64);
    let activity =
        json!({"type":"Like","_misskey_reaction":":blobcat:","content":":blobcat:","tag":[tag]});
    let (name, descriptor) = emoji::reaction(&activity).unwrap();
    assert_eq!(name, ":blobcat:");
    assert_eq!(descriptor.unwrap()["url"], tag["icon"]["url"]);
    assert_eq!(
        emoji::plain_content("<p>Hello <img src=\"https://cdn.example/x\" alt=\":blobcat:\"></p>"),
        "Hello :blobcat:\n"
    );
    for url in [
        "javascript:alert(1)",
        "data:image/svg+xml,evil",
        "https://user:password@example.com/image",
        "http://example.com/image",
    ] {
        let mut bad = tag.clone();
        bad["icon"]["url"] = json!(url);
        assert!(emoji::tags(&json!([bad])).is_empty());
    }
    let mut invalid = activity.clone();
    invalid["tag"] = json!([]);
    assert!(emoji::reaction(&invalid).is_err());
    assert_eq!(
        emoji::reaction(&json!({"type":"Like","content":"A favourite, not an emoji extension"}))
            .unwrap()
            .0,
        "❤️"
    );
    assert_eq!(
        emoji::reaction(&json!({"_misskey_reaction":"👍"}))
            .unwrap()
            .0,
        "👍"
    );
}

#[test]
fn public_announces_never_trust_third_party_identity_or_private_note_audiences() {
    let object = "https://peer.example/notes/original";
    let valid = json!({"id":object,"type":"Note","attributedTo":"https://peer.example/users/author","content":"Public original","to":[PUBLIC],"published":"2026-01-01T00:00:00Z","updated":"2026-01-02T00:00:00Z"});
    let (_, visibility, published, updated) =
        reposts::validate_public_note(&valid, object).unwrap();
    assert_eq!(visibility, "public");
    assert!(updated > published);
    let mut private = valid.clone();
    private["to"] = json!(["https://peer.example/users/author/followers"]);
    assert!(reposts::validate_public_note(&private, object).is_err());
    private["cc"] = json!([PUBLIC]);
    assert_eq!(
        reposts::validate_public_note(&private, object).unwrap().1,
        "unlisted"
    );
    let mut forged = valid.clone();
    forged["attributedTo"] = json!("https://impostor.example/actor");
    assert!(reposts::validate_public_note(&forged, object).is_err());
    assert!(reposts::validate_public_note(&valid, "https://peer.example/notes/different").is_err());
    let mut future = valid.clone();
    future["updated"] =
        json!((jiff::Timestamp::now() + jiff::SignedDuration::from_secs(600)).to_string());
    assert!(reposts::validate_public_note(&future, object).is_err());
}

#[tokio::test]
#[ignore = "requires migrated PostgreSQL and ACTIVITYPUB_BASE_URL"]
async fn actor_and_emoji_registration_requires_a_valid_inbox_signature() -> anyhow::Result<()> {
    let config = crate::config::Config::from_env();
    let mut db = crate::db::init_db(&config.database_url).await?;
    let state = AppState::new(db.clone(), config);
    let user = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
    let key = keys(&state, &user).await?;
    let remote = format!("https://remote.example/users/{}", uuid::Uuid::new_v4());
    let emojis = emoji::tags(
        &json!([{"type":"Emoji","name":":test:","icon":{"url":format!("https://cdn.remote.example/{}.png",uuid::Uuid::new_v4())}}]),
    );
    let candidate = json!({"id":remote,"inbox":format!("{remote}/inbox"),"followers_url":format!("{remote}/followers"),"user_name":"remote","display_name":"Remote","key_id":format!("{remote}#main-key"),"public_key":key["public_key"],"emojis":emojis});
    let body = b"{}";
    let url = reqwest::Url::parse("https://zoo.example.com/inbox")?;
    let signed = transport::sign_headers(
        "POST",
        &url,
        Some(body),
        &remote,
        string(&key, "private_key")?,
    )?;
    let headers: axum::http::HeaderMap = signed
        .iter()
        .map(|(k, v)| {
            Ok((
                axum::http::HeaderName::from_bytes(k.as_bytes())?,
                axum::http::HeaderValue::from_str(v)?,
            ))
        })
        .collect::<Result<_, anyhow::Error>>()?;
    assert!(
        inbox::verify_and_store_actor(&state, &headers, "/inbox", b"tampered", &candidate, true)
            .await
            .is_err()
    );
    let mut tx = db.transaction().await?;
    assert!(
        query(
            &mut tx,
            "SELECT '{}'::text FROM apub_actors WHERE id=$1",
            &[&remote]
        )
        .await?
        .is_empty()
    );
    assert!(
        query(
            &mut tx,
            "SELECT '{}'::text FROM apub_emoji_images WHERE id=$1",
            &[string(&emojis[0], "image_id")?]
        )
        .await?
        .is_empty()
    );
    drop(tx);
    inbox::verify_and_store_actor(&state, &headers, "/inbox", body, &candidate, true).await?;
    let mut tx = db.transaction().await?;
    assert_eq!(
        query(
            &mut tx,
            "SELECT '{}'::text FROM apub_actors WHERE id=$1",
            &[&remote]
        )
        .await?
        .len(),
        1
    );
    assert_eq!(
        query(
            &mut tx,
            "SELECT '{}'::text FROM apub_emoji_images WHERE id=$1",
            &[string(&emojis[0], "image_id")?]
        )
        .await?
        .len(),
        1
    );
    toasty::sql::statement(
        "UPDATE apub_emoji_images SET last_seen_at=now()-interval '31 days' WHERE id=$1",
    )
    .bind(string(&emojis[0], "image_id")?)
    .exec(&mut tx)
    .await?;
    // Viewing an authorized post renews its emoji instead of expiring a still-used URL.
    remember_display_emojis(&mut tx, &[json!({"emojis":emojis})]).await?;
    assert_eq!(query(&mut tx,"SELECT '{}'::text FROM apub_emoji_images WHERE id=$1 AND last_seen_at>now()-interval '1 day'",&[string(&emojis[0],"image_id")?]).await?.len(),1);
    toasty::sql::statement(
        "UPDATE apub_emoji_images SET last_seen_at=now()-interval '31 days' WHERE id=$1",
    )
    .bind(string(&emojis[0], "image_id")?)
    .exec(&mut tx)
    .await?;
    emoji::remember(&mut tx, &[]).await?;
    assert!(
        query(
            &mut tx,
            "SELECT '{}'::text FROM apub_emoji_images WHERE id=$1",
            &[string(&emojis[0], "image_id")?]
        )
        .await?
        .is_empty()
    );
    tx.commit().await?;
    toasty::sql::statement("DELETE FROM users WHERE id=$1")
        .bind(user)
        .exec(&mut db)
        .await?;
    Ok(())
}
