mod preview;

use crate::{
    error::AppError,
    models::{ApnsSubscription, Notification},
    push::NotificationPreview,
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

fn fallback_payload(notification: &Notification) -> Value {
    // 未対応の端末や復号できない端末にも汎用通知で遷移先を届ける。
    json!({"aps": {"alert": {"title": "Zoo", "body": "新しい通知があります"}, "sound": "default", "thread-id": notification.channel_id},
           "notification_id": notification.id, "channel_id": notification.channel_id,
           "message_id": notification.message_id, "user_id": notification.user_id})
}

fn payload(
    notification: &Notification,
    preview: &NotificationPreview,
    revocation_hash: &str,
) -> Value {
    let aad = format!(
        "zoo-preview-v1|{}|{}|{}|{}",
        notification.user_id,
        notification.id,
        notification.channel_id,
        notification.message_id.as_deref().unwrap_or_default()
    );
    preview::attach(
        fallback_payload(notification),
        preview,
        revocation_hash,
        &aad,
    )
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

pub(crate) fn dispatch(db: toasty::Db, notifications: Vec<Notification>) {
    tokio::spawn(async move {
        if deliver_batch(db, &notifications).await.is_err() {
            tracing::warn!("iPhoneへの通知を送信できませんでした");
        }
    });
}

fn client() -> anyhow::Result<reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = crate::push::push_client()?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

async fn deliver_batch(mut db: toasty::Db, notifications: &[Notification]) -> anyhow::Result<()> {
    let configs = delivery_configs()?;
    if configs.is_empty() {
        return Ok(());
    }
    let client = client()?;
    for notification in notifications {
        if deliver(&client, &configs, &mut db, notification)
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
    for subscription in subscriptions {
        let Some((_, config)) = configs
            .iter()
            .find(|(name, _)| *name == subscription.environment)
        else {
            continue;
        };
        let result = config
            .send(client, db, &subscription, PushEvent::Channel(notification))
            .await;
        match result {
            Ok(DeliveryOutcome::Invalid(invalid_at)) => {
                if delete_invalid(db, &subscription, invalid_at).await.is_err() {
                    tracing::warn!("失効したiPhoneの通知登録を削除できませんでした");
                }
            }
            Ok(DeliveryOutcome::Skipped | DeliveryOutcome::Accepted) => {}
            Err(_) => tracing::warn!("端末への通知を配信できませんでした"),
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PushEvent<'a> {
    Channel(&'a Notification),
    Test(&'a str),
}

impl PushEvent<'_> {
    fn id(&self) -> &str {
        match self {
            Self::Channel(n) => &n.id,
            Self::Test(id) => id,
        }
    }
}

async fn authorized_payload(
    db: &mut dyn toasty::db::Executor,
    subscription: &ApnsSubscription,
    event: PushEvent<'_>,
) -> anyhow::Result<Option<Value>> {
    let notification = match event {
        PushEvent::Channel(n) => Some(n),
        PushEvent::Test(_) => None,
    };
    // 配信先の権限と、編集・暗号化切り替え後の本文を端末ごとに同じスナップショットで決定する。
    let rows = toasty::sql::query(
        "SELECT COALESCE(NULLIF(actor.display_name,''),actor.user_name), c.channel_name, c.is_dm,
             CASE WHEN NOT c.encrypted THEN left(m.content,240) END
         FROM apns_subscriptions s JOIN users u ON u.id=s.user_id
         LEFT JOIN channels c ON c.id=$4
         LEFT JOIN messages m ON m.id=$5 AND m.channel_id=c.id AND m.deleted_at IS NULL
         LEFT JOIN users actor ON actor.id=$7
         WHERE s.id=$1 AND s.revocation_hash=$2 AND s.user_id=$3 AND NOT u.is_deactivated
           AND ($4::text IS NULL OR (c.id IS NOT NULL AND c.deleted_at IS NULL AND actor.id IS NOT NULL
           AND (NOT c.is_private AND NOT c.is_dm AND NOT u.is_guest OR EXISTS(
               SELECT 1 FROM channel_user WHERE user_id=u.id AND channel_id=c.id))
           AND NOT EXISTS(SELECT 1 FROM user_event_settings WHERE user_id=u.id AND dnd_until>now())
           AND (m.id IS NOT NULL OR ($5::text IS NULL AND $6='missed_call'))))",
    )
    .bind(subscription.id.as_str())
    .bind(subscription.revocation_hash.as_str())
    .bind(notification.map_or(subscription.user_id.as_str(), |n| n.user_id.as_str()))
    .bind_typed(notification.map(|n| n.channel_id.as_str()), toasty::schema::db::Type::Text)
    .bind_typed(notification.and_then(|n| n.message_id.as_deref()), toasty::schema::db::Type::Text)
    .bind(notification.map_or("", |n| n.notification_type.as_str()))
    .bind_typed(notification.map(|n| n.actor_id.as_str()), toasty::schema::db::Type::Text)
    .exec(db).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let PushEvent::Channel(notification) = event else {
        return Ok(Some(test_payload(
            &subscription.user_id,
            event.id(),
            &subscription.revocation_hash,
        )));
    };
    use toasty::stmt::Value as Column;
    let [
        Column::String(name),
        Column::String(channel),
        Column::Bool(is_dm),
        content,
    ] = row.as_record_unwrap().fields.as_slice()
    else {
        anyhow::bail!("通知に必要な送信者またはチャンネルを取得できませんでした");
    };
    let content = match content {
        Column::String(text) => Some(text.as_str()),
        Column::Null => None,
        _ => anyhow::bail!("通知のメッセージを取得できませんでした"),
    };
    let preview = NotificationPreview::new(notification, name, channel, *is_dm, content);
    Ok(Some(payload(
        notification,
        &preview,
        &subscription.revocation_hash,
    )))
}

// TRUNCATE と同じ順序で履歴表、容量行をロックし、相互に解放を待つのを防ぐ。
pub(crate) async fn prune_revocations(tx: &mut toasty::Transaction<'_>) -> Result<(), AppError> {
    toasty::sql::statement("LOCK TABLE apns_revocations IN ROW EXCLUSIVE MODE")
        .exec(tx)
        .await?;
    toasty::sql::query("SELECT id::text FROM apns_revocation_capacity WHERE id FOR UPDATE")
        .exec(tx)
        .await?;
    toasty::sql::statement(
        "WITH expired AS (
             SELECT id FROM apns_revocations
             WHERE revoked_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '11 minutes'
             ORDER BY revoked_at LIMIT 128 FOR UPDATE SKIP LOCKED
         ) DELETE FROM apns_revocations r USING expired WHERE r.id=expired.id",
    )
    .exec(tx)
    .await?;
    Ok(())
}

async fn delete_invalid(
    db: &mut toasty::Db,
    subscription: &ApnsSubscription,
    invalid_at: jiff::civil::DateTime,
) -> anyhow::Result<()> {
    let mut tx = db.transaction().await?;
    lock_registration(&mut tx, &subscription.id).await?;
    prune_revocations(&mut tx).await?;
    // 失効後に届く登録を拒否し、実際に削除した世代は後のログアウトでも本人と認識する。
    let capability = digest(&format!(
        "{}:{}",
        subscription.id, subscription.revocation_hash
    ));
    toasty::sql::statement(
        "WITH removed AS (
            DELETE FROM apns_subscriptions WHERE id=$1 AND revocation_hash=$2
              AND created_at <= $3::text::timestamp RETURNING id
         ), markers AS (
            SELECT $1::text AS id UNION ALL SELECT $4::text FROM removed
         )
         INSERT INTO apns_revocations(id,revoked_at,invalidated_at)
         SELECT id, statement_timestamp() AT TIME ZONE 'UTC', $3::text::timestamp FROM markers
         ON CONFLICT (id) DO UPDATE SET revoked_at=EXCLUDED.revoked_at,
             invalidated_at=GREATEST(apns_revocations.invalidated_at, EXCLUDED.invalidated_at)
         WHERE apns_revocations.invalidated_at IS NOT NULL
            OR apns_revocations.revoked_at < (statement_timestamp() AT TIME ZONE 'UTC') - interval '11 minutes'",
    )
    .bind(subscription.id.as_str())
    .bind(subscription.revocation_hash.as_str())
    .bind(invalid_at.to_string())
    .bind(capability)
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
    let client = client()?;
    let result = config
        .send(&client, db, subscription, PushEvent::Test(test_id))
        .await?;
    match result {
        DeliveryOutcome::Accepted => Ok(()),
        DeliveryOutcome::Skipped => Err(AppError::NotFound(
            "この端末の通知登録を確認できません".into(),
        )),
        DeliveryOutcome::Invalid(invalid_at) => {
            delete_invalid(db, subscription, invalid_at).await?;
            Err(AppError::BadGateway(
                "Appleがこの端末トークンの失効を通知しました".into(),
            ))
        }
    }
}

fn test_payload(user: &str, test_id: &str, revocation_hash: &str) -> Value {
    let body = "このiPhoneでZooの通知を受信できました。";
    let fallback = json!({"aps":{"alert":{"title":"Zoo","body":body},"sound":"default"},
                          "type":"push_test","user_id":user, "test_id":test_id});
    preview::attach(
        fallback,
        &NotificationPreview {
            title: "Zoo · #通知テスト".into(),
            body: body.into(),
            notification_type: "push_test".into(),
        },
        revocation_hash,
        &format!("zoo-preview-v1|{user}|{test_id}||"),
    )
}

enum DeliveryOutcome {
    Skipped,
    Accepted,
    Invalid(jiff::civil::DateTime),
}

impl ApnsConfig {
    async fn send(
        &self,
        client: &reqwest::Client,
        db: &mut toasty::Db,
        sub: &ApnsSubscription,
        event: PushEvent<'_>,
    ) -> Result<DeliveryOutcome, AppError> {
        let url = endpoint(&sub.environment, &sub.device_token)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(anyhow::Error::from)?;
        let request = client
            .post(url)
            .version(reqwest::Version::HTTP_2)
            .bearer_auth(self.authorization().await?)
            .header("apns-topic", "works.zli.zoo")
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-expiration", (now.as_secs() + 300).to_string())
            .header("apns-collapse-id", event.id());
        send_if_allowed(request, authorized_payload(db, sub, event)).await
    }
}

async fn send_if_allowed(
    request: reqwest::RequestBuilder,
    authorized: impl Future<Output = anyhow::Result<Option<Value>>>,
) -> Result<DeliveryOutcome, AppError> {
    let Some(body) = authorized.await? else {
        return Ok(DeliveryOutcome::Skipped);
    };
    let response = request
        .json(&body)
        .send()
        .await
        .map_err(|_| AppError::BadGateway("Appleに接続できませんでした".into()))?;
    delivery_response(response)
        .await
        .map_err(|_| AppError::BadGateway("Appleが通知を受理しませんでした".into()))
}

async fn delivery_response(response: reqwest::Response) -> anyhow::Result<DeliveryOutcome> {
    if response.status() == reqwest::StatusCode::OK {
        return Ok(DeliveryOutcome::Accepted);
    }
    anyhow::ensure!(
        response.status() == reqwest::StatusCode::GONE,
        "Appleが通知を受理しませんでした"
    );
    let json = response.json::<Value>().await?;
    Ok(DeliveryOutcome::Invalid(invalidation_time(&json)?))
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

    async fn can_deliver(
        db: &mut dyn toasty::db::Executor,
        sub: &ApnsSubscription,
        notification: Option<&Notification>,
    ) -> anyhow::Result<bool> {
        Ok(authorized_payload(
            db,
            sub,
            notification.map_or(PushEvent::Test("テスト"), PushEvent::Channel),
        )
        .await?
        .is_some())
    }

    #[tokio::test]
    #[ignore = "マイグレーションを適用したPostgreSQLを使用"]
    async fn delivery_requires_current_device_membership_and_visible_message() -> anyhow::Result<()>
    {
        let mut db = crate::db::init_db(&crate::config::Config::from_env().database_url).await?;
        let mut tx = db.transaction().await?;
        let user = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        for id in [&user, &other] {
            toasty::sql::statement(
                "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'通知テスト')",
            )
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
        }
        let channel = uuid::Uuid::new_v4().to_string();
        let message = uuid::Uuid::new_v4().to_string();
        toasty::sql::statement("INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1,$1,$2,true,true)")
            .bind(channel.as_str()).bind(user.as_str()).exec(&mut tx).await?;
        toasty::sql::statement(
            "INSERT INTO messages(id,channel_id,user_id,content) VALUES($1,$2,$3,'通知テスト')",
        )
        .bind(message.as_str())
        .bind(channel.as_str())
        .bind(user.as_str())
        .exec(&mut tx)
        .await?;
        let sub = ApnsSubscription::create()
            .id(digest(&user))
            .user_id(user.as_str())
            .device_token(digest(&other))
            .environment("sandbox")
            .revocation_hash(digest(&message))
            .created_at(crate::models::now_utc_millis())
            .exec(&mut tx)
            .await?;
        let notification = Notification {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user.clone(),
            actor_id: other.clone(),
            channel_id: channel.clone(),
            message_id: Some(message.clone()),
            notification_type: "dm".into(),
            is_read: false,
            created_at: crate::models::now_utc_millis(),
        };
        assert!(!can_deliver(&mut tx, &sub, Some(&notification)).await?);
        toasty::sql::statement(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
        )
        .bind(user.as_str())
        .bind(channel.as_str())
        .exec(&mut tx)
        .await?;
        assert!(can_deliver(&mut tx, &sub, Some(&notification)).await?);
        let other_device = ApnsSubscription::create()
            .id(digest(&channel))
            .user_id(user.as_str())
            .device_token(digest(&message))
            .environment("production")
            .revocation_hash(digest(&other))
            .created_at(crate::models::now_utc_millis())
            .exec(&mut tx)
            .await?;
        let preview = |envelope: Value, n: &Notification, sub: &ApnsSubscription| {
            let aad = format!(
                "zoo-preview-v1|{}|{}|{}|{}",
                n.user_id,
                n.id,
                n.channel_id,
                n.message_id.as_deref().unwrap_or_default()
            );
            preview::tests::open_for_subscription(&envelope, &aad, &sub.revocation_hash).unwrap()
        };
        let first = authorized_payload(&mut tx, &sub, PushEvent::Channel(&notification))
            .await?
            .unwrap();
        assert_eq!(
            preview(first, &notification, &sub),
            json!({"title":"通知テスト","body":"通知テスト","notificationType":"dm"})
        );
        toasty::sql::statement("SAVEPOINT before_edit")
            .exec(&mut tx)
            .await?;
        toasty::sql::statement("UPDATE messages SET content='編集後' WHERE id=$1")
            .bind(message.as_str())
            .exec(&mut tx)
            .await?;
        let edited = authorized_payload(&mut tx, &other_device, PushEvent::Channel(&notification))
            .await?
            .unwrap();
        assert_eq!(
            preview(edited, &notification, &other_device)["body"],
            "編集後"
        );
        toasty::sql::statement("UPDATE channels SET encrypted=true WHERE id=$1")
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
        let redacted =
            authorized_payload(&mut tx, &other_device, PushEvent::Channel(&notification))
                .await?
                .unwrap();
        assert_eq!(
            preview(redacted, &notification, &other_device)["body"],
            crate::push::ENCRYPTED_MESSAGE_BODY
        );
        let missed = Notification {
            message_id: None,
            notification_type: "missed_call".into(),
            ..notification.clone()
        };
        let call = authorized_payload(&mut tx, &sub, PushEvent::Channel(&missed))
            .await?
            .unwrap();
        assert_eq!(preview(call, &missed, &sub)["body"], "不在着信");
        toasty::sql::statement("ROLLBACK TO SAVEPOINT before_edit")
            .exec(&mut tx)
            .await?;
        toasty::sql::statement("SAVEPOINT original_device")
            .exec(&mut tx)
            .await?;
        toasty::sql::statement(
            "UPDATE apns_subscriptions SET revocation_hash='新世代' WHERE id=$1",
        )
        .bind(sub.id.as_str())
        .exec(&mut tx)
        .await?;
        assert!(!can_deliver(&mut tx, &sub, Some(&notification)).await?);
        let renewed = ApnsSubscription::get_by_id(&mut tx, sub.id.as_str()).await?;
        assert!(!can_deliver(&mut tx, &sub, None).await?);
        assert!(can_deliver(&mut tx, &renewed, None).await?);
        assert!(can_deliver(&mut tx, &renewed, Some(&notification)).await?);
        toasty::sql::statement("UPDATE apns_subscriptions SET user_id=$2 WHERE id=$1")
            .bind(sub.id.as_str())
            .bind(other.as_str())
            .exec(&mut tx)
            .await?;
        assert!(!can_deliver(&mut tx, &renewed, Some(&notification)).await?);
        assert!(!can_deliver(&mut tx, &renewed, None).await?);
        toasty::sql::statement("ROLLBACK TO SAVEPOINT original_device")
            .exec(&mut tx)
            .await?;
        toasty::sql::statement(
            "INSERT INTO user_event_settings(user_id,dnd_until) VALUES($1,now()+interval '1 hour')
             ON CONFLICT(user_id) DO UPDATE SET dnd_until=EXCLUDED.dnd_until",
        )
        .bind(user.as_str())
        .exec(&mut tx)
        .await?;
        assert!(!can_deliver(&mut tx, &sub, Some(&notification)).await?);
        assert!(can_deliver(&mut tx, &sub, None).await?);
        toasty::sql::statement("ROLLBACK TO SAVEPOINT original_device")
            .exec(&mut tx)
            .await?;
        toasty::sql::statement("UPDATE users SET is_deactivated=true WHERE id=$1")
            .bind(user.as_str())
            .exec(&mut tx)
            .await?;
        assert!(!can_deliver(&mut tx, &sub, Some(&notification)).await?);
        toasty::sql::statement("ROLLBACK TO SAVEPOINT original_device")
            .exec(&mut tx)
            .await?;
        toasty::sql::statement("SET LOCAL search_path=pg_catalog")
            .exec(&mut tx)
            .await?;
        assert!(
            can_deliver(&mut tx, &sub, Some(&notification))
                .await
                .is_err()
        );
        toasty::sql::statement("ROLLBACK TO SAVEPOINT original_device")
            .exec(&mut tx)
            .await?;
        assert!(can_deliver(&mut tx, &sub, Some(&notification)).await?);
        toasty::sql::statement("UPDATE messages SET deleted_at=now() WHERE id=$1")
            .bind(message.as_str())
            .exec(&mut tx)
            .await?;
        assert!(!can_deliver(&mut tx, &sub, Some(&notification)).await?);
        let missed = Notification {
            notification_type: "missed_call".into(),
            message_id: None,
            ..notification
        };
        assert!(can_deliver(&mut tx, &sub, Some(&missed)).await?);
        toasty::sql::statement("UPDATE channels SET deleted_at=now() WHERE id=$1")
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
        assert!(!can_deliver(&mut tx, &sub, Some(&missed)).await?);
        tx.rollback().await?;
        Ok(())
    }
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
        let marker = toasty::sql::query("SELECT 1::text FROM apns_revocations WHERE id=$1 AND invalidated_at=$2::text::timestamp")
            .bind(id.as_str()).bind(invalid_at.to_string()).exec(&mut db).await?;
        assert_eq!(marker.len(), 1);
        assert!(
            ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
                .first()
                .exec(&mut db)
                .await?
                .is_some()
        );
        // 410が本当に行を削除した場合は、後からログアウトする同じ世代の解除情報も保持する。
        let next_invalidation = crate::models::now_utc_millis();
        delete_invalid(&mut db, &sub, next_invalidation).await?;
        assert!(
            ApnsSubscription::filter(ApnsSubscription::fields().id().eq(id.as_str()))
                .first()
                .exec(&mut db)
                .await?
                .is_none()
        );
        let capability = digest(&format!("{id}:{}", sub.revocation_hash));
        let markers = toasty::sql::query(
            "SELECT id FROM apns_revocations WHERE id IN ($1,$2) AND invalidated_at=$3::text::timestamp",
        ).bind(id.as_str()).bind(capability).bind(next_invalidation.to_string()).exec(&mut db).await?;
        assert_eq!(markers.len(), 2);
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
    async fn authorization_failure_is_distinct_from_provider_failure() {
        let server = wiremock::MockServer::start().await;
        let client = client().unwrap();
        let request = || client.post(server.uri());
        let failed = send_if_allowed(request(), async {
            anyhow::bail!("端末の照会に失敗")
        })
        .await;
        assert!(matches!(failed, Err(AppError::Internal(_))));
        assert!(matches!(
            send_if_allowed(request(), async { Ok(None) }).await,
            Ok(DeliveryOutcome::Skipped)
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(matches!(
            send_if_allowed(request(), async { Ok(Some(json!({}))) }).await,
            Err(AppError::BadGateway(_))
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn self_test_is_generic_and_requires_apns_acceptance() {
        let payload = test_payload("自分のID", "今回の通知ID", &digest("テスト端末"));
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
                200 => assert!(matches!(result, Ok(DeliveryOutcome::Accepted))),
                410 => assert!(matches!(result, Ok(DeliveryOutcome::Invalid(_)))),
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
