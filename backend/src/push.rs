//! RFC 8291 encrypted notification payloads with RFC 8292 VAPID authentication.
use crate::{
    error::AppError,
    models::{Channel, Message, Notification, PushSubscription, User},
    state::AppState,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct PushConfig {
    pub public_key: String,
    private_key: EncodingKey,
    subject: String,
}

impl PushConfig {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let public = std::env::var("VAPID_PUBLIC_KEY").unwrap_or_default();
        let private = std::env::var("VAPID_PRIVATE_KEY").unwrap_or_default();
        let subject = std::env::var("VAPID_SUBJECT").unwrap_or_default();
        Self::parse(public, private, subject)
    }

    fn parse(public: String, private: String, subject: String) -> anyhow::Result<Option<Self>> {
        if public.is_empty() && private.is_empty() && subject.is_empty() {
            return Ok(None);
        }
        anyhow::ensure!(
            public.len() == 87
                && public.starts_with('B')
                && public
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "VAPID_PUBLIC_KEY must be an uncompressed P-256 base64url public key"
        );
        anyhow::ensure!(
            subject.starts_with("mailto:") || subject.starts_with("https://"),
            "VAPID_SUBJECT must be a mailto: or https: contact"
        );
        let private_key = EncodingKey::from_ec_pem(private.as_bytes()).map_err(|_| {
            anyhow::anyhow!("VAPID_PRIVATE_KEY must be an EC P-256 PEM private key")
        })?;
        jsonwebtoken::encode(
            &Header::new(Algorithm::ES256),
            &serde_json::json!({}),
            &private_key,
        )
        .map_err(|_| anyhow::anyhow!("VAPID_PRIVATE_KEY must be a PKCS#8 P-256 PEM private key"))?;
        Ok(Some(Self {
            public_key: public,
            private_key,
            subject,
        }))
    }
}

/// Prevent authenticated clients from using push delivery for arbitrary server-side requests.
pub fn validate_endpoint(endpoint: &str) -> Result<Url, AppError> {
    let url =
        Url::parse(endpoint).map_err(|_| AppError::BadRequest("Invalid push endpoint".into()))?;
    let host = url.host_str().unwrap_or_default();
    let allowed = matches!(
        host,
        "fcm.googleapis.com"
            | "android.googleapis.com"
            | "updates.push.services.mozilla.com"
            | "web.push.apple.com"
    ) || (host.ends_with(".notify.windows.com")
        && host.len() > ".notify.windows.com".len());
    if endpoint.len() > 4096
        || url.scheme() != "https"
        || !allowed
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::BadRequest("Unsupported push endpoint".into()));
    }
    Ok(url)
}

#[derive(Serialize)]
struct Claims<'a> {
    aud: String,
    exp: u64,
    sub: &'a str,
}

#[derive(Deserialize)]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

impl SubscriptionKeys {
    pub fn validate(&self) -> anyhow::Result<()> {
        let (public, auth) = self.decode()?;
        // Importing/encrypting also rejects invalid points on the P-256 curve.
        ece::encrypt(&public, &auth, b"validation")?;
        Ok(())
    }

    fn decode(&self) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        anyhow::ensure!(
            self.p256dh.len() <= 88 && self.auth.len() <= 24,
            "Invalid push keys"
        );
        let public = URL_SAFE_NO_PAD.decode(&self.p256dh)?;
        let auth = URL_SAFE_NO_PAD.decode(&self.auth)?;
        anyhow::ensure!(
            public.len() == 65 && public[0] == 4 && auth.len() == 16,
            "Invalid push keys"
        );
        Ok((public, auth))
    }

    fn encrypt(&self, payload: &NotificationPayload) -> anyhow::Result<Vec<u8>> {
        let (public, auth) = self.decode()?;
        let body = ece::encrypt(&public, &auth, &serde_json::to_vec(payload)?)?;
        anyhow::ensure!(body.len() <= 4096, "Push payload is too large");
        Ok(body)
    }
}

