use crate::{
    error::AppError,
    models::{ApnsSubscription, Notification},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
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
            _ => anyhow::bail!("APNs環境が正しくありません"),
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
            "APNsのチームIDまたは鍵IDが正しくありません"
        );
        let ec = openssl::pkey::PKey::private_key_from_pem(pem.as_bytes())
            .and_then(|key| key.ec_key())
            .map_err(|_| anyhow::anyhow!("APNsの署名鍵が正しくありません"))?;
        anyhow::ensure!(
            ec.group().curve_name() == Some(openssl::nid::Nid::X9_62_PRIME256V1),
            "APNsにはP-256の鍵が必要です"
        );
        let private_key = EncodingKey::from_ec_pem(pem.as_bytes())
            .map_err(|_| anyhow::anyhow!("APNsの署名鍵が正しくありません"))?;
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

pub(crate) async fn lock_registration(
    tx: &mut toasty::Transaction<'_>,
    id: &str,
) -> Result<(), AppError> {
    // 登録行を作る前に届いた解除も同じ端末の登録と直列化する。
    toasty::sql::query("SELECT 1::text FROM pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(id)
        .exec(tx)
        .await?;
    Ok(())
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
            "端末通知の登録情報が正しくありません".into(),
        ));
    }
    Ok(())
}

fn endpoint(environment: &str, token: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        is_hex(token, 2, 512),
        "APNsの端末トークンが正しくありません"
    );
    let host = match environment {
        "sandbox" => "api.sandbox.push.apple.com",
        "production" => "api.push.apple.com",
        _ => anyhow::bail!("APNs環境が正しくありません"),
    };
    Ok(format!("https://{host}/3/device/{token}"))
}

fn payload(notification: &Notification) -> Value {
    // ロック画面やログアウト直後の端末に、送信者や本文を表示しない。
    json!({"aps": {"alert": {"title": "Zoo", "body": "新しい通知があります"}, "sound": "default", "thread-id": notification.channel_id},
           "notification_id": notification.id, "channel_id": notification.channel_id,
           "message_id": notification.message_id, "user_id": notification.user_id})
}

fn delivery_configs() -> anyhow::Result<Vec<(&'static str, ApnsConfig)>> {
    let mut configs = Vec::new();
    for environment in ["sandbox", "production"] {
        if let Some(config) = ApnsConfig::from_env(environment)? {
            configs.push((environment, config));
        }
    }
    Ok(configs)
}

pub(crate) async fn deliver_batch(
    mut db: toasty::Db,
    notifications: &[Notification],
) -> anyhow::Result<()> {
    let configs = delivery_configs()?;
    if configs.is_empty() {
        return Ok(());
    }
    let client = crate::push::push_client()?;
    for notification in notifications {
        if crate::push::notification_is_deliverable(&mut db, notification).await?
            && deliver(&client, &configs, &mut db, notification)
                .await
                .is_err()
        {
            tracing::warn!("iPhoneへの通知を送信できませんでした");
        }
    }
    Ok(())
}

async fn deliver(
    client: &reqwest::Client,
    configs: &[(&str, ApnsConfig)],
    db: &mut toasty::Db,
    notification: &Notification,
) -> anyhow::Result<()> {
    let subscriptions = ApnsSubscription::filter(
        ApnsSubscription::fields()
            .user_id()
            .eq(notification.user_id.as_str()),
    )
    .exec(db)
    .await?;
    let body = payload(notification);
    for subscription in subscriptions {
        let Some((_, config)) = configs
            .iter()
            .find(|(name, _)| *name == subscription.environment)
        else {
            continue;
        };
        let result = config
            .send(client, &subscription, &body, &notification.id)
            .await;
        match result {
            Ok(Some(invalid_at)) => {
                delete_invalid(db, &subscription, invalid_at).await?;
            }
            Ok(None) => {}
            Err(_) => tracing::warn!("Appleが通知を受理しませんでした"),
        }
    }
    Ok(())
}

