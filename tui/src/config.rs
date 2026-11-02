use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// frontend の `VITE_DEV_USER_ID` の例と同じ、dev モードの既定ユーザー。
pub const DEFAULT_DEV_USER: &str = "00000000-0000-0000-0000-000000000001";
const DEFAULT_API_BASE_URL: &str = "http://localhost:8000";
/// backend の初回ログインで UserInfo を呼ぶため `profile` を、トークン更新のため `offline_access` を要求する。
const DEFAULT_SCOPE: &str = "openid profile offline_access";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub api_base_url: String,
    pub auth: AuthConfig,
    /// 起動時に GitHub Releases の新しい版を調べるか。
    pub check_updates: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthConfig {
    /// backend の `DEV_AUTH_ENABLED=true` 向け。`X-Dev-User` でユーザーを指定する。
    Dev { user_id: String },
    /// AuthZ のデバイスコードログインで得たアクセストークンを使う。
    Oidc(OidcConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    /// RFC 8707 の resource。backend の `OIDC_AUDIENCE` と一致する JWT を得るために必要。
    pub resource: Option<String>,
    pub scope: String,
}

/// `config.toml` の内容。すべて省略可能で、環境変数が優先される。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    api_base_url: Option<String>,
    dev_user: Option<String>,
    oidc_issuer: Option<String>,
    oidc_client_id: Option<String>,
    oidc_resource: Option<String>,
    oidc_scope: Option<String>,
    check_updates: Option<bool>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let file = match config_dir() {
            Some(dir) => read_file_config(&dir.join("config.toml"))?,
            None => FileConfig::default(),
        };
        let env: HashMap<String, String> = std::env::vars()
            .filter(|(key, _)| key.starts_with("ZOO_"))
            .collect();
        resolve(file, &env)
    }
}

/// 設定ファイルとトークンを置くディレクトリ (`~/.config/zoo2-tui` など)。
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "zoo2-tui").map(|dirs| dirs.config_dir().to_path_buf())
}

fn read_file_config(path: &std::path::Path) -> Result<FileConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .with_context(|| format!("設定ファイルを読み込めません: {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(err) => {
            Err(err).with_context(|| format!("設定ファイルを開けません: {}", path.display()))
        }
    }
}

fn resolve(file: FileConfig, env: &HashMap<String, String>) -> Result<Config> {
    let pick = |key: &str, fallback: Option<String>| {
        env.get(key)
            .cloned()
            .or(fallback)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };

    let api_base_url = pick("ZOO_API_BASE_URL", file.api_base_url)
        .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_string())
        .trim_end_matches('/')
        .to_string();
    url::Url::parse(&api_base_url)
        .with_context(|| format!("api_base_url が URL ではありません: {api_base_url}"))?;

    let dev_user = pick("ZOO_DEV_USER", file.dev_user);
    let issuer = pick("ZOO_OIDC_ISSUER", file.oidc_issuer);
    let client_id = pick("ZOO_OIDC_CLIENT_ID", file.oidc_client_id);

    let auth = match (dev_user, issuer, client_id) {
        // dev_user を明示したときは OIDC 設定があっても dev モードにする。
        (Some(user_id), _, _) => AuthConfig::Dev { user_id },
        (None, Some(issuer), Some(client_id)) => AuthConfig::Oidc(OidcConfig {
            issuer: issuer.trim_end_matches('/').to_string(),
            client_id,
            resource: pick("ZOO_OIDC_RESOURCE", file.oidc_resource),
            scope: pick("ZOO_OIDC_SCOPE", file.oidc_scope)
                .unwrap_or_else(|| DEFAULT_SCOPE.to_string()),
        }),
        (None, Some(_), None) | (None, None, Some(_)) => {
            bail!("OIDC を使うには oidc_issuer と oidc_client_id の両方が必要です")
        }
        (None, None, None) => AuthConfig::Dev {
            user_id: DEFAULT_DEV_USER.to_string(),
        },
    };

    let check_updates = match pick("ZOO_CHECK_UPDATES", None) {
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => bail!("ZOO_CHECK_UPDATES は true か false にしてください: {value}"),
        },
        None => file.check_updates.unwrap_or(true),
    };

    Ok(Config {
        api_base_url,
        auth,
        check_updates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_to_local_dev_mode() {
        let config = resolve(FileConfig::default(), &env(&[])).unwrap();
        assert_eq!(config.api_base_url, "http://localhost:8000");
        assert_eq!(
            config.auth,
            AuthConfig::Dev {
                user_id: DEFAULT_DEV_USER.to_string()
            }
        );
    }

    #[test]
    fn env_overrides_file_and_trims_trailing_slash() {
        let file = FileConfig {
            api_base_url: Some("https://file.example".into()),
            ..Default::default()
        };
        let config = resolve(file, &env(&[("ZOO_API_BASE_URL", "https://env.example/")])).unwrap();
        assert_eq!(config.api_base_url, "https://env.example");
    }

    #[test]
    fn oidc_needs_issuer_and_client_id() {
        let file = FileConfig {
            oidc_issuer: Some("https://account.example/api/auth/".into()),
            oidc_client_id: Some("zoo2-tui".into()),
            oidc_resource: Some("https://zli.chat".into()),
            ..Default::default()
        };
        let config = resolve(file, &env(&[])).unwrap();
        assert_eq!(
            config.auth,
            AuthConfig::Oidc(OidcConfig {
                issuer: "https://account.example/api/auth".into(),
                client_id: "zoo2-tui".into(),
                resource: Some("https://zli.chat".into()),
                scope: DEFAULT_SCOPE.into(),
            })
        );

        let err = resolve(
            FileConfig::default(),
            &env(&[("ZOO_OIDC_ISSUER", "https://a")]),
        );
        assert!(err.is_err());
    }

    #[test]
    fn explicit_dev_user_wins_over_oidc() {
        let config = resolve(
            FileConfig::default(),
            &env(&[
                ("ZOO_DEV_USER", "11111111-1111-1111-1111-111111111111"),
                ("ZOO_OIDC_ISSUER", "https://a"),
                ("ZOO_OIDC_CLIENT_ID", "c"),
            ]),
        )
        .unwrap();
        assert!(matches!(config.auth, AuthConfig::Dev { .. }));
    }

    #[test]
    fn update_check_is_on_unless_disabled() {
        assert!(
            resolve(FileConfig::default(), &env(&[]))
                .unwrap()
                .check_updates
        );
        let file = FileConfig {
            check_updates: Some(false),
            ..Default::default()
        };
        assert!(!resolve(file, &env(&[])).unwrap().check_updates);
        let config = resolve(FileConfig::default(), &env(&[("ZOO_CHECK_UPDATES", "off")]));
        assert!(!config.unwrap().check_updates);
        assert!(
            resolve(
                FileConfig::default(),
                &env(&[("ZOO_CHECK_UPDATES", "maybe")])
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_invalid_url_and_unknown_keys() {
        assert!(
            resolve(
                FileConfig::default(),
                &env(&[("ZOO_API_BASE_URL", "not a url")])
            )
            .is_err()
        );
        assert!(toml::from_str::<FileConfig>("unknown = 1").is_err());
    }
}
