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
    if activity["type"] == "Announce" {
        reposts::resolve_announce(&state, &user, &activity).await?;
    }
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
    // Notes and reposts are shared across inbox recipients; serialize by the verified actor.
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('inbox-actor:' || $1,0))) locked", &[actor_id]).await?;
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
            toasty::sql::statement(
                "DELETE FROM apub_reposts WHERE actor_id=$1 AND activity_id=$2 AND user_id IS NULL",
            )
            .bind(actor_id)
            .bind(undone)
            .exec(&mut tx)
            .await?;
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
            one(&mut tx,"SELECT '{}'::text FROM home_posts p WHERE p.id=$1 AND p.user_id=$2 AND p.apub AND p.deleted_at IS NULL AND (p.visibility IN ('public','unlisted') OR p.reply_actor=$3 OR EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$2 AND actor_id=$3 AND direction='incoming' AND accepted))", &[post,user,actor_id]).await?;
            let undone=query(&mut tx,"SELECT '{}'::text FROM apub_undone_activities WHERE activity_id=$1 AND actor_id=$2 AND user_id=$3", &[id,actor_id,user]).await?;
            if undone.is_empty() {
                let (reaction, reaction_emoji) = emoji::reaction(activity)?;
                if let Some(e) = &reaction_emoji {
                    emoji::remember(&mut tx, std::slice::from_ref(e)).await?;
                }
                let inserted=query(&mut tx,"INSERT INTO apub_reactions(post_id,actor_id,activity_id,activity_json,reaction,emoji) VALUES($1,$2,$3,$4,$5,$6::jsonb) ON CONFLICT(post_id,actor_id) DO UPDATE SET activity_id=EXCLUDED.activity_id,activity_json=EXCLUDED.activity_json,reaction=EXCLUDED.reaction,emoji=EXCLUDED.emoji RETURNING json_build_object('inserted',xmax=0)::text", &[post,actor_id,id,&activity.to_string(),&reaction,&json!(reaction_emoji).to_string()]).await?;
                if inserted.first().is_some_and(|row| row["inserted"] == true) {
                    interactions::notify(&mut tx, user, actor_id, id, "like", post).await?;
                    toasty::sql::statement("UPDATE apub_notifications SET reaction=$2,emoji=$3::jsonb WHERE activity_id=$1 AND user_id=$4").bind(id).bind(reaction).bind(json!(reaction_emoji).to_string()).bind(user).exec(&mut tx).await?;
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
            let parent_uri = object["inReplyTo"]
                .as_str()
                .or_else(|| object["inReplyTo"]["id"].as_str());
            let prefix = format!("{}/api/activitypub/notes/", base_url()?);
            let parent_id = parent_uri.map(|uri| uri.strip_prefix(&prefix).unwrap_or(uri));
            let parents = if let Some(parent) = parent_id {
                query(&mut tx,"SELECT row_to_json(p)::text FROM home_posts p WHERE p.id=$1 AND p.user_id=$2 AND p.apub AND p.deleted_at IS NULL AND (p.visibility IN ('public','unlisted') OR p.reply_actor=$3 OR EXISTS(SELECT 1 FROM apub_follows WHERE user_id=$2 AND actor_id=$3 AND direction='incoming' AND accepted))",&[parent,user,actor_id]).await?
            } else {
                vec![]
            };
            let followed=query(&mut tx,"SELECT '{}'::text FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' AND accepted",&[user,actor_id]).await?;
            if followed.is_empty() && parents.is_empty() {
                return Err(AppError::Forbidden(
                    "Actor is not followed and Note is not an authorized reply".into(),
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
                || (!parents.is_empty()
                    && (to.contains(&local.as_str()) || cc.contains(&local.as_str())))
            {
                "followers"
            } else {
                return Err(AppError::BadRequest(
                    "Direct messages are not supported".into(),
                ));
            };
            if parents
                .first()
                .is_some_and(|p| p["visibility"] == "followers")
                && visibility != "followers"
            {
                return Err(AppError::BadRequest(
                    "Reply broadens private audience".into(),
                ));
            }
            let raw = string(object, "content")?;
            if raw.len() > 128 * 1024 {
                return Err(AppError::BadRequest("Note too large".into()));
            }
            let content = emoji::plain_content(raw);
            let note_emojis = emoji::tags(&object["tag"]);
            emoji::remember(&mut tx, &note_emojis).await?;
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
                toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,parent_id,emojis) VALUES($1,$2,$3,$4,$5::timestamptz,$6::timestamptz,NULLIF($7,''),$8::jsonb) ON CONFLICT DO NOTHING").bind(post_id).bind(actor_id).bind(content).bind(visibility).bind(published.to_string()).bind(updated.to_string()).bind(parent_id.unwrap_or("")).bind(json!(note_emojis).to_string()).exec(&mut tx).await?;
            } else {
                toasty::sql::statement("UPDATE apub_remote_posts SET content=$3,visibility=$4,updated_at=$5::timestamptz,parent_id=NULLIF($6,''),emojis=$7::jsonb WHERE id=$1 AND actor_id=$2 AND deleted_at IS NULL AND updated_at<$5::timestamptz").bind(post_id).bind(actor_id).bind(content).bind(visibility).bind(updated.to_string()).bind(parent_id.unwrap_or("")).bind(json!(note_emojis).to_string()).exec(&mut tx).await?;
            }
            if activity["type"] == "Create" && !parents.is_empty() {
                interactions::notify(&mut tx, user, actor_id, id, "reply", post_id).await?;
            }
            // Per-recipient inboxes preserve audience isolation, even for followers-only notes.
            toasty::sql::statement("INSERT INTO apub_remote_recipients(post_id,user_id) SELECT id,$2 FROM apub_remote_posts WHERE id=$1 AND actor_id=$3 ON CONFLICT DO NOTHING").bind(post_id).bind(user).bind(actor_id).exec(&mut tx).await?;
        }
        "Announce" => {
            let original = object_id(&activity["object"])
                .ok_or_else(|| AppError::BadRequest("Missing Announce object".into()))?;
            let prefix = format!("{}/api/activitypub/notes/", base_url()?);
            let post_id = original.strip_prefix(&prefix).unwrap_or(original);
            let target=one(&mut tx,"SELECT json_build_object('author',p.user_id,'local',true)::text FROM home_posts p WHERE p.id=$1 AND p.apub AND p.deleted_at IS NULL AND p.visibility IN ('public','unlisted') UNION ALL SELECT json_build_object('author',p.actor_id,'local',false)::text FROM apub_remote_posts p JOIN apub_actors a ON a.id=p.actor_id WHERE p.id=$1 AND p.deleted_at IS NULL AND NOT a.blocked AND p.visibility IN ('public','unlisted')",&[post_id]).await?;
            let following=query(&mut tx,"SELECT '{}'::text FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' AND accepted",&[user,actor_id]).await?;
            if following.is_empty() && !(target["local"] == true && target["author"] == user) {
                return Err(AppError::Forbidden("Unsolicited Announce".into()));
            }
            let to = audience(activity, "to");
            let cc = audience(activity, "cc");
            let visibility = if to.contains(&PUBLIC) {
                "public"
            } else if cc.contains(&PUBLIC) {
                "unlisted"
            } else if to.contains(&string(actor, "followers_url")?) {
                "followers"
            } else {
                return Err(AppError::BadRequest("Unsupported Announce audience".into()));
            };
            let undone=query(&mut tx,"SELECT '{}'::text FROM apub_undone_activities WHERE activity_id=$1 AND actor_id=$2",&[id,actor_id]).await?;
            if undone.is_empty() {
                toasty::sql::statement(
                    "DELETE FROM apub_reposts WHERE post_id=$1 AND actor_id=$2 AND activity_id<>$3",
                )
                .bind(post_id)
                .bind(actor_id)
                .bind(id)
                .exec(&mut tx)
                .await?;
                let inserted=query(&mut tx,"INSERT INTO apub_reposts(post_id,actor_id,activity_id,activity_json,visibility) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING RETURNING '{}'::text",&[post_id,actor_id,id,&activity.to_string(),visibility]).await?;
                toasty::sql::statement("INSERT INTO apub_repost_recipients(activity_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(id).bind(user).exec(&mut tx).await?;
                if !inserted.is_empty() && target["local"] == true && target["author"] == user {
                    interactions::notify(&mut tx, user, actor_id, id, "repost", post_id).await?;
                }
            }
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
