//! 通話の SFU の設定（docs/calls-sfu.md）。
//!
//! `CALL_SFUS_JSON` が未設定か空なら SFU は無効で、通話は今までどおり P2P mesh だけで動く。
//! 設定するときは、Cloudflare Realtime は必須（LAN の外の人と、LiveKit に届かない人の行き先）。
//! LiveKit と sfu-bridge は任意。秘密の値は別の環境変数で受け取る。

use std::{env, fmt, net::IpAddr};

use serde::Deserialize;

pub const DEFAULT_SFU_THRESHOLD: usize = 4;
pub const DEFAULT_SFU_MAX_PARTICIPANTS: usize = 50;
pub const DEFAULT_CF_API_BASE: &str = "https://rtc.live.cloudflare.com/v1";

#[derive(Clone)]
pub struct CallSfuConfig {
    /// mesh の人数がこれを超えたら（全員が対応していれば）SFU に切り替える。
    pub threshold: usize,
    /// SFU モードの定員。
    pub max_participants: usize,
    pub cloudflare: CloudflareConfig,
    pub livekit: Option<LiveKitConfig>,
    /// sfu-bridge が backend の内部 API を呼ぶときのトークン。`None` なら bridge は無効で、
    /// Cloudflare の参加者がいる通話では LiveKit に割り当てない。
    pub bridge_token: Option<String>,
}

#[derive(Clone)]
pub struct CloudflareConfig {
    pub api_base: String,
    pub app_id: String,
    pub app_secret: String,
}

#[derive(Clone)]
pub struct LiveKitConfig {
    /// クライアントがつなぐ wss の URL（Cloudflare Tunnel 経由）。
    pub ws_url: String,
    /// backend から Twirp API を呼ぶ URL（クラスタ内）。
    pub api_url: String,
    pub api_key: String,
    pub api_secret: String,
    /// この出口 IP（NAT 後）から来た人だけを LiveKit に割り当てる。
    pub lan_egress: Vec<Cidr>,
}

