//! デバイス認可グラント (RFC 8628)。端末にコードを出し、利用者がブラウザで承認するまで
//! トークンエンドポイントをポーリングする。

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use tokio::time::Instant;

use super::{Discovery, TokenResponse, TokenResult, token_request};
use crate::{config::OidcConfig, http::error_for_status};

pub const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    /// 省略時は 5 秒 (RFC 8628 3.2)。
    pub interval: Option<u64>,
}

pub async fn start(
    http: &reqwest::Client,
    discovery: &Discovery,
    oidc: &OidcConfig,
) -> Result<DeviceAuthorization> {
    let endpoint = discovery
        .device_authorization_endpoint
        .as_deref()
        .ok_or_else(|| anyhow!("この IdP はデバイスコードログインに対応していません"))?;
    let mut form = vec![
        ("client_id", oidc.client_id.as_str()),
        ("scope", oidc.scope.as_str()),
    ];
    if let Some(resource) = &oidc.resource {
        form.push(("resource", resource));
    }
    let response = http
        .post(endpoint)
        .form(&form)
        .send()
        .await
        .context("デバイス認可エンドポイントに接続できません")?;
    Ok(error_for_status(response).await?.json().await?)
}

/// 承認されるまでポーリングする。`slow_down` を受けたら間隔を `slow_down_step` だけ延ばす。
pub struct Poller {
    pub slow_down_step: Duration,
}

impl Default for Poller {
    fn default() -> Self {
        Self {
            slow_down_step: Duration::from_secs(5),
        }
    }
}

impl Poller {
    pub async fn poll(
        &self,
        http: &reqwest::Client,
        discovery: &Discovery,
        oidc: &OidcConfig,
        authorization: &DeviceAuthorization,
    ) -> Result<TokenResponse> {
        let deadline = Instant::now() + Duration::from_secs(authorization.expires_in);
        let mut interval = Duration::from_secs(authorization.interval.unwrap_or(5));
        loop {
            tokio::time::sleep(interval).await;
            if Instant::now() >= deadline {
                bail!("コードの有効期限が切れました。もう一度ログインしてください");
            }
            let form = vec![
                ("grant_type", GRANT_TYPE.to_string()),
                ("device_code", authorization.device_code.clone()),
            ];
            match token_request(http, discovery, oidc, form).await? {
                TokenResult::Ok(tokens) => return Ok(tokens),
                TokenResult::Err { error, description } => match error.as_str() {
                    "authorization_pending" => {}
                    "slow_down" => interval += self.slow_down_step,
                    "access_denied" => bail!("ログインが拒否されました"),
                    "expired_token" => {
                        bail!("コードの有効期限が切れました。もう一度ログインしてください")
                    }
                    _ => bail!(
                        "ログインに失敗しました: {error}{}",
                        description.map(|d| format!(" ({d})")).unwrap_or_default()
                    ),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use serde_json::json;
    use wiremock::{
        Mock, MockServer, Request, Respond, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;

    fn oidc() -> OidcConfig {
        OidcConfig {
            issuer: "https://issuer".into(),
            client_id: "zoo2-tui".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        }
    }

    fn discovery(server: &MockServer) -> Discovery {
        Discovery {
            issuer: "https://issuer".into(),
            token_endpoint: format!("{}/token", server.uri()),
            device_authorization_endpoint: Some(format!("{}/device/code", server.uri())),
            revocation_endpoint: None,
        }
    }

    fn authorization(expires_in: u64) -> DeviceAuthorization {
        DeviceAuthorization {
            device_code: "dc".into(),
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://issuer/device".into(),
            verification_uri_complete: None,
            expires_in,
            interval: Some(0),
        }
    }

    /// 決めた順に応答を返す。
    struct Sequence {
        responses: Vec<ResponseTemplate>,
        calls: Arc<AtomicUsize>,
    }

    impl Respond for Sequence {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let index = self.calls.fetch_add(1, Ordering::SeqCst);
            self.responses[index.min(self.responses.len() - 1)].clone()
        }
    }

    fn oauth_error(error: &str) -> ResponseTemplate {
        ResponseTemplate::new(400).set_body_json(json!({ "error": error }))
    }

    fn poller() -> Poller {
        Poller {
            slow_down_step: Duration::from_millis(1),
        }
    }

    #[tokio::test]
    async fn start_sends_client_scope_and_resource() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/device/code"))
            .and(body_string_contains("client_id=zoo2-tui"))
            .and(body_string_contains("scope=openid+profile+offline_access"))
            .and(body_string_contains("resource=https%3A%2F%2Fzli.chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "dc", "user_code": "ABCD-EFGH",
                "verification_uri": "https://issuer/device",
                "verification_uri_complete": "https://issuer/device?user_code=ABCD-EFGH",
                "expires_in": 600, "interval": 5
            })))
            .expect(1)
            .mount(&server)
            .await;

        let authorization = start(&reqwest::Client::new(), &discovery(&server), &oidc())
            .await
            .unwrap();
        assert_eq!(authorization.user_code, "ABCD-EFGH");
        assert_eq!(authorization.interval, Some(5));
    }

    #[tokio::test]
    async fn polls_through_pending_and_slow_down_until_tokens() {
        let server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains(
                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code",
            ))
            .and(body_string_contains("device_code=dc"))
            .and(body_string_contains("resource=https%3A%2F%2Fzli.chat"))
            .respond_with(Sequence {
                responses: vec![
                    oauth_error("authorization_pending"),
                    oauth_error("slow_down"),
                    ResponseTemplate::new(200).set_body_json(json!({
                        "access_token": "at", "token_type": "Bearer",
                        "expires_in": 300, "refresh_token": "rt"
                    })),
                ],
                calls: calls.clone(),
            })
            .mount(&server)
            .await;

        let tokens = poller()
            .poll(
                &reqwest::Client::new(),
                &discovery(&server),
                &oidc(),
                &authorization(600),
            )
            .await
            .unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn stops_on_denied_expired_and_deadline() {
        for (error, expected) in [("access_denied", "拒否"), ("expired_token", "有効期限")] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(oauth_error(error))
                .mount(&server)
                .await;
            let err = poller()
                .poll(
                    &reqwest::Client::new(),
                    &discovery(&server),
                    &oidc(),
                    &authorization(600),
                )
                .await
                .unwrap_err();
            assert!(err.to_string().contains(expected), "{error}: {err}");
        }

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(oauth_error("authorization_pending"))
            .expect(0)
            .mount(&server)
            .await;
        let err = poller()
            .poll(
                &reqwest::Client::new(),
                &discovery(&server),
                &oidc(),
                &authorization(0),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("有効期限"));
    }
}
