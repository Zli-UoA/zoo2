//! トークンの保存と更新。トークンは設定ディレクトリの `token.json` に、所有者だけが読める
//! 権限で置く。アクセストークンは期限の 60 秒前に refresh_token で更新する。

use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{Discovery, TokenResponse, TokenResult, discover, token_request};
use crate::{
    config::OidcConfig,
    http::{BearerSource, HttpError},
};

/// 期限までの余裕。frontend の `REFRESH_SKEW_MS` と同じ。
const REFRESH_SKEW_SECONDS: i64 = 60;
/// `expires_in` がない応答の既定値。frontend と同じく 300 秒とみなす。
const DEFAULT_EXPIRES_IN: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTokens {
    /// どの IdP とクライアントで得たトークンか。設定が変わったら使わない。
    pub issuer: String,
    pub client_id: String,
    pub resource: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// UNIX 秒。
    pub expires_at: i64,
}

impl StoredTokens {
    pub fn new(oidc: &OidcConfig, response: TokenResponse, now: i64) -> Self {
        Self {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_at: now + response.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
        }
    }

    fn matches(&self, oidc: &OidcConfig) -> bool {
        self.issuer == oidc.issuer
            && self.client_id == oidc.client_id
            && self.resource == oidc.resource
    }
}

pub fn token_path(config_dir: &Path) -> PathBuf {
    config_dir.join("token.json")
}

/// 保存済みのトークン。ないか、別の IdP やクライアントのものなら None。
pub fn load(path: &Path, oidc: &OidcConfig) -> Result<Option<StoredTokens>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("{} を読めません", path.display())),
    };
    let tokens: StoredTokens = serde_json::from_str(&text)
        .with_context(|| format!("{} を解釈できません", path.display()))?;
    Ok(tokens.matches(oidc).then_some(tokens))
}

pub fn save(path: &Path, tokens: &StoredTokens) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("{} を作れません", dir.display()))?;
    }
    // 一時ファイルに書いてから置き換え、書きかけのファイルを残さない。
    let temporary = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("{} に書けません", temporary.display()))?;
    std::io::Write::write_all(&mut file, serde_json::to_string_pretty(tokens)?.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

pub fn delete(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("{} を消せません", path.display())),
    }
}

pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

struct State {
    tokens: StoredTokens,
    discovery: Option<Discovery>,
}

/// REST とシェイプにアクセストークンを渡す。同時に期限切れを迎えても、ロックで更新を1回にまとめる。
pub struct TokenManager {
    http: reqwest::Client,
    oidc: OidcConfig,
    path: PathBuf,
    state: Mutex<State>,
}

impl TokenManager {
    pub fn new(
        http: reqwest::Client,
        oidc: OidcConfig,
        path: PathBuf,
        tokens: StoredTokens,
    ) -> Self {
        Self {
            http,
            oidc,
            path,
            state: Mutex::new(State {
                tokens,
                discovery: None,
            }),
        }
    }

    pub async fn access_token(&self) -> Result<String> {
        let mut state = self.state.lock().await;
        if state.tokens.expires_at - REFRESH_SKEW_SECONDS > now() {
            return Ok(state.tokens.access_token.clone());
        }
        let Some(refresh_token) = state.tokens.refresh_token.clone() else {
            return Err(HttpError::Unauthorized.into());
        };
        if state.discovery.is_none() {
            state.discovery = Some(discover(&self.http, &self.oidc.issuer).await?);
        }
        let discovery = state.discovery.as_ref().expect("直前に設定した");
        let form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.clone()),
        ];
        let response = match token_request(&self.http, discovery, &self.oidc, form).await? {
            TokenResult::Ok(response) => response,
            // invalid_grant などは refresh_token が使えないので、再ログインしてもらう。
            TokenResult::Err { .. } => return Err(HttpError::Unauthorized.into()),
        };
        let mut tokens = StoredTokens::new(&self.oidc, response, now());
        // ローテーションしない IdP では refresh_token が返らないので、前のものを使い続ける。
        tokens.refresh_token = tokens.refresh_token.or(Some(refresh_token));
        save(&self.path, &tokens)?;
        state.tokens = tokens;
        Ok(state.tokens.access_token.clone())
    }
}

impl BearerSource for TokenManager {
    fn bearer(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>> {
        Box::pin(self.access_token())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;

    fn oidc(issuer: &str) -> OidcConfig {
        OidcConfig {
            issuer: issuer.into(),
            client_id: "zoo2-tui".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        }
    }

    fn tokens(oidc: &OidcConfig, expires_at: i64, refresh: Option<&str>) -> StoredTokens {
        StoredTokens {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            access_token: "old".into(),
            refresh_token: refresh.map(str::to_string),
            expires_at,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zoo2-tui-test-{}-{name}", uuid::Uuid::new_v4()));
        token_path(&dir)
    }

    async fn issuer(server: &MockServer) -> String {
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer, "token_endpoint": format!("{}/api/auth/oauth2/token", server.uri())
            })))
            .mount(server)
            .await;
        issuer
    }

    #[test]
    fn saves_privately_and_ignores_tokens_of_other_clients() {
        let path = temp_path("save");
        let config = oidc("https://issuer");
        save(&path, &tokens(&config, 100, Some("rt"))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(
            load(&path, &config)
                .unwrap()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("rt")
        );
        let other = OidcConfig {
            client_id: "other".into(),
            ..config.clone()
        };
        assert_eq!(load(&path, &other).unwrap(), None);
        assert!(delete(&path).unwrap());
        assert!(!delete(&path).unwrap());
        assert_eq!(load(&path, &config).unwrap(), None);
    }

    #[tokio::test]
    async fn returns_fresh_token_without_network() {
        let config = oidc("http://127.0.0.1:9");
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            temp_path("fresh"),
            tokens(&config, now() + 600, None),
        );
        assert_eq!(manager.access_token().await.unwrap(), "old");
    }

    #[tokio::test]
    async fn refreshes_once_near_expiry_and_keeps_refresh_token() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=rt"))
            .and(body_string_contains("client_id=zoo2-tui"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "new", "expires_in": 300 })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let path = temp_path("refresh");
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            tokens(&config, now() + 30, Some("rt")),
        );
        let (a, b) = tokio::join!(manager.access_token(), manager.access_token());
        assert_eq!(
            (a.unwrap(), b.unwrap()),
            ("new".to_string(), "new".to_string())
        );
        let saved = load(&path, &config).unwrap().unwrap();
        assert_eq!(saved.access_token, "new");
        assert_eq!(saved.refresh_token.as_deref(), Some("rt"));
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn asks_for_login_when_refresh_is_impossible() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })),
            )
            .mount(&server)
            .await;

        for refresh in [None, Some("revoked")] {
            let manager = TokenManager::new(
                reqwest::Client::new(),
                config.clone(),
                temp_path("expired"),
                tokens(&config, now() - 1, refresh),
            );
            let err = manager.access_token().await.unwrap_err();
            assert!(matches!(
                err.downcast_ref::<HttpError>(),
                Some(HttpError::Unauthorized)
            ));
        }
    }
}
