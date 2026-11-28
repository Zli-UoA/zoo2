//! Raster images shared by Home drafts, timelines and ActivityPub attachments.
use crate::{
    apps::{one, query},
    auth::AuthUser,
    error::AppError,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

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
pub(crate) static DECODERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Bound decoded pixels as well as compressed bytes; preserve animated originals.
pub(crate) fn image_blurhash(mime: &str, bytes: &[u8]) -> Option<String> {
    let (image, orientation) = decode_image(mime, bytes)?;
    encode_blurhash(&image, orientation)
}

fn decode_image(
    mime: &str,
    bytes: &[u8],
) -> Option<(image::DynamicImage, image::metadata::Orientation)> {
    use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
    let format = match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::WebP,
        _ => return None,
    };
    let Ok((width, height)) =
        ImageReader::with_format(std::io::Cursor::new(bytes), format).into_dimensions()
    else {
        return None;
    };
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 20_000_000
    {
        return None;
    }
    let mut reader = ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let image = image::DynamicImage::from_decoder(decoder).ok()?;
    Some((image, orientation))
}

#[derive(serde::Serialize)]
pub(crate) struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub blurhash: String,
}

pub(crate) const THUMBNAIL_EDGE: u32 = 768;
pub(crate) const THUMBNAIL_MIME: &str = "image/webp";
const IMAGE_CACHE_CONTROL: &str = "private, max-age=300";

pub(crate) fn thumbnail_key(key: &str) -> String {
    format!("{key}.thumbnail.webp")
}

// None denotes an active repair. Completed attempts have a bounded cooldown.
static THUMBNAIL_REPAIRS: LazyLock<Mutex<HashMap<String, Option<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static THUMBNAIL_REPAIR_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

fn schedule_thumbnail_repair(client: &aws_sdk_s3::Client, bucket: &str, original: &str) {
    let Ok(slot) = THUMBNAIL_REPAIR_SLOTS.try_acquire() else {
        return;
    };
    {
        let mut repairs = THUMBNAIL_REPAIRS.lock().unwrap();
        let now = Instant::now();
        repairs.retain(|_, retry_at| retry_at.is_none_or(|retry_at| retry_at > now));
        if repairs.contains_key(original) || repairs.len() >= 1024 {
            return;
        }
        repairs.insert(original.to_owned(), None);
    }
    let client = client.clone();
    let bucket = bucket.to_owned();
    let original = original.to_owned();
    tokio::spawn(async move {
        let _slot = slot;
        let _ = repair_thumbnail(&client, &bucket, &original).await;
        THUMBNAIL_REPAIRS
            .lock()
            .unwrap()
            .insert(original, Some(Instant::now() + Duration::from_secs(60)));
    });
}

async fn repair_thumbnail(client: &aws_sdk_s3::Client, bucket: &str, original: &str) -> Option<()> {
    let head = |key: String| async move {
        tokio::time::timeout(
            Duration::from_secs(15),
            client.head_object().bucket(bucket).key(key).send(),
        )
        .await
        .ok()?
        .ok()
    };
    let variant = head(thumbnail_key(original)).await?;
    let source = head(original.to_owned()).await?;
    if variant
        .metadata()
        .and_then(|values| values.get("source-etag"))
        .map(String::as_str)
        .is_some_and(|etag| source.e_tag() == Some(etag))
    {
        return Some(());
    }
    crate::storage::verify::uploaded_image_metadata(
        client,
        bucket,
        original,
        source.content_length()?,
        source.content_type()?,
        Some(variant.e_tag()?),
    )
    .await
    .ok()?;
    Some(())
}

pub(crate) async fn stored_thumbnail_key(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    original: &str,
) -> Option<String> {
    let thumbnail = thumbnail_key(original);
    let inspect = || async {
        client
            .head_object()
            .bucket(bucket)
            .key(&thumbnail)
            .send()
            .await
            .ok()
    };
    let variant = inspect().await?;
    if original.starts_with("home-images/") {
        return (variant.content_length()? > 0).then_some(thumbnail);
    }
    let Some(source_etag) = variant
        .metadata()
        .and_then(|values| values.get("source-etag"))
    else {
        schedule_thumbnail_repair(client, bucket, original);
        return None;
    };
    match client
        .head_object()
        .bucket(bucket)
        .key(original)
        .if_match(source_etag)
        .send()
        .await
    {
        Ok(_) => Some(thumbnail),
        Err(error) => {
            if error
                .raw_response()
                .is_some_and(|response| response.status().as_u16() == 412)
            {
                schedule_thumbnail_repair(client, bucket, original);
            }
            None
        }
    }
}

