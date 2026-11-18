//! Native Android delivery. Provider credentials and registration tokens are never logged.
use super::{NotificationPayload, PushDevice, PushRecipient};
use crate::models::PushSubscription;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Mutex, time::Instant};

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Clone)]
struct AccessToken {
    value: String,
    usable_until: Instant,
}

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

    async fn access_token(
        &self,
        client: &reqwest::Client,
        token_url: &str,
        before_refresh: impl Future<Output = anyhow::Result<bool>>,
    ) -> anyhow::Result<Option<AccessToken>> {
        struct CachedToken {
            key: Vec<u8>,
            token: AccessToken,
        }
        static CACHE: OnceLock<Mutex<Option<CachedToken>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(None));
        let cached_token = |entry: &Option<CachedToken>| {
            entry
                .as_ref()
                .filter(|v| v.key == self.cache_key && v.token.usable_until > Instant::now())
                .map(|v| v.token.clone())
        };
        if let Some(token) = cached_token(&*cache.lock().await) {
            return Ok(Some(token));
        }
        // 配信を見送る端末のためのOAuth更新を省く。権限照会中は他の配送のキャッシュを塞がない。
        if !before_refresh.await? {
            return Ok(None);
        }
        let mut cache = cache.lock().await;
        if let Some(token) = cached_token(&cache) {
            return Ok(Some(token));
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
        let requested_at = Instant::now();
        let response = client
            .post(token_url)
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
        anyhow::ensure!(
            !token.access_token.is_empty() && token.expires_in > 0,
            "FCMの認証トークンが無効です"
        );
        let lifetime = token.expires_in.min(3600);
        let token = AccessToken {
            value: token.access_token,
            usable_until: requested_at + Duration::from_secs(lifetime - 60.min(lifetime / 2)),
        };
        *cache = Some(CachedToken {
            key: self.cache_key.clone(),
            token: token.clone(),
        });
        Ok(Some(token))
    }

    /// True only for an explicitly unregistered token; configuration errors must not delete devices.
    pub(super) async fn send(
        &self,
        client: &reqwest::Client,
        db: &mut toasty::Db,
        recipient: PushRecipient<'_>,
        subscription: &PushSubscription,
        payload: &NotificationPayload,
    ) -> anyhow::Result<bool> {
        let (token, binding) = registration(&subscription.endpoint)
            .ok_or_else(|| anyhow::anyhow!("Invalid Android registration"))?;
        let body = message(token, binding, recipient.user_id(), payload);
        anyhow::ensure!(
            serde_json::to_vec(&body["message"]["data"])?.len() <= 4096,
            "FCM data payload too large"
        );
        let device = PushDevice::WebOrFcm(subscription);
        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.project_id
        );
        // 最終照会の間に期限を過ぎた場合だけ、一度更新してから端末の権限も再確認する。
        for _ in 0..2 {
            let Some(access) = self
                .access_token(client, TOKEN_URL, recipient.can_receive(db, device))
                .await?
            else {
                return Ok(false);
            };
            if let Some(unregistered) = post_message(
                client,
                &url,
                &access,
                &body,
                recipient.can_receive(db, device),
            )
            .await?
            {
                return Ok(unregistered);
            }
        }
        anyhow::bail!("端末の確認中にFCMの認証トークンが失効しました")
    }
}

// None の場合だけ送信前にトークンが失効した。Some は購読を削除するかを表す。
async fn post_message(
    client: &reqwest::Client,
    url: &str,
    access: &AccessToken,
    body: &Value,
    can_receive: impl Future<Output = anyhow::Result<bool>>,
) -> anyhow::Result<Option<bool>> {
    // OAuthの更新を待つ間にメンバー資格や通知設定が変わることがある。
    if !can_receive.await? {
        return Ok(Some(false));
    }
    if access.usable_until <= Instant::now() {
        return Ok(None);
    }
    let response = client
        .post(url)
        .bearer_auth(&access.value)
        .json(body)
        .send()
        .await?;
    if response.status().is_success() {
        return Ok(Some(false));
    }
    let status = response.status();
    let error: Value = response.json().await.unwrap_or(Value::Null);
    if is_unregistered(&error) {
        return Ok(Some(true));
    }
    anyhow::bail!("FCM provider rejected delivery (HTTP {})", status.as_u16())
}

