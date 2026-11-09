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

/// Read bounded image bytes before entering the message transaction. Other files keep HEAD-only validation.
pub(crate) async fn uploaded_image_metadata(
    client: &Client,
    bucket: &str,
    key: &str,
    expected_size: i64,
    content_type: &str,
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
    let download_permit = IMAGE_DOWNLOADS
        .acquire()
        .await
        .map_err(|_| AppError::RateLimited)?;
    let object = match client.get_object().bucket(bucket).key(key).send().await {
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
    let mut body = object.body;
    let mut bytes = Vec::new();
    loop {
        let chunk = match body.try_next().await {
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
    let permit = crate::routes::home_images::DECODERS
        .acquire()
        .await
        .map_err(|_| AppError::RateLimited)?;
    match tokio::task::spawn_blocking(move || {
        let _download_permit = download_permit;
        let _permit = permit;
        crate::routes::home_images::image_metadata(&mime, &bytes)
    })
    .await
    {
        Ok(metadata) => Ok(metadata),
        Err(error) => {
            tracing::warn!(%error, key, "Skipping optional image metadata: decoder task failed");
            Ok(None)
        }
    }
}
