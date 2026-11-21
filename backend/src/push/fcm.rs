//! Native Android delivery. Provider credentials and registration tokens are never logged.
use super::NotificationPayload;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

pub struct FcmConfig {
    pub project_id: String,
    email: String,
    key: EncodingKey,
    cache_key: Vec<u8>,
}

#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
}

impl FcmConfig {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        Self::parse(&std::env::var("FCM_SERVICE_ACCOUNT_JSON").unwrap_or_default())
    }

    fn parse(raw: &str) -> anyhow::Result<Option<Self>> {
        if raw.is_empty() {
            return Ok(None);
        }
        let account: ServiceAccount = serde_json::from_str(raw)
            .map_err(|_| anyhow::anyhow!("Invalid FCM_SERVICE_ACCOUNT_JSON"))?;
        anyhow::ensure!(
            !account.project_id.is_empty()
                && account.project_id.len() <= 100
                && account
                    .project_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                && account.client_email.ends_with(".iam.gserviceaccount.com"),
            "Invalid FCM service account project or email"
        );
        let key = EncodingKey::from_rsa_pem(account.private_key.as_bytes())
            .map_err(|_| anyhow::anyhow!("Invalid FCM service account private key"))?;
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &json!({}), &key)
            .map_err(|_| anyhow::anyhow!("Invalid FCM service account private key"))?;
        Ok(Some(Self {
            project_id: account.project_id,
            email: account.client_email,
            key,
            cache_key: Sha256::digest(raw.as_bytes()).to_vec(),
        }))
    }

    async fn access_token(&self, client: &reqwest::Client) -> anyhow::Result<String> {
        struct CachedToken {
            key: Vec<u8>,
            token: String,
            until: Instant,
        }
        static CACHE: OnceLock<Mutex<Option<CachedToken>>> = OnceLock::new();
        let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock().await;
        if let Some(cached) = cache
            .as_ref()
            .filter(|v| v.key == self.cache_key && v.until > Instant::now())
        {
            return Ok(cached.token.clone());
        }
        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            scope: &'a str,
            aud: &'a str,
            iat: u64,
            exp: u64,
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let assertion = jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &Claims {
                iss: &self.email,
                scope: "https://www.googleapis.com/auth/firebase.messaging",
                aud: TOKEN_URL,
                iat: now,
                exp: now + 3600,
            },
            &self.key,
        )
        .map_err(|_| anyhow::anyhow!("FCM OAuth assertion failed"))?;
        let response = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "FCM OAuth token exchange failed"
        );
        #[derive(Deserialize)]
        struct Token {
            access_token: String,
            expires_in: u64,
        }
        let token: Token = response.json().await?;
        anyhow::ensure!(!token.access_token.is_empty(), "Empty FCM OAuth token");
        let until =
            Instant::now() + Duration::from_secs(token.expires_in.min(3600).saturating_sub(60));
        *cache = Some(CachedToken {
            key: self.cache_key.clone(),
            token: token.access_token.clone(),
            until,
        });
        Ok(token.access_token)
    }

    /// True only for an explicitly unregistered token; configuration errors must not delete devices.
    pub(super) async fn send(
        &self,
        client: &reqwest::Client,
        endpoint: &str,
        user: &str,
        payload: &NotificationPayload,
        ttl: u32,
    ) -> anyhow::Result<bool> {
        let (token, binding) = registration(endpoint)
            .ok_or_else(|| anyhow::anyhow!("Invalid Android registration"))?;
        let body = message(token, binding, user, payload, ttl);
        anyhow::ensure!(
            serde_json::to_vec(&body["message"]["data"])?.len() <= 4096,
            "FCM data payload too large"
        );
        let access = self.access_token(client).await?;
        post_message(
            client,
            &format!(
                "https://fcm.googleapis.com/v1/projects/{}/messages:send",
                self.project_id
            ),
            &access,
            &body,
        )
        .await
    }
}

async fn post_message(
    client: &reqwest::Client,
    url: &str,
    access: &str,
    body: &Value,
) -> anyhow::Result<bool> {
    let response = client
        .post(url)
        .bearer_auth(access)
        .json(body)
        .send()
        .await?;
    if response.status().is_success() {
        return Ok(false);
    }
    let status = response.status();
    let error: Value = response.json().await.unwrap_or(Value::Null);
    if is_unregistered(&error) {
        return Ok(true);
    }
    anyhow::bail!("FCM provider rejected delivery (HTTP {})", status.as_u16())
}

