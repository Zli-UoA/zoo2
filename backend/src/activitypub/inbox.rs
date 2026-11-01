use super::*;
use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, StatusCode},
};
static INBOX_CONCURRENCY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

pub async fn receive(
    State(state): State<AppState>,
    Path(user): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let _permit = INBOX_CONCURRENCY
        .try_acquire()
        .map_err(|_| AppError::RateLimited)?;
    base_url()?;
    let activity: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Invalid activity".into()))?;
    let actor = string(&activity, "actor")?;
    let id = string(&activity, "id")?;
    if actor.len() > 2048 || id.len() > 2048 || !same_origin(id, actor) {
        return Err(AppError::BadRequest("Invalid activity identity".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    one(
        &mut tx,
        "SELECT json_build_object('id',id)::text FROM users WHERE id=$1 AND NOT is_bot",
        &[&user],
    )
    .await?;
    drop(tx);
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path());
    transport::validate_envelope(&headers, target, &body)?;
    let remote = remote_actor(&state, Some(&user), actor, false).await?;
    transport::verify_headers(&headers, target, &body, &remote)?;
    receive_verified(&state, &user, &remote, &activity).await?;
    Ok(StatusCode::ACCEPTED)
}
fn object_id(v: &Value) -> Option<&str> {
    v.as_str().or_else(|| v["id"].as_str())
}
fn audience<'a>(v: &'a Value, key: &str) -> Vec<&'a str> {
    match &v[key] {
        Value::String(s) => vec![s],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    }
}

