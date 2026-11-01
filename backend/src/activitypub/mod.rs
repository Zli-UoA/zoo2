//! Zoo federation: durable delivery, signed inboxes, local channel-linked posts.
mod emoji;
mod emoji_cache;
mod inbox;
pub(crate) mod interactions;
mod reposts;
mod routes;
mod transport;
use crate::{
    apps::{one, query},
    error::AppError,
    state::AppState,
};
pub use routes::router;
use serde_json::{Value, json};
use std::time::Duration;
pub const PUBLIC: &str = "https://www.w3.org/ns/activitystreams#Public";
pub const CONTEXT: &str = "https://www.w3.org/ns/activitystreams";
pub const CONTENT_TYPE: &str = "application/activity+json";
fn internal(e: impl Into<anyhow::Error>) -> AppError {
    AppError::Internal(e.into())
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, AppError> {
    v[key]
        .as_str()
        .ok_or_else(|| AppError::BadRequest(format!("Missing or invalid {key}")))
}
pub fn base_url() -> Result<String, AppError> {
    let raw = std::env::var("ACTIVITYPUB_BASE_URL")
        .map_err(|_| AppError::NotFound("Federation is not configured".into()))?;
    if raw.is_empty() {
        return Err(AppError::NotFound("Federation is not configured".into()));
    }
    let url = reqwest::Url::parse(&raw).map_err(internal)?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || (url.scheme() == "http"
            && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        return Err(AppError::BadRequest("ACTIVITYPUB_BASE_URL must be a canonical HTTPS origin (HTTP loopback only for development)".into()));
    }
    if url.scheme() == "http" && std::env::var("ENVIRONMENT").as_deref() == Ok("production") {
        return Err(AppError::BadRequest(
            "Federation requires HTTPS in production".into(),
        ));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
pub fn actor_id(user: &str) -> Result<String, AppError> {
    Ok(format!("{}/api/activitypub/users/{user}", base_url()?))
}
pub async fn keys(state: &AppState, user: &str) -> Result<Value, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('apub-key:' || $1,0))) locked",&[user]).await?;
    let found = query(
        &mut tx,
        "SELECT row_to_json(k)::text FROM apub_keys k WHERE user_id=$1",
        &[user],
    )
    .await?;
    if let Some(key) = found.into_iter().next() {
        return Ok(key);
    }
    let (private, public) = tokio::task::spawn_blocking(|| -> Result<_, AppError> {
        let rsa = openssl::rsa::Rsa::generate(2048).map_err(internal)?;
        let key = openssl::pkey::PKey::from_rsa(rsa).map_err(internal)?;
        Ok((
            String::from_utf8(key.private_key_to_pem_pkcs8().map_err(internal)?)
                .map_err(internal)?,
            String::from_utf8(key.public_key_to_pem().map_err(internal)?).map_err(internal)?,
        ))
    })
    .await
    .map_err(internal)??;
    toasty::sql::statement("INSERT INTO apub_keys(user_id,private_key,public_key) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(user).bind(private).bind(public).exec(&mut tx).await?;
    let key = one(
        &mut tx,
        "SELECT row_to_json(k)::text FROM apub_keys k WHERE user_id=$1",
        &[user],
    )
    .await?;
    tx.commit().await?;
    Ok(key)
}
pub fn note_value(post: &Value) -> Result<Value, AppError> {
    let id = string(post, "id")?;
    let author = actor_id(string(post, "user_id")?)?;
    let followers = format!("{author}/followers");
    let visibility = string(post, "visibility")?;
    let (to, cc) = match visibility {
        "public" => (vec![PUBLIC], vec![followers.as_str()]),
        "unlisted" => (vec![followers.as_str()], vec![PUBLIC]),
        "followers" => (vec![followers.as_str()], vec![]),
        _ => return Err(AppError::NotFound("Local post".into())),
    };
    let content = html_escape::encode_safe(string(post, "content")?).replace('\n', "<br>");
    let mut note = json!({"@context":CONTEXT,"type":"Note","id":format!("{}/api/activitypub/notes/{id}",base_url()?),"attributedTo":author,"published":post["created_at"],"updated":post["updated_at"],"content":content,"mediaType":"text/html","likes":format!("{}/api/activitypub/notes/{id}/likes",base_url()?),"to":to,"cc":cc});
    if let Some(parent) = post["parent_uri"].as_str() {
        note["inReplyTo"] = json!(parent);
    }
    if let Some(recipient) = post["reply_actor"].as_str() {
        let cc = note["cc"].as_array_mut().unwrap();
        if !cc.iter().any(|v| v == recipient) {
            cc.push(json!(recipient));
        }
        note["tag"] = json!([{"type":"Mention","href":recipient,"name":recipient}]);
    }
    Ok(note)
}
fn delivery_activity(payload: &Value) -> Result<Value, AppError> {
    if payload.get("zoo_post").is_none() {
        return Ok(payload.clone());
    }
    let p = &payload["zoo_post"];
    let mut note = note_value(p)?;
    if payload["type"] == "Delete" {
        note = json!({"id":note["id"],"type":"Tombstone"});
    }
    let audience = note_value(p)?;
    Ok(
        json!({"@context":CONTEXT,"id":format!("{}/api/activitypub/activities/{}",base_url()?,string(payload,"activity_id")?),"type":payload["type"],"actor":actor_id(string(p,"user_id")?)?,"object":note,"to":audience["to"],"cc":audience["cc"]}),
    )
}
async fn queue(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    actor: &str,
    payload: &Value,
) -> Result<(), AppError> {
    toasty::sql::statement(
        "INSERT INTO apub_deliveries(id,user_id,actor_id,payload) VALUES($1,$2,$3,$4)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(user)
    .bind(actor)
    .bind(payload.to_string())
    .exec(tx)
    .await?;
    Ok(())
}
fn same_origin(a: &str, b: &str) -> bool {
    match (reqwest::Url::parse(a), reqwest::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}
async fn load_remote_actor(
    state: &AppState,
    user: Option<&str>,
    id: &str,
    force: bool,
) -> Result<(Value, bool), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"SELECT row_to_json(a)::text FROM apub_actors a WHERE id=$1 AND fetched_at>now()-interval '5 minutes'",&[id]).await?;
    if !force && let Some(a) = rows.into_iter().next() {
        if a["blocked"] == true {
            return Err(AppError::Forbidden("Actor is blocked".into()));
        }
        return Ok((a, false));
    }
    drop(tx);
    if same_origin(id, &base_url()?) {
        return Err(AppError::BadRequest("Use a remote actor".into()));
    }
    let doc = transport::fetch_json(state, user, id).await?;
    if string(&doc, "id")? != id
        || !matches!(
            doc["type"].as_str(),
            Some("Person" | "Service" | "Application" | "Group")
        )
        || doc["publicKey"]["owner"] != id
    {
        return Err(AppError::BadRequest(
            "Remote actor identity mismatch".into(),
        ));
    }
    let key = &doc["publicKey"];
    let key_id = string(key, "id")?;
    let pem = string(key, "publicKeyPem")?;
    let rsa = openssl::pkey::PKey::public_key_from_pem(pem.as_bytes()).and_then(|k| k.rsa());
    if !same_origin(key_id, id)
        || pem.len() > 16384
        || !rsa.is_ok_and(|k| (2048..=8192).contains(&(k.size() * 8)))
    {
        return Err(AppError::BadRequest("Invalid actor public key".into()));
    }
    let inbox = string(&doc, "inbox")?;
    if !same_origin(inbox, id) {
        return Err(AppError::BadRequest(
            "Actor inbox must share its origin".into(),
        ));
    }
    // Inbox URLs are validated again at delivery time, including DNS pinning.
    let username = doc["preferredUsername"].as_str().unwrap_or(id);
    let display = doc["name"].as_str().unwrap_or(username);
    let followers = string(&doc, "followers")?;
    if !same_origin(followers, id) {
        return Err(AppError::BadRequest("Invalid followers collection".into()));
    }
    Ok((
        json!({"id":id,"inbox":inbox,"followers_url":followers,"user_name":username,"display_name":display,"key_id":key_id,"public_key":pem,"emojis":emoji::tags(&doc["tag"])}),
        true,
    ))
}
async fn save_remote_actor(state: &AppState, candidate: &Value) -> Result<Value, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let actor_emojis = candidate["emojis"].as_array().cloned().unwrap_or_default();
    emoji::remember(&mut tx, &actor_emojis).await?;
    let actor=one(&mut tx,"INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key,emojis) VALUES($1,$2,$3,$4,$5,$6,$7,$8::jsonb) ON CONFLICT(id) DO UPDATE SET inbox=EXCLUDED.inbox,followers_url=EXCLUDED.followers_url,user_name=EXCLUDED.user_name,display_name=EXCLUDED.display_name,key_id=EXCLUDED.key_id,public_key=EXCLUDED.public_key,emojis=EXCLUDED.emojis,fetched_at=now() RETURNING row_to_json(apub_actors)::text",&[string(candidate,"id")?,string(candidate,"inbox")?,string(candidate,"followers_url")?,string(candidate,"user_name")?,string(candidate,"display_name")?,string(candidate,"key_id")?,string(candidate,"public_key")?,&json!(actor_emojis).to_string()]).await?;
    if actor["blocked"] == true {
        return Err(AppError::Forbidden("Actor is blocked".into()));
    }
    tx.commit().await?;
    Ok(actor)
}
async fn remote_actor(
    state: &AppState,
    user: Option<&str>,
    id: &str,
    force: bool,
) -> Result<Value, AppError> {
    let (candidate, fetched) = load_remote_actor(state, user, id, force).await?;
    if fetched {
        save_remote_actor(state, &candidate).await
    } else {
        Ok(candidate)
    }
}
pub(crate) async fn remember_display_emojis(
    tx: &mut toasty::Transaction<'_>,
    items: &[Value],
) -> Result<(), AppError> {
    let mut emojis = vec![];
    for item in items {
        for key in ["emojis", "actor_emojis"] {
            if let Some(values) = item[key].as_array() {
                emojis.extend(values.iter().cloned());
            }
        }
        if item["emoji"].is_object() {
            emojis.push(item["emoji"].clone());
        }
        if let Some(reactions) = item["reactions"].as_array() {
            for reaction in reactions {
                if reaction["emoji"].is_object() {
                    emojis.push(reaction["emoji"].clone());
                }
            }
        }
    }
    // Tags are already normalized and authorized by the containing feed/notification query.
    // Multiple rows may refer to the same URL; avoid PostgreSQL updating it twice in one insert.
    emojis.sort_by_key(|v| v["image_id"].as_str().unwrap_or("").to_owned());
    emojis.dedup_by(|a, b| a["image_id"] == b["image_id"]);
    if !emojis.is_empty() {
        emoji::remember(tx, &emojis).await?;
    }
    Ok(())
}
pub async fn worker(state: AppState) {
    loop {
        match deliver_next(&state).await {
            Ok(true) => {}
            Ok(false) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(e) => {
                tracing::warn!(error=%e,"ActivityPub delivery worker failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
pub async fn deliver_next(state: &AppState) -> Result<bool, AppError> {
    if base_url().is_err() {
        return Ok(false);
    }
    let Some(delivery) = claim_delivery(state).await? else {
        return Ok(false);
    };
    let delivery = &delivery;
    let actor = string(delivery, "actor_id")?;
    let user = string(delivery, "user_id")?;
    let payload: Value = serde_json::from_str(string(delivery, "payload")?).map_err(internal)?;
    let result = async {
        let a = remote_actor(state, Some(user), actor, false).await?;
        let activity = delivery_activity(&payload)?;
        transport::request(
            state,
            Some(user),
            reqwest::Method::POST,
            string(&a, "inbox")?,
            Some(&activity),
        )
        .await
    }
    .await;
    complete_delivery(state, delivery, result).await?;
    Ok(true)
}
async fn claim_delivery(state: &AppState) -> Result<Option<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows=query(&mut tx,"UPDATE apub_deliveries SET lease_until=now()+interval '60 seconds',attempts=attempts+1 WHERE id=(SELECT d.id FROM apub_deliveries d WHERE delivered_at IS NULL AND NOT failed AND next_attempt_at<=now() AND (lease_until IS NULL OR lease_until<now()) AND NOT EXISTS(SELECT 1 FROM apub_deliveries earlier WHERE earlier.user_id=d.user_id AND earlier.actor_id=d.actor_id AND earlier.delivered_at IS NULL AND NOT earlier.failed AND earlier.sequence<d.sequence) ORDER BY sequence LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING row_to_json(apub_deliveries)::text",&[]).await?;
    tx.commit().await?;
    Ok(rows.into_iter().next())
}
async fn complete_delivery(
    state: &AppState,
    delivery: &Value,
    result: Result<reqwest::Response, AppError>,
) -> Result<(), AppError> {
    let id = string(delivery, "id")?;
    let status = result.as_ref().map(|r| r.status().as_u16()).unwrap_or(0);
    let success = (200..300).contains(&status);
    let terminal = delivery["attempts"].as_i64().unwrap_or(1) >= 10
        || matches!(status, 404 | 410)
        || matches!(
            result,
            Err(AppError::Forbidden(_) | AppError::BadRequest(_))
        );
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement("UPDATE apub_deliveries SET delivered_at=CASE WHEN $2::boolean THEN now() ELSE NULL END, failed=$3::boolean,last_status=$4::integer,lease_until=NULL,next_attempt_at=now()+make_interval(secs=>LEAST(86400,power(2,attempts)::integer*30)) WHERE id=$1 AND attempts=$5::integer")
        .bind(id).bind(success.to_string()).bind((!success && terminal).to_string()).bind(status.to_string()).bind(delivery["attempts"].as_i64().unwrap_or(1).to_string()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests;
