use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

// 本番 (zli.chat) の接続先。設定ファイルや環境変数で何も指定しなくても使えるようにする。
// client_id は AuthZ に登録したクライアントごとの public client ID で、TUI・デスクトップ・モバイル等で分けて管理する。
const DEFAULT_API_BASE_URL: &str = "https://zli.chat";
const DEFAULT_OIDC_ISSUER: &str = "https://account.zli.works/api/auth";
const TUI_DEFAULT_OIDC_CLIENT_ID: &str = "ldwuljsObQHzxCdMwnTsYLTrCwgHaRii";
/// Tauri 版 (`desktop-tauri`) 向けの既定 client_id。AuthZ の「Zoo for Desktop」(デバイスコードと refresh_token を許可した public client)。
const DESKTOP_TAURI_DEFAULT_OIDC_CLIENT_ID: &str = "MwrfTPwsaaelMmxBdQGjWYuywWrsMHMe";
/// AuthZ に登録した API の URL で、backend の `OIDC_AUDIENCE` と完全に一致させる。これを要求すると `typ: at+jwt` のアクセストークンが得られる。
const DEFAULT_OIDC_RESOURCE: &str = "https://zli.chat/api";
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
    /// デスクトップ版 (Tauri) の画面の種類。`Config` には入れず、`desktop_ui_for` で読む。
    desktop_ui: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_for("zoo2-tui")
    }

    pub fn load_for(app_name: &str) -> Result<Self> {
        let file = match config_dir_for(app_name) {
            Some(dir) => read_file_config(&dir.join("config.toml"))?,
            None => FileConfig::default(),
        };
        let env: HashMap<String, String> = std::env::vars()
            .filter(|(key, _)| key.starts_with("ZOO_"))
            .collect();
        resolve_for(app_name, file, &env)
    }
}

/// デスクトップ版 (Tauri) の画面の種類 (`desktop_ui`、`ZOO_DESKTOP_UI`)。環境変数が優先される。
/// 値の検査と既定はデスクトップ版が決める。ほかのクライアントの起動に影響させないため、`Config` には入れない。
pub fn desktop_ui_for(app_name: &str) -> Result<Option<String>> {
    let file = match config_dir_for(app_name) {
        Some(dir) => read_file_config(&dir.join("config.toml"))?,
        None => FileConfig::default(),
    };
    Ok(pick_desktop_ui(
        file.desktop_ui,
        std::env::var("ZOO_DESKTOP_UI").ok(),
    ))
}

fn pick_desktop_ui(file: Option<String>, env: Option<String>) -> Option<String> {
    env.or(file)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_oidc_client_id(app_name: &str) -> &str {
    match app_name {
        "zoo2-tui" => TUI_DEFAULT_OIDC_CLIENT_ID,
        "zoo2-desktop-tauri" => DESKTOP_TAURI_DEFAULT_OIDC_CLIENT_ID,
        other => other,
    }
}

/// 設定ファイルとトークンを置くディレクトリ (`~/.config/zoo2-tui` など)。
pub fn config_dir() -> Option<PathBuf> {
    config_dir_for("zoo2-tui")
}

/// 指定したアプリ名の設定ディレクトリ (`~/.config/<app_name>` など)。
pub fn config_dir_for(app_name: &str) -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", app_name).map(|dirs| dirs.config_dir().to_path_buf())
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

/// 秘密を送ってよい接続先か。HTTPS と、この端末の中で閉じるループバックの HTTP。
pub(crate) fn is_secure_transport(url: &url::Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => match url.host() {
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        },
        _ => false,
    }
}

