//! E2EE の直前の再認証（docs/e2ee.md の「初回設定と解錠の流れ」の 7 と「クロス署名」の API。#360）。
//!
//! TUI とデスクトップは、ふだんのログインにデバイス認可グラントを使うので、`max_age` を付けた
//! ID トークンを取れない。再認証のときだけ、認可コードフロー（PKCE、`max_age=0`、`state`、`nonce`）を
//! ブラウザーで開き、ループバックのリダイレクト（`http://127.0.0.1:<空いているポート>/callback`、
//! RFC 8252 の 7.3）で認可コードを受け取って、トークンエンドポイントで ID トークンに換える。
//! 今のセッションのトークンは変えない。受け取った ID トークンは `X-Zoo-Reauth` に入れる。
//!
//! PKCE の code verifier と `nonce` はメモリーにだけ置く。

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{Discovery, TokenResult, token_request};
use crate::config::OidcConfig;

/// ループバックのリダイレクト先のパス。
pub const CALLBACK_PATH: &str = "/callback";
/// 利用者がログインし直すのを待つ時間（Web と同じ 10 分）。
pub const REAUTH_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// 1 つの接続から要求を読む期限。
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// ID トークンの `exp` の許容のずれ（Web と同じ 60 秒）。
const CLOCK_SKEW_SECS: i64 = 60;

/// 再認証の失敗。
#[derive(Debug, thiserror::Error)]
pub enum ReauthError {
    /// AuthZ がエラーを返した、または利用者が取り消した。
    #[error("再認証が拒否されました: {0}")]
    Denied(String),
    /// 時間内に終わらなかった。
    #[error("再認証が時間内に終わりませんでした")]
    Timeout,
    /// ID トークンを受け付けられない（iss、aud、exp、nonce の不一致など）。
    #[error("ID トークンを受け付けられません: {0}")]
    InvalidIdToken(String),
}

/// 待ち受けを始めた再認証。[`LoopbackReauth::url`] をブラウザーで開いてもらい、
/// [`LoopbackReauth::finish`] で ID トークンを受け取る。
pub struct LoopbackReauth {
    listener: TcpListener,
    redirect_uri: String,
    url: String,
    state: String,
    nonce: String,
    verifier: String,
}

impl std::fmt::Debug for LoopbackReauth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // verifier と nonce は出さない。
        f.debug_struct("LoopbackReauth")
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

/// UUID v4 2 つ分（244 ビットの乱数）の 64 文字。PKCE の verifier（43〜128 文字の unreserved）にも使える。
fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// PKCE の S256 の code challenge。
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// 再認証では refresh token を作らない（`offline_access` を外す）。ID トークンには `openid` が要る。
fn reauth_scope(scope: &str) -> String {
    let mut scopes: Vec<&str> = scope
        .split_whitespace()
        .filter(|scope| *scope != "offline_access")
        .collect();
    if !scopes.contains(&"openid") {
        scopes.insert(0, "openid");
    }
    scopes.join(" ")
}

impl LoopbackReauth {
    /// `127.0.0.1` の空いているポートで待ち受けを始め、認可リクエストの URL を作る。
    pub async fn start(discovery: &Discovery, oidc: &OidcConfig) -> Result<Self> {
        let endpoint = discovery
            .authorization_endpoint
            .as_deref()
            .ok_or_else(|| anyhow!("この IdP は認可コードフローに対応していません"))?;
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("再認証の受け口を開けません")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let state = random_token();
        let nonce = random_token();
        let verifier = random_token();
        let mut url = url::Url::parse(endpoint).context("認可エンドポイントの URL が不正です")?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &oidc.client_id)
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", &reauth_scope(&oidc.scope));
            if let Some(resource) = &oidc.resource {
                query.append_pair("resource", resource);
            }
            query
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge", &code_challenge(&verifier))
                .append_pair("code_challenge_method", "S256")
                // 既存の AuthZ のセッションがあっても、必ずログインし直させる。
                .append_pair("max_age", "0");
        }
        Ok(Self {
            listener,
            redirect_uri,
            url: url.into(),
            state,
            nonce,
            verifier,
        })
    }

    /// ブラウザーで開く URL。
    pub fn url(&self) -> &str {
        &self.url
    }

    /// 待ち受けているポート（SSH の先で動かしているときの `ssh -L` の案内に使う）。
    pub fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .map(|addr| addr.port())
            .unwrap_or_default()
    }

    /// リダイレクト先の URI。
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// 認可コードを受け取り、ID トークンに換える。待つのは `timeout` まで。
    pub async fn finish(
        self,
        http: &reqwest::Client,
        discovery: &Discovery,
        oidc: &OidcConfig,
        timeout: Duration,
    ) -> Result<String> {
        let code = tokio::time::timeout(timeout, self.receive_code())
            .await
            .map_err(|_| ReauthError::Timeout)??;
        let form = vec![
            ("grant_type", "authorization_code".to_owned()),
            ("code", code),
            ("redirect_uri", self.redirect_uri.clone()),
            ("code_verifier", self.verifier.clone()),
        ];
        // 成功応答の `access_token` と `refresh_token` は使わない（今のセッションのトークンは変えない）。
        let id_token = match token_request(http, discovery, oidc, form).await? {
            TokenResult::Ok(tokens) => tokens
                .id_token
                .ok_or_else(|| ReauthError::InvalidIdToken("id_token がありません".into()))?,
            TokenResult::Err { error, description } => {
                return Err(ReauthError::Denied(format!(
                    "{error}{}",
                    description.map(|d| format!(" ({d})")).unwrap_or_default()
                ))
                .into());
            }
        };
        validate_id_token(
            &id_token,
            &discovery.issuer,
            &oidc.client_id,
            &self.nonce,
            crate::auth::token::now(),
        )?;
        Ok(id_token)
    }

    /// 自分の `state` のリダイレクトが来るまで接続を受ける。ほかの要求（favicon など）や、
    /// `state` が違う要求には答えるだけで、待ち続ける。
    async fn receive_code(&self) -> Result<String> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            match handle_connection(stream, &self.state).await {
                Ok(Some(Callback::Code(code))) => return Ok(code),
                Ok(Some(Callback::Error(error))) => return Err(ReauthError::Denied(error).into()),
                // 読めない接続は捨てて待ち続ける。
                Ok(None) | Err(_) => {}
            }
        }
    }
}

