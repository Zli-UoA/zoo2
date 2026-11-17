//! Server-only Janus SIP control. App/Web exchange WebRTC SDP/ICE, never SIP credentials.
use crate::error::AppError;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{fmt, time::Duration};

#[derive(Clone, Default)]
pub struct GatewayConfig {
    pub url: Option<String>,
    pub api_secret: Option<String>,
    pub pbx_uri: Option<String>,
    pub pbx_config_secret: Option<String>,
    pub media_mode: Option<String>,
}
impl fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayConfig")
            .field("enabled", &self.enabled())
            .finish()
    }
}
impl GatewayConfig {
    pub fn from_env() -> Self {
        let read = |key| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        Self {
            url: read("SIP_GATEWAY_URL"),
            api_secret: read("SIP_GATEWAY_API_SECRET"),
            pbx_uri: read("SIP_PBX_URI"),
            pbx_config_secret: read("SIP_PBX_CONFIG_SECRET"),
            media_mode: read("SIP_MEDIA_MODE"),
        }
    }
    pub fn relay_only(&self) -> bool {
        self.media_mode.as_deref() != Some("lan")
    }
    pub fn enabled(&self) -> bool {
        self.url.is_some() && self.api_secret.is_some() && self.pbx_uri.is_some()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.url.is_none()
            && self.api_secret.is_none()
            && self.pbx_uri.is_none()
            && self.pbx_config_secret.is_none()
            && self.media_mode.is_none()
        {
            return Ok(());
        }
        if self
            .media_mode
            .as_deref()
            .is_some_and(|mode| !matches!(mode, "lan" | "relay"))
        {
            return Err("SIP_MEDIA_MODE must be lan or relay".into());
        }
        if !self.enabled() {
            return Err(
                "Set all of SIP_GATEWAY_URL, SIP_GATEWAY_API_SECRET and SIP_PBX_URI".into(),
            );
        }
        let url = reqwest::Url::parse(self.url.as_deref().unwrap())
            .map_err(|_| "Invalid SIP_GATEWAY_URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "SIP_GATEWAY_URL must be an HTTP(S) URL without credentials, query or fragment"
                    .into(),
            );
        }
        let pbx = self.pbx_uri.as_deref().unwrap();
        let authority = pbx
            .strip_prefix("sip:")
            .or_else(|| pbx.strip_prefix("sips:"))
            .ok_or("SIP_PBX_URI must use sip: or sips:")?;
        let mut pieces = authority.split(';');
        let host = pieces.next().unwrap();
        if host.is_empty()
            || !host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".:-[]".contains(&c))
            || pieces.any(|p| !matches!(p, "transport=udp" | "transport=tcp" | "transport=tls"))
        {
            return Err(
                "SIP_PBX_URI must identify the configured PBX, without user information or headers"
                    .into(),
            );
        }
        Ok(())
    }
}
fn unavailable() -> AppError {
    AppError::ServiceUnavailable("Phone gateway unavailable".into())
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(40))
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .expect("phone HTTP client")
}