async fn delete_invalid(
    db: &mut toasty::Db,
    subscription: &ApnsSubscription,
    invalid_at: jiff::civil::DateTime,
) -> anyhow::Result<()> {
    let mut tx = db.transaction().await?;
    lock_registration(&mut tx, &subscription.id).await?;
    // 配信中に解除・再登録された端末や、Appleの失効時刻より新しい登録を消さない。
    ApnsSubscription::filter(
        ApnsSubscription::fields()
            .id()
            .eq(subscription.id.as_str())
            .and(
                ApnsSubscription::fields()
                    .revocation_hash()
                    .eq(subscription.revocation_hash.as_str()),
            )
            .and(ApnsSubscription::fields().created_at().le(invalid_at)),
    )
    .delete()
    .exec(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn test_device(
    db: &mut toasty::Db,
    subscription: &ApnsSubscription,
    test_id: &str,
) -> Result<(), AppError> {
    let config = ApnsConfig::from_env(&subscription.environment)?
        .ok_or_else(|| AppError::ServiceUnavailable("端末の通知環境が設定されていません".into()))?;
    let client = crate::push::push_client()?;
    let result = config
        .send(
            &client,
            subscription,
            &test_payload(&subscription.user_id, test_id),
            test_id,
        )
        .await
        .map_err(|_| AppError::BadGateway("Appleがテスト通知を受理しませんでした".into()))?;
    if let Some(invalid_at) = result {
        delete_invalid(db, subscription, invalid_at).await?;
        return Err(AppError::BadGateway(
            "Appleがこの端末トークンの失効を通知しました".into(),
        ));
    }
    Ok(())
}

fn test_payload(user: &str, test_id: &str) -> Value {
    json!({"aps":{"alert":{"title":"Zoo","body":"このiPhoneでZooの通知を受信できました。"},"sound":"default"},
           "type":"push_test","user_id":user, "test_id":test_id})
}

impl ApnsConfig {
    async fn send(
        &self,
        client: &reqwest::Client,
        sub: &ApnsSubscription,
        body: &Value,
        collapse_id: &str,
    ) -> anyhow::Result<Option<jiff::civil::DateTime>> {
        let url = endpoint(&sub.environment, &sub.device_token)?;
        let response = client
            .post(url)
            .version(reqwest::Version::HTTP_2)
            .bearer_auth(self.authorization().await?)
            .header("apns-topic", "works.zli.zoo")
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header(
                "apns-expiration",
                (SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 300).to_string(),
            )
            .header("apns-collapse-id", collapse_id)
            .json(body)
            .send()
            .await?;
        delivery_response(response).await
    }
}

async fn delivery_response(
    response: reqwest::Response,
) -> anyhow::Result<Option<jiff::civil::DateTime>> {
    if response.status() == reqwest::StatusCode::OK {
        return Ok(None);
    }
    anyhow::ensure!(
        response.status() == reqwest::StatusCode::GONE,
        "Appleが通知を受理しませんでした"
    );
    let json = response.json::<Value>().await?;
    Ok(Some(invalidation_time(&json)?))
}

fn invalidation_time(body: &Value) -> anyhow::Result<jiff::civil::DateTime> {
    anyhow::ensure!(
        matches!(
            body["reason"].as_str(),
            Some("Unregistered" | "ExpiredToken")
        ),
        "端末トークンの失効を確認できませんでした"
    );
    let millis = body["timestamp"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("失効時刻がありません"))?;
    Ok(jiff::Timestamp::from_millisecond(millis)?
        .to_zoned(jiff::tz::TimeZone::UTC)
        .datetime())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "マイグレーションを適用したPostgreSQLを使用"]
    async fn apns_invalidation_waits_for_device_registration() -> anyhow::Result<()> {
        use std::time::Duration;
        let mut db = crate::db::init_db(&crate::config::Config::from_env().database_url).await?;
        let user = uuid::Uuid::new_v4().to_string();
        let id = digest(&user);
        toasty::sql::statement(
            "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'再登録テスト')",
        )
        .bind(user.as_str())
        .exec(&mut db)
        .await?;
        let invalid_at = crate::models::now_utc_millis();
        let sub = ApnsSubscription::create()
            .id(id.as_str())
            .user_id(user.as_str())
            .device_token(id.as_str())
            .environment("sandbox")
            .revocation_hash(digest("テスト用解除情報"))
            .created_at(invalid_at)
            .exec(&mut db)
            .await?;
        let mut other = db.clone();
        let mut tx = db.transaction().await?;
        lock_registration(&mut tx, &id).await?;
        let mut expired = std::pin::pin!(delete_invalid(&mut other, &sub, invalid_at));
        // 既存行を読み、更新しようとしている登録処理がロック中なら、410は先に削除できない。
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut expired)
                .await
                .is_err()
        );
        ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
            .update()
            .created_at(crate::models::now_utc_millis())
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
        expired.await?;
        assert!(
            ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
                .first()
                .exec(&mut db)
                .await?
                .is_some()
        );
        Ok(())
    }

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
    async fn self_test_is_generic_and_requires_apns_acceptance() {
        let payload = test_payload("自分のID", "今回の通知ID");
        assert_eq!(payload["test_id"], "今回の通知ID");
        assert_eq!(payload["type"], "push_test");
        assert_eq!(payload["user_id"], "自分のID");
        assert_eq!(
            payload["aps"]["alert"]["body"],
            "このiPhoneでZooの通知を受信できました。"
        );
        assert!(payload.get("channel_id").is_none());
        for status in [200, 400, 403, 410, 429, 503] {
            let body = json!({"reason":"Unregistered","timestamp":1_800_000_000_000_i64});
            let response = axum::http::Response::builder()
                .status(status)
                .body(body.to_string())
                .unwrap();
            let result = delivery_response(reqwest::Response::from(response)).await;
            match status {
                200 => assert!(matches!(result, Ok(None))),
                410 => assert!(matches!(result, Ok(Some(_)))),
                _ => assert!(result.is_err()),
            }
        }
    }

    #[test]
    fn invalidation_uses_apples_millisecond_timestamp() {
        let expected = jiff::Timestamp::from_millisecond(1_800_000_000_000)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .datetime();
        for reason in ["Unregistered", "ExpiredToken"] {
            assert_eq!(
                invalidation_time(&json!({"reason":reason, "timestamp":1_800_000_000_000_i64}))
                    .unwrap(),
                expected
            );
        }
        assert!(invalidation_time(&json!({"reason":"Unregistered"})).is_err());
        assert!(
            invalidation_time(
                &json!({"reason":"BadDeviceToken", "timestamp":1_800_000_000_000_i64})
            )
            .is_err()
        );
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