/// A binding changes on logout/account switch, so delayed pushes cannot cross sessions.
const DM_CALLS_SUFFIX: &str = ":dm-calls";

pub(super) fn accepts_notification(endpoint: &str, kind: &str) -> bool {
    match kind {
        "missed_call" => false,
        "call_invite" | "call_invite_end" => {
            endpoint.ends_with(DM_CALLS_SUFFIX) && registration(endpoint).is_some()
        }
        _ => true,
    }
}

pub fn registration(endpoint: &str) -> Option<(&str, &str)> {
    let endpoint = endpoint.strip_suffix(DM_CALLS_SUFFIX).unwrap_or(endpoint);
    let (token, binding) = endpoint.strip_prefix("fcm:")?.rsplit_once(':')?;
    if token.is_empty()
        || token.len() > 4096
        || !token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_:".contains(&c))
        || uuid::Uuid::parse_str(binding).is_err()
    {
        return None;
    }
    Some((token, binding))
}

fn message(
    token: &str,
    binding: &str,
    user: &str,
    payload: &NotificationPayload,
    ttl: u32,
) -> Value {
    // Data-only messages reach FirebaseMessagingService in the foreground and after process death.
    // Never attach E2EE ciphertext: Android displays the existing generic encrypted-message body.
    let mut data = json!({"binding": binding, "userId": user, "title": payload.title,
        "body": payload.body, "url": payload.url, "tag": payload.tag,
        "notificationType": payload.notification_type});
    if let Some(id) = &payload.invite_id {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        data["inviteId"] = json!(id);
        data["expiresAt"] = json!((now + u64::from(ttl)).to_string());
    }
    let priority = if payload.notification_type == "call_invite_end" {
        "NORMAL"
    } else {
        "HIGH"
    };
    json!({"message": {"token": token, "android": {"priority": priority, "ttl": format!("{ttl}s")},
        "data": data}})
}