/// None means S3 could not confirm the object's state. Keep its reservation until reconciliation.
async fn probe_home_thumbnail(state: &AppState, id: &str) -> Option<i64> {
    let key = thumbnail_key(&format!("home-images/{id}"));
    match state
        .s3_client
        .head_object()
        .bucket(&state.config.s3.bucket)
        .key(&key)
        .send()
        .await
    {
        Ok(object) => object.content_length(),
        Err(error) if error.as_service_error().is_some_and(|e| e.is_not_found()) => Some(0),
        Err(error) => {
            tracing::debug!(%error, %key, "Could not reconcile image thumbnail");
            None
        }
    }
}

async fn record_home_thumbnail(
    db: &mut impl toasty::db::Executor,
    id: &str,
    bytes: i64,
    check_at: &str,
) -> Result<bool, toasty::Error> {
    let rows = toasty::sql::query(
        "UPDATE home_images SET thumbnail_ready=$2::boolean,thumbnail_bytes=$3::bigint
         WHERE id=$1 AND thumbnail_ready IS NULL
           AND thumbnail_check_at=$4::timestamptz
         RETURNING id",
    )
    .bind(id)
    .bind((bytes > 0).to_string())
    .bind(bytes.to_string())
    .bind(check_at)
    .exec(db)
    .await?;
    Ok(!rows.is_empty())
}

// Only a storage-side condition can rule out a late PUT after a client timeout.
async fn confirm_missing_thumbnail(
    state: &AppState,
    id: &str,
    check_at: &str,
) -> Result<Option<i64>, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        "SELECT json_build_object('reserved',thumbnail_bytes>0)::text
         FROM home_images WHERE id=$1 AND thumbnail_ready IS NULL
           AND thumbnail_check_at=$2::timestamptz FOR UPDATE SKIP LOCKED",
        &[id, check_at],
    )
    .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let client = &state.s3_client;
    let bucket = &state.config.s3.bucket;
    let original = format!("home-images/{id}");
    let key = thumbnail_key(&original);
    let legacy = row["reserved"] != true;
    if legacy {
        // A paused old uploader may still be about to write the original.
        let original_exists = tokio::time::timeout(
            Duration::from_secs(5),
            client.head_object().bucket(bucket).key(&original).send(),
        )
        .await;
        if !matches!(original_exists, Ok(Ok(_))) {
            return Ok(None);
        }
    }
    let allow_legacy_finalization = state.config.s3.home_image_legacy_writes_drained;
    let result = if legacy && !allow_legacy_finalization {
        tokio::time::timeout(Duration::from_secs(5), probe_home_thumbnail(state, id))
            .await
            .ok()
            .flatten()
    } else {
        // The real thumbnail PUT uses the same condition. Whichever write wins is final.
        let marker = tokio::time::timeout(
            Duration::from_secs(5),
            client
                .put_object()
                .bucket(bucket)
                .key(&key)
                .if_none_match("*")
                .body(Vec::<u8>::new().into())
                .send(),
        )
        .await;
        if matches!(marker, Ok(Ok(_))) {
            Some(0)
        } else {
            // A failed conditional PUT is inconclusive unless HEAD finds the winning object.
            tokio::time::timeout(
                Duration::from_secs(5),
                client.head_object().bucket(bucket).key(&key).send(),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|object| object.content_length())
        }
    };

    let Some(bytes) = result else { return Ok(None) };
    if bytes == 0 && legacy && !allow_legacy_finalization {
        // A legacy PUT was unconditional. Retain its quota reservation until
        // operators confirm that previous-version storage writes have drained.
        toasty::sql::query(
            "UPDATE home_images SET thumbnail_check_at=statement_timestamp()+interval '30 minutes'
             WHERE id=$1",
        )
        .bind(id)
        .exec(&mut tx)
        .await?;
        tx.commit().await?;
        return Ok(None);
    }
    let recorded = record_home_thumbnail(&mut tx, id, bytes, check_at).await?;
    tx.commit().await?;
    Ok(recorded.then_some(bytes))
}

