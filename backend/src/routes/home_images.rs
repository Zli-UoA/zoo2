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
        .route("/api/home/images/{id}", get(image).delete(discard))
}
#[derive(Deserialize)]
struct Upload {
    name: String,
    content_type: String,
    data: String,
}
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_USER_BYTES: i64 = 1024 * 1024 * 1024;
static DECODERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Bound decoded pixels as well as compressed bytes; preserve animated originals.
pub(crate) fn valid_image(mime: &str, bytes: &[u8]) -> bool {
    use image::{ImageFormat, ImageReader, Limits};
    let format = match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::WebP,
        _ => return false,
    };
    let Ok((width, height)) =
        ImageReader::with_format(std::io::Cursor::new(bytes), format).into_dimensions()
    else {
        return false;
    };
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 20_000_000
    {
        return false;
    }
    let mut reader = ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().is_ok()
}

async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<Upload>,
) -> Result<Json<Value>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rate = one(&mut tx,"INSERT INTO home_image_upload_rates(user_id,window_start,requests) VALUES($1,date_trunc('minute',now()),1) ON CONFLICT(user_id,window_start) DO UPDATE SET requests=home_image_upload_rates.requests+1 RETURNING json_build_object('requests',requests)::text",&[&user.id]).await?;
    tx.commit().await?;
    if rate["requests"].as_i64().unwrap_or(31) > 30 {
        return Err(AppError::RateLimited);
    }
    let permit = DECODERS.try_acquire().map_err(|_| AppError::RateLimited)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&body.data)
        .map_err(|_| AppError::BadRequest("Invalid image data".into()))?;
    if bytes.is_empty()
        || bytes.len() > MAX_IMAGE_BYTES
        || bytes.len() as u64 > state.config.s3.max_file_size
        || body.name.chars().count() > 255
        || body.name.contains('\0')
    {
        return Err(AppError::BadRequest("Expected an image up to 8 MiB".into()));
    }
    let mime = body.content_type.clone();
    let (bytes, _permit) = tokio::task::spawn_blocking(move || {
        if valid_image(&mime, &bytes) {
            Ok((bytes, permit))
        } else {
            Err(AppError::BadRequest(
                "Invalid image format, dimensions or pixel count".into(),
            ))
        }
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))??;
    let mut tx = db.transaction().await?;
    query(&mut tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('home-image-upload:' || $1,0))) locked",&[&user.id]).await?;
    let usage = one(&mut tx,"SELECT json_build_object('bytes',(SELECT COALESCE(sum(file_size),0) FROM home_images WHERE uploader_id=$1)+(SELECT COALESCE(sum(file_size),0) FROM home_image_deletions WHERE uploader_id=$1))::text",&[&user.id]).await?;
    if usage["bytes"].as_i64().unwrap_or(MAX_USER_BYTES) + bytes.len() as i64 > MAX_USER_BYTES {
        return Err(AppError::BadRequest(
            "Home image storage limit is 1 GiB per user".into(),
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
    let saved = async {
        toasty::sql::statement("INSERT INTO home_images(id,uploader_id,content_type,name,file_size) VALUES($1,$2,$3,$4,$5::bigint)").bind(id.as_str()).bind(user.id.as_str()).bind(body.content_type).bind(body.name).bind(file_size).exec(&mut tx).await?;
        tx.commit().await?;
        Ok::<_,AppError>(())
    }.await;
    if saved.is_err() {
        let _ =
            crate::storage::delete_storage_object(&state.s3_client, &state.config.s3.bucket, &key)
                .await;
    }
    saved?;
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
        let rows = query(tx, "SELECT json_build_object('name',name,'media_type',content_type)::text FROM home_images WHERE id=$1 AND uploader_id=$2 FOR SHARE", &[id,user]).await?;
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
    let item = one(&mut tx, "SELECT item::text FROM (SELECT json_build_object('name',name,'media_type',content_type,'storage_key','home-images/' || id) AS item FROM home_images h WHERE id=$1 AND EXISTS(SELECT 1 FROM home_posts p LEFT JOIN channels c ON c.id=p.channel_id WHERE p.deleted_at IS NULL AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL) AND p.media @> jsonb_build_array(jsonb_build_object('url','/api/home/images/' || h.id))) UNION ALL SELECT json_build_object('name',a.file_name,'media_type',a.content_type,'storage_key',a.storage_key) FROM attachments a JOIN home_posts p ON p.id=a.message_id JOIN channels c ON c.id=p.channel_id WHERE a.id=$1 AND p.deleted_at IS NULL AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL AND a.content_type IN ('image/png','image/jpeg','image/gif','image/webp')) images LIMIT 1", &[&id]).await?;
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
/// Locks serialize publishing, explicit discard and garbage collection.
async fn referenced(tx: &mut toasty::Transaction<'_>, id: &str) -> Result<bool, AppError> {
    let result = one(tx,"SELECT json_build_object('used',EXISTS(SELECT 1 FROM home_posts WHERE deleted_at IS NULL AND media @> jsonb_build_array(jsonb_build_object('url','/api/home/images/' || $1))) OR EXISTS(SELECT 1 FROM attachments a JOIN messages m ON m.id=a.message_id JOIN channels c ON c.id=m.channel_id AND c.deleted_at IS NULL WHERE a.storage_key='home-images/' || $1 AND m.deleted_at IS NULL))::text",&[id]).await?;
    Ok(result["used"] == true)
}
async fn discard(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    one(
        &mut tx,
        "SELECT '{}'::text FROM home_images WHERE id=$1 AND uploader_id=$2 FOR UPDATE",
        &[&id, &user.id],
    )
    .await?;
    if referenced(&mut tx, &id).await? {
        return Err(AppError::BadRequest(
            "Published images cannot be discarded".into(),
        ));
    }
    toasty::sql::statement("DELETE FROM home_images WHERE id=$1")
        .bind(id)
        .exec(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
pub async fn collect_garbage(state: &AppState) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let candidates = query(&mut tx,"SELECT json_build_object('id',id)::text FROM home_images WHERE created_at<now()-interval '24 hours' AND NOT EXISTS(SELECT 1 FROM home_posts p WHERE p.deleted_at IS NULL AND p.media @> jsonb_build_array(jsonb_build_object('url','/api/home/images/' || home_images.id))) AND NOT EXISTS(SELECT 1 FROM attachments a JOIN messages m ON m.id=a.message_id JOIN channels c ON c.id=m.channel_id AND c.deleted_at IS NULL WHERE a.storage_key='home-images/' || home_images.id AND m.deleted_at IS NULL) ORDER BY created_at LIMIT 100 FOR UPDATE SKIP LOCKED",&[]).await?;
    for candidate in candidates {
        let id = candidate["id"].as_str().unwrap();
        if !referenced(&mut tx, id).await? {
            toasty::sql::statement("DELETE FROM home_images WHERE id=$1")
                .bind(id)
                .exec(&mut tx)
                .await?;
        }
    }
    toasty::sql::statement(
        "DELETE FROM home_image_upload_rates WHERE window_start<now()-interval '1 hour'",
    )
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    let mut tx = db.transaction().await?;
    let removed = query(&mut tx,"SELECT json_build_object('id',id)::text FROM home_image_deletions ORDER BY created_at LIMIT 100 FOR UPDATE SKIP LOCKED",&[]).await?;
    for row in removed {
        let id = row["id"].as_str().unwrap();
        crate::storage::delete_storage_object(
            &state.s3_client,
            &state.config.s3.bucket,
            &format!("home-images/{id}"),
        )
        .await?;
        toasty::sql::statement("DELETE FROM home_image_deletions WHERE id=$1")
            .bind(id)
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn worker(state: AppState) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        timer.tick().await;
        if let Err(error) = collect_garbage(&state).await {
            tracing::warn!(%error,"Home image cleanup failed; queued objects will be retried");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn images_require_valid_pixels_and_matching_mime() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(valid_image("image/png", bytes.get_ref()));
        assert!(!valid_image("image/jpeg", bytes.get_ref()));
        assert!(!valid_image("image/png", b"\x89PNG\r\n\x1a\n"));
        assert!(!valid_image("image/svg+xml", b"<svg/>"));
        let mut oversized = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_luma8(8193, 1)
            .write_to(&mut oversized, image::ImageFormat::Png)
            .unwrap();
        assert!(!valid_image("image/png", oversized.get_ref()));
    }
}
