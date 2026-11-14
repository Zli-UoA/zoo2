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
        anyhow::ensure!(body.len() <= MAX_PUSH_BYTES, "Push payload is too large");
        Ok(body)
    }

    /// 暗号化チャンネルの通知で、暗号文を入れると上限を超えるなら、暗号文を外して送る。
    /// Service Worker は暗号文がなければ「新しいメッセージ」のまま出す（docs/e2ee.md の「Web Push」）。
    fn encrypt_fitting(&self, payload: &NotificationPayload) -> anyhow::Result<Vec<u8>> {
        match self.encrypt(payload) {
            Err(_) if payload.e2ee.is_some() => self.encrypt(&payload.without_ciphertext()),
            result => result,
        }
    }
}

/// 暗号化した Push の本文の上限（RFC 8030 の 4096 バイト。RFC 8291 の暗号化のあとの長さ）。
pub const MAX_PUSH_BYTES: usize = 4096;

#[derive(Clone, Serialize)]
pub struct NotificationPayload {
    title: String,
    body: String,
    url: String,
    tag: String,
    #[serde(rename = "notificationType")]
    notification_type: String,
    /// 暗号化チャンネルの通知だけに付ける。Service Worker がこれを復号して本文を出す (#365)。
    #[serde(skip_serializing_if = "Option::is_none")]
    e2ee: Option<EncryptedPush>,
    /// 通話の招待 (`call_invite` / `call_invite_end`) だけに付ける、招待の id。
    #[serde(rename = "inviteId", skip_serializing_if = "Option::is_none")]
    invite_id: Option<String>,
}

/// 暗号化チャンネルの通知に付ける、受信端末が復号に使う値。どれもサーバーが元から持つ値で、
/// 平文を含まない（`ciphertext` は Megolm の暗号文のまま）。
#[derive(Clone, Serialize)]
pub struct EncryptedPush {
    /// 通知の宛先。Service Worker が、このユーザーの暗号ストアを選ぶのに使う。
    recipient_id: String,
    /// zoo-crypto の `EncryptedMessage` と同じ形の、メッセージの行（受信時の検証の照合に使う）。
    message: EncryptedRow,
}

#[derive(Clone, Serialize)]
pub struct EncryptedRow {
    channel_id: String,
    message_id: String,
    parent_id: Option<String>,
    user_id: String,
    message_kind: String,
    epoch: i64,
    revision: i64,
    ciphertext: serde_json::Value,
}

impl NotificationPayload {
    fn without_ciphertext(&self) -> Self {
        Self {
            e2ee: None,
            ..self.clone()
        }
    }
}

/// 暗号化チャンネルの Web Push の本文。
pub const ENCRYPTED_MESSAGE_BODY: &str = "新しいメッセージ";

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
        // 暗号化チャンネルの本文はサーバーで読めないので、「新しいメッセージ」を送り、暗号文を
        // `e2ee` に付ける (#358、#365)。Service Worker が復号できたら本文に置き換える。
        body: if channel.encrypted {
            ENCRYPTED_MESSAGE_BODY.to_string()
        } else {
            message.content.chars().take(240).collect()
        },
        url: format!("/channels/{}?message={}", channel.id, message.id),
        tag: format!("zoo-{}", notification.id),
        notification_type: notification.notification_type.clone(),
        e2ee: None,
        invite_id: None,
    }
}

/// 暗号化チャンネルのメッセージの、受信端末が復号に使う列を読む。暗号文がない（削除済みなど）なら `None`。
async fn encrypted_push(
    db: &mut toasty::Db,
    notification: &Notification,
    message: &Message,
) -> anyhow::Result<Option<EncryptedPush>> {
    let mut tx = db.transaction().await?;
    let row = crate::apps::query(
        &mut tx,
        "SELECT json_build_object('ciphertext', ciphertext, 'epoch', epoch, 'revision', revision,
            'message_kind', message_kind)::text
         FROM messages WHERE id = $1 AND deleted_at IS NULL AND ciphertext IS NOT NULL",
        &[&message.id],
    )
    .await?
    .into_iter()
    .next();
    tx.commit().await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(EncryptedPush {
        recipient_id: notification.user_id.clone(),
        message: EncryptedRow {
            channel_id: message.channel_id.clone(),
            message_id: message.id.clone(),
            parent_id: message.parent_id.clone(),
            user_id: message.user_id.clone(),
            message_kind: row["message_kind"].as_str().unwrap_or("message").to_owned(),
            epoch: row["epoch"].as_i64().unwrap_or(0),
            revision: row["revision"].as_i64().unwrap_or(0),
            ciphertext: row["ciphertext"].clone(),
        },
    }))
}

