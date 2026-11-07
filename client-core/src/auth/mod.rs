//! AuthZ の OAuth 2.0 デバイス認可グラント (RFC 8628) によるログインと、トークンの保存・更新。

pub mod device;
pub mod token;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::OidcConfig;

/// `/.well-known/openid-configuration` のうち使うもの。
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub token_endpoint: String,
    pub device_authorization_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
}

pub async fn discover(http: &reqwest::Client, issuer: &str) -> Result<Discovery> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let discovery: Discovery = http
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .with_context(|| format!("OIDC の設定を取得できません: {url}"))?
        .json()
        .await
        .context("OIDC の設定を解釈できません")?;
    // 別の発行者の設定を掴まされないよう、issuer が一致することを確かめる。
    if discovery.issuer.trim_end_matches('/') != issuer.trim_end_matches('/') {
        bail!(
            "OIDC の issuer が一致しません: 設定は {issuer}、応答は {}",
            discovery.issuer
        );
    }
    Ok(discovery)
}

/// トークンエンドポイントの成功応答。
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: Option<i64>,
    pub refresh_token: Option<String>,
}

/// トークンエンドポイントのエラー応答 (RFC 6749 5.2)。
#[derive(Debug, Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

pub enum TokenResult {
    Ok(TokenResponse),
    Err {
        error: String,
        description: Option<String>,
    },
}

/// トークンエンドポイントにフォームを送る。`resource` があれば RFC 8707 の resource も付ける。
pub async fn token_request(
    http: &reqwest::Client,
    discovery: &Discovery,
    oidc: &OidcConfig,
    mut form: Vec<(&str, String)>,
) -> Result<TokenResult> {
    form.push(("client_id", oidc.client_id.clone()));
    if let Some(resource) = &oidc.resource {
        form.push(("resource", resource.clone()));
    }
    let response = http
        .post(&discovery.token_endpoint)
        .form(&form)
        .send()
        .await
        .context("トークンエンドポイントに接続できません")?;
    let status = response.status();
    let body = response.text().await?;
    if status.is_success() {
        return Ok(TokenResult::Ok(
            serde_json::from_str(&body).context("トークンの応答を解釈できません")?,
        ));
    }
    match serde_json::from_str::<OAuthError>(&body) {
        Ok(err) => Ok(TokenResult::Err {
            error: err.error,
            description: err.error_description,
        }),
        Err(_) => bail!("トークンエンドポイントがエラーを返しました: {status}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;

    #[tokio::test]
    async fn rejects_mismatched_issuer() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": "https://evil.example/api/auth",
                "token_endpoint": "https://evil.example/token"
            })))
            .mount(&server)
            .await;

        let issuer = format!("{}/api/auth", server.uri());
        let err = discover(&reqwest::Client::new(), &issuer)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("issuer が一致しません"));
    }
}
