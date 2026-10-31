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

    #[tokio::test]
    async fn test_generate_presigned_get_url_preserves_query_bytes_with_non_ascii_filename() {
        use crate::config::S3Config;
        use crate::storage::client::create_s3_client;

        let s3_config = S3Config {
            endpoint: "https://example.r2.cloudflarestorage.com".to_string(),
            bucket: "zoo-attachments-prod".to_string(),
            region: "auto".to_string(),
            access_key_id: "test_key".to_string(),
            secret_access_key: "test_secret".to_string(),
            force_path_style: true,
            presigned_put_expiry_secs: 900,
            presigned_get_expiry_secs: 300,
            max_file_size: 52428800,
            public_endpoint: None,
        };
        let client = create_s3_client(&s3_config);

        let japanese_filename = "日本語 ファイル名 (2026年) [テスト].png";
        let content_type = "image/png";
        let key = "attachments/ch1/att1";
        let expiry = Duration::from_secs(300);

        // 1. 公開エンドポイントなしで元の presigned GET URL を生成
        let original_url = generate_presigned_get_url(
            &client,
            &s3_config.bucket,
            key,
            content_type,
            japanese_filename,
            expiry,
            None,
        )
        .await
        .unwrap();

        // 2. 公開エンドポイントを指定して書き換え後の presigned GET URL を生成
        let public_endpoint = "https://attachments.zliusercontent.com";
        let rewritten_url = generate_presigned_get_url(
            &client,
            &s3_config.bucket,
            key,
            content_type,
            japanese_filename,
            expiry,
            Some(public_endpoint),
        )
        .await
        .unwrap();

        // ホスト名が書き換わっていることを確認
        assert!(
            rewritten_url.starts_with(
                "https://attachments.zliusercontent.com/zoo-attachments-prod/attachments/ch1/att1?"
            ),
            "rewritten url should start with public endpoint, got {rewritten_url}"
        );

        // クエリ文字列部分を取り出す
        let orig_query = original_url
            .split_once('?')
            .expect("original url should have query")
            .1;
        let rewritten_query = rewritten_url
            .split_once('?')
            .expect("rewritten url should have query")
            .1;

        // クエリに非 ASCII Content-Disposition 由来の % エンコードが含まれていることを確認
        assert!(
            orig_query.contains("response-content-disposition="),
            "query should contain response-content-disposition"
        );
        assert!(
            orig_query.contains("%"),
            "query should contain percent-encoded characters"
        );

        // 書き換え前後のクエリ文字列がバイト単位で完全一致することを検証
        assert_eq!(
            orig_query.as_bytes(),
            rewritten_query.as_bytes(),
            "query bytes must be preserved identically after URL rewriting"
        );
    }
}