#[derive(Serialize)]
struct NotificationPayload {
    title: String,
    body: String,
    url: String,
    tag: String,
    #[serde(rename = "notificationType")]
    notification_type: String,
}

fn notification_payload(
    notification: &Notification,
    actor: &User,
    channel: &Channel,
    message: &Message,
) -> NotificationPayload {
    let name = if actor.display_name.is_empty() {
        &actor.user_name
    } else {
        &actor.display_name
    };
    let channel_name: String = channel.channel_name.chars().take(60).collect();
    NotificationPayload {
        title: if channel.is_dm {
            name.chars().take(60).collect()
        } else {
            format!(
                "{} · #{}",
                name.chars().take(60).collect::<String>(),
                channel_name
            )
        },
        body: message.content.chars().take(240).collect(),
        url: format!("/channels/{}?message={}", channel.id, message.id),
        tag: format!("zoo-{}", notification.id),
        notification_type: notification.notification_type.clone(),
    }
}

/// Run only after commit. Delivery errors must never roll back a message or expose endpoint URLs.
pub fn dispatch(state: AppState, notifications: Vec<Notification>) {
    if notifications.is_empty() {
        return;
    }
    crate::apns::dispatch(state.clone(), notifications.clone());
    tokio::spawn(async move {
        if deliver(&state, notifications).await.is_err() {
            tracing::warn!("Web Push delivery failed");
        }
    });
}