pub(crate) fn image_assets(mime: &str, bytes: &[u8]) -> Option<(ImageMetadata, Option<Vec<u8>>)> {
    use image::metadata::Orientation;
    let (image, orientation) = decode_image(mime, bytes)?;
    let (width, height) = match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (image.height(), image.width()),
        _ => (image.width(), image.height()),
    };
    let metadata = ImageMetadata {
        width,
        height,
        blurhash: encode_blurhash(&image, orientation)?,
    };
    let thumbnail = (width.max(height) > THUMBNAIL_EDGE && supports_static_thumbnail(mime, bytes))
        .then(|| {
            let mut thumbnail = image.resize(
                THUMBNAIL_EDGE,
                THUMBNAIL_EDGE,
                image::imageops::FilterType::Triangle,
            );
            thumbnail.apply_orientation(orientation);
            let mut output = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(thumbnail.to_rgba8())
                .write_to(&mut output, image::ImageFormat::WebP)
                .ok()?;
            let encoded = output.into_inner();
            (encoded.len() < bytes.len()).then_some(encoded)
        })
        .flatten();
    Some((metadata, thumbnail))
}

fn supports_static_thumbnail(mime: &str, bytes: &[u8]) -> bool {
    let source = std::io::Cursor::new(bytes);
    match mime {
        "image/jpeg" => true,
        "image/png" => image::codecs::png::PngDecoder::new(source)
            .and_then(|decoder| decoder.is_apng())
            .is_ok_and(|animated| !animated),
        "image/webp" => image::codecs::webp::WebPDecoder::new(source)
            .is_ok_and(|decoder| !decoder.has_animation()),
        // Keep the original GIF so every frame stays available in the timeline.
        _ => false,
    }
}

fn encode_blurhash(
    image: &image::DynamicImage,
    orientation: image::metadata::Orientation,
) -> Option<String> {
    // Rotate the tiny preview to avoid an extra allocation of the full raster.
    let mut preview = image.thumbnail(64, 64);
    preview.apply_orientation(orientation);
    let preview = preview.to_rgba8();
    blurhash::encode(4, 3, preview.width(), preview.height(), preview.as_raw()).ok()
}

pub(crate) async fn store_thumbnail(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    bytes: Option<Vec<u8>>,
    source_etag: Option<&str>,
    match_etag: Option<&str>,
) -> bool {
    let Some(bytes) = bytes else { return false };
    let mut put = client
        .put_object()
        .bucket(bucket)
        .key(thumbnail_key(key))
        .content_type(THUMBNAIL_MIME)
        .cache_control(IMAGE_CACHE_CONTROL)
        .set_if_match(match_etag.map(str::to_owned))
        .body(bytes.into());
    if let Some(etag) = source_etag {
        put = put.metadata("source-etag", etag);
    }
    let result = if key.starts_with("home-images/") {
        put.if_none_match("*")
            .customize()
            .config_override(home_image_put_config())
            .send()
            .await
    } else {
        put.send().await
    };
    if let Err(error) = result {
        tracing::warn!(%error, key, "Could not store image thumbnail");
        return false;
    }
    true
}

// A successful single-attempt PUT cannot leave an earlier SDK retry in flight.
fn home_image_put_config() -> aws_sdk_s3::config::Builder {
    aws_sdk_s3::config::Builder::new()
        .retry_config(aws_sdk_s3::config::retry::RetryConfig::standard().with_max_attempts(1))
}

async fn home_quota(tx: &mut toasty::Transaction<'_>, user: &str) -> Result<(i64, i64), AppError> {
    query(tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('home-image-upload:' || $1,0))) locked",&[user]).await?;
    let usage = one(tx,
        "WITH current AS (
           SELECT COALESCE(sum(file_size + CASE WHEN thumbnail_ready THEN thumbnail_bytes ELSE 0 END),0) AS bytes,
                  COALESCE(sum(CASE WHEN thumbnail_ready IS NULL THEN
                       CASE WHEN thumbnail_bytes>0 THEN thumbnail_bytes
                            WHEN home_image_can_have_thumbnail(content_type,width,height) THEN file_size
                            ELSE 0 END ELSE 0 END),0) AS uncertain
           FROM home_images WHERE uploader_id=$1
         ), pending AS (
           SELECT COALESCE(sum(file_size),0) AS bytes,
                  COALESCE(sum(CASE WHEN thumbnail_accounted THEN unconfirmed_thumbnail_bytes
                             ELSE file_size END),0) AS uncertain
           FROM home_image_deletion_queue WHERE uploader_id=$1
         )
         SELECT json_build_object('known',current.bytes+pending.bytes,
           'uncertain',current.uncertain+pending.uncertain)::text
         FROM current,pending", &[user]).await?;
    Ok((
        usage["known"].as_i64().unwrap_or(MAX_USER_BYTES),
        usage["uncertain"].as_i64().unwrap_or(0),
    ))
}