fn resolve_for(app_name: &str, file: FileConfig, env: &HashMap<String, String>) -> Result<Config> {
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
    let api_url = url::Url::parse(&api_base_url)
        .with_context(|| format!("api_base_url が URL ではありません: {api_base_url}"))?;

    let dev_user = pick("ZOO_DEV_USER", file.dev_user);
    let issuer = pick("ZOO_OIDC_ISSUER", file.oidc_issuer);
    let client_id = pick("ZOO_OIDC_CLIENT_ID", file.oidc_client_id);

    let auth = match dev_user {
        // dev_user を明示したときだけ dev モードにする。
        Some(user_id) => AuthConfig::Dev { user_id },
        None => AuthConfig::Oidc(OidcConfig {
            issuer: issuer
                .unwrap_or_else(|| DEFAULT_OIDC_ISSUER.to_string())
                .trim_end_matches('/')
                .to_string(),
            client_id: client_id.unwrap_or_else(|| default_oidc_client_id(app_name).to_string()),
            resource: Some(
                pick("ZOO_OIDC_RESOURCE", file.oidc_resource)
                    .unwrap_or_else(|| DEFAULT_OIDC_RESOURCE.to_string()),
            ),
            scope: pick("ZOO_OIDC_SCOPE", file.oidc_scope)
                .unwrap_or_else(|| DEFAULT_SCOPE.to_string()),
        }),
    };

    // ログインで得たトークンは、経路上で読める平文の HTTP には送らない。
    if let AuthConfig::Oidc(oidc) = &auth {
        if !is_secure_transport(&api_url) {
            bail!(
                "api_base_url は https:// にしてください (http:// はこの端末の中の localhost などに限ります): {api_base_url}"
            );
        }
        if !url::Url::parse(&oidc.issuer).is_ok_and(|issuer| is_secure_transport(&issuer)) {
            bail!(
                "oidc_issuer は https:// にしてください (http:// はこの端末の中の localhost などに限ります): {}",
                oidc.issuer
            );
        }
    }

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

    fn resolve(file: FileConfig, env: &HashMap<String, String>) -> Result<Config> {
        resolve_for("zoo2-tui", file, env)
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_to_production_login_with_per_app_client_id() {
        let tui = resolve_for("zoo2-tui", FileConfig::default(), &env(&[])).unwrap();
        assert_eq!(tui.api_base_url, "https://zli.chat");
        assert_eq!(
            tui.auth,
            AuthConfig::Oidc(OidcConfig {
                issuer: "https://account.zli.works/api/auth".into(),
                client_id: TUI_DEFAULT_OIDC_CLIENT_ID.into(),
                resource: Some("https://zli.chat/api".into()),
                scope: DEFAULT_SCOPE.into(),
            })
        );

        let tauri = resolve_for("zoo2-desktop-tauri", FileConfig::default(), &env(&[])).unwrap();
        assert_eq!(
            tauri.auth,
            AuthConfig::Oidc(OidcConfig {
                issuer: "https://account.zli.works/api/auth".into(),
                client_id: DESKTOP_TAURI_DEFAULT_OIDC_CLIENT_ID.into(),
                resource: Some("https://zli.chat/api".into()),
                scope: DEFAULT_SCOPE.into(),
            })
        );
        assert_ne!(
            TUI_DEFAULT_OIDC_CLIENT_ID,
            DESKTOP_TAURI_DEFAULT_OIDC_CLIENT_ID
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
    fn oidc_settings_override_the_defaults() {
        let file = FileConfig {
            oidc_issuer: Some("https://account.example/api/auth/".into()),
            oidc_client_id: Some("zoo2-tui".into()),
            oidc_resource: Some("https://example.test".into()),
            ..Default::default()
        };
        let config = resolve(file, &env(&[("ZOO_OIDC_CLIENT_ID", "from-env")])).unwrap();
        assert_eq!(
            config.auth,
            AuthConfig::Oidc(OidcConfig {
                issuer: "https://account.example/api/auth".into(),
                client_id: "from-env".into(),
                resource: Some("https://example.test".into()),
                scope: DEFAULT_SCOPE.into(),
            })
        );
    }

    #[test]
    fn login_requires_https_except_on_this_machine() {
        for (url, accepted) in [
            ("https://zoo.example", true),
            ("http://localhost:8000", true),
            ("http://127.0.0.1:8000", true),
            ("http://[::1]:8000", true),
            ("http://zoo.example", false),
            ("http://10.0.0.1:8000", false),
            ("ftp://zoo.example", false),
        ] {
            let config = resolve(FileConfig::default(), &env(&[("ZOO_API_BASE_URL", url)]));
            assert_eq!(config.is_ok(), accepted, "{url}");
        }
        for (issuer, accepted) in [
            ("https://account.example/api/auth", true),
            ("http://127.0.0.1:8080/api/auth", true),
            ("http://account.example/api/auth", false),
        ] {
            let config = resolve(FileConfig::default(), &env(&[("ZOO_OIDC_ISSUER", issuer)]));
            assert_eq!(config.is_ok(), accepted, "{issuer}");
        }
        // 開発用の `X-Dev-User` の接続は、トークンを送らないので HTTP でもよい。
        let dev = env(&[
            ("ZOO_API_BASE_URL", "http://10.0.0.1:8000"),
            ("ZOO_DEV_USER", "00000000-0000-0000-0000-000000000001"),
        ]);
        assert!(resolve(FileConfig::default(), &dev).is_ok());
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
    fn desktop_ui_prefers_env_over_file() {
        assert_eq!(super::pick_desktop_ui(None, None), None);
        assert_eq!(
            super::pick_desktop_ui(Some("leptos".into()), None).as_deref(),
            Some("leptos")
        );
        assert_eq!(
            super::pick_desktop_ui(Some("leptos".into()), Some(" remote ".into())).as_deref(),
            Some("remote")
        );
        assert_eq!(super::pick_desktop_ui(None, Some("".into())), None);
        // TUI などは desktop_ui を読まない。設定ファイルにあっても、値が何でも起動を止めない。
        let file: FileConfig = toml::from_str("desktop_ui = \"web\"").unwrap();
        assert!(resolve(file, &env(&[("ZOO_DESKTOP_UI", "web")])).is_ok());
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