/// A binding changes on logout/account switch, so delayed pushes cannot cross sessions.
pub fn registration(endpoint: &str) -> Option<(&str, &str)> {
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

fn message(token: &str, binding: &str, user: &str, payload: &NotificationPayload) -> Value {
    // Data-only messages reach FirebaseMessagingService in the foreground and after process death.
    // Never attach E2EE ciphertext: Android displays the existing generic encrypted-message body.
    json!({"message": {"token": token, "android": {"priority": "HIGH", "ttl": "300s"},
        "data": {"binding": binding, "userId": user, "title": payload.title, "body": payload.body,
            "url": payload.url, "tag": payload.tag, "notificationType": payload.notification_type}}})
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

    fn test_access(value: &str) -> AccessToken {
        AccessToken {
            value: value.into(),
            usable_until: Instant::now() + Duration::from_secs(60),
        }
    }

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
        let body = message("token", "binding", "user", &payload);
        assert_eq!(body["message"]["android"]["priority"], "HIGH");
        assert_eq!(body["message"]["data"]["binding"], "binding");
        assert_eq!(body["message"]["data"]["userId"], "user");
        assert!(body["message"].get("notification").is_none());
        assert!(body["message"]["data"].get("e2ee").is_none());
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
                &test_access("test-oauth-token"),
                &body,
                std::future::ready(Ok(true)),
            )
            .await;
            match expected {
                Some(value) => assert_eq!(result.unwrap(), Some(value)),
                None => assert!(result.is_err()),
            }
        }
    }

    #[tokio::test]
    async fn denied_devices_skip_oauth_and_cached_tokens_skip_the_precheck() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token":"test-token","expires_in":3600})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let rsa =
            openssl::pkey::PKey::from_rsa(openssl::rsa::Rsa::generate(2048).unwrap()).unwrap();
        let config = FcmConfig {
            project_id: "test".into(),
            email: "test@example.com".into(),
            key: EncodingKey::from_rsa_pem(&rsa.private_key_to_pem_pkcs8().unwrap()).unwrap(),
            cache_key: uuid::Uuid::new_v4().as_bytes().to_vec(),
        };
        let client = super::super::push_client().unwrap();
        let url = server.uri();
        for permission in [Ok(false), Err(anyhow::anyhow!("照会失敗"))] {
            let unavailable = permission.is_err();
            let token = config
                .access_token(&client, &url, std::future::ready(permission))
                .await;
            assert_eq!(token.is_err(), unavailable);
            assert!(token.ok().flatten().is_none());
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            config
                .access_token(&client, &url, async { Ok(true) })
                .await
                .unwrap()
                .as_ref()
                .map(|token| token.value.as_str()),
            Some("test-token")
        );
        let cached = config
            .access_token(&client, &url, async {
                panic!("キャッシュ利用時に事前照会が走りました")
            })
            .await
            .unwrap();
        assert_eq!(
            cached.as_ref().map(|token| token.value.as_str()),
            Some("test-token")
        );
    }

    #[tokio::test]
    async fn permission_denied_or_unavailable_skips_fcm_request() {
        let server = wiremock::MockServer::start().await;
        let client = super::super::push_client().unwrap();
        for permission in [Ok(false), Err(anyhow::anyhow!("照会失敗"))] {
            let expected = permission.as_ref().ok().copied();
            let result = post_message(
                &client,
                &server.uri(),
                &test_access("test-token"),
                &json!({}),
                std::future::ready(permission),
            )
            .await;
            assert_eq!(result.ok(), expected.map(Some));
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn token_expiring_during_permission_check_is_never_sent() {
        let server = wiremock::MockServer::start().await;
        let client = super::super::push_client().unwrap();
        tokio::time::pause();
        let access = test_access("old-token");
        let result = post_message(&client, &server.uri(), &access, &json!({}), async {
            tokio::time::advance(Duration::from_secs(65)).await;
            Ok(true)
        })
        .await
        .unwrap();
        tokio::time::resume();
        assert_eq!(result, None);
        let refreshed = test_access("new-token");
        assert_eq!(
            post_message(&client, &server.uri(), &refreshed, &json!({}), async {
                Ok(false)
            })
            .await
            .unwrap(),
            Some(false)
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
