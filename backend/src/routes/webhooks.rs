//! Token authentication and card persistence for Slack-compatible incoming webhooks.
use super::messages::{CreateMessageRequest, create_message_impl};
use crate::components::Ingested;
use crate::{auth::AuthUser, error::AppError, models::Message, state::AppState};
use axum::http::StatusCode;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

pub(super) struct Settings {
    pub(super) channel: String,
    pub(super) user: String,
}

pub(super) const WEBHOOK_TOKEN_CONTEXT: &[u8] = b"zoo:incoming-webhook:v1";

pub(super) fn webhook_token(secret: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(WEBHOOK_TOKEN_CONTEXT);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(super) fn verify_token(secret: &str, hex: &str) -> bool {
    if hex.len() != 64 || !hex.is_ascii() {
        return false;
    }
    let Ok(bytes) = (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(WEBHOOK_TOKEN_CONTEXT);
    mac.verify_slice(&bytes).is_ok()
}

/// 受信 Webhook の投稿を保存する。部品があれば `components` に入れ、本文には代わりの文字を入れる。
/// 本文のメンションは通知しない（`notify` は false）。
pub(super) async fn persist_components(
    state: AppState,
    settings: Settings,
    ingested: Ingested,
    delivery: &str,
) -> Result<StatusCode, AppError> {
    let components = ingested.doc.as_ref().map(|d| d.to_json());
    let content: String = ingested
        .content
        .chars()
        .take(crate::validation::MAX_MESSAGE_CONTENT_LENGTH)
        .collect();
    uuid::Uuid::parse_str(delivery)
        .map_err(|_| AppError::BadRequest("Missing or invalid webhook delivery ID".into()))?;
    // Retain the legacy namespace so retries of existing generic deliveries remain idempotent.
    let digest = Sha256::digest(format!("github:{}:{delivery}", settings.channel));
    let id = uuid::Uuid::from_slice(&digest[..16])
        .expect("16 bytes")
        .to_string();
    let mut db = state.db.clone();
    if Message::filter(Message::fields().id().eq(id.as_str()))
        .first()
        .exec(&mut db)
        .await?
        .is_some()
    {
        return Ok(StatusCode::OK);
    }
    let _ = create_message_impl(
        state,
        AuthUser {
            id: settings.user,
            user_name: "webhook".into(),
            display_name: "Webhook".into(),
        },
        CreateMessageRequest {
            id: Some(id),
            channel_id: settings.channel,
            content,
            parent_id: None,
            attachment_ids: None,
            sensitive: false,
            sensitive_attachment_ids: vec![],
            kind: "message".into(),
            broadcast: false,
            metadata: None,
            ciphertext: None,
            epoch: None,
            revision: None,
            mentions: None,
        },
        false,
        components,
        None,
        None,
        None,
    )
    .await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_includes_slack_text_and_flux_attachment_bodies() {
        let i = crate::components::webhook(br#"{"text":"Deployment results","attachments":[{"text":"Reconciliation failed"},{"title":"Production","text":"Ready"},{"title":"Title only"}]}"#).unwrap();
        assert_eq!(
            i.content,
            "Deployment results\nReconciliation failed\n**Production**\nReady\n**Title only**"
        );
    }

    #[test]
    fn content_is_bounded_without_splitting_unicode() {
        let payload = serde_json::json!({"attachments": vec![serde_json::json!({"title": "a".repeat(100), "text": "通知😀".repeat(20)}); 30]});
        let i = crate::components::webhook(payload.to_string().as_bytes()).unwrap();
        assert!(i.content.chars().count() <= crate::validation::MAX_MESSAGE_CONTENT_LENGTH);
        assert!(crate::validation::validate_message_content(&i.content).is_ok());
        assert!(i.content.contains("通知😀"));
    }
    #[test]
    fn verifies_webhook_url_tokens() {
        let token = webhook_token("secret");
        assert!(verify_token("secret", &token));
        assert!(!verify_token("rotated", &token));
        assert!(!verify_token("secret", "bad"));
        assert!(!verify_token("secret", &"0".repeat(64)));
    }
}
