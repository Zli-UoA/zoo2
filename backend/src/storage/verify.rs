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