async fn reconcile_uploader_thumbnails(
    state: &AppState,
    user: &str,
    mut needed: i64,
) -> Result<(), AppError> {
    let mut db = state.db.clone();
    for _ in 0..25 {
        let mut tx = db.transaction().await?;
        // Claim only the four objects that can start immediately, even if the caller times out.
        let rows = query(&mut tx,
            "WITH candidates AS MATERIALIZED (
               (SELECT id, file_size AS reserved FROM home_images WHERE uploader_id=$1
                 AND thumbnail_ready IS NULL AND NOT (thumbnail_bytes>0)
                 AND home_image_can_have_thumbnail(content_type,width,height)
                 AND thumbnail_check_at<=now()+interval '5 minutes'
               ORDER BY thumbnail_check_at,id LIMIT 32)
               UNION ALL
               (SELECT id, thumbnail_bytes AS reserved FROM home_images WHERE uploader_id=$1
                 AND thumbnail_ready IS NULL AND (thumbnail_bytes>0)
                 AND home_image_can_have_thumbnail(content_type,width,height)
                 AND thumbnail_check_at<=now()
               ORDER BY thumbnail_check_at,id LIMIT 32)
             )
             UPDATE home_images SET thumbnail_check_at=now()+interval '10 minutes'
             WHERE id IN (SELECT h.id FROM candidates c JOIN home_images h ON h.id=c.id
               WHERE h.thumbnail_ready IS NULL
                 AND CASE WHEN h.thumbnail_bytes>0 THEN h.thumbnail_check_at<=now()
                   ELSE h.created_at<now()-interval '5 minutes'
                     AND h.thumbnail_check_at<=now()+interval '5 minutes' END
               ORDER BY c.reserved DESC,h.id LIMIT 4 FOR UPDATE OF h SKIP LOCKED)
             RETURNING json_build_object('id',id,'check_at',thumbnail_check_at,'eligible',true,'reserved',CASE WHEN thumbnail_bytes>0 THEN thumbnail_bytes ELSE file_size END)::text",
            &[user]).await?;
        tx.commit().await?;
        if rows.is_empty() {
            break;
        }
        needed -= reconcile_claimed_home_thumbnails(state, rows).await?;
        if needed <= 0 {
            break;
        }
    }
    Ok(())
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
    let (bytes, width, height, blurhash, thumbnail) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if let Some((metadata, thumbnail)) = image_assets(&mime, &bytes) {
            Ok((
                bytes,
                metadata.width,
                metadata.height,
                metadata.blurhash,
                thumbnail,
            ))
        } else {
            Err(AppError::BadRequest(
                "Invalid image format, dimensions or pixel count".into(),
            ))
        }
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))??;
    let thumbnail_bytes = thumbnail.as_ref().map_or(0, Vec::len);
    let required = (bytes.len() + thumbnail_bytes) as i64;
    let mut tx = db.transaction().await?;
    let (mut known, mut uncertain) = home_quota(&mut tx, &user.id).await?;
    if known + required <= MAX_USER_BYTES && known + uncertain + required > MAX_USER_BYTES {
        tx.commit().await?;
        // Do not hold the uploader lock or a transaction during object-store I/O.
        if let Ok(result) = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            reconcile_uploader_thumbnails(
                &state,
                &user.id,
                known + uncertain + required - MAX_USER_BYTES,
            ),
        )
        .await
        {
            result?;
        }
        tx = db.transaction().await?;
        (known, uncertain) = home_quota(&mut tx, &user.id).await?;
    }
    if known + required > MAX_USER_BYTES {
        return Err(AppError::BadRequest(
            "Home image storage limit is 1 GiB per user".into(),
        ));
    }
    if known + uncertain + required > MAX_USER_BYTES {
        return Err(AppError::ServiceUnavailable(
            "Home image storage is being checked; retry the upload shortly".into(),
        ));
    }
    let file_size = bytes.len().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("home-images/{id}");
    // Reserve durable metadata before starting S3 I/O so crashes leave a GC candidate.
    toasty::sql::statement("INSERT INTO home_images(id,uploader_id,content_type,name,file_size,width,height,blurhash,thumbnail_bytes,thumbnail_ready,thumbnail_conditional_write,thumbnail_write_pending) VALUES($1,$2,$3,$4,$5::bigint,$6::integer,$7::integer,$8,$9::bigint,CASE WHEN $9::bigint=0 THEN false ELSE NULL END,true,$9::bigint>0)").bind(id.as_str()).bind(user.id.as_str()).bind(body.content_type.as_str()).bind(body.name).bind(file_size).bind(width.to_string()).bind(height.to_string()).bind(blurhash.as_str()).bind(thumbnail_bytes.to_string()).exec(&mut tx).await?;
    tx.commit().await?;
    let mut tx = db.transaction().await?;
    // Account deletion and GC must not delete the object before PUT finishes.
    let reserved = one(
        &mut tx,
        "SELECT json_build_object('thumbnail_reserved',thumbnail_ready IS NULL
             AND thumbnail_bytes=$2::bigint AND $2::bigint>0,'check_at',thumbnail_check_at)::text
         FROM home_images WHERE id=$1 FOR UPDATE",
        &[&id, &thumbnail_bytes.to_string()],
    )
    .await?;
    let uploaded = state
        .s3_client
        .put_object()
        .bucket(&state.config.s3.bucket)
        .key(&key)
        .content_type(&body.content_type)
        .cache_control(IMAGE_CACHE_CONTROL)
        .if_none_match("*")
        .body(bytes.into())
        .customize()
        .config_override(home_image_put_config())
        .send()
        .await;
    if uploaded.is_err() {
        // Queue even failed PUTs: a transport error can hide a completed upload.
        toasty::sql::statement("DELETE FROM home_images WHERE id=$1")
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
        return Err(AppError::Internal(anyhow::anyhow!("Image upload failed")));
    }
    toasty::sql::statement("UPDATE home_images SET original_write_confirmed=true WHERE id=$1")
        .bind(id.as_str())
        .exec(&mut tx)
        .await?;
    if reserved["thumbnail_reserved"] == true
        && let Some(check_at) = reserved["check_at"].as_str()
    {
        if store_thumbnail(
            &state.s3_client,
            &state.config.s3.bucket,
            &key,
            thumbnail,
            None,
            None,
        )
        .await
        {
            record_home_thumbnail(&mut tx, &id, thumbnail_bytes as i64, check_at).await?;
            toasty::sql::statement(
                "UPDATE home_images SET thumbnail_write_pending=false WHERE id=$1",
            )
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
        } else {
            // An ambiguous PUT can still finish. Delay the HEAD and retain the reservation.
            toasty::sql::statement(
                "UPDATE home_images SET thumbnail_check_at=statement_timestamp()+interval '5 minutes' WHERE id=$1",
            )
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(Json(
        json!({"id":id,"width":width,"height":height,"blurhash":blurhash}),
    ))
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
        let rows = query(tx, "SELECT json_build_object('name',name,'media_type',content_type,'width',width,'height',height,'blurhash',blurhash)::text FROM home_images WHERE id=$1 AND uploader_id=$2 FOR SHARE", &[id,user]).await?;
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
    fn collect_channels(post: &Value, ids: &mut std::collections::HashSet<String>) {
        if let Some(id) = post["channel_id"].as_str() {
            ids.insert(id.to_owned());
        }
        if post["parent"].is_object() {
            collect_channels(&post["parent"], ids);
        }
    }
    let mut channels = std::collections::HashSet::new();
    for post in posts.iter() {
        collect_channels(post, &mut channels);
    }
    let nsfw = if channels.is_empty() {
        vec![]
    } else {
        query(
            tx,
            "SELECT json_build_object('id',c.id)::text
             FROM jsonb_array_elements_text($1::jsonb) ids(id)
             CROSS JOIN LATERAL (
               SELECT id FROM channels WHERE id=ids.id AND is_nsfw OFFSET 0
             ) c",
            &[&json!(channels).to_string()],
        )
        .await?
    };
    let nsfw = nsfw
        .iter()
        .filter_map(|row| row["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    fn protect(post: &mut Value, nsfw: &std::collections::HashSet<&str>) {
        if post["channel_id"]
            .as_str()
            .is_some_and(|id| nsfw.contains(id))
        {
            post["sensitive"] = json!(true);
            for key in ["media", "attachments"] {
                if let Some(images) = post[key].as_array_mut() {
                    for image in images {
                        image["is_sensitive"] = json!(true);
                    }
                }
            }
        }
        if !post["parent"].is_null() {
            protect(&mut post["parent"], nsfw);
        }
    }
    for post in posts.iter_mut() {
        protect(post, &nsfw);
    }
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
    let rows = query(tx, "SELECT json_build_object('id',p.id,'media',(SELECT COALESCE(jsonb_agg(jsonb_build_object('url','/api/home/images/' || a.id,'name',a.file_name,'media_type',a.content_type,'width',a.width,'height',a.height,'blurhash',a.blurhash,'is_sensitive',a.is_sensitive OR c.is_nsfw) ORDER BY a.created_at,a.id),'[]'::jsonb) FROM (SELECT a.id,a.file_name,a.content_type,a.created_at,a.width,a.height,a.blurhash,a.is_sensitive FROM attachments a WHERE a.message_id=p.id AND a.content_type IN ('image/png','image/jpeg','image/gif','image/webp') ORDER BY a.created_at,a.id LIMIT 10) a))::text FROM home_posts p JOIN channels c ON c.id=p.channel_id WHERE p.id IN (SELECT jsonb_array_elements_text($1::jsonb)) AND p.deleted_at IS NULL AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL", &[&json!(ids).to_string()]).await?;
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
#[derive(Deserialize)]
struct ImageVariant {
    #[serde(default)]
    thumbnail: bool,
}

// Unguessable URLs are bearer capabilities, as required for remote image consumers.
async fn image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(variant): Query<ImageVariant>,
) -> Response {
    match image_redirect(state, id, variant).await {
        Ok(redirect) => redirect.into_response(),
        Err(error) => ([(header::CACHE_CONTROL, "no-store")], error).into_response(),
    }
}

async fn image_redirect(
    state: AppState,
    id: String,
    variant: ImageVariant,
) -> Result<impl IntoResponse, AppError> {
    uuid::Uuid::parse_str(&id).map_err(|_| AppError::NotFound("Image".into()))?;
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let item = one(&mut tx, "SELECT item::text FROM (SELECT json_build_object('name',name,'media_type',content_type,'storage_key','home-images/' || id,'thumbnail_ready',CASE WHEN NOT home_image_can_have_thumbnail(h.content_type,h.width,h.height) THEN false ELSE h.thumbnail_ready END,'thumbnail_check_at',h.thumbnail_check_at,'thumbnail_probe_allowed',h.thumbnail_bytes=0 OR h.thumbnail_check_at<=now()) AS item FROM home_images h WHERE id=$1 AND EXISTS(SELECT 1 FROM home_posts p LEFT JOIN channels c ON c.id=p.channel_id WHERE p.deleted_at IS NULL AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL) AND p.media @> jsonb_build_array(jsonb_build_object('url','/api/home/images/' || h.id))) UNION ALL SELECT json_build_object('name',a.file_name,'media_type',a.content_type,'storage_key',a.storage_key,'thumbnail_ready',CASE WHEN NOT home_image_can_have_thumbnail(h.content_type,h.width,h.height) THEN false ELSE h.thumbnail_ready END,'thumbnail_check_at',h.thumbnail_check_at,'thumbnail_probe_allowed',h.thumbnail_bytes=0 OR h.thumbnail_check_at<=now()) FROM attachments a LEFT JOIN home_images h ON h.id=substring(a.storage_key from 13) AND a.storage_key LIKE 'home-images/%' JOIN home_posts p ON p.id=a.message_id JOIN channels c ON c.id=p.channel_id WHERE a.id=$1 AND p.deleted_at IS NULL AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL AND a.content_type IN ('image/png','image/jpeg','image/gif','image/webp')) images LIMIT 1", &[&id]).await?;
    let key = item["storage_key"].as_str().unwrap();
    let content_type = item["media_type"].as_str().unwrap();
    drop(tx);
    let thumbnail = if variant.thumbnail {
        match item["thumbnail_ready"].as_bool() {
            Some(true) => Some(thumbnail_key(key)),
            Some(false) => None,
            None if key.starts_with("home-images/") => {
                if item["thumbnail_probe_allowed"] == false {
                    None
                } else {
                    let image_id = key.trim_start_matches("home-images/");
                    let bytes = probe_home_thumbnail(&state, image_id).await;
                    // Only reconciliation may release a reservation when storage reports absence.
                    if let Some(bytes) = bytes.filter(|bytes| *bytes > 0)
                        && let Some(check_at) = item["thumbnail_check_at"].as_str()
                        && let Err(error) =
                            record_home_thumbnail(&mut state.db.clone(), image_id, bytes, check_at)
                                .await
                    {
                        // The object is usable even when its quota update has to be retried.
                        tracing::warn!(%error, image_id, "Could not record thumbnail size");
                    }
                    bytes.filter(|bytes| *bytes > 0).map(|_| thumbnail_key(key))
                }
            }
            None => stored_thumbnail_key(&state.s3_client, &state.config.s3.bucket, key).await,
        }
    } else {
        None
    };
    let (key, content_type) = thumbnail
        .as_deref()
        .map_or((key, content_type), |key| (key, THUMBNAIL_MIME));
    let url = crate::storage::generate_presigned_get_url(
        &state.s3_client,
        &state.config.s3.bucket,
        key,
        content_type,
        item["name"].as_str().unwrap(),
        std::time::Duration::from_secs(900),
        state.config.s3.public_endpoint.as_deref(),
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "private, no-cache")],
        axum::response::Redirect::temporary(&url),
    ))
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
    let deletion_candidates = if state.config.s3.home_image_legacy_writes_drained {
        ""
    } else {
        "WHERE NOT legacy_write_pending"
    };
    let removed = query(
        &mut tx,
        &format!(
            "SELECT json_build_object('id',id,'original_pending',original_write_pending,
                  'thumbnail_pending',thumbnail_write_pending)::text FROM home_image_deletion_queue
                  {deletion_candidates} ORDER BY created_at LIMIT 100 FOR UPDATE SKIP LOCKED"
        ),
        &[],
    )
    .await?;
    for row in removed {
        let id = row["id"].as_str().unwrap();
        let key = format!("home-images/{id}");
        remove_home_image_object(
            state,
            &thumbnail_key(&key),
            row["thumbnail_pending"] == true,
        )
        .await?;
        remove_home_image_object(state, &key, row["original_pending"] == true).await?;
        toasty::sql::statement("DELETE FROM home_image_deletion_queue WHERE id=$1")
            .bind(id)
            .exec(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
async fn remove_home_image_object(
    state: &AppState,
    key: &str,
    write_pending: bool,
) -> Result<(), AppError> {
    let client = &state.s3_client;
    let bucket = &state.config.s3.bucket;
    if write_pending {
        // An in-flight conditional PUT can recreate a deleted key. The original UUID
        // is never reused. Keep an empty object so all delayed writes remain blocked.
        client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(Vec::<u8>::new().into())
            .send()
            .await
            .map_err(|error| AppError::Internal(error.into()))?;
        return Ok(());
    }
    crate::storage::delete_storage_object(client, bucket, key).await
}

async fn reconcile_home_thumbnails(state: &AppState) -> Result<(), AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        "UPDATE home_images SET thumbnail_check_at=now()+interval '10 minutes'
         WHERE id IN (SELECT id FROM home_images WHERE thumbnail_ready IS NULL
           AND thumbnail_check_at<=now() ORDER BY thumbnail_check_at,id LIMIT 100
           FOR UPDATE SKIP LOCKED)
         RETURNING json_build_object('id',id,'check_at',thumbnail_check_at,
           'eligible',home_image_can_have_thumbnail(content_type,width,height))::text",
        &[],
    )
    .await?;
    tx.commit().await?;
    reconcile_claimed_home_thumbnails(state, rows).await?;
    Ok(())
}

