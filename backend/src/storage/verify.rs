use aws_sdk_s3::Client;

use crate::error::AppError;

/// ストレージ上のオブジェクトが存在し、サイズと Content-Type が申告通りか検証する
pub async fn verify_uploaded_object(
    client: &Client,
    bucket: &str,
    key: &str,
    expected_size: i64,
    expected_content_type: &str,
) -> Result<(), AppError> {
    let head = client
        .head_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|err| {
            tracing::warn!("HeadObject failed for key {key} in bucket {bucket}: {err:?}");
            AppError::BadRequest(format!(
                "Attachment object not found or inaccessible: {key}"
            ))
        })?;

    let actual_size = head.content_length().unwrap_or(-1);
    if actual_size != expected_size {
        return Err(AppError::BadRequest(format!(
            "Attachment size mismatch for {key}: expected {expected_size}, got {actual_size}"
        )));
    }

    if let Some(actual_ct) = head.content_type() {
        // charset 等のパラメータがある場合を考慮し、base MIME タイプで比較
        let actual_base = actual_ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let expected_base = expected_content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();

        if actual_base != expected_base {
            return Err(AppError::BadRequest(format!(
                "Attachment content-type mismatch for {key}: expected {expected_content_type}, got {actual_ct}"
            )));
        }
    }

    Ok(())
}

// Keep compressed buffers bounded without occupying the Home decoder slots during S3 I/O.
static IMAGE_DOWNLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

async fn image_storage_step<T>(
    repair: bool,
    step: impl std::future::Future<Output = T>,
) -> Option<T> {
    if repair {
        tokio::time::timeout(std::time::Duration::from_secs(15), step)
            .await
            .ok()
    } else {
        Some(step.await)
    }
}

/// Read bounded image bytes before entering the message transaction. Other files keep HEAD-only validation.
pub(crate) async fn uploaded_image_metadata(
    client: &Client,
    bucket: &str,
    key: &str,
    expected_size: i64,
    content_type: &str,
    legacy_thumbnail_etag: Option<&str>,
) -> Result<Option<crate::routes::home_images::ImageMetadata>, AppError> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    const MAX_BYTES: usize = 50 * 1024 * 1024;
    if !matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    ) || expected_size <= 0
        || expected_size > MAX_BYTES as i64
    {
        return Ok(None);
    }
    let repair = legacy_thumbnail_etag.is_some();
    let download_permit = image_storage_step(repair, IMAGE_DOWNLOADS.acquire())
        .await
        .ok_or(AppError::RateLimited)?
        .map_err(|_| AppError::RateLimited)?;
    let get = client.get_object().bucket(bucket).key(key).send();
    let Some(object) = image_storage_step(repair, get).await else {
        return Ok(None);
    };
    let object = match object {
        Ok(object) => object,
        Err(error) => {
            tracing::warn!(%error, key, "Skipping optional image metadata: S3 GET failed");
            return Ok(None);
        }
    };
    if object
        .content_length()
        .is_some_and(|length| length != expected_size)
    {
        return Err(AppError::BadGateway("Attachment image size changed".into()));
    }
    let source_etag = object.e_tag().map(str::to_owned);
    let mut body = object.body;
    let mut bytes = Vec::new();
    loop {
        let next = image_storage_step(repair, async {
            loop {
                match body.try_next().await {
                    Ok(Some(bytes)) if bytes.is_empty() => continue,
                    result => break result,
                }
            }
        })
        .await;
        let Some(next) = next else {
            tracing::debug!(key, "Attachment thumbnail repair transfer stalled");
            return Ok(None);
        };
        let chunk = match next {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, key, "Skipping optional image metadata: transfer failed");
                return Ok(None);
            }
        };
        if bytes.len() + chunk.len() > expected_size as usize {
            return Err(AppError::BadGateway("Attachment image size changed".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != expected_size as usize {
        return Err(AppError::BadGateway("Attachment image size changed".into()));
    }
    let permit = image_storage_step(repair, crate::routes::home_images::DECODERS.acquire())
        .await
        .ok_or(AppError::RateLimited)?
        .map_err(|_| AppError::RateLimited)?;
    match tokio::task::spawn_blocking(move || {
        let _download_permit = download_permit;
        let _permit = permit;
        crate::routes::home_images::image_assets(&mime, &bytes)
    })
    .await
    {
        Ok(assets) => {
            let (metadata, thumbnail) = assets.map_or((None, None), |(metadata, thumbnail)| {
                (Some(metadata), thumbnail)
            });
            if let Some(bytes) = thumbnail {
                if let Some(etag) = source_etag.as_deref() {
                    image_storage_step(
                        repair,
                        crate::routes::home_images::store_thumbnail(
                            client,
                            bucket,
                            key,
                            Some(bytes),
                            Some(etag),
                            legacy_thumbnail_etag,
                        ),
                    )
                    .await;
                }
            } else if let Some(legacy_etag) = legacy_thumbnail_etag {
                // The decoded original cannot produce a thumbnail. Retire only the stale object we read.
                let delete = client
                    .delete_object()
                    .bucket(bucket)
                    .key(crate::routes::home_images::thumbnail_key(key))
                    .if_match(legacy_etag)
                    .send();
                if let Some(Err(error)) = image_storage_step(repair, delete).await {
                    tracing::debug!(%error, key, "Could not remove obsolete attachment thumbnail");
                }
            }
            Ok(metadata)
        }
        Err(error) => {
            tracing::warn!(%error, key, "Skipping optional image metadata: decoder task failed");
            Ok(None)
        }
    }
}