async fn deliver(state: &AppState, notifications: Vec<Notification>) -> anyhow::Result<()> {
    let Some(config) = PushConfig::from_env()? else {
        return Ok(());
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut db = state.db.clone();
    for notification in notifications {
        let Some(message) =
            Message::filter(Message::fields().id().eq(notification.message_id.as_str()))
                .first()
                .exec(&mut db)
                .await?
        else {
            continue;
        };
        if message.deleted_at.is_some() {
            continue;
        }
        let actor = User::get_by_id(&mut db, notification.actor_id.as_str()).await?;
        let channel = Channel::get_by_id(&mut db, notification.channel_id.as_str()).await?;
        let payload = notification_payload(&notification, &actor, &channel, &message);
        let subscriptions = PushSubscription::filter(
            PushSubscription::fields()
                .user_id()
                .eq(notification.user_id.as_str()),
        )
        .exec(&mut db)
        .await?;
        for subscription in subscriptions {
            let Ok(endpoint) = validate_endpoint(&subscription.endpoint) else {
                continue;
            };
            let body = match (subscription.p256dh, subscription.auth_secret) {
                (Some(p256dh), Some(auth)) => {
                    match (SubscriptionKeys { p256dh, auth }).encrypt(&payload) {
                        Ok(body) => Some(body),
                        Err(_) => {
                            tracing::warn!("Invalid Web Push encryption keys");
                            continue;
                        }
                    }
                }
                // Older registrations receive a generic alert until the browser refreshes its keys.
                _ => None,
            };
            let result = send(&client, &config, endpoint, body).await;
            match result {
                Ok(response)
                    if response.status().as_u16() == 404 || response.status().as_u16() == 410 =>
                {
                    PushSubscription::filter(
                        PushSubscription::fields().id().eq(subscription.id.as_str()),
                    )
                    .delete()
                    .exec(&mut db)
                    .await?;
                }
                Ok(response) if response.status().is_success() => {}
                _ => tracing::warn!("Web Push provider did not accept delivery"),
            }
        }
    }
    Ok(())
}

async fn send(
    client: &reqwest::Client,
    config: &PushConfig,
    endpoint: Url,
    body: Option<Vec<u8>>,
) -> anyhow::Result<reqwest::Response> {
    let claims = Claims {
        aud: endpoint.origin().ascii_serialization(),
        exp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 3600,
        sub: &config.subject,
    };
    let token = jsonwebtoken::encode(&Header::new(Algorithm::ES256), &claims, &config.private_key)?;
    let request = client
        .post(endpoint)
        .header(
            "Authorization",
            format!("vapid t={token}, k={}", config.public_key),
        )
        .header("TTL", "300");
    let request = match body {
        Some(body) => request
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .body(body),
        None => request.header("Content-Length", "0"),
    };
    Ok(request.send().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_validation_prevents_ssrf() {
        for endpoint in [
            "http://fcm.googleapis.com/a",
            "https://127.0.0.1/a",
            "https://fcm.googleapis.com.evil.example/a",
            "https://user@fcm.googleapis.com/a",
            "https://fcm.googleapis.com:444/a",
            "https://fcm.googleapis.com/a#b",
            "https://evil.example/?next=https://web.push.apple.com",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
        }
        for endpoint in [
            "https://fcm.googleapis.com/fcm/send/test",
            "https://updates.push.services.mozilla.com/wpush/v2/test",
            "https://web.push.apple.com/test",
            "https://wns2.notify.windows.com/test",
        ] {
            assert!(validate_endpoint(endpoint).is_ok(), "{endpoint}");
        }
    }
    // Public test fixture only; never use this key for a deployment.
    const TEST_PRIVATE: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgNTLZ8xmO7ci3rc/O
pup2/Pw9uu5tLby4HMstZjD7D1ShRANCAAR2MMTJH8KJftvWJgrrjFBsR1QfP++g
UdVeohAJI+VGOla+3dFZIArucgJHO20BQX3Hq4rjyEPc7QUhgmYVRhCf
-----END PRIVATE KEY-----
"#;
    const TEST_PUBLIC_PEM: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEdjDEyR/CiX7b1iYK64xQbEdUHz/v
oFHVXqIQCSPlRjpWvt3RWSAK7nICRzttAUF9x6uK48hD3O0FIYJmFUYQnw==
-----END PUBLIC KEY-----
"#;
    const TEST_PUBLIC: &str =
        "BHYwxMkfwol-29YmCuuMUGxHVB8_76BR1V6iEAkj5UY6Vr7d0VkgCu5yAkc7bQFBfceriuPIQ9ztBSGCZhVGEJ8";

    #[test]
    fn config_requires_complete_settings_and_a_p256_private_key() {
        assert!(
            PushConfig::parse("".into(), "".into(), "".into())
                .unwrap()
                .is_none()
        );
        assert!(
            PushConfig::parse(
                TEST_PUBLIC.into(),
                "".into(),
                "mailto:test@example.com".into()
            )
            .is_err()
        );
        assert!(
            PushConfig::parse(
                "invalid".into(),
                TEST_PRIVATE.into(),
                "mailto:test@example.com".into()
            )
            .is_err()
        );
        assert!(
            PushConfig::parse(TEST_PUBLIC.into(), TEST_PRIVATE.into(), "invalid".into()).is_err()
        );
        assert!(
            PushConfig::parse(
                TEST_PUBLIC.into(),
                TEST_PRIVATE.into(),
                "mailto:test@example.com".into()
            )
            .unwrap()
            .is_some()
        );
    }

    #[tokio::test]
    async fn sends_empty_web_push_with_verifiable_vapid_claims() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_string, header, method},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("TTL", "300"))
            .and(body_string(""))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        let config = PushConfig::parse(
            TEST_PUBLIC.into(),
            TEST_PRIVATE.into(),
            "mailto:test@example.com".into(),
        )
        .unwrap()
        .unwrap();
        let response = send(
            &reqwest::Client::new(),
            &config,
            Url::parse(&server.uri()).unwrap(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(response.status().as_u16(), 201);
        let requests = server.received_requests().await.unwrap();
        let auth = requests[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        let token = auth
            .strip_prefix("vapid t=")
            .unwrap()
            .split(", k=")
            .next()
            .unwrap();
        assert!(auth.ends_with(TEST_PUBLIC));
        let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
        validation.set_audience(&[&server.uri()]);
        let key = jsonwebtoken::DecodingKey::from_ec_pem(TEST_PUBLIC_PEM.as_bytes()).unwrap();
        let claims = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
            .unwrap()
            .claims;
        assert_eq!(claims["sub"], "mailto:test@example.com");
        assert_eq!(claims["aud"], server.uri());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(claims["exp"].as_u64().unwrap() <= now + 3600);
    }
    #[tokio::test]
    async fn encrypted_push_delivers_readable_notification_payload_to_the_subscribed_browser() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method},
        };
        let (receiver, auth) = ece::generate_keypair_and_auth_secret().unwrap();
        let keys = SubscriptionKeys {
            p256dh: URL_SAFE_NO_PAD.encode(receiver.pub_as_raw().unwrap()),
            auth: URL_SAFE_NO_PAD.encode(auth),
        };
        keys.validate().unwrap();
        let payload = NotificationPayload {
            title: "Alice · #general".into(),
            body: "こんにちは、Bob!".into(),
            url: "/channels/123?message=456".into(),
            tag: "zoo-notification".into(),
            notification_type: "mention".into(),
        };
        let encrypted = keys.encrypt(&payload).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("Content-Encoding", "aes128gcm"))
            .and(header("Content-Type", "application/octet-stream"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        let config = PushConfig::parse(
            TEST_PUBLIC.into(),
            TEST_PRIVATE.into(),
            "mailto:test@example.com".into(),
        )
        .unwrap()
        .unwrap();
        send(
            &reqwest::Client::new(),
            &config,
            Url::parse(&server.uri()).unwrap(),
            Some(encrypted),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        let decoded = ece::decrypt(
            &receiver.raw_components().unwrap(),
            &auth,
            &requests[0].body,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoded).unwrap(),
            serde_json::to_value(payload).unwrap()
        );
        assert!(
            !(SubscriptionKeys {
                p256dh: "invalid".into(),
                auth: "invalid".into()
            })
            .validate()
            .is_ok()
        );
    }
    #[test]
    fn payload_previews_unicode_messages_and_links_to_the_correct_message() {
        use serde_json::json;
        let now = crate::models::now_utc_millis();
        let actor: User = serde_json::from_value(json!({
            "id":"actor", "user_name":"alice", "display_name":"Alice", "is_bot":false,
            "created_at":now, "updated_at":null
        }))
        .unwrap();
        let mut channel: Channel = serde_json::from_value(json!({
            "id":"channel", "channel_name":"general", "description":null, "created_user_id":"actor",
            "is_private":false, "is_dm":false, "created_at":now, "updated_at":null, "archive_at":null, "deleted_at":null
        })).unwrap();
        let message: Message = serde_json::from_value(json!({
            "id":"message", "user_id":"actor", "channel_id":"channel", "content":"こんにちは🦊".repeat(100),
            "card":null, "created_at":now, "updated_at":null, "deleted_at":null, "parent_id":null
        })).unwrap();
        let notification: Notification = serde_json::from_value(json!({
            "id":"notification", "user_id":"recipient", "actor_id":"actor", "channel_id":"channel",
            "message_id":"message", "notification_type":"mention", "is_read":false, "created_at":now
        }))
        .unwrap();
        let payload = notification_payload(&notification, &actor, &channel, &message);
        assert_eq!(payload.title, "Alice · #general");
        assert_eq!(
            payload.body,
            message.content.chars().take(240).collect::<String>()
        );
        assert_eq!(payload.url, "/channels/channel?message=message");
        let (receiver, auth) = ece::generate_keypair_and_auth_secret().unwrap();
        let keys = SubscriptionKeys {
            p256dh: URL_SAFE_NO_PAD.encode(receiver.pub_as_raw().unwrap()),
            auth: URL_SAFE_NO_PAD.encode(auth),
        };
        assert!(keys.encrypt(&payload).unwrap().len() <= 4096);
        channel.is_dm = true;
        assert_eq!(
            notification_payload(&notification, &actor, &channel, &message).title,
            "Alice"
        );
    }
}