async fn reconcile_claimed_home_thumbnails(
    state: &AppState,
    rows: Vec<Value>,
) -> Result<i64, AppError> {
    use futures_util::{StreamExt, TryStreamExt};
    futures_util::stream::iter(rows)
        .map(|row| async move {
            if let (Some(id), Some(check_at)) = (row["id"].as_str(), row["check_at"].as_str()) {
                if row["eligible"] == true {
                    let actual = match probe_home_thumbnail(state, id).await {
                        Some(0) => confirm_missing_thumbnail(state, id, check_at).await?,
                        Some(actual) => {
                            record_home_thumbnail(&mut state.db.clone(), id, actual, check_at)
                                .await?
                                .then_some(actual)
                        }
                        None => None,
                    };
                    if let (Some(reserved), Some(actual)) = (row["reserved"].as_i64(), actual) {
                        return Ok::<i64, AppError>((reserved - actual).max(0));
                    }
                } else {
                    record_home_thumbnail(&mut state.db.clone(), id, 0, check_at).await?;
                }
            }
            Ok::<i64, AppError>(0)
        })
        .buffer_unordered(4)
        .try_fold(0, |released, bytes| async move { Ok(released + bytes) })
        .await
}

pub async fn worker(state: AppState) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        timer.tick().await;
        if let Err(error) = collect_garbage(&state).await {
            tracing::warn!(%error,"Home image cleanup failed; queued objects will be retried");
        }
        if let Err(error) = reconcile_home_thumbnails(&state).await {
            tracing::warn!(%error, "Home image thumbnail reconciliation failed");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn list_thumbnail_is_smaller_and_preserves_aspect_ratio() {
        let mut value = 1_u32;
        let raster = image::RgbImage::from_fn(1536, 768, |_, _| {
            value ^= value << 13;
            value ^= value >> 17;
            value ^= value << 5;
            image::Rgb([value as u8, (value >> 8) as u8, (value >> 16) as u8])
        });
        let mut original = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(raster)
            .write_to(&mut original, image::ImageFormat::Png)
            .unwrap();
        let (metadata, thumbnail) = image_assets("image/png", original.get_ref()).unwrap();
        let thumbnail = thumbnail.unwrap();
        assert_eq!((metadata.width, metadata.height), (1536, 768));
        assert!(thumbnail.len() < original.get_ref().len());
        let thumbnail =
            image::load_from_memory_with_format(&thumbnail, image::ImageFormat::WebP).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (768, 384));
    }

    #[test]
    fn animated_gif_keeps_its_original_for_the_timeline() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                let frame = image::RgbaImage::from_pixel(800, 40, image::Rgba(color));
                encoder.encode_frame(image::Frame::new(frame)).unwrap();
            }
        }
        let (metadata, thumbnail) = image_assets("image/gif", &bytes).unwrap();
        assert_eq!((metadata.width, metadata.height), (800, 40));
        assert!(!metadata.blurhash.is_empty());
        assert!(thumbnail.is_none());
    }

    #[test]
    fn metadata_matches_exif_orientation_and_preview_colors() {
        let raster = image::RgbImage::from_fn(80, 40, |x, _| {
            if x < 40 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(raster)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        // 6 places the red half above blue; 8 places blue above red.
        for (orientation, red_on_top) in [(6_u8, true), (8, false)] {
            let mut exif =
                b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0"
                    .to_vec();
            exif[24] = orientation;
            let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1];
            bytes.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
            bytes.extend_from_slice(&exif);
            bytes.extend_from_slice(&jpeg.get_ref()[2..]);
            let (decoded, _) = decode_image("image/jpeg", &bytes).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (80, 40));
            let metadata = image_assets("image/jpeg", &bytes).unwrap().0;
            assert_eq!((metadata.width, metadata.height), (40, 80));
            let pixels = blurhash::decode(&metadata.blurhash, 8, 16, 1.0).unwrap();
            let top = (2 * 8 + 4) * 4;
            let bottom = (13 * 8 + 4) * 4;
            assert_eq!(pixels[top] > pixels[top + 2], red_on_top);
            assert_eq!(pixels[bottom] > pixels[bottom + 2], !red_on_top);
        }
    }

    #[test]
    fn images_require_valid_pixels_and_matching_mime() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(image_blurhash("image/png", bytes.get_ref()).is_some());
        assert!(image_blurhash("image/jpeg", bytes.get_ref()).is_none());
        assert!(image_blurhash("image/png", b"\x89PNG\r\n\x1a\n").is_none());
        assert!(image_blurhash("image/svg+xml", b"<svg/>").is_none());
        let mut oversized = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_luma8(8193, 1)
            .write_to(&mut oversized, image::ImageFormat::Png)
            .unwrap();
        assert!(image_blurhash("image/png", oversized.get_ref()).is_none());
    }
}
