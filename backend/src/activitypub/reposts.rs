use super::*;
use crate::auth::AuthUser;
use axum::{Json, Router, extract::State, routing::post};
use serde::Deserialize;
pub fn router() -> Router<AppState> {
    Router::new().route("/api/activitypub/reposts", post(create).delete(remove))
}
#[derive(Deserialize)]
struct Repost {
    post_id: String,
}
pub(super) async fn recipients(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    direct: Option<&str>,
) -> Result<Vec<Value>, AppError> {
    query(tx,"SELECT json_build_object('id',a.id)::text FROM apub_actors a WHERE NOT a.blocked AND (a.id=$2 OR EXISTS(SELECT 1 FROM apub_follows f WHERE f.user_id=$1 AND f.actor_id=a.id AND f.direction='incoming' AND f.accepted))",&[user,direct.unwrap_or("")]).await
}
// The remote post and quote snapshot are each selected by their primary key.
pub(super) async fn remote_repost_target(
    tx: &mut toasty::Transaction<'_>,
    id: &str,
    require_verified: bool,
) -> Result<Option<Value>, AppError> {
    let rows = query(tx, r#"
        SELECT json_build_object('author',p.actor_id,'visibility',p.visibility,'local',false)::text
        FROM apub_remote_posts p JOIN apub_actors a ON a.id=p.actor_id
        WHERE p.id=$1 AND p.deleted_at IS NULL AND NOT a.blocked AND p.visibility IN ('public','unlisted')
          AND (NOT $2::boolean OR p.origin_verified)
          AND NOT EXISTS(SELECT 1 FROM apub_quote_originals q WHERE q.id=p.id AND (
              (q.actor_id<>p.actor_id AND NOT p.origin_verified)
              OR (q.actor_id=p.actor_id AND (q.updated_at>p.updated_at
                OR (q.updated_at=p.updated_at AND NOT q.superseded_by_public)))
            ) OFFSET 0)
    "#, &[id, if require_verified { "true" } else { "false" }]).await?;
    Ok(rows.into_iter().next())
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Repost>,
) -> Result<Json<Value>, AppError> {
    let target = interactions::target(&state, &user, &body.post_id).await?;
    if !matches!(
        target["visibility"].as_str(),
        Some("public" | "unlisted" | "local")
    ) {
        return Err(AppError::BadRequest(
            "Private posts cannot be reposted".into(),
        ));
    }
    let federate = target["apub"] == true;
    if federate {
        keys(&state, &user.id).await?;
    }
    let local_actor = if federate {
        actor_id(&user.id)?
    } else {
        format!("local:{}", user.id)
    };
    let id = if federate {
        format!(
            "{}/api/activitypub/activities/{}",
            base_url()?,
            uuid::Uuid::new_v4()
        )
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let object = if target["remote"] == true {
        body.post_id.clone()
    } else if federate {
        format!("{}/api/activitypub/notes/{}", base_url()?, body.post_id)
    } else {
        body.post_id.clone()
    };
    let visibility = if target["visibility"] == "unlisted" {
        "unlisted"
    } else if federate {
        "public"
    } else {
        "local"
    };
    let (to, cc) = if visibility == "public" {
        (json!([PUBLIC]), json!([format!("{local_actor}/followers")]))
    } else {
        (
            json!([format!("{local_actor}/followers")]),
            if visibility == "unlisted" {
                json!([PUBLIC])
            } else {
                json!([])
            },
        )
    };
    let activity = json!({"@context":CONTEXT,"id":id,"type":"Announce","actor":local_actor,"object":object,"published":jiff::Timestamp::now().to_string(),"to":to,"cc":cc});
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if target["remote"] == true {
        note_identity::lock(&mut tx, &body.post_id).await?;
        let current = remote_repost_target(&mut tx, &body.post_id, false).await?;
        if !current.is_some_and(|post| {
            post["visibility"] == target["visibility"] && post["author"] == target["author"]
        }) {
            return Err(AppError::BadRequest(
                "The post is no longer available for reposting".into(),
            ));
        }
    }
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('repost:'||$1||':'||$2,0))) locked",&[&user.id,&body.post_id]).await?;
    let inserted=query(&mut tx,"INSERT INTO apub_reposts(post_id,actor_id,user_id,activity_id,activity_json,visibility) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING RETURNING '{}'::text",&[&body.post_id,&local_actor,&user.id,&id,&activity.to_string(),visibility]).await?;
    if !inserted.is_empty() {
        if federate {
            for peer in recipients(
                &mut tx,
                &user.id,
                if target["remote"] == true {
                    target["author"].as_str()
                } else {
                    None
                },
            )
            .await?
            {
                queue(&mut tx, &user.id, string(&peer, "id")?, &activity).await?;
            }
        }
        if target["remote"] != true && target["author"] != user.id {
            interactions::notify(
                &mut tx,
                string(&target, "author")?,
                &format!("local:{}", user.id),
                &id,
                "repost",
                &body.post_id,
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Repost>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('repost:'||$1||':'||$2,0))) locked",&[&user.id,&body.post_id]).await?;
    let rows=query(&mut tx,"DELETE FROM apub_reposts WHERE post_id=$1 AND user_id=$2 RETURNING row_to_json(apub_reposts)::text",&[&body.post_id,&user.id]).await?;
    for row in rows {
        let activity: Value =
            serde_json::from_str(string(&row, "activity_json")?).map_err(internal)?;
        if activity["actor"]
            .as_str()
            .is_some_and(|v| v.starts_with("https://") || v.starts_with("http://"))
        {
            let undo = json!({"@context":CONTEXT,"id":format!("{}/api/activitypub/activities/{}",base_url()?,uuid::Uuid::new_v4()),"type":"Undo","actor":activity["actor"],"object":activity,"to":activity["to"],"cc":activity["cc"]});
            // Include original delivery peers even if the follow was removed after the Announce.
            let peers=query(&mut tx,"SELECT DISTINCT json_build_object('id',actor_id)::text FROM apub_deliveries WHERE user_id=$1 AND payload::jsonb->>'id'=$2",&[&user.id,string(&activity,"id")?]).await?;
            for peer in peers {
                queue(&mut tx, &user.id, string(&peer, "id")?, &undo).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}

/// An Announce cannot vouch for a third party's embedded content. Resolve public originals at their HTTPS origin.
pub(super) async fn resolve_announce(
    state: &AppState,
    user: &str,
    activity: &Value,
) -> Result<(), AppError> {
    let object = activity["object"]
        .as_str()
        .or_else(|| activity["object"]["id"].as_str())
        .ok_or_else(|| AppError::BadRequest("Missing Announce object".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let prefix = format!("{}/api/activitypub/notes/", base_url()?);
    if object.starts_with(&prefix) {
        return Ok(());
    }
    if remote_repost_target(&mut tx, object, true).await?.is_some() {
        return Ok(());
    }
    one(&mut tx,"SELECT '{}'::text FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' AND accepted",&[user,string(activity,"actor")?]).await?;
    drop(tx);
    resolve_public_note(state, user, object).await
}

/// Fetch exactly one original from its origin; never trust third-party embedded content.
pub(super) async fn resolve_public_note(
    state: &AppState,
    user: &str,
    object: &str,
) -> Result<(), AppError> {
    let note = transport::fetch_json_with_retry(state, Some(user), object).await?;
    store_public_note(state, user, object, &note).await
}

pub(super) async fn store_public_note(
    state: &AppState,
    user: &str,
    object: &str,
    note: &Value,
) -> Result<(), AppError> {
    store_original(state, user, object, note, false).await
}

pub(super) async fn store_quoted_note(
    state: &AppState,
    user: &str,
    object: &str,
    note: &Value,
) -> Result<(), AppError> {
    store_original(state, user, object, note, true).await
}

async fn store_original(
    state: &AppState,
    user: &str,
    object: &str,
    note: &Value,
    allow_followers: bool,
) -> Result<(), AppError> {
    let (author, visibility, published, updated) =
        validate_original(note, object, allow_followers)?;
    let actor = remote_actor(state, Some(user), &author, false).await?;
    if visibility == "followers" {
        let followers = string(&actor, "followers_url")?;
        if followers.is_empty()
            || !["to", "cc"]
                .iter()
                .any(|key| inbox::audience(note, key).contains(&followers))
        {
            return Err(AppError::Forbidden("Unsupported quote audience".into()));
        }
    }
    let mut db = state.db.clone();
    let proof = note_identity::Proof::from_note(note, object)?;
    let emojis = emoji::tags(&note["tag"]);
    let media = emoji::note_media(note);
    let mut tx = db.transaction().await?;
    if visibility == "followers" {
        one(&mut tx, "SELECT '{}'::text FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' AND accepted", &[user, &author]).await?;
    }
    emoji::remember(&mut tx, &emojis).await?;
    emoji::remember(&mut tx, &media).await?;
    let content = emoji::note_content(note)?;
    let published = published.to_string();
    let updated = updated.to_string();
    let emojis = json!(emojis).to_string();
    let media_json = json!(media).to_string();
    let quote = emoji::note_quote_uri(note)
        .map(|url| url.to_string())
        .unwrap_or_default();
    let prefix = format!("{}/api/activitypub/notes/", base_url()?);
    let parent = inbox::note_parent(note, &prefix).unwrap_or("");
    if visibility == "followers" {
        note_identity::lock(&mut tx, object).await?;
        // No Inbox row or delivery is changed by a private fetch. Equal timestamps
        // only grant access when the signer received the exact cached payload.
        query(&mut tx, r#"
            WITH saved AS (
              INSERT INTO apub_quote_originals(id,actor_id,content,published,updated_at,parent_id,emojis,media)
              SELECT $1,$3,$5,$9::timestamptz,$4::timestamptz,NULLIF($8,''),$6::jsonb,$7::jsonb
              WHERE NOT EXISTS(SELECT 1 FROM apub_remote_tombstones WHERE id=$1 AND actor_id=$3)
                AND NOT EXISTS(SELECT 1 FROM apub_remote_posts p WHERE p.id=$1 AND p.actor_id=$3
                  AND (p.deleted_at IS NOT NULL OR p.updated_at>$4::timestamptz OR
                    (p.updated_at=$4::timestamptz AND (p.visibility='followers' AND
                      (p.content,p.published,p.parent_id,p.emojis) IS DISTINCT FROM ($5,$9::timestamptz,NULLIF($8,''),$6::jsonb)))))
              ON CONFLICT(id) DO UPDATE SET content=EXCLUDED.content, published=EXCLUDED.published, updated_at=EXCLUDED.updated_at,
                parent_id=EXCLUDED.parent_id, emojis=EXCLUDED.emojis, media=EXCLUDED.media,
                superseded=false, superseded_by_public=false, revision=apub_quote_originals.revision+1
              WHERE apub_quote_originals.actor_id=EXCLUDED.actor_id AND (
                apub_quote_originals.updated_at<EXCLUDED.updated_at OR
                (apub_quote_originals.updated_at=EXCLUDED.updated_at AND apub_quote_originals.superseded))
              RETURNING id,revision
            ), version AS (
              SELECT id,revision FROM saved
              UNION ALL
              SELECT q.id,q.revision FROM apub_quote_originals q
              WHERE NOT EXISTS(SELECT 1 FROM saved) AND q.id=$1 AND q.actor_id=$3 AND NOT q.superseded AND q.updated_at=$4::timestamptz
                AND (q.content,q.published,q.parent_id,q.emojis,q.media)
                  IS NOT DISTINCT FROM ($5,$9::timestamptz,NULLIF($8,''),$6::jsonb,$7::jsonb)
            )
            INSERT INTO apub_quote_grants(post_id,user_id,note_revision)
            SELECT id,$2,revision FROM version
            ON CONFLICT(post_id,user_id) DO UPDATE SET note_revision=EXCLUDED.note_revision
            WHERE apub_quote_grants.note_revision<EXCLUDED.note_revision
            RETURNING '{}'::text
        "#, &[object, user, &author, &updated, &content, &emojis, &media_json, parent, &published]).await?;
    } else {
        note_identity::reconcile(&mut tx, object, &author, Some(&proof)).await?;
        let saved = query(&mut tx, r#"
            INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,emojis,origin_verified,media,quote_uri,parent_id)
            SELECT $1,$2,$3,$4,$5::timestamptz,$6::timestamptz,$7::jsonb,true,$8::jsonb,NULLIF($9,''),NULLIF($10,'')
            WHERE NOT EXISTS(SELECT 1 FROM apub_remote_tombstones WHERE id=$1 AND actor_id=$2)
            ON CONFLICT(id) DO UPDATE SET origin_verified=true,content=EXCLUDED.content,visibility=EXCLUDED.visibility,
              published=EXCLUDED.published,emojis=EXCLUDED.emojis,media=EXCLUDED.media,quote_uri=EXCLUDED.quote_uri,
              parent_id=EXCLUDED.parent_id,updated_at=EXCLUDED.updated_at
            WHERE apub_remote_posts.actor_id=EXCLUDED.actor_id AND apub_remote_posts.deleted_at IS NULL
              AND apub_remote_posts.updated_at<=EXCLUDED.updated_at
            RETURNING '{}'::text
        "#, &[object, &author, &content, visibility, &published, &updated, &emojis, &media_json, &quote, parent]).await?;
        if !saved.is_empty() {
            // A canonical public response wins against a private snapshot with the same timestamp.
            toasty::sql::statement("UPDATE apub_quote_originals SET superseded=true,superseded_by_public=true WHERE id=$1 AND actor_id=$2 AND updated_at<=$3::timestamptz AND NOT superseded_by_public")
                .bind(object).bind(author.as_str()).bind(updated.as_str()).exec(&mut tx).await?;
            emoji::queue_media_blurhash(&mut tx, object, &media).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_public_note(
    note: &Value,
    object: &str,
) -> Result<(String, &'static str, jiff::Timestamp, jiff::Timestamp), AppError> {
    validate_original(note, object, false)
}

fn validate_original(
    note: &Value,
    object: &str,
    allow_followers: bool,
) -> Result<(String, &'static str, jiff::Timestamp, jiff::Timestamp), AppError> {
    let author = string(note, "attributedTo")?;
    if note["id"] != object || note["type"] != "Note" || !same_origin(object, author) {
        return Err(AppError::BadRequest(
            "Announced Note identity mismatch".into(),
        ));
    }
    emoji::note_content(note)?;
    let visibility = if inbox::audience(note, "to").contains(&PUBLIC) {
        "public"
    } else if inbox::audience(note, "cc").contains(&PUBLIC) {
        "unlisted"
    } else if allow_followers {
        // Confirm the actual followers collection and accepted follow before caching.
        "followers"
    } else {
        return Err(AppError::BadRequest(
            "Private objects cannot be announced".into(),
        ));
    };
    let published = string(note, "published")?
        .parse::<jiff::Timestamp>()
        .map_err(|_| AppError::BadRequest("Invalid Note timestamp".into()))?;
    let updated = note["updated"]
        .as_str()
        .map(str::parse::<jiff::Timestamp>)
        .transpose()
        .map_err(|_| AppError::BadRequest("Invalid updated timestamp".into()))?
        .unwrap_or(published);
    if published > jiff::Timestamp::now() + jiff::SignedDuration::from_secs(300)
        || updated > jiff::Timestamp::now() + jiff::SignedDuration::from_secs(300)
    {
        return Err(AppError::BadRequest("Future Note".into()));
    }
    Ok((author.to_owned(), visibility, published, updated))
}
