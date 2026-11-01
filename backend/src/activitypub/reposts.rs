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
    let known = query(
        &mut tx,
        "SELECT '{}'::text FROM apub_remote_posts WHERE id=$1",
        &[object],
    )
    .await?;
    if !known.is_empty() {
        return Ok(());
    }
    one(&mut tx,"SELECT '{}'::text FROM apub_follows WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing' AND accepted",&[user,string(activity,"actor")?]).await?;
    drop(tx);
    let note = transport::fetch_json(state, Some(user), object).await?;
    let (author, visibility, published, updated) = validate_public_note(&note, object)?;
    remote_actor(state, Some(user), &author, false).await?;
    let emojis = emoji::tags(&note["tag"]);
    let mut tx = db.transaction().await?;
    emoji::remember(&mut tx, &emojis).await?;
    toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,emojis) VALUES($1,$2,$3,$4,$5::timestamptz,$6::timestamptz,$7::jsonb) ON CONFLICT DO NOTHING").bind(object).bind(author).bind(emoji::plain_content(string(&note,"content")?)).bind(visibility).bind(published.to_string()).bind(updated.to_string()).bind(json!(emojis).to_string()).exec(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) fn validate_public_note(
    note: &Value,
    object: &str,
) -> Result<(String, &'static str, jiff::Timestamp, jiff::Timestamp), AppError> {
    let author = string(note, "attributedTo")?;
    if note["id"] != object || note["type"] != "Note" || !same_origin(object, author) {
        return Err(AppError::BadRequest(
            "Announced Note identity mismatch".into(),
        ));
    }
    if string(note, "content")?.len() > 128 * 1024 {
        return Err(AppError::BadRequest("Note too large".into()));
    }
    let has = |key: &str| {
        note[key].as_str() == Some(PUBLIC)
            || note[key]
                .as_array()
                .is_some_and(|values| values.iter().any(|v| v == PUBLIC))
    };
    let visibility = if has("to") {
        "public"
    } else if has("cc") {
        "unlisted"
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