pub(super) async fn receive_verified(
    state: &AppState,
    user: &str,
    actor: &Value,
    activity: &Value,
) -> Result<(), AppError> {
    let actor_id = string(actor, "id")?;
    let local = super::actor_id(user)?;
    let id = string(activity, "id")?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // Serialize relationship changes and Undo tombstones for this actor and inbox.
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended($1 || ':' || $2,0))) locked", &[user,actor_id]).await?;
    // The receipt and all effects commit together. Replays cannot repeat delivery or resurrect posts.
    let inserted=query(&mut tx,"INSERT INTO apub_inbox_receipts(id,actor_id,user_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING RETURNING '{}'::text",&[id,actor_id,user]).await?;
    if inserted.is_empty() {
        return Ok(());
    }
    match string(activity, "type")? {
        "Follow" => {
            if object_id(&activity["object"]) != Some(local.as_str()) {
                return Err(AppError::BadRequest("Wrong follow target".into()));
            }
            let undone=query(&mut tx,"SELECT '{}'::text FROM apub_undone_activities WHERE activity_id=$1 AND actor_id=$2 AND user_id=$3", &[id,actor_id,user]).await?;
            if undone.is_empty() {
                let relation=one(&mut tx,"INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,activity_json) VALUES($1,$2,'incoming',$3,$4) ON CONFLICT(user_id,actor_id,direction) DO UPDATE SET activity_id=EXCLUDED.activity_id,activity_json=EXCLUDED.activity_json RETURNING row_to_json(apub_follows)::text", &[user,actor_id,id,&activity.to_string()]).await?;
                if relation["accepted"] == true {
                    let accept = json!({"@context":CONTEXT,"id":format!("{}/api/activitypub/activities/{}",base_url()?,uuid::Uuid::new_v4()),"type":"Accept","actor":local,"object":activity,"to":[actor_id]});
                    queue(&mut tx, user, actor_id, &accept).await?;
                } else {
                    interactions::notify(&mut tx, user, actor_id, id, "follow_request", "").await?;
                }
            }
        }
        "Accept" | "Reject" => {
            let followed = object_id(&activity["object"])
                .ok_or_else(|| AppError::BadRequest("Invalid follow response".into()))?;
            if activity["object"].is_object()
                && (activity["object"]["type"] != "Follow"
                    || object_id(&activity["object"]["actor"]) != Some(local.as_str())
                    || object_id(&activity["object"]["object"]) != Some(actor_id))
            {
                return Err(AppError::BadRequest("Invalid embedded Follow".into()));
            }
            if activity["type"] == "Accept" {
                let changed=query(&mut tx,"UPDATE apub_follows SET accepted=true WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' AND activity_id=$3 AND NOT accepted RETURNING '{}'::text",&[user,actor_id,followed]).await?;
                if !changed.is_empty() {
                    interactions::notify(&mut tx, user, actor_id, id, "follow_accepted", "")
                        .await?;
                }
            } else {
                toasty::sql::statement("DELETE FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' AND activity_id=$3").bind(user).bind(actor_id).bind(followed).exec(&mut tx).await?;
            }
        }
        "Undo" => {
            let object = &activity["object"];
            let undone = object_id(object)
                .ok_or_else(|| AppError::BadRequest("Missing Undo object".into()))?;
            if undone.len() > 2048
                || !same_origin(undone, actor_id)
                || (object.is_object() && object_id(&object["actor"]) != Some(actor_id))
            {
                return Err(AppError::BadRequest(
                    "Undo actor does not own activity".into(),
                ));
            }
            toasty::sql::statement("INSERT INTO apub_undone_activities(activity_id,actor_id,user_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(undone).bind(actor_id).bind(user).exec(&mut tx).await?;
            toasty::sql::statement("DELETE FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='incoming' AND activity_id=$3").bind(user).bind(actor_id).bind(undone).exec(&mut tx).await?;
            toasty::sql::statement("DELETE FROM apub_reactions r USING home_posts p WHERE r.activity_id=$1 AND r.actor_id=$2 AND r.user_id IS NULL AND r.post_id=p.id AND p.user_id=$3").bind(undone).bind(actor_id).bind(user).exec(&mut tx).await?;
            toasty::sql::statement("UPDATE apub_notifications SET is_read=true WHERE user_id=$1 AND actor_id=$2 AND activity_id=$3").bind(user).bind(actor_id).bind(undone).exec(&mut tx).await?;
        }
        "Like" => {
            let object = object_id(&activity["object"])
                .ok_or_else(|| AppError::BadRequest("Missing Like object".into()))?;
            let prefix = format!("{}/api/activitypub/notes/", base_url()?);
            let post = object
                .strip_prefix(&prefix)
                .filter(|id| !id.is_empty() && !id.contains(['/', '?', '#']))
                .ok_or_else(|| AppError::BadRequest("Unknown Like target".into()))?;
            one(&mut tx,"SELECT '{}'::text FROM home_posts p WHERE p.id=$1 AND p.user_id=$2 AND p.apub AND p.deleted_at IS NULL AND (p.visibility IN ('public','unlisted') OR EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$2 AND actor_id=$3 AND direction='incoming' AND accepted))", &[post,user,actor_id]).await?;
            let undone=query(&mut tx,"SELECT '{}'::text FROM apub_undone_activities WHERE activity_id=$1 AND actor_id=$2 AND user_id=$3", &[id,actor_id,user]).await?;
            if undone.is_empty() {
                let inserted=query(&mut tx,"INSERT INTO apub_reactions(post_id,actor_id,activity_id,activity_json) VALUES($1,$2,$3,$4) ON CONFLICT(post_id,actor_id) DO UPDATE SET activity_id=EXCLUDED.activity_id,activity_json=EXCLUDED.activity_json RETURNING json_build_object('inserted',xmax=0)::text", &[post,actor_id,id,&activity.to_string()]).await?;
                if inserted.first().is_some_and(|row| row["inserted"] == true) {
                    interactions::notify(&mut tx, user, actor_id, id, "like", post).await?;
                }
            }
        }
        "Create" | "Update" => {
            let object = &activity["object"];
            let post_id = string(object, "id")?;
            if object["type"] != "Note"
                || object_id(&object["attributedTo"]) != Some(actor_id)
                || !same_origin(post_id, actor_id)
                || post_id.len() > 2048
            {
                return Err(AppError::BadRequest("Invalid note attribution".into()));
            }
            let followed=query(&mut tx,"SELECT json_build_object('user_id',user_id)::text FROM apub_follows WHERE actor_id=$1 AND direction='outgoing' AND accepted",&[actor_id]).await?;
            if !followed.iter().any(|f| f["user_id"] == user) {
                return Err(AppError::Forbidden(
                    "Actor is not followed by inbox owner".into(),
                ));
            }
            let mut to = audience(object, "to");
            to.extend(audience(activity, "to"));
            let mut cc = audience(object, "cc");
            cc.extend(audience(activity, "cc"));
            let visibility = if to.contains(&PUBLIC) {
                "public"
            } else if cc.contains(&PUBLIC) {
                "unlisted"
            } else if to.contains(&string(actor, "followers_url")?)
                || cc.contains(&string(actor, "followers_url")?)
            {
                "followers"
            } else {
                return Err(AppError::BadRequest(
                    "Direct messages are not supported".into(),
                ));
            };
            let raw = string(object, "content")?;
            if raw.len() > 128 * 1024 {
                return Err(AppError::BadRequest("Note too large".into()));
            }
            let linebreaks = regex::Regex::new("(?i)<br\\s*/?>|</p>")
                .unwrap()
                .replace_all(raw, "\n");
            let stripped = regex::Regex::new("<[^>]*>")
                .unwrap()
                .replace_all(&linebreaks, "");
            let content = html_escape::decode_html_entities(&stripped).into_owned();
            let published = object["published"]
                .as_str()
                .unwrap_or("")
                .parse::<jiff::Timestamp>()
                .map_err(|_| AppError::BadRequest("Invalid published date".into()))?;
            let updated = object["updated"]
                .as_str()
                .map(str::parse::<jiff::Timestamp>)
                .transpose()
                .map_err(|_| AppError::BadRequest("Invalid updated date".into()))?
                .unwrap_or(published);
            if updated > jiff::Timestamp::now() + jiff::SignedDuration::from_secs(300) {
                return Err(AppError::BadRequest("Future note timestamp".into()));
            }
            if activity["type"] == "Create" {
                toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at) VALUES($1,$2,$3,$4,$5::timestamptz,$6::timestamptz) ON CONFLICT DO NOTHING").bind(post_id).bind(actor_id).bind(content).bind(visibility).bind(published.to_string()).bind(updated.to_string()).exec(&mut tx).await?;
            } else {
                toasty::sql::statement("UPDATE apub_remote_posts SET content=$3,visibility=$4,updated_at=$5::timestamptz WHERE id=$1 AND actor_id=$2 AND deleted_at IS NULL AND updated_at<$5::timestamptz").bind(post_id).bind(actor_id).bind(content).bind(visibility).bind(updated.to_string()).exec(&mut tx).await?;
            }
            // Per-recipient inboxes preserve audience isolation, even for followers-only notes.
            toasty::sql::statement("INSERT INTO apub_remote_recipients(post_id,user_id) SELECT id,$2 FROM apub_remote_posts WHERE id=$1 AND actor_id=$3 ON CONFLICT DO NOTHING").bind(post_id).bind(user).bind(actor_id).exec(&mut tx).await?;
        }
        "Delete" => {
            let post = object_id(&activity["object"])
                .ok_or_else(|| AppError::BadRequest("Invalid delete object".into()))?;
            if !same_origin(post, actor_id) {
                return Err(AppError::BadRequest("Wrong delete origin".into()));
            }
            toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,deleted_at) VALUES($1,$2,'','public',now(),now(),now()) ON CONFLICT(id) DO UPDATE SET deleted_at=now() WHERE apub_remote_posts.actor_id=EXCLUDED.actor_id").bind(post).bind(actor_id).exec(&mut tx).await?;
        }
        _ => {}
    }
    tx.commit().await?;
    Ok(())
}