impl fmt::Debug for CallSfuConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallSfuConfig")
            .field("threshold", &self.threshold)
            .field("max_participants", &self.max_participants)
            .field("cloudflare_api_base", &self.cloudflare.api_base)
            .field("livekit_ws_url", &self.livekit.as_ref().map(|l| &l.ws_url))
            .field(
                "livekit_lan_egress",
                &self.livekit.as_ref().map(|l| l.lan_egress.len()),
            )
            .field("bridge", &self.bridge_token.is_some())
            .finish()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    cloudflare: RawCloudflare,
    #[serde(default)]
    livekit: Option<RawLiveKit>,
    #[serde(default)]
    bridge: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCloudflare {
    #[serde(default)]
    api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLiveKit {
    ws_url: String,
    api_url: String,
    lan_egress_cidrs: Vec<String>,
}

fn secret(name: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    lookup(name)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn usize_var(
    name: &str,
    default: usize,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<usize, String> {
    match secret(name, lookup) {
        None => Ok(default),
        Some(v) => v
            .parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("{name} must be a positive integer")),
    }
}

impl CallSfuConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_lookup(&|name| env::var(name).ok())
    }

    /// 環境変数の読み取りを差し替えられる形（テスト用）。
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let Some(json) = secret("CALL_SFUS_JSON", lookup) else {
            // SFU を使わないのに秘密の値だけがあるのは設定ミス。
            for name in ["CF_REALTIME_APP_ID", "LIVEKIT_API_KEY", "SFU_BRIDGE_TOKEN"] {
                if secret(name, lookup).is_some() {
                    return Err(format!("{name} is set but CALL_SFUS_JSON is empty"));
                }
            }
            return Ok(None);
        };
        let raw: RawConfig = serde_json::from_str(&json)
            .map_err(|e| format!("Invalid JSON in CALL_SFUS_JSON: {e}"))?;

        let app_id = secret("CF_REALTIME_APP_ID", lookup);
        let app_secret = secret("CF_REALTIME_APP_SECRET", lookup);
        let (Some(app_id), Some(app_secret)) = (app_id, app_secret) else {
            return Err(
                "CF_REALTIME_APP_ID and CF_REALTIME_APP_SECRET are required with CALL_SFUS_JSON"
                    .into(),
            );
        };
        let api_base = raw
            .cloudflare
            .api_base
            .unwrap_or_else(|| DEFAULT_CF_API_BASE.to_string())
            .trim_end_matches('/')
            .to_string();
        require_http(&api_base, "cloudflare.api_base", true)?;

        let livekit = match raw.livekit {
            None => {
                if secret("LIVEKIT_API_KEY", lookup).is_some()
                    || secret("LIVEKIT_API_SECRET", lookup).is_some()
                {
                    return Err("LIVEKIT_API_KEY is set but CALL_SFUS_JSON has no livekit".into());
                }
                None
            }
            Some(lk) => {
                let (Some(api_key), Some(api_secret)) = (
                    secret("LIVEKIT_API_KEY", lookup),
                    secret("LIVEKIT_API_SECRET", lookup),
                ) else {
                    return Err(
                        "LIVEKIT_API_KEY and LIVEKIT_API_SECRET are required with livekit in CALL_SFUS_JSON"
                            .into(),
                    );
                };
                if api_secret.len() < 32 {
                    return Err("LIVEKIT_API_SECRET must be at least 32 characters".into());
                }
                let ws_url = lk.ws_url.trim().trim_end_matches('/').to_string();
                if !(ws_url.starts_with("wss://") || ws_url.starts_with("ws://")) {
                    return Err("livekit.ws_url must start with wss:// or ws://".into());
                }
                let api_url = lk.api_url.trim().trim_end_matches('/').to_string();
                require_http(&api_url, "livekit.api_url", false)?;
                if lk.lan_egress_cidrs.is_empty() {
                    return Err("livekit.lan_egress_cidrs must not be empty".into());
                }
                let lan_egress = lk
                    .lan_egress_cidrs
                    .iter()
                    .map(|c| Cidr::parse(c))
                    .collect::<Result<Vec<_>, _>>()?;
                Some(LiveKitConfig {
                    ws_url,
                    api_url,
                    api_key,
                    api_secret,
                    lan_egress,
                })
            }
        };

        let bridge_token = secret("SFU_BRIDGE_TOKEN", lookup);
        let bridge_token = match (raw.bridge, bridge_token) {
            (false, None) => None,
            (false, Some(_)) => {
                return Err("SFU_BRIDGE_TOKEN is set but CALL_SFUS_JSON has bridge: false".into());
            }
            (true, None) => {
                return Err("SFU_BRIDGE_TOKEN is required with bridge: true".into());
            }
            (true, Some(token)) => {
                if livekit.is_none() {
                    return Err("bridge: true requires livekit in CALL_SFUS_JSON".into());
                }
                if token.len() < 32 {
                    return Err("SFU_BRIDGE_TOKEN must be at least 32 characters".into());
                }
                Some(token)
            }
        };

        let threshold = usize_var("CALL_SFU_THRESHOLD", DEFAULT_SFU_THRESHOLD, lookup)?;
        let max_participants = usize_var(
            "CALL_SFU_MAX_PARTICIPANTS",
            DEFAULT_SFU_MAX_PARTICIPANTS,
            lookup,
        )?;
        if threshold >= crate::routes::calls::MAX_CALL_PARTICIPANTS {
            return Err(format!(
                "CALL_SFU_THRESHOLD must be less than the mesh capacity ({})",
                crate::routes::calls::MAX_CALL_PARTICIPANTS
            ));
        }
        if max_participants <= threshold {
            return Err("CALL_SFU_MAX_PARTICIPANTS must be greater than CALL_SFU_THRESHOLD".into());
        }

        Ok(Some(Self {
            threshold,
            max_participants,
            cloudflare: CloudflareConfig {
                api_base,
                app_id,
                app_secret,
            },
            livekit,
            bridge_token,
        }))
    }
}

fn require_http(url: &str, field: &str, https_only: bool) -> Result<(), String> {
    let ok = url.starts_with("https://") || (!https_only && url.starts_with("http://"));
    if ok {
        Ok(())
    } else if https_only {
        Err(format!("{field} must start with https://"))
    } else {
        Err(format!("{field} must start with http:// or https://"))
    }
}

