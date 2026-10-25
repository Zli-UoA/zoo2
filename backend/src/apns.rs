use crate::{
    error::AppError,
    models::{ApnsSubscription, Channel, ChannelUser, Message, Notification},
    state::AppState,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

pub struct ApnsConfig {
    team_id: String,
    key_id: String,
    private_key: EncodingKey,
    fingerprint: String,
}

impl ApnsConfig {
    pub fn from_env(environment: &str) -> anyhow::Result<Option<Self>> {
        let prefix = match environment {
            "sandbox" => "APNS_SANDBOX",
            "production" => "APNS_PRODUCTION",
            _ => anyhow::bail!("Invalid APNs environment"),
        };
        Self::from_values(
            &std::env::var("APPLE_TEAM_ID").unwrap_or_default(),
            &std::env::var(format!("{prefix}_KEY_ID")).unwrap_or_default(),
            &std::env::var(format!("{prefix}_PRIVATE_KEY")).unwrap_or_default(),
        )
    }

    fn from_values(team: &str, key: &str, pem: &str) -> anyhow::Result<Option<Self>> {
        if key.is_empty() && pem.is_empty() {
            return Ok(None);
        }
        anyhow::ensure!(
            [team, key].iter().all(|s| s.len() == 10
                && s.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())),
            "Invalid APNs team/key identifier"
        );
        let ec = openssl::pkey::PKey::private_key_from_pem(pem.as_bytes())
            .and_then(|key| key.ec_key())
            .map_err(|_| anyhow::anyhow!("Invalid APNs signing key"))?;
        anyhow::ensure!(
            ec.group().curve_name() == Some(openssl::nid::Nid::X9_62_PRIME256V1),
            "APNs requires a P-256 key"
        );
        let private_key = EncodingKey::from_ec_pem(pem.as_bytes())
            .map_err(|_| anyhow::anyhow!("Invalid APNs signing key"))?;
        Ok(Some(Self {
            team_id: team.into(),
            key_id: key.into(),
            private_key,
            fingerprint: digest(&format!("{team}:{key}:{pem}")),
        }))
    }

    async fn authorization(&self) -> anyhow::Result<String> {
        struct Cached {
            fingerprint: String,
            issued: u64,
            token: String,
        }
        static CACHE: OnceLock<Mutex<Vec<Cached>>> = OnceLock::new();
        let mut cache = CACHE.get_or_init(|| Mutex::new(Vec::new())).lock().await;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        cache.retain(|cached| now >= cached.issued && now - cached.issued < 3000);
        if let Some(cached) = cache
            .iter()
            .find(|cached| cached.fingerprint == self.fingerprint)
        {
            return Ok(cached.token.clone());
        }
        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            iat: u64,
        }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let token = jsonwebtoken::encode(
            &header,
            &Claims {
                iss: &self.team_id,
                iat: now,
            },
            &self.private_key,
        )?;
        if cache.len() == 2 {
            cache.remove(0);
        }
        cache.push(Cached {
            fingerprint: self.fingerprint.clone(),
            issued: now,
            token: token.clone(),
        });
        Ok(token)
    }
}