#[derive(Clone)]
pub struct Gateway {
    client: reqwest::Client,
    config: GatewayConfig,
}
impl Gateway {
    pub fn new(client: reqwest::Client, config: &GatewayConfig) -> Result<Self, AppError> {
        if !config.enabled() {
            return Err(unavailable());
        }
        Ok(Self {
            client,
            config: config.clone(),
        })
    }
    fn url(&self, path: &str) -> String {
        format!(
            "{}{}",
            self.config.url.as_deref().unwrap().trim_end_matches('/'),
            path
        )
    }
    async fn response(response: reqwest::Response) -> Result<Value, AppError> {
        if !response.status().is_success() || response.content_length().is_some_and(|n| n > 262_144)
        {
            return Err(unavailable());
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| unavailable())?;
            if bytes.len() + chunk.len() > 262_144 {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if value["janus"] == "error" {
            return Err(unavailable());
        }
        Ok(value)
    }
    async fn post(&self, path: &str, mut body: Value) -> Result<Value, AppError> {
        body["transaction"] = json!(uuid::Uuid::new_v4().to_string());
        body["apisecret"] = json!(self.config.api_secret.as_deref().unwrap());
        let response = self
            .client
            .post(self.url(path))
            .timeout(Duration::from_secs(10))
            .json(&body)
            .send()
            .await
            .map_err(|_| unavailable())?;
        Self::response(response).await
    }
    pub async fn create(&self) -> Result<u64, AppError> {
        let response = self.post("", json!({"janus":"create"})).await?;
        response["data"]["id"]
            .as_u64()
            .filter(|id| *id != 0)
            .ok_or_else(unavailable)
    }
    pub async fn attach(&self, session: u64) -> Result<u64, AppError> {
        let response = self
            .post(
                &format!("/{session}"),
                json!({"janus":"attach","plugin":"janus.plugin.sip"}),
            )
            .await?;
        response["data"]["id"]
            .as_u64()
            .filter(|id| *id != 0)
            .ok_or_else(unavailable)
    }
    pub async fn register(
        &self,
        session: u64,
        handle: u64,
        domain: &str,
        username: &str,
        aor: &str,
        password: &str,
    ) -> Result<(), AppError> {
        // Both registrar and outbound next hop are infrastructure-controlled.
        // The line's realm cannot turn the gateway into an arbitrary SIP proxy.
        self.message(session, handle, json!({"request":"register","username":format!("sip:{aor}@{domain}"),"authuser":username,"secret":password,"proxy":self.config.pbx_uri,"outbound_proxy":self.config.pbx_uri,"send_register":true,"force_tcp":self.config.pbx_uri.as_ref().unwrap().contains("transport=tcp") || self.config.pbx_uri.as_ref().unwrap().starts_with("sips:"),"sips":self.config.pbx_uri.as_ref().unwrap().starts_with("sips:")}), None).await
    }
    pub async fn message(
        &self,
        session: u64,
        handle: u64,
        body: Value,
        jsep: Option<Value>,
    ) -> Result<(), AppError> {
        let mut request = json!({"janus":"message","body":body});
        if let Some(jsep) = jsep {
            request["jsep"] = jsep;
        }
        let response = self.post(&format!("/{session}/{handle}"), request).await?;
        if !matches!(response["janus"].as_str(), Some("ack" | "success"))
            || response["plugindata"]["data"]["error_code"].is_number()
        {
            return Err(unavailable());
        }
        Ok(())
    }
    pub async fn trickle(
        &self,
        session: u64,
        handle: u64,
        candidate: Value,
    ) -> Result<(), AppError> {
        self.post(
            &format!("/{session}/{handle}"),
            json!({"janus":"trickle","candidate":candidate}),
        )
        .await?;
        Ok(())
    }
    pub async fn poll(&self, session: u64) -> Result<Value, AppError> {
        let rid = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| unavailable())?
            .as_millis()
            .to_string();
        let response = self
            .client
            .get(self.url(&format!("/{session}")))
            .query(&[
                ("rid", rid.as_str()),
                ("maxev", "1"),
                ("apisecret", self.config.api_secret.as_deref().unwrap()),
            ])
            .timeout(Duration::from_secs(35))
            .send()
            .await
            .map_err(|_| unavailable())?;
        Self::response(response).await
    }
    pub async fn keepalive(&self, session: u64) -> Result<(), AppError> {
        self.post(&format!("/{session}"), json!({"janus":"keepalive"}))
            .await?;
        Ok(())
    }
    pub async fn destroy(&self, session: u64) {
        let _ = self
            .post(&format!("/{session}"), json!({"janus":"destroy"}))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gateway_configuration_is_optional_but_partial_settings_are_rejected() {
        assert!(GatewayConfig::default().validate().is_ok());
        let mut config = GatewayConfig {
            url: Some("http://gateway:8088/janus".into()),
            api_secret: None,
            pbx_uri: None,
            pbx_config_secret: None,
            media_mode: None,
        };
        assert!(config.validate().is_err());
        config.api_secret = Some("dummy-private".into());
        config.pbx_uri = Some("sip:pbx:5060;transport=udp".into());
        assert!(config.validate().is_ok());
        assert!(!format!("{config:?}").contains("dummy-private"));
        assert!(config.relay_only());
        config.media_mode = Some("lan".into());
        assert!(config.validate().is_ok());
        assert!(!config.relay_only());
        config.media_mode = Some("invalid".into());
        assert!(config.validate().is_err());
        config.media_mode = None;
        for uri in [
            "sip:user@evil",
            "sip:pbx?header=x",
            "http://pbx",
            "sip:pbx;transport=udp;foo=x",
        ] {
            config.pbx_uri = Some(uri.into());
            assert!(config.validate().is_err());
        }
    }
}