/// IPv4 / IPv6 の CIDR。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (text, None),
        };
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("Invalid CIDR '{text}' in livekit.lan_egress_cidrs"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("Invalid CIDR prefix in '{text}'"))?,
        };
        Ok(Self {
            addr: addr.to_canonical(),
            prefix,
        })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    const SECRET32: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn unset_is_disabled() {
        assert!(CallSfuConfig::from_lookup(&lookup(&[])).unwrap().is_none());
        assert!(
            CallSfuConfig::from_lookup(&lookup(&[("CALL_SFUS_JSON", "  ")]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn secrets_without_json_is_an_error() {
        let err = CallSfuConfig::from_lookup(&lookup(&[("CF_REALTIME_APP_ID", "x")])).unwrap_err();
        assert!(err.contains("CALL_SFUS_JSON"));
    }

    #[test]
    fn cloudflare_only() {
        let cfg = CallSfuConfig::from_lookup(&lookup(&[
            ("CALL_SFUS_JSON", r#"{"cloudflare":{}}"#),
            ("CF_REALTIME_APP_ID", "app"),
            ("CF_REALTIME_APP_SECRET", "secret"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(cfg.cloudflare.api_base, DEFAULT_CF_API_BASE);
        assert!(cfg.livekit.is_none());
        assert!(cfg.bridge_token.is_none());
        assert_eq!(cfg.threshold, DEFAULT_SFU_THRESHOLD);
        assert_eq!(cfg.max_participants, DEFAULT_SFU_MAX_PARTICIPANTS);
    }

    #[test]
    fn cloudflare_secret_is_required() {
        let err = CallSfuConfig::from_lookup(&lookup(&[
            ("CALL_SFUS_JSON", r#"{"cloudflare":{}}"#),
            ("CF_REALTIME_APP_ID", "app"),
        ]))
        .unwrap_err();
        assert!(err.contains("CF_REALTIME_APP_SECRET"));
    }

    #[test]
    fn livekit_and_bridge() {
        let json = r#"{"cloudflare":{},"bridge":true,"livekit":{"ws_url":"wss://livekit.example/","api_url":"http://livekit:7880","lan_egress_cidrs":["203.0.113.4","2001:db8::/48"]}}"#;
        let cfg = CallSfuConfig::from_lookup(&lookup(&[
            ("CALL_SFUS_JSON", json),
            ("CF_REALTIME_APP_ID", "app"),
            ("CF_REALTIME_APP_SECRET", "secret"),
            ("LIVEKIT_API_KEY", "key"),
            ("LIVEKIT_API_SECRET", SECRET32),
            ("SFU_BRIDGE_TOKEN", SECRET32),
        ]))
        .unwrap()
        .unwrap();
        let lk = cfg.livekit.unwrap();
        assert_eq!(lk.ws_url, "wss://livekit.example");
        assert_eq!(lk.lan_egress.len(), 2);
        assert!(cfg.bridge_token.is_some());
    }

    #[test]
    fn partial_livekit_is_an_error() {
        let json = r#"{"cloudflare":{},"livekit":{"ws_url":"wss://l","api_url":"http://l","lan_egress_cidrs":["10.0.0.0/8"]}}"#;
        let err = CallSfuConfig::from_lookup(&lookup(&[
            ("CALL_SFUS_JSON", json),
            ("CF_REALTIME_APP_ID", "app"),
            ("CF_REALTIME_APP_SECRET", "secret"),
            ("LIVEKIT_API_KEY", "key"),
        ]))
        .unwrap_err();
        assert!(err.contains("LIVEKIT_API_SECRET"));
    }

    #[test]
    fn bridge_needs_livekit_and_token() {
        let base = [
            ("CF_REALTIME_APP_ID", "app"),
            ("CF_REALTIME_APP_SECRET", "secret"),
        ];
        let mut vars = base.to_vec();
        vars.push(("CALL_SFUS_JSON", r#"{"cloudflare":{},"bridge":true}"#));
        assert!(CallSfuConfig::from_lookup(&lookup(&vars)).is_err());
        vars.push(("SFU_BRIDGE_TOKEN", SECRET32));
        let err = CallSfuConfig::from_lookup(&lookup(&vars)).unwrap_err();
        assert!(err.contains("livekit"));
    }

    #[test]
    fn threshold_must_fit_mesh_capacity() {
        let err = CallSfuConfig::from_lookup(&lookup(&[
            ("CALL_SFUS_JSON", r#"{"cloudflare":{}}"#),
            ("CF_REALTIME_APP_ID", "app"),
            ("CF_REALTIME_APP_SECRET", "secret"),
            ("CALL_SFU_THRESHOLD", "6"),
        ]))
        .unwrap_err();
        assert!(err.contains("CALL_SFU_THRESHOLD"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = CallSfuConfig::from_lookup(&lookup(&[
            ("CALL_SFUS_JSON", r#"{"cloudflare":{},"kubecamp":{}}"#),
            ("CF_REALTIME_APP_ID", "app"),
            ("CF_REALTIME_APP_SECRET", "secret"),
        ]))
        .unwrap_err();
        assert!(err.contains("CALL_SFUS_JSON"));
    }

    #[test]
    fn cidr_matching() {
        let v4 = Cidr::parse("192.0.2.0/24").unwrap();
        assert!(v4.contains("192.0.2.200".parse().unwrap()));
        assert!(!v4.contains("192.0.3.1".parse().unwrap()));
        assert!(v4.contains("::ffff:192.0.2.5".parse().unwrap()));
        let host = Cidr::parse("203.0.113.4").unwrap();
        assert!(host.contains("203.0.113.4".parse().unwrap()));
        assert!(!host.contains("203.0.113.5".parse().unwrap()));
        let v6 = Cidr::parse("2001:db8::/48").unwrap();
        assert!(v6.contains("2001:db8:0:1::1".parse().unwrap()));
        assert!(!v6.contains("2001:db9::1".parse().unwrap()));
        assert!(!v6.contains("192.0.2.1".parse().unwrap()));
        let all = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(all.contains("8.8.8.8".parse().unwrap()));
        assert!(Cidr::parse("192.0.2.0/33").is_err());
        assert!(Cidr::parse("nope").is_err());
    }
}