pub fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn is_hex(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value.len().is_multiple_of(2)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn validate_registration(token: &str, environment: &str, secret: &str) -> Result<(), AppError> {
    if !is_hex(token, 2, 512)
        || !["sandbox", "production"].contains(&environment)
        || !is_hex(secret, 64, 64)
    {
        return Err(AppError::BadRequest(
            "Invalid native push registration".into(),
        ));
    }
    Ok(())
}

fn endpoint(environment: &str, token: &str) -> anyhow::Result<String> {
    anyhow::ensure!(is_hex(token, 2, 512), "Invalid APNs device token");
    let host = match environment {
        "sandbox" => "api.sandbox.push.apple.com",
        "production" => "api.push.apple.com",
        _ => anyhow::bail!("Invalid APNs environment"),
    };
    Ok(format!("https://{host}/3/device/{token}"))
}

fn payload(notification: &Notification) -> Value {
    // Generic alerts avoid exposing conversation content on a locked or logged-out device.
    json!({"aps": {"alert": {"title": "Zoo", "body": "新しい通知があります"}, "sound": "default", "thread-id": notification.channel_id},
           "notification_id": notification.id, "channel_id": notification.channel_id,
           "message_id": notification.message_id, "user_id": notification.user_id})
}

pub fn dispatch(state: AppState, notifications: Vec<Notification>) {
    tokio::spawn(async move {
        if deliver(&state, notifications).await.is_err() {
            tracing::warn!("APNs delivery failed");
        }
    });
}

async fn deliver(state: &AppState, notifications: Vec<Notification>) -> anyhow::Result<()> {
    let configurations = [
        ("sandbox", ApnsConfig::from_env("sandbox")?),
        ("production", ApnsConfig::from_env("production")?),
    ];
    if configurations.iter().all(|(_, config)| config.is_none()) {
        return Ok(());
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut db = state.db.clone();
    for notification in notifications {
        let channel = Channel::get_by_id(&mut db, notification.channel_id.as_str()).await?;
        let message = Message::get_by_id(&mut db, notification.message_id.as_str()).await?;
        if channel.deleted_at.is_some() || message.deleted_at.is_some() {
            continue;
        }
        if channel.is_private {
            let member = ChannelUser::filter(
                ChannelUser::fields()
                    .channel_id()
                    .eq(channel.id.as_str())
                    .and(
                        ChannelUser::fields()
                            .user_id()
                            .eq(notification.user_id.as_str()),
                    ),
            )
            .first()
            .exec(&mut db)
            .await?;
            if member.is_none() {
                continue;
            }
        }
        let subscriptions = ApnsSubscription::filter(
            ApnsSubscription::fields()
                .user_id()
                .eq(notification.user_id.as_str()),
        )
        .exec(&mut db)
        .await?;
        for subscription in subscriptions {
            let Some(config) = configurations
                .iter()
                .find(|(environment, _)| *environment == subscription.environment)
                .and_then(|(_, config)| config.as_ref())
            else {
                continue;
            };
            let bearer = config.authorization().await?;
            let destination = endpoint(&subscription.environment, &subscription.device_token)?;
            let result = client
                .post(destination)
                .bearer_auth(&bearer)
                .header("apns-topic", "works.zli.zoo")
                .header("apns-push-type", "alert")
                .header("apns-priority", "10")
                .header("apns-expiration", "0")
                .header("apns-collapse-id", &notification.id)
                .json(&payload(&notification))
                .send()
                .await;
            match result {
                Ok(response) if response.status().as_u16() == 410 => {
                    // Do not delete a newer registration created while this request was in flight.
                    ApnsSubscription::filter(
                        ApnsSubscription::fields()
                            .id()
                            .eq(subscription.id.as_str())
                            .and(
                                ApnsSubscription::fields()
                                    .revocation_hash()
                                    .eq(subscription.revocation_hash.as_str()),
                            ),
                    )
                    .delete()
                    .exec(&mut db)
                    .await?;
                }
                Ok(response) if response.status().is_success() => {}
                _ => tracing::warn!("APNs did not accept delivery"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_rejects_urls_and_invalid_secrets() {
        let secret = "a".repeat(64);
        assert!(validate_registration(&"b".repeat(64), "production", &secret).is_ok());
        assert!(validate_registration(&"b".repeat(128), "sandbox", &secret).is_ok());
        for token in ["", "aa/../../", "AA", "https://evil.example", "a"] {
            assert!(validate_registration(token, "sandbox", &secret).is_err());
        }
        assert!(validate_registration("aa", "https://evil.example", &secret).is_err());
        assert!(validate_registration("aa", "sandbox", "short").is_err());
        assert_eq!(
            endpoint("sandbox", "aabb").unwrap(),
            "https://api.sandbox.push.apple.com/3/device/aabb"
        );
        assert!(endpoint("evil", "aa").is_err());
    }
    #[test]
    fn incomplete_config_fails_closed() {
        assert!(ApnsConfig::from_values("", "", "").unwrap().is_none());
        assert!(ApnsConfig::from_values("ABCDEFGHIJ", "ABCDEFGHIJ", "not a key").is_err());
    }
    #[tokio::test]
    async fn provider_token_is_cached_and_signed_for_team() {
        use openssl::{
            ec::{EcGroup, EcKey},
            nid::Nid,
            pkey::PKey,
        };
        let key = PKey::from_ec_key(
            EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap(),
        )
        .unwrap();
        let pem = String::from_utf8(key.private_key_to_pem_pkcs8().unwrap()).unwrap();
        let config = ApnsConfig::from_values("ABCDEFGHIJ", "KLMNOPQRST", &pem)
            .unwrap()
            .unwrap();
        let token = config.authorization().await.unwrap();
        assert_eq!(config.authorization().await.unwrap(), token);
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("KLMNOPQRST"));
        let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        validation.set_issuer(&["ABCDEFGHIJ"]);
        let decoded = jsonwebtoken::decode::<Value>(
            &token,
            &jsonwebtoken::DecodingKey::from_ec_pem(&key.public_key_to_pem().unwrap()).unwrap(),
            &validation,
        )
        .unwrap();
        assert!(decoded.claims["iat"].as_u64().is_some());
    }
}