fn is_unregistered(error: &Value) -> bool {
    error["error"]["details"].as_array().is_some_and(|details| {
        details.iter().any(|detail| {
            detail["@type"] == "type.googleapis.com/google.firebase.fcm.v1.FcmError"
                && detail["errorCode"] == "UNREGISTERED"
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_is_optional_but_malformed_values_fail_closed() {
        assert!(FcmConfig::parse("").unwrap().is_none());
        assert!(FcmConfig::parse("{}").is_err());
        assert!(FcmConfig::parse(r#"{"project_id":"../evil","client_email":"test.iam.gserviceaccount.com","private_key":"secret"}"#).is_err());
    }

    #[test]
    fn startup_rejects_public_keys_and_accepts_signing_private_keys() {
        let rsa = openssl::rsa::Rsa::generate(2048).unwrap();
        let key = openssl::pkey::PKey::from_rsa(rsa).unwrap();
        let account = |pem: Vec<u8>| {
            json!({
            "project_id": "test-project", "client_email": "test@test-project.iam.gserviceaccount.com",
            "private_key": String::from_utf8(pem).unwrap(),
        }).to_string()
        };
        assert!(FcmConfig::parse(&account(key.public_key_to_pem().unwrap())).is_err());
        assert!(
            FcmConfig::parse(&account(key.private_key_to_pem_pkcs8().unwrap()))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn registration_rejects_urls_empty_tokens_and_invalid_bindings() {
        let binding = uuid::Uuid::new_v4().to_string();
        let endpoint = format!("fcm:device:token_123:{binding}");
        assert_eq!(
            registration(&endpoint),
            Some(("device:token_123", binding.as_str()))
        );
        for invalid in ["fcm::bad", "https://localhost/", "fcm:token:bad"] {
            assert!(registration(invalid).is_none());
        }
        assert!(registration(&format!("fcm:https://localhost:{binding}")).is_none());
        assert!(registration(&format!("fcm:{}:{binding}", "a".repeat(4097))).is_none());
    }

    #[test]
    fn only_opted_in_subscriptions_receive_dm_invites_and_ends() {
        let endpoint = format!("fcm:test:{}", uuid::Uuid::new_v4());
        let opted_in = format!("{endpoint}{DM_CALLS_SUFFIX}");
        assert_eq!(registration(&endpoint), registration(&opted_in));
        for kind in ["call_invite", "call_invite_end"] {
            assert!(!accepts_notification(&endpoint, kind));
            assert!(accepts_notification(&opted_in, kind));
            assert!(!accepts_notification("fcm:invalid:dm-calls", kind));
        }
        assert!(accepts_notification(&endpoint, "mention"));
        assert!(!accepts_notification(&opted_in, "missed_call"));
    }

    #[test]
    fn only_unregistered_tokens_are_removed() {
        assert!(!is_unregistered(&json!({"error":{"status":"NOT_FOUND"}})));
        assert!(!is_unregistered(
            &json!({"error":{"details":[{"errorCode":"INVALID_ARGUMENT"}]}})
        ));
        assert!(is_unregistered(
            &json!({"error":{"details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}})
        ));
    }

    #[test]
    fn high_priority_data_carries_account_binding_without_browser_payload() {
        let payload = NotificationPayload {
            title: "Actor".into(),
            body: "Message".into(),
            url: "/channels/channel?message=message".into(),
            tag: "zoo-notification".into(),
            notification_type: "dm".into(),
            e2ee: None,
            invite_id: None,
        };
        let body = message("token", "binding", "user", &payload, 300);
        assert_eq!(body["message"]["android"]["priority"], "HIGH");
        assert_eq!(body["message"]["data"]["binding"], "binding");
        assert_eq!(body["message"]["data"]["userId"], "user");
        assert!(body["message"].get("notification").is_none());
        assert!(body["message"]["data"].get("e2ee").is_none());
    }
    #[test]
    fn call_invite_carries_its_short_ttl_and_identifier() {
        let payload = NotificationPayload {
            title: "Caller".into(),
            body: "Incoming call".into(),
            url: "/channels/dm?call_invite=invite".into(),
            tag: "zoo-call-invite".into(),
            notification_type: "call_invite".into(),
            e2ee: None,
            invite_id: Some("invite".into()),
        };
        let body = message("token", "binding", "user", &payload, 17);
        assert_eq!(body["message"]["android"]["ttl"], "17s");
        assert_eq!(body["message"]["data"]["inviteId"], "invite");
        assert!(
            body["message"]["data"]["expiresAt"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .is_ok()
        );
    }

    #[test]
    fn call_end_is_normal_priority() {
        let payload = NotificationPayload {
            title: String::new(),
            body: String::new(),
            url: "/channels/dm".into(),
            tag: "zoo-call-invite".into(),
            notification_type: "call_invite_end".into(),
            e2ee: None,
            invite_id: Some("invite".into()),
        };
        let body = message("token", "binding", "user", &payload, 60);
        assert_eq!(body["message"]["android"]["priority"], "NORMAL");
    }

    #[tokio::test]
    async fn http_delivery_authenticates_and_preserves_devices_on_project_errors() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, header, method, path},
        };
        let server = MockServer::start().await;
        let body =
            json!({"message":{"token":"test-device-token","data":{"binding":"test-binding"}}});
        let client = super::super::push_client().unwrap();
        for (status, response, expected) in [
            (200, json!({"name":"projects/test/messages/1"}), Some(false)),
            (
                404,
                json!({"error":{"details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}),
                Some(true),
            ),
            (404, json!({"error":{"status":"NOT_FOUND"}}), None),
            (401, json!({"error":{"status":"UNAUTHENTICATED"}}), None),
        ] {
            server.reset().await;
            Mock::given(method("POST"))
                .and(path("/v1/projects/test/messages:send"))
                .and(header("authorization", "Bearer test-oauth-token"))
                .and(body_json(&body))
                .respond_with(ResponseTemplate::new(status).set_body_json(response))
                .expect(1)
                .mount(&server)
                .await;
            let result = post_message(
                &client,
                &format!("{}/v1/projects/test/messages:send", server.uri()),
                "test-oauth-token",
                &body,
            )
            .await;
            match expected {
                Some(value) => assert_eq!(result.unwrap(), value),
                None => assert!(result.is_err()),
            }
        }
    }
}