enum Callback {
    Code(String),
    Error(String),
}

const DONE_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>zoo2-tui</title>\
<p>再認証が終わりました。このタブを閉じて、zoo2-tui に戻ってください。</p>";
const ERROR_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>zoo2-tui</title>\
<p>再認証できませんでした。zoo2-tui に戻って、表示を確かめてください。</p>";

async fn handle_connection(mut stream: TcpStream, state: &str) -> Result<Option<Callback>> {
    let target = tokio::time::timeout(READ_TIMEOUT, read_request_target(&mut stream))
        .await
        .map_err(|_| anyhow!("要求を読めません"))??;
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
    if url.path() != CALLBACK_PATH {
        respond(&mut stream, "404 Not Found", "").await;
        return Ok(None);
    }
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if param("state").as_deref() != Some(state) {
        respond(&mut stream, "400 Bad Request", ERROR_PAGE).await;
        return Ok(None);
    }
    if let Some(error) = param("error") {
        respond(&mut stream, "200 OK", ERROR_PAGE).await;
        let description = param("error_description")
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        return Ok(Some(Callback::Error(format!("{error}{description}"))));
    }
    match param("code").filter(|code| !code.is_empty()) {
        Some(code) => {
            respond(&mut stream, "200 OK", DONE_PAGE).await;
            Ok(Some(Callback::Code(code)))
        }
        None => {
            respond(&mut stream, "200 OK", ERROR_PAGE).await;
            Ok(Some(Callback::Error("認可コードがありません".into())))
        }
    }
}

/// 要求行の対象（`GET <対象> HTTP/1.1`）を読む。ヘッダーの終わりまで読み捨てる。
async fn read_request_target(stream: &mut TcpStream) -> Result<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 16 * 1024 {
            bail!("要求が長すぎます");
        }
    }
    let text = String::from_utf8_lossy(&buffer);
    let line = text.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Ok(target.to_owned()),
        _ => bail!("GET の要求ではありません"),
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[derive(Deserialize)]
struct IdTokenClaims {
    iss: Option<String>,
    aud: Option<serde_json::Value>,
    exp: Option<i64>,
    nonce: Option<String>,
}