/// 通知から、Web Push で送る内容を作る。メッセージが消えていれば `None`。
async fn build_payload(
    db: &mut toasty::Db,
    notification: &Notification,
) -> anyhow::Result<Option<NotificationPayload>> {
    let Some(message_id) = notification.message_id.as_deref() else {
        // メッセージを持たない通知は不在着信だけ。
        if notification.notification_type == "missed_call" {
            return missed_call_payload(db, notification).await.map(Some);
        }
        return Ok(None);
    };
    let Some(message) = Message::filter(Message::fields().id().eq(message_id))
        .first()
        .exec(db)
        .await?
    else {
        return Ok(None);
    };
    if message.deleted_at.is_some() {
        return Ok(None);
    }
    let actor = User::get_by_id(db, notification.actor_id.as_str()).await?;
    let channel = Channel::get_by_id(db, notification.channel_id.as_str()).await?;
    let mut payload = notification_payload(notification, &actor, &channel, &message);
    if channel.encrypted {
        payload.e2ee = encrypted_push(db, notification, &message).await?;
    }
    Ok(Some(payload))
}

/// 通知 `notification` について、Web Push の暗号化（RFC 8291）の前の本文の JSON を返す（結合テスト用）。
/// 送る本文は、これを購読ごとに暗号化したもの（上限を超えるなら `e2ee` を外す。`encrypt_fitting`）。
pub async fn payload_json(
    db: &mut toasty::Db,
    notification: &Notification,
) -> anyhow::Result<Option<serde_json::Value>> {
    Ok(build_payload(db, notification)
        .await?
        .map(|payload| serde_json::to_value(payload).expect("文字列と JSON だけの構造体")))
}

/// Run only after commit. Delivery errors must never roll back a message or expose endpoint URLs.
pub fn dispatch(state: AppState, notifications: Vec<Notification>) {
    dispatch_db(state.db, notifications);
}

/// `dispatch` の、`AppState` を持たない呼び出し元 (定期処理) 向け。
pub fn dispatch_db(db: toasty::Db, notifications: Vec<Notification>) {
    if notifications.is_empty() {
        return;
    }
    tokio::spawn(async move {
        if deliver(db, notifications).await.is_err() {
            tracing::warn!("Web Push delivery failed");
        }
    });
}

async fn deliver(mut db: toasty::Db, notifications: Vec<Notification>) -> anyhow::Result<()> {
    let Some(config) = PushConfig::from_env()? else {
        return Ok(());
    };
    let client = push_client()?;
    for notification in notifications {
        let Some(payload) = build_payload(&mut db, &notification).await? else {
            continue;
        };
        push_to_user(
            &client,
            &config,
            &mut db,
            &notification.user_id,
            &payload,
            &Delivery::NORMAL,
        )
        .await?;
    }
    Ok(())
}

fn push_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?)
}

/// Web Push の送り方。通話の着信は、短い TTL と `Urgency: high` で送る。
struct Delivery {
    ttl: u32,
    urgent: bool,
    /// 応答不可 (DND) のユーザーには送らない。取り消しの通知は DND でも送る。
    respect_dnd: bool,
}

impl Delivery {
    const NORMAL: Self = Self {
        ttl: 300,
        urgent: false,
        respect_dnd: true,
    };
}

