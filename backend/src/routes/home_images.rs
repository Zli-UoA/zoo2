//! Raster images shared by Home drafts, timelines and ActivityPub attachments.
use crate::{
    apps::{one, query},
    auth::AuthUser,
    error::AppError,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    routing::{get, post},
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/home/images",
            post(upload).layer(DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .route("/api/home/images/{id}", get(image))
}
#[derive(Deserialize)]
struct Upload {
    name: String,
    content_type: String,
    data: String,
}
fn valid_image(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => false,
    }
}
async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Upload>,
) -> Result<Json<Value>, AppError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&body.data)
        .map_err(|_| AppError::BadRequest("Invalid image data".into()))?;
    if bytes.is_empty()
        || bytes.len() > 8 * 1024 * 1024
        || bytes.len() as u64 > state.config.s3.max_file_size
        || !valid_image(&body.content_type, &bytes)
        || body.name.chars().count() > 255
        || body.name.contains('\0')
    {
        return Err(AppError::BadRequest(
            "Expected a PNG, JPEG, GIF or WebP image up to 8 MiB".into(),
        ));
    }
    let file_size = bytes.len().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("home-images/{id}");
    state
        .s3_client
        .put_object()
        .bucket(&state.config.s3.bucket)
        .key(&key)
        .content_type(&body.content_type)
        .body(bytes.into())
        .send()
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("Image upload failed")))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    toasty::sql::statement(
        "INSERT INTO home_images(id,uploader_id,content_type,name,file_size) VALUES($1,$2,$3,$4,$5::bigint)",
    )
    .bind(id.as_str())
    .bind(user.id)
    .bind(body.content_type)
    .bind(body.name)
    .bind(file_size)
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id":id})))
}
pub(super) async fn resolve(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    ids: &[String],
) -> Result<Value, AppError> {
    if ids.len() > 10 {
        return Err(AppError::BadRequest("Maximum 10 images".into()));
    }
    let mut seen = std::collections::HashSet::new();
    let mut media = vec![];
    for id in ids {
        if !seen.insert(id) || uuid::Uuid::parse_str(id).is_err() {
            return Err(AppError::BadRequest("Invalid image ID".into()));
        }
        let rows = query(tx, "SELECT json_build_object('name',name,'media_type',content_type)::text FROM home_images WHERE id=$1 AND uploader_id=$2", &[id,user]).await?;
        let mut item = rows
            .into_iter()
            .next()
            .ok_or_else(|| AppError::BadRequest("Image unavailable".into()))?;
        item["url"] = json!(format!("/api/home/images/{id}"));
        media.push(item);
    }
    Ok(json!(media))
}
/// Older channel posts already have images in the shared attachment table.
pub(crate) async fn decorate(
    tx: &mut toasty::Transaction<'_>,
    posts: &mut [Value],
) -> Result<(), AppError> {
    let ids = posts
        .iter()
        .filter(|post| {
            post["remote"] != true
                && !post["media"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
        })
        .filter_map(|post| post["id"].as_str())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(());
    }
    let rows = query(tx, "SELECT json_build_object('id',p.id,'media',(SELECT COALESCE(jsonb_agg(jsonb_build_object('url','/api/home/images/' || a.id,'name',a.file_name,'media_type',a.content_type) ORDER BY a.created_at,a.id),'[]'::jsonb) FROM (SELECT id,file_name,content_type,created_at FROM attachments WHERE message_id=p.id AND content_type IN ('image/png','image/jpeg','image/gif','image/webp') ORDER BY created_at,id LIMIT 10) a))::text FROM home_posts p JOIN channels c ON c.id=p.channel_id WHERE p.id IN (SELECT jsonb_array_elements_text($1::jsonb)) AND p.deleted_at IS NULL AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL", &[&json!(ids).to_string()]).await?;
    let media = rows
        .into_iter()
        .filter_map(|row| Some((row["id"].as_str()?.to_owned(), row["media"].clone())))
        .collect::<std::collections::HashMap<_, _>>();
    for post in posts {
        if let Some(images) = post["id"].as_str().and_then(|id| media.get(id)) {
            post["media"] = images.clone();
        }
    }
    Ok(())
}
// Unguessable URLs are bearer capabilities, as required for remote image consumers.
async fn image(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::response::Redirect, AppError> {
    uuid::Uuid::parse_str(&id).map_err(|_| AppError::NotFound("Image".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let item = one(&mut tx, "SELECT item::text FROM (SELECT json_build_object('name',name,'media_type',content_type,'storage_key','home-images/' || id) AS item FROM home_images WHERE id=$1 UNION ALL SELECT json_build_object('name',a.file_name,'media_type',a.content_type,'storage_key',a.storage_key) FROM attachments a JOIN home_posts p ON p.id=a.message_id JOIN channels c ON c.id=p.channel_id WHERE a.id=$1 AND p.deleted_at IS NULL AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL AND a.content_type IN ('image/png','image/jpeg','image/gif','image/webp')) images LIMIT 1", &[&id]).await?;
    let url = crate::storage::generate_presigned_get_url(
        &state.s3_client,
        &state.config.s3.bucket,
        item["storage_key"].as_str().unwrap(),
        item["media_type"].as_str().unwrap(),
        item["name"].as_str().unwrap(),
        std::time::Duration::from_secs(900),
        state.config.s3.public_endpoint.as_deref(),
    )
    .await?;
    Ok(axum::response::Redirect::temporary(&url))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_matching_passive_raster_types_are_accepted() {
        assert!(valid_image("image/png", b"\x89PNG\r\n\x1a\n"));
        assert!(valid_image("image/jpeg", &[0xff, 0xd8, 0xff]));
        assert!(valid_image("image/gif", b"GIF89a"));
        assert!(valid_image("image/webp", b"RIFF0000WEBP"));
        assert!(!valid_image("image/svg+xml", b"<svg/>"));
        assert!(!valid_image("image/png", b"GIF89a"));
    }
}
