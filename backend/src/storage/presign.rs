use aws_sdk_s3::Client;
use aws_sdk_s3::presigning::PresigningConfig;
use std::time::Duration;

use super::format::{format_content_disposition, is_inline_allowed_mime};
use crate::error::AppError;

/// Presigned PUT URL を生成する
/// 署名に Content-Length と Content-Type を含めて強制する
pub async fn generate_presigned_put_url(
    client: &Client,
    bucket: &str,
    key: &str,
    content_length: i64,
    content_type: &str,
    expiry: Duration,
) -> Result<String, AppError> {
    let presigning_config = PresigningConfig::expires_in(expiry).map_err(|err| {
        AppError::Internal(anyhow::anyhow!("Failed to create presigning config: {err}"))
    })?;

    let presigned_req = client
        .put_object()
        .bucket(bucket)
        .key(key)
        .content_length(content_length)
        .content_type(content_type)
        .presigned(presigning_config)
        .await
        .map_err(|err| {
            AppError::Internal(anyhow::anyhow!(
                "Failed to generate presigned PUT url: {err}"
            ))
        })?;

    Ok(presigned_req.uri().to_string())
}

/// Presigned GET URL を生成する
/// response-content-type と response-content-disposition (RFC 6266/5987) を署名に付与する
pub async fn generate_presigned_get_url(
    client: &Client,
    bucket: &str,
    key: &str,
    content_type: &str,
    file_name: &str,
    expiry: Duration,
) -> Result<String, AppError> {
    let presigning_config = PresigningConfig::expires_in(expiry).map_err(|err| {
        AppError::Internal(anyhow::anyhow!("Failed to create presigning config: {err}"))
    })?;

    let disposition_type = if is_inline_allowed_mime(content_type) {
        "inline"
    } else {
        "attachment"
    };
    let content_disposition = format_content_disposition(disposition_type, file_name);

    let presigned_req = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .response_content_type(content_type)
        .response_content_disposition(content_disposition)
        .presigned(presigning_config)
        .await
        .map_err(|err| {
            AppError::Internal(anyhow::anyhow!(
                "Failed to generate presigned GET url: {err}"
            ))
        })?;

    Ok(presigned_req.uri().to_string())
}