/// 1 人のユーザーの、すべての購読へ送る。
async fn push_to_user(
    client: &reqwest::Client,
    config: &PushConfig,
    db: &mut toasty::Db,
    user_id: &str,
    payload: &NotificationPayload,
    delivery: &Delivery,
) -> anyhow::Result<()> {
    if delivery.respect_dnd {
        let mut tx = db.transaction().await?;
        if crate::apps::one(&mut tx,"SELECT json_build_object('quiet',EXISTS(SELECT 1 FROM user_event_settings WHERE user_id=$1 AND dnd_until>now()))::text",&[user_id]).await?["quiet"]==true{return Ok(());}
    }
    let subscriptions = PushSubscription::filter(PushSubscription::fields().user_id().eq(user_id))
        .exec(db)
        .await?;
    for subscription in subscriptions {
        let Ok(endpoint) = validate_endpoint(&subscription.endpoint) else {
            continue;
        };
        let body = match (subscription.p256dh, subscription.auth_secret) {
            (Some(p256dh), Some(auth)) => {
                match (SubscriptionKeys { p256dh, auth }).encrypt_fitting(payload) {
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
        let result = send_with(client, config, endpoint, body, delivery).await;
        match result {
            Ok(response)
                if response.status().as_u16() == 404 || response.status().as_u16() == 410 =>
            {
                PushSubscription::filter(
                    PushSubscription::fields().id().eq(subscription.id.as_str()),
                )
                .delete()
                .exec(db)
                .await?;
            }
            Ok(response) if response.status().is_success() => {}
            _ => tracing::warn!("Web Push provider did not accept delivery"),
        }
    }
    Ok(())
}

fn display_name(user: &User) -> String {
    let name = if user.display_name.is_empty() {
        &user.user_name
    } else {
        &user.display_name
    };
    name.chars().take(60).collect()
}

/// 不在着信の Web Push の本文。
async fn missed_call_payload(
    db: &mut toasty::Db,
    notification: &Notification,
) -> anyhow::Result<NotificationPayload> {
    let actor = User::get_by_id(db, notification.actor_id.as_str()).await?;
    let channel = Channel::get_by_id(db, notification.channel_id.as_str()).await?;
    let name = display_name(&actor);
    Ok(NotificationPayload {
        title: if channel.is_dm {
            name
        } else {
            format!(
                "{name} · #{}",
                channel.channel_name.chars().take(60).collect::<String>()
            )
        },
        body: "不在着信".into(),
        url: format!("/channels/{}", channel.id),
        tag: format!("zoo-{}", notification.id),
        notification_type: "missed_call".into(),
        e2ee: None,
        invite_id: None,
    })
}

/// 通話の招待の、Web Push に必要な値。
#[derive(Debug, Clone, Deserialize)]
pub struct CallInvitePush {
    pub id: String,
    pub channel_id: String,
    pub inviter_id: String,
    pub invitee_id: String,
}

/// 招待の着信を、呼ばれた人の Web Push へ送る。コミットの後に呼ぶ。失敗しても呼び出しは続く。
pub fn dispatch_call_invites(db: toasty::Db, invites: Vec<CallInvitePush>) {
    if invites.is_empty() {
        return;
    }
    tokio::spawn(async move {
        if deliver_call_invites(db, invites, false).await.is_err() {
            tracing::warn!("Web Push call invite delivery failed");
        }
    });
}

/// 招待が終わった (応答・拒否・取り消し・不在) ことを、出ている通知を閉じるために送る。
pub fn dispatch_call_invite_ends(db: toasty::Db, invites: Vec<CallInvitePush>) {
    if invites.is_empty() {
        return;
    }
    tokio::spawn(async move {
        if deliver_call_invites(db, invites, true).await.is_err() {
            tracing::warn!("Web Push call invite end delivery failed");
        }
    });
}

fn call_invite_tag(invite_id: &str) -> String {
    format!("zoo-call-{invite_id}")
}

async fn deliver_call_invites(
    mut db: toasty::Db,
    invites: Vec<CallInvitePush>,
    end: bool,
) -> anyhow::Result<()> {
    let Some(config) = PushConfig::from_env()? else {
        return Ok(());
    };
    let client = push_client()?;
    for invite in invites {
        let payload = if end {
            NotificationPayload {
                title: String::new(),
                body: String::new(),
                url: format!("/channels/{}", invite.channel_id),
                tag: call_invite_tag(&invite.id),
                notification_type: "call_invite_end".into(),
                e2ee: None,
                invite_id: Some(invite.id.clone()),
            }
        } else {
            let actor = User::get_by_id(&mut db, invite.inviter_id.as_str()).await?;
            let channel = Channel::get_by_id(&mut db, invite.channel_id.as_str()).await?;
            let name = display_name(&actor);
            NotificationPayload {
                title: if channel.is_dm {
                    name
                } else {
                    format!(
                        "{name} · #{}",
                        channel.channel_name.chars().take(60).collect::<String>()
                    )
                },
                body: if channel.is_dm {
                    "着信".into()
                } else {
                    "通話に招待されています".into()
                },
                url: format!("/channels/{}?call_invite={}", channel.id, invite.id),
                tag: call_invite_tag(&invite.id),
                notification_type: "call_invite".into(),
                e2ee: None,
                invite_id: Some(invite.id.clone()),
            }
        };
        let delivery = Delivery {
            ttl: if end { 60 } else { CALL_INVITE_TTL_SECONDS },
            urgent: true,
            respect_dnd: !end,
        };
        push_to_user(
            &client,
            &config,
            &mut db,
            &invite.invitee_id,
            &payload,
            &delivery,
        )
        .await?;
    }
    Ok(())
}

/// 招待の有効な長さ。Web Push の TTL にも使い、期限の過ぎた着信が後から届かないようにする。
pub const CALL_INVITE_TTL_SECONDS: u32 = 30;

#[cfg(test)]
async fn send(
    client: &reqwest::Client,
    config: &PushConfig,
    endpoint: Url,
    body: Option<Vec<u8>>,
) -> anyhow::Result<reqwest::Response> {
    send_with(client, config, endpoint, body, &Delivery::NORMAL).await
}

async fn send_with(
    client: &reqwest::Client,
    config: &PushConfig,
    endpoint: Url,
    body: Option<Vec<u8>>,
    delivery: &Delivery,
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
        .header("TTL", delivery.ttl.to_string());
    let request = if delivery.urgent {
        request.header("Urgency", "high")
    } else {
        request
    };
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
            e2ee: None,
            invite_id: None,
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
            "is_private":false, "is_nsfw":false, "is_dm":false, "created_at":now, "updated_at":null, "archive_at":null, "deleted_at":null
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
        channel.encrypted = true;
        assert_eq!(
            notification_payload(&notification, &actor, &channel, &message).body,
            ENCRYPTED_MESSAGE_BODY
        );
    }

    fn encrypted_payload(ciphertext_len: usize) -> NotificationPayload {
        NotificationPayload {
            title: "Alice · #secret".into(),
            body: ENCRYPTED_MESSAGE_BODY.into(),
            url: "/channels/c?message=m".into(),
            tag: "zoo-n".into(),
            notification_type: "mention".into(),
            invite_id: None,
            e2ee: Some(EncryptedPush {
                recipient_id: "00000000-0000-4000-8000-000000000001".into(),
                message: EncryptedRow {
                    channel_id: "00000000-0000-4000-8000-000000000002".into(),
                    message_id: "00000000-0000-4000-8000-000000000003".into(),
                    parent_id: None,
                    user_id: "00000000-0000-4000-8000-000000000004".into(),
                    message_kind: "message".into(),
                    epoch: 3,
                    revision: 0,
                    ciphertext: serde_json::json!({
                        "algorithm": "m.megolm.v1.aes-sha2",
                        "sender_key": "k".repeat(43),
                        "device_id": "00000000-0000-4000-8000-000000000005",
                        "session_id": "s".repeat(43),
                        "ciphertext": "c".repeat(ciphertext_len),
                    }),
                },
            }),
        }
    }

    fn decrypt_push(
        receiver: &ece::EcKeyComponents,
        auth: &[u8],
        body: &[u8],
    ) -> serde_json::Value {
        assert!(body.len() <= MAX_PUSH_BYTES);
        serde_json::from_slice(&ece::decrypt(receiver, auth, body).unwrap()).unwrap()
    }

    #[test]
    fn encrypted_channels_send_the_ciphertext_only_while_it_fits() {
        let (receiver, auth) = ece::generate_keypair_and_auth_secret().unwrap();
        let keys = SubscriptionKeys {
            p256dh: URL_SAFE_NO_PAD.encode(receiver.pub_as_raw().unwrap()),
            auth: URL_SAFE_NO_PAD.encode(auth),
        };
        let components = receiver.raw_components().unwrap();

        // 収まるなら暗号文を付けて送る（本文は「新しいメッセージ」のまま）
        let small = encrypted_payload(1000);
        let sent = decrypt_push(&components, &auth, &keys.encrypt_fitting(&small).unwrap());
        assert_eq!(sent["body"], ENCRYPTED_MESSAGE_BODY);
        assert_eq!(
            sent["e2ee"]["recipient_id"],
            "00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(sent["e2ee"]["message"]["epoch"], 3);
        assert_eq!(sent["e2ee"]["message"]["message_kind"], "message");
        assert_eq!(
            sent["e2ee"]["message"]["parent_id"],
            serde_json::Value::Null
        );
        assert_eq!(
            sent["e2ee"]["message"]["ciphertext"],
            small.e2ee.as_ref().unwrap().message.ciphertext
        );

        // 上限を超えるなら、暗号文を外して「新しいメッセージ」だけを送る
        let large = encrypted_payload(4000);
        assert!(keys.encrypt(&large).is_err());
        let sent = decrypt_push(&components, &auth, &keys.encrypt_fitting(&large).unwrap());
        assert_eq!(sent["body"], ENCRYPTED_MESSAGE_BODY);
        assert!(sent.get("e2ee").is_none(), "{sent}");

        // 平文チャンネルは e2ee を付けない
        let mut plain = encrypted_payload(0);
        plain.e2ee = None;
        let sent = decrypt_push(&components, &auth, &keys.encrypt_fitting(&plain).unwrap());
        assert!(sent.get("e2ee").is_none(), "{sent}");
    }

    /// 本物の Megolm の暗号文で、どのくらいの本文まで暗号文を付けて送れるか（docs/e2ee.md の
    /// 「Web Push の復号」の目安。長いタイトルと、メンション 1 人のとき）。
    #[test]
    fn megolm_ciphertext_fits_bodies_up_to_about_600_japanese_characters() {
        use zoo_crypto::{
            DeviceAccount, Mentions, MessageContent, OutboundSession, OutgoingMessage,
        };
        let account = DeviceAccount::new(
            "00000000-0000-4000-8000-000000000004",
            "00000000-0000-4000-8000-000000000005",
        )
        .unwrap();
        let (receiver, auth) = ece::generate_keypair_and_auth_secret().unwrap();
        let keys = SubscriptionKeys {
            p256dh: URL_SAFE_NO_PAD.encode(receiver.pub_as_raw().unwrap()),
            auth: URL_SAFE_NO_PAD.encode(auth),
        };
        let fits = |chars: usize| {
            let mut outbound = OutboundSession::new("00000000-0000-4000-8000-000000000002", 3, 0);
            let ciphertext = outbound
                .encrypt(
                    &account,
                    &OutgoingMessage {
                        message_id: "00000000-0000-4000-8000-000000000003".into(),
                        parent_id: None,
                        revision: 0,
                        content: MessageContent {
                            body: "あ".repeat(chars),
                            format: "mfm".into(),
                            kind: "message".into(),
                        },
                        mentions: Mentions {
                            user_ids: vec!["00000000-0000-4000-8000-000000000001".into()],
                            ..Mentions::default()
                        },
                        attachments: Vec::new(),
                    },
                )
                .unwrap();
            let mut payload = encrypted_payload(0);
            payload.title = format!("{} · #{}", "名".repeat(60), "チ".repeat(60));
            payload.e2ee.as_mut().unwrap().message.ciphertext = ciphertext;
            keys.encrypt(&payload).is_ok()
        };
        assert!(fits(550));
        assert!(!fits(700));
    }
}
