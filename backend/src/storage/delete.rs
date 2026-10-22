use aws_sdk_s3::Client;

use crate::error::AppError;

/// ストレージ上のオブジェクトを削除する
pub async fn delete_storage_object(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<(), AppError> {
    client
        .delete_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|err| {
            tracing::warn!("DeleteObject failed for key {key} in bucket {bucket}: {err:?}");
            AppError::Internal(anyhow::anyhow!(
                "Failed to delete object from storage: {err}"
            ))
        })?;

    Ok(())
}
