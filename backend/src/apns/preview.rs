use crate::push::{MAX_PUSH_BYTES, NotificationPreview};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, KeyInit as HmacKeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

pub(super) fn attach(
    fallback: Value,
    preview: &NotificationPreview,
    revocation_hash: &str,
    aad: &str,
) -> Value {
    let mut nonce = [0; 12];
    let encrypted = openssl::rand::rand_bytes(&mut nonce)
        .map_err(anyhow::Error::from)
        .and_then(|_| seal(preview, revocation_hash, aad, &nonce));
    let Ok(encrypted) = encrypted else {
        tracing::warn!("iPhoneの通知プレビューを暗号化できませんでした");
        return fallback;
    };
    let mut envelope = fallback.clone();
    envelope["preview_v1"] = json!(encrypted);
    envelope["aps"]["mutable-content"] = json!(1);
    if envelope.to_string().len() <= MAX_PUSH_BYTES {
        envelope
    } else {
        fallback
    }
}

fn cipher(revocation_hash: &str) -> anyhow::Result<Aes256Gcm> {
    let mut mac = <Hmac<Sha256> as HmacKeyInit>::new_from_slice(revocation_hash.as_bytes())?;
    mac.update(b"works.zli.zoo.apns.preview.v1");
    let key = mac.finalize().into_bytes();
    Ok(Aes256Gcm::new_from_slice(&key)?)
}

fn seal(
    preview: &NotificationPreview,
    revocation_hash: &str,
    aad: &str,
    nonce: &[u8; 12],
) -> anyhow::Result<String> {
    let cipher = cipher(revocation_hash)?;
    let plaintext = serde_json::to_vec(preview)?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| anyhow::anyhow!("通知プレビューを暗号化できませんでした"))?;
    let mut combined = nonce.to_vec();
    combined.extend(encrypted);
    Ok(STANDARD.encode(combined))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{apns, models::Notification};

    const USER: &str = "00000000-0000-4000-8000-000000000001";
    const NOTICE: &str = "00000000-0000-4000-8000-000000000002";
    const CHANNEL: &str = "00000000-0000-4000-8000-000000000003";
    const MESSAGE: &str = "00000000-0000-4000-8000-000000000004";

    fn preview() -> NotificationPreview {
        NotificationPreview {
            title: "テスト · #動物園".into(),
            body: "明日🐼を見に行く".into(),
            notification_type: "reaction".into(),
        }
    }

    fn hash() -> String {
        apns::digest(&"ab".repeat(32))
    }

    fn open(envelope: &Value, aad: &str) -> Option<Value> {
        open_for_subscription(envelope, aad, &hash())
    }

    pub(in crate::apns) fn open_for_subscription(
        envelope: &Value,
        aad: &str,
        revocation_hash: &str,
    ) -> Option<Value> {
        let combined = STANDARD.decode(envelope["preview_v1"].as_str()?).ok()?;
        let (nonce, encrypted) = combined.split_at(12);
        let cipher = cipher(revocation_hash).ok()?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: encrypted,
                    aad: aad.as_bytes(),
                },
            )
            .ok()?;
        serde_json::from_slice(&plaintext).ok()
    }

    #[test]
    fn matches_swift_preview_vector() {
        // Node.js の node:crypto で生成した、Swift の復号テストと共通の暗号文。
        let aad = format!("zoo-preview-v1|{USER}|{NOTICE}|{CHANNEL}|{MESSAGE}");
        assert_eq!(
            seal(
                &preview(),
                &hash(),
                &aad,
                &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
            )
            .unwrap(),
            "AAECAwQFBgcICQoLC1FCs/PJe1i1A/cNMCU14PcyVg4nlyrikzDzEQM0+B/vf/jqRnOjmpc22pNE3kgqULM2qOy1rzRitOxPOzcTSvakUaXieEPiKt88VAuGz+TzYtMHdNxD5r9zWKOTGYfmRh/nfKMCFuCUsDopi3K9DP6H/eY="
        );
    }

    #[test]
    fn delivery_encrypts_for_the_current_subscription_and_preserves_routes_within_apns_limit() {
        let notification = Notification {
            id: NOTICE.into(),
            user_id: USER.into(),
            actor_id: "送信者".into(),
            channel_id: CHANNEL.into(),
            message_id: Some(MESSAGE.into()),
            notification_type: "reaction".into(),
            is_read: false,
            created_at: crate::models::now_utc_millis(),
        };
        let aad = format!("zoo-preview-v1|{USER}|{NOTICE}|{CHANNEL}|{MESSAGE}");
        let envelope = apns::payload(&notification, &preview(), &hash());
        assert_eq!(envelope["aps"]["mutable-content"], 1);
        assert_eq!(envelope["aps"]["thread-id"], CHANNEL);
        assert_eq!(envelope["aps"]["alert"]["body"], "新しい通知があります");
        assert!(!envelope.to_string().contains("動物園"));
        assert!(!envelope.to_string().contains("明日"));
        assert_eq!(
            open(&envelope, &aad).unwrap(),
            serde_json::to_value(preview()).unwrap()
        );
        assert!(open(&envelope, &aad.replace(CHANNEL, USER)).is_none());
        let other = apns::payload(&notification, &preview(), &apns::digest("別の端末"));
        assert!(open(&other, &aad).is_none());
        let second = apns::payload(&notification, &preview(), &hash());
        assert_ne!(envelope["preview_v1"], second["preview_v1"]);

        let mut longest = NotificationPreview {
            title: format!("{} · #{}", "\u{0001}".repeat(60), "\u{0002}".repeat(60)),
            body: "\u{0003}".repeat(240),
            notification_type: "thread_reply".into(),
        };
        let within_limit = apns::payload(&notification, &longest, &hash());
        assert!(within_limit.to_string().len() <= MAX_PUSH_BYTES);
        assert_eq!(open(&within_limit, &aad).unwrap()["body"], longest.body);
        longest.body = "超".repeat(4096);
        let fallback = apns::payload(&notification, &longest, &hash());
        assert_eq!(fallback, apns::fallback_payload(&notification));
        assert!(fallback.get("preview_v1").is_none());
        assert!(fallback["aps"].get("mutable-content").is_none());
        assert_eq!(fallback["notification_id"], NOTICE);
        assert_eq!(fallback["channel_id"], CHANNEL);
        assert_eq!(fallback["message_id"], MESSAGE);
        assert_eq!(fallback["user_id"], USER);
    }

    #[test]
    fn self_test_binds_the_preview_to_the_same_recipient_and_test_id() {
        let envelope = apns::test_payload(USER, NOTICE, &hash());
        let encrypted = open(&envelope, &format!("zoo-preview-v1|{USER}|{NOTICE}||")).unwrap();
        assert_eq!(
            encrypted,
            json!({"title":"Zoo · #通知テスト", "body":"このiPhoneでZooの通知を受信できました。", "notificationType":"push_test"})
        );
        assert_eq!(envelope["aps"]["alert"]["title"], "Zoo");
        assert!(!envelope.to_string().contains("通知テスト"));
        assert!(open(&envelope, &format!("zoo-preview-v1|{NOTICE}|{USER}||")).is_none());
    }
}