/// ID トークンの iss、aud、exp、nonce を確かめる（Web の `validateIdToken` と同じ。署名はサーバーが
/// AuthZ の JWKS で確かめる）。
pub fn validate_id_token(
    id_token: &str,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now: i64,
) -> Result<(), ReauthError> {
    let invalid = |message: &str| ReauthError::InvalidIdToken(message.to_owned());
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or_else(|| invalid("JWT の形式ではありません"))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| invalid("ペイロードを読めません"))?;
    let claims: IdTokenClaims =
        serde_json::from_slice(&bytes).map_err(|_| invalid("ペイロードを読めません"))?;
    if claims.iss.as_deref().map(|iss| iss.trim_end_matches('/'))
        != Some(issuer.trim_end_matches('/'))
    {
        return Err(invalid("iss が一致しません"));
    }
    let audience_ok = match &claims.aud {
        Some(serde_json::Value::String(aud)) => aud == client_id,
        Some(serde_json::Value::Array(auds)) => auds.iter().any(|aud| aud == client_id),
        _ => false,
    };
    if !audience_ok {
        return Err(invalid("aud が一致しません"));
    }
    if claims.exp.is_none_or(|exp| exp <= now - CLOCK_SKEW_SECS) {
        return Err(invalid("期限切れです"));
    }
    if claims.nonce.as_deref() != Some(nonce) {
        return Err(invalid("nonce が一致しません"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;

    const ISSUER: &str = "https://issuer.example/api/auth";

    fn oidc() -> OidcConfig {
        OidcConfig {
            issuer: ISSUER.into(),
            client_id: "zoo2-tui".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        }
    }

    fn discovery(server: &MockServer) -> Discovery {
        Discovery {
            issuer: ISSUER.into(),
            token_endpoint: format!("{}/token", server.uri()),
            authorization_endpoint: Some("https://issuer.example/api/auth/oauth2/authorize".into()),
            device_authorization_endpoint: None,
            revocation_endpoint: None,
        }
    }

    fn id_token(claims: serde_json::Value) -> String {
        let encode = |value: &serde_json::Value| URL_SAFE_NO_PAD.encode(value.to_string());
        format!(
            "{}.{}.sig",
            encode(&json!({"alg": "RS256"})),
            encode(&claims)
        )
    }

    fn query(url: &str, name: &str) -> String {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    }

    #[test]
    fn validates_the_id_token_claims() {
        let now = 1_000_000;
        let good = json!({"iss": ISSUER, "aud": ["zoo2-tui"], "exp": now + 300, "nonce": "n"});
        assert!(validate_id_token(&id_token(good.clone()), ISSUER, "zoo2-tui", "n", now).is_ok());
        for (field, value) in [
            ("iss", json!("https://evil.example")),
            ("aud", json!("zoo2-web")),
            ("exp", json!(now - 61)),
            ("nonce", json!("other")),
        ] {
            let mut claims = good.clone();
            claims[field] = value;
            assert!(
                validate_id_token(&id_token(claims), ISSUER, "zoo2-tui", "n", now).is_err(),
                "{field} の不一致を受け付けた"
            );
        }
        assert!(validate_id_token("not-a-jwt", ISSUER, "zoo2-tui", "n", now).is_err());
    }

    #[test]
    fn reauth_scope_drops_offline_access() {
        assert_eq!(
            reauth_scope("openid profile offline_access"),
            "openid profile"
        );
        assert_eq!(reauth_scope("profile"), "openid profile");
    }

    #[tokio::test]
    async fn receives_the_code_on_the_loopback_and_exchanges_it_for_an_id_token() {
        let server = MockServer::start().await;
        let reauth = LoopbackReauth::start(&discovery(&server), &oidc())
            .await
            .unwrap();
        let url = reauth.url().to_owned();
        assert_eq!(query(&url, "max_age"), "0");
        assert_eq!(query(&url, "response_type"), "code");
        assert_eq!(query(&url, "code_challenge_method"), "S256");
        assert_eq!(query(&url, "scope"), "openid profile");
        assert_eq!(query(&url, "resource"), "https://zli.chat");
        assert_eq!(query(&url, "redirect_uri"), reauth.redirect_uri());
        assert!(reauth.redirect_uri().starts_with("http://127.0.0.1:"));
        let nonce = query(&url, "nonce");
        let state = query(&url, "state");
        let token = id_token(json!({
            "iss": ISSUER, "aud": "zoo2-tui", "exp": crate::auth::token::now() + 300, "nonce": nonce,
        }));
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .and(body_string_contains("code_verifier="))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "at", "token_type": "Bearer", "id_token": token,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let redirect = reauth.redirect_uri().to_owned();
        let browser = tokio::spawn(async move {
            let http = reqwest::Client::new();
            // 別の要求と、state の違うリダイレクトは無視して待ち続ける。
            let favicon = redirect.replace(CALLBACK_PATH, "/favicon.ico");
            assert_eq!(http.get(&favicon).send().await.unwrap().status(), 404);
            let forged = http
                .get(format!("{redirect}?code=forged&state=other"))
                .send()
                .await
                .unwrap();
            assert_eq!(forged.status(), 400);
            let ok = http
                .get(format!("{redirect}?code=the-code&state={state}"))
                .send()
                .await
                .unwrap();
            assert_eq!(ok.status(), 200);
            assert!(ok.text().await.unwrap().contains("再認証が終わりました"));
        });
        let received = reauth
            .finish(
                &reqwest::Client::new(),
                &discovery(&server),
                &oidc(),
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        browser.await.unwrap();
        assert_eq!(received, token);
    }

    #[tokio::test]
    async fn reports_denial_and_timeout() {
        let server = MockServer::start().await;
        let reauth = LoopbackReauth::start(&discovery(&server), &oidc())
            .await
            .unwrap();
        let state = query(reauth.url(), "state");
        let redirect = reauth.redirect_uri().to_owned();
        tokio::spawn(async move {
            let _ = reqwest::get(format!("{redirect}?error=access_denied&state={state}")).await;
        });
        let err = reauth
            .finish(
                &reqwest::Client::new(),
                &discovery(&server),
                &oidc(),
                Duration::from_secs(10),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ReauthError>(),
            Some(ReauthError::Denied(_))
        ));

        let reauth = LoopbackReauth::start(&discovery(&server), &oidc())
            .await
            .unwrap();
        let err = reauth
            .finish(
                &reqwest::Client::new(),
                &discovery(&server),
                &oidc(),
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ReauthError>(),
            Some(ReauthError::Timeout)
        ));
    }
}
