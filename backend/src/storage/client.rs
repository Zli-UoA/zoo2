use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Region;

use crate::config::S3Config;

/// S3Config から S3 クライアントを初期化する
pub fn create_s3_client(config: &S3Config) -> Client {
    let credentials = Credentials::new(
        &config.access_key_id,
        &config.secret_access_key,
        None,
        None,
        "static",
    );

    let region = Region::new(config.region.clone());

    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(region)
        .endpoint_url(&config.endpoint)
        .force_path_style(config.force_path_style)
        .build();

    Client::from_conf(s3_config)
}
