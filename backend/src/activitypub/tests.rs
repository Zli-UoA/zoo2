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
