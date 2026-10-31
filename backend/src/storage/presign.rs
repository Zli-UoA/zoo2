use aws_sdk_s3::Client;
use aws_sdk_s3::presigning::PresigningConfig;
use std::time::Duration;

use super::format::{format_content_disposition, is_inline_allowed_mime};
use crate::error::AppError;

/// Presigned PUT URL を生成する
/// 署名に Content-Length と Content-Type を含めて強制する
/// `public_endpoint` が指定されている場合はホスト名を書き換えて返却する
pub async fn generate_presigned_put_url(
    client: &Client,
    bucket: &str,
    key: &str,
    content_length: i64,
    content_type: &str,
    expiry: Duration,
    public_endpoint: Option<&str>,
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

    rewrite_presigned_url(presigned_req.uri(), public_endpoint)
}

/// Presigned GET URL を生成する
/// response-content-type と response-content-disposition (RFC 6266/5987) を署名に付与する
/// `public_endpoint` が指定されている場合はホスト名を書き換えて返却する
pub async fn generate_presigned_get_url(
    client: &Client,
    bucket: &str,
    key: &str,
    content_type: &str,
    file_name: &str,
    expiry: Duration,
    public_endpoint: Option<&str>,
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

    rewrite_presigned_url(presigned_req.uri(), public_endpoint)
}

/// Presigned URL のオリジン（scheme, host, port）を公開用エンドポイントに書き換える
///
/// `public_endpoint` が未指定（`None` または空文字）の場合は `url_str` をそのまま返す。
/// 指定されている場合は、クエリパラメータやパスを維持したまま、オリジン（scheme + host + port）を
/// `public_endpoint` のものに置き換える。
pub fn rewrite_presigned_url(
    url_str: &str,
    public_endpoint: Option<&str>,
) -> Result<String, AppError> {
    let Some(public_endpoint) = public_endpoint.filter(|s| !s.trim().is_empty()) else {
        return Ok(url_str.to_string());
    };

    let mut url = reqwest::Url::parse(url_str).map_err(|err| {
        AppError::Internal(anyhow::anyhow!("Failed to parse presigned url: {err}"))
    })?;
    let public_url = reqwest::Url::parse(public_endpoint)
        .map_err(|err| AppError::Internal(anyhow::anyhow!("Invalid public endpoint: {err}")))?;

    url.set_scheme(public_url.scheme()).map_err(|_| {
        AppError::Internal(anyhow::anyhow!("Failed to set scheme on presigned url"))
    })?;
    url.set_host(public_url.host_str())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("Failed to set host on presigned url")))?;
    url.set_port(public_url.port())
        .map_err(|_| AppError::Internal(anyhow::anyhow!("Failed to set port on presigned url")))?;

    let prefix = public_url.path().trim_matches('/');
    if !prefix.is_empty() {
        let original_path = url.path().trim_start_matches('/');
        url.set_path(&format!("{prefix}/{original_path}"));
    }

    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_presigned_url_none() {
        let original = "https://r2.example.com/bucket/key?X-Amz-Signature=abc&X-Amz-Date=123";
        assert_eq!(rewrite_presigned_url(original, None).unwrap(), original);
        assert_eq!(rewrite_presigned_url(original, Some("")).unwrap(), original);
        assert_eq!(
            rewrite_presigned_url(original, Some("   ")).unwrap(),
            original
        );
    }

    #[test]
    fn test_rewrite_presigned_url_custom_domain() {
        let original = "https://c9918f2caa050736a07867e6f968c0a2.r2.cloudflarestorage.com/zoo-attachments-prod/attachments/ch1/att1?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=test%2F20261031%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20261031T000000Z&X-Amz-Expires=900&X-Amz-Signature=abcdef";
        let public_endpoint = "https://attachments.zliusercontent.com";

        let rewritten = rewrite_presigned_url(original, Some(public_endpoint)).unwrap();
        assert_eq!(
            rewritten,
            "https://attachments.zliusercontent.com/zoo-attachments-prod/attachments/ch1/att1?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=test%2F20261031%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20261031T000000Z&X-Amz-Expires=900&X-Amz-Signature=abcdef"
        );
    }

    #[test]
    fn test_rewrite_presigned_url_with_port() {
        let original = "http://localhost:9000/bucket/key?query=1";
        let public_endpoint = "http://127.0.0.1:8787";

        let rewritten = rewrite_presigned_url(original, Some(public_endpoint)).unwrap();
        assert_eq!(rewritten, "http://127.0.0.1:8787/bucket/key?query=1");
    }

    #[test]
    fn test_rewrite_presigned_url_with_path_prefix() {
        let original = "https://r2.example.com/bucket/key?query=1";
        let public_endpoint = "https://attachments.zliusercontent.com/storage";

        let rewritten = rewrite_presigned_url(original, Some(public_endpoint)).unwrap();
        assert_eq!(
            rewritten,
            "https://attachments.zliusercontent.com/storage/bucket/key?query=1"
        );
    }
}
