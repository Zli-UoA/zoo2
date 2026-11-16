//! Authenticated Home/LTL and atomic channel cross-posts.
use crate::{
    activitypub,
    apps::{one, query},
    auth::AuthUser,
    error::AppError,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Default, Deserialize)]
struct Page {
    /// Entry IDs for refreshing a bounded viewport, including repost entries.
    #[serde(skip)]
    entries: Option<Vec<String>>,
    before: Option<String>,
    timeline: Option<String>,
    limit: Option<usize>,
    post_id: Option<String>,
    author: Option<String>,
    #[serde(default)]
    thread: bool,
}
#[derive(Deserialize)]
struct EntryWindow {
    entries: Vec<String>,
    timeline: Option<String>,
    author: Option<String>,
}
async fn refresh_entries(
    State(state): State<AppState>,
    user: AuthUser,
    Json(window): Json<EntryWindow>,
) -> Result<Json<Vec<Value>>, AppError> {
    list(
        State(state),
        user,
        Query(Page {
            entries: Some(window.entries),
            timeline: window.timeline,
            author: window.author,
            ..Page::default()
        }),
    )
    .await
}
#[derive(Deserialize)]
struct NewPost {
    content: String,
    #[serde(default)]
    image_ids: Vec<String>,
    #[serde(default)]
    sensitive: bool,
    #[serde(default)]
    sensitive_image_ids: Vec<String>,
    #[serde(default)]
    apub: bool,
    #[serde(default = "local_visibility")]
    visibility: String,
    channel_id: Option<String>,
    in_reply_to: Option<String>,
}
fn local_visibility() -> String {
    "local".into()
}
#[derive(Deserialize)]
struct EditPost {
    content: String,
}
#[derive(Deserialize)]
struct Settings {
    ltl_channel_pattern: String,
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/home", get(list).post(create))
        .route("/api/home/entries", post(refresh_entries))
        .merge(super::home_images::router())
        .route("/api/home/live", get(super::home_live::upgrade))
        .route("/api/home/settings", get(settings).put(update_settings))
        .route("/api/home/{id}", get(detail).patch(edit).delete(delete))
        .route("/api/home/{id}/thread", get(thread))
}
fn validate_content(content: &str) -> Result<(), AppError> {
    if content.trim().is_empty() || content.chars().count() > 1000 || content.contains('\0') {
        return Err(AppError::BadRequest(
            "Content must contain 1–1000 characters".into(),
        ));
    }
    Ok(())
}
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(mut body): Json<NewPost>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    if body.image_ids.is_empty() {
        validate_content(&body.content)?;
    } else if body.content.chars().count() > 1000 || body.content.contains('\0') {
        return Err(AppError::BadRequest("Invalid content".into()));
    }
    if !matches!(
        body.visibility.as_str(),
        "local" | "public" | "unlisted" | "followers"
    ) || (body.apub && body.visibility == "local")
        || (body.channel_id.is_some() && body.visibility == "followers")
    {
        return Err(AppError::BadRequest(
            "Invalid ActivityPub audience or channel association".into(),
        ));
    }
    let parent = if let Some(id) = &body.in_reply_to {
        Some(activitypub::interactions::target(&state, &user, id).await?)
    } else {
        None
    };
    if let Some(p) = &parent {
        if body.apub && p["apub"] != true {
            return Err(AppError::BadRequest(
                "Local-only posts cannot have federated replies".into(),
            ));
        }
        if p["visibility"] == "followers" && body.visibility != "followers"
            || p["visibility"] == "unlisted"
                && matches!(body.visibility.as_str(), "public" | "local")
        {
            return Err(AppError::BadRequest(
                "Reply audience cannot exceed the original post".into(),
            ));
        }
    }
    // The reply target, never a saved composer preference, determines its channel.
    if let Some(parent) = &parent {
        body.channel_id = parent["channel_id"].as_str().map(str::to_owned);
    }
    let parent_uri = parent.as_ref().and_then(|p| {
        if p["remote"] == true {
            p["id"].as_str().map(str::to_owned)
        } else if p["apub"] == true {
            activitypub::base_url()
                .ok()
                .map(|base| format!("{base}/api/activitypub/notes/{}", p["id"].as_str().unwrap()))
        } else {
            None
        }
    });
    let reply_actor = parent.as_ref().and_then(|p| {
        if p["remote"] == true {
            p["author"].as_str().map(str::to_owned)
        } else if p["apub"] == true {
            activitypub::actor_id(p["author"].as_str().unwrap()).ok()
        } else {
            None
        }
    });
    let reply_user = parent
        .as_ref()
        .filter(|p| p["remote"] != true)
        .and_then(|p| p["author"].as_str());
    if body.apub {
        activitypub::base_url()?;
        activitypub::keys(&state, &user.id).await?;
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if let Some(parent_id) = parent.as_ref().and_then(|p| p["id"].as_str()) {
        let deleted_root = query(&mut tx,
            "SELECT '{}'::text FROM home_posts WHERE id=home_thread_root($1) AND deleted_at IS NOT NULL AND (SELECT phase FROM thread_rollout WHERE id)<>'collecting' UNION ALL SELECT '{}'::text FROM messages WHERE id=home_thread_root($1) AND deleted_at IS NOT NULL AND (SELECT phase FROM thread_rollout WHERE id)<>'collecting'",
            &[parent_id]).await?;
        if !deleted_root.is_empty() {
            return Err(AppError::NotFound("Thread root has been deleted".into()));
        }
    }
    // A nonmember can still reply to a public channel post in Home. Do not
    // silently join the channel or bypass channel posting permissions.
    if parent.is_some()
        && let Some(channel) = &body.channel_id
    {
        let member = query(&mut tx,
                "SELECT json_build_object('id',user_id)::text FROM channel_user WHERE user_id=$1 AND channel_id=$2",
                &[&user.id, channel]).await?;
        if member.is_empty() {
            body.channel_id = None;
        }
    }
    if body.channel_id.is_some() && body.visibility == "followers" {
        return Err(AppError::BadRequest(
            "Channel replies cannot use a followers-only audience".into(),
        ));
    }
    if let Some(channel) = &body.channel_id {
        let actor = crate::apps::events::Actor {
            user: user.clone(),
            principal: None,
        };
        let c = actor.channel(&mut tx, channel, true).await?;
        if c["is_private"] == true || c["is_dm"] == true {
            return Err(AppError::BadRequest(
                "Home requires a public channel".into(),
            ));
        }
        crate::apps::events::posting(&mut tx, &user.id, channel, state.config.dev_auth_enabled)
            .await?;
    }
    if body
        .sensitive_image_ids
        .iter()
        .any(|id| !body.image_ids.contains(id))
    {
        return Err(AppError::BadRequest(
            "Sensitive image IDs must belong to this post".into(),
        ));
    }
    let mut media = super::home_images::resolve(&mut tx, &user.id, &body.image_ids).await?;
    let nsfw = if let Some(channel) = &body.channel_id {
        one(
            &mut tx,
            "SELECT json_build_object('is_nsfw',is_nsfw)::text FROM channels WHERE id=$1",
            &[channel],
        )
        .await?["is_nsfw"]
            == true
    } else {
        false
    };
    let sensitive = body.sensitive || nsfw;
    if let Some(images) = media.as_array_mut() {
        for (image, id) in images.iter_mut().zip(&body.image_ids) {
            image["is_sensitive"] = json!(sensitive || body.sensitive_image_ids.contains(id));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    toasty::sql::statement("INSERT INTO home_posts(id,user_id,content,apub,visibility,channel_id,parent_id,parent_uri,reply_actor,reply_user_id,media) VALUES($1,$2,$3,$4::boolean,$5,NULLIF($6,''),NULLIF($7,''),NULLIF($8,''),NULLIF($9,''),NULLIF($10,''),$11::jsonb)")
        .bind(id.as_str()).bind(user.id.as_str()).bind(body.content).bind(body.apub.to_string()).bind(body.visibility).bind(body.channel_id.as_deref().unwrap_or("")).bind(body.in_reply_to.as_deref().unwrap_or("")).bind(parent_uri.as_deref().unwrap_or("")).bind(reply_actor.as_deref().unwrap_or("")).bind(reply_user.unwrap_or("")).bind(media.to_string()).exec(&mut tx).await?;
    if let Some(channel) = &body.channel_id {
        // The Home trigger has already created the channel message in this transaction.
        toasty::sql::statement("INSERT INTO attachments(id,message_id,uploader_id,channel_id,file_name,file_size,content_type,storage_key,created_at,width,height,blurhash,is_sensitive) SELECT gen_random_uuid()::text,$1,$2,$3,h.name,h.file_size,h.content_type,'home-images/' || h.id,date_trunc('milliseconds',statement_timestamp()) + (selected.ordinality-1)*interval '1 millisecond',h.width,h.height,h.blurhash,($5::boolean OR selected.id IN (SELECT jsonb_array_elements_text($6::jsonb))) FROM jsonb_array_elements_text($4::jsonb) WITH ORDINALITY AS selected(id,ordinality) JOIN home_images h ON h.id=selected.id WHERE h.uploader_id=$2 ORDER BY selected.ordinality")
            .bind(id.as_str()).bind(user.id.as_str()).bind(channel.as_str()).bind(json!(body.image_ids).to_string()).bind(sensitive.to_string()).bind(json!(body.sensitive_image_ids).to_string()).exec(&mut tx).await?;
    }
    if let Some(recipient) = reply_user
        && recipient != user.id
        && body.channel_id.is_none()
    {
        activitypub::interactions::notify(
            &mut tx,
            recipient,
            &format!("local:{}", user.id),
            &id,
            "reply",
            &id,
        )
        .await?;
    }
    let notifications = if body.channel_id.is_some() {
        let message = crate::models::Message::get_by_id(&mut tx, id.as_str()).await?;
        crate::notifications::create_message_notifications(
            &mut tx, &message, &user.id, reply_user, None,
        )
        .await?
    } else {
        vec![]
    };
    tx.commit().await?;
    crate::push::dispatch(state, notifications);
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}
async fn mutable_post(
    tx: &mut toasty::Transaction<'_>,
    state: &AppState,
    user: &AuthUser,
    id: &str,
) -> Result<Value, AppError> {
    let p=one(tx,"SELECT row_to_json(p)::text FROM home_posts p WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL FOR UPDATE",&[id,&user.id]).await?;
    if let Some(channel) = p["channel_id"].as_str() {
        let actor = crate::apps::events::Actor {
            user: user.clone(),
            principal: None,
        };
        actor.channel(tx, channel, true).await?;
        crate::apps::events::posting(tx, &user.id, channel, state.config.dev_auth_enabled).await?;
    }
    Ok(p)
}
async fn edit(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<EditPost>,
) -> Result<StatusCode, AppError> {
    if body.content.contains('\0') {
        return Err(AppError::BadRequest("Invalid content".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let p = mutable_post(&mut tx, &state, &user, &id).await?;
    if p["channel_id"].is_string() {
        drop(tx);
        let _ = crate::routes::messages::update_message(
            State(state),
            user,
            Path(id),
            Json(crate::routes::messages::UpdateMessageRequest {
                content: body.content,
                ciphertext: None,
                epoch: None,
                revision: None,
            }),
        )
        .await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    crate::validation::validate_message_content_with_options(
        &body.content,
        p["media"].as_array().is_some_and(|items| !items.is_empty()),
    )?;
    toasty::sql::statement("UPDATE home_posts SET content=$2,updated_at=now() WHERE id=$1")
        .bind(id)
        .bind(body.content)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let p=one(&mut tx,"SELECT row_to_json(p)::text FROM home_posts p WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL",&[&id,&user.id]).await?;
    if p["channel_id"].is_string() {
        drop(tx);
        let _ = crate::routes::messages::delete_message(State(state), user, Path(id)).await?;
    } else {
        toasty::sql::statement("UPDATE home_posts SET deleted_at=now(),updated_at=now() WHERE id=$1 AND user_id=$2 AND deleted_at IS NULL").bind(id).bind(user.id).exec(&mut tx).await?;
        tx.commit().await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(page): Query<Page>,
) -> Result<Json<Vec<Value>>, AppError> {
    let timeline = page.timeline.as_deref().unwrap_or("home");
    if !matches!(timeline, "home" | "local" | "social") {
        return Err(AppError::BadRequest("Unknown timeline".into()));
    }
    let mut entries = page.entries;
    let limit = if entries.is_some() {
        100
    } else {
        page.limit.unwrap_or(50).clamp(1, 500)
    };
    if let Some(ref ids) = entries {
        validate_entries(ids)?;
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let guest = crate::apps::events::guest(&mut tx, &user.id).await?;
    // The planner's cost for this query passes the JIT thresholds, and compiling it
    // takes longer than running it (production standby, 2026-11-11: thread view
    // 6.4s with JIT, 0.33s without). display_views.sql turns JIT off the same way.
    toasty::sql::statement("SET LOCAL jit = off")
        .exec(&mut tx)
        .await?;
    // Seek eligible keys first; render/decorate only this page and its parents.
    // This stays within the existing request and transaction on every client.
    let seek_page = entries.is_none() && page.post_id.as_deref().is_none_or(|id| id.is_empty());
    if seek_page {
        let keys = query(
            &mut tx,
            &timeline_page_query(),
            &[
                &user.id,
                page.before.as_deref().unwrap_or(""),
                timeline,
                if guest { "true" } else { "false" },
                &limit.to_string(),
                "",
                page.author.as_deref().unwrap_or(""),
                "false",
                "[]",
            ],
        )
        .await?;
        let selected: Vec<String> = keys
            .into_iter()
            .filter_map(|row| row["entry_id"].as_str().map(str::to_owned))
            .collect();
        if selected.is_empty() {
            return Ok(Json(vec![]));
        }
        entries = Some(selected);
    }
    let sql = timeline_query();
    let mut rows = query(
        &mut tx,
        &sql,
        &[
            &user.id,
            if seek_page {
                ""
            } else {
                page.before.as_deref().unwrap_or("")
            },
            timeline,
            if guest { "true" } else { "false" },
            &limit.to_string(),
            page.post_id.as_deref().unwrap_or(""),
            page.author.as_deref().unwrap_or(""),
            if page.thread { "true" } else { "false" },
            &json!(entries.unwrap_or_default()).to_string(),
        ],
    )
    .await?;

    activitypub::quotes::decorate(&mut tx, &mut rows).await?;
    super::home_images::decorate(&mut tx, &mut rows).await?;
    activitypub::channel_reactions::decorate(&mut tx, &mut rows).await?;
    let parents: Vec<Value> = rows
        .iter()
        .filter_map(|row| row.get("parent").filter(|p| p.is_object()).cloned())
        .collect();
    activitypub::remember_display_emojis(&mut tx, &parents).await?;
    activitypub::remember_display_emojis(&mut tx, &rows).await?;
    tx.commit().await?;
    Ok(Json(rows))
}
fn validate_entries(entries: &[String]) -> Result<(), AppError> {
    if entries.is_empty()
        || entries.len() > 100
        || entries
            .iter()
            .any(|id| id.is_empty() || id.len() > 2048 || id.contains('\0'))
    {
        return Err(AppError::BadRequest("Invalid timeline entries".into()));
    }
    Ok(())
}
async fn detail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let Json(rows) = list(
        State(state),
        user,
        Query(Page {
            post_id: Some(id),
            ..Page::default()
        }),
    )
    .await?;
    rows.into_iter()
        .next()
        .map(Json)
        .ok_or_else(|| AppError::NotFound("Post".into()))
}
// Uses the same audience-filtered base as the timeline; inaccessible parents and replies are omitted.
async fn thread(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<Value>>, AppError> {
    let Json(rows) = list(
        State(state),
        user,
        Query(Page {
            post_id: Some(id.clone()),
            thread: true,
            limit: Some(500),
            ..Page::default()
        }),
    )
    .await?;
    // `picked` prioritizes the requested post before applying the limit. Its
    // absence means it is missing or inaccessible, even if relatives are visible.
    if !rows.iter().any(|post| post["id"].as_str() == Some(&id)) {
        return Err(AppError::NotFound("Post".into()));
    }
    Ok(Json(rows))
}
async fn settings(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let admin =
        crate::auth::rbac::is_workspace_admin(&mut tx, &user.id, state.config.dev_auth_enabled)
            .await?;
    let setting = one(
        &mut tx,
        "SELECT row_to_json(s)::text FROM activitypub_settings s",
        &[],
    )
    .await?;
    Ok(Json(
        json!({"ltl_channel_pattern":setting["ltl_channel_pattern"],"can_manage":admin}),
    ))
}
async fn update_settings(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Settings>,
) -> Result<Json<Value>, AppError> {
    if body.ltl_channel_pattern.chars().count() > 200 || body.ltl_channel_pattern.contains('\0') {
        return Err(AppError::BadRequest("Invalid channel pattern".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    if !crate::auth::rbac::is_workspace_admin(&mut tx, &user.id, state.config.dev_auth_enabled)
        .await?
    {
        return Err(AppError::Forbidden("Administrator required".into()));
    }
    let validation = one(
        &mut tx,
        "SELECT json_build_object('valid',valid_ltl_channel_regex($1))::text",
        &[&body.ltl_channel_pattern],
    )
    .await?;
    if validation["valid"] != true {
        return Err(AppError::BadRequest(
            "Invalid channel regular expression".into(),
        ));
    }
    let updated=one(&mut tx,"UPDATE activitypub_settings SET ltl_channel_pattern=$1 RETURNING row_to_json(activitypub_settings)::text",&[&body.ltl_channel_pattern]).await?;
    tx.commit().await?;
    Ok(Json(updated))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_entries_are_bounded_and_typed() {
        assert!(validate_entries(&["p".into(), "repost:r".into()]).is_ok());
        for invalid in [
            vec![],
            vec![String::new()],
            vec!["\0".into()],
            vec!["x".repeat(2049)],
            vec!["p".into(); 101],
        ] {
            assert!(validate_entries(&invalid).is_err());
        }
        assert!(serde_json::from_value::<EntryWindow>(json!({"entries":[1]})).is_err());
    }
    #[test]
    fn content_limits() {
        assert!(validate_content(" \n").is_err());
        assert!(validate_content("text\0").is_err());
        assert!(validate_content(&"あ".repeat(1000)).is_ok());
        assert!(validate_content(&"あ".repeat(1001)).is_err());
    }
}

/// The same audience and entry rules are used by the key picker and renderer.
pub fn timeline_query() -> String {
    fill_audience(
        include_str!("home_timeline.sql")
            .replace("/* reply_counts */", include_str!("home_reply_counts.sql"))
            .replace("/* visible_posts */", include_str!("home_visible.sql")),
    )
    .replace(
        "/* timeline_entries */",
        &include_str!("home_entries.sql").replace("/* entry_posts */", "base"),
    )
    .replace("/* repost_entries */", "scoped_reposts")
    .replace("/* local_posts */", "home_posts")
    .replace("/* remote_posts */", "apub_remote_posts")
}

pub fn timeline_page_query() -> String {
    fill_audience(
        include_str!("home_page.sql")
            .replace("/* visible_posts */", include_str!("home_visible.sql")),
    )
    .replace(
        "/* timeline_entries */",
        &include_str!("home_entries.sql").replace("/* entry_posts */", "visible"),
    )
    .replace("/* repost_entries */", "apub_reposts")
    .replace("/* local_posts */", "home_posts")
    .replace("/* remote_posts */", "apub_remote_posts")
}

// Reuse the exact audience predicates for DTOs and lean reply aggregates.
fn fill_audience(sql: String) -> String {
    sql.replace(
        "/* local_audience */",
        include_str!("home_local_audience.sql").trim(),
    )
    .replace(
        "/* remote_original_audience */",
        include_str!("home_remote_original_audience.sql").trim(),
    )
}
