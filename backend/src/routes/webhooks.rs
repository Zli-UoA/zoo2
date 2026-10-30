//! Token authentication and card persistence for Slack-compatible incoming webhooks.
use super::messages::{CreateMessageRequest, create_message_impl};
use crate::{
    auth::AuthUser, cards::MessageCard, error::AppError, models::Message, state::AppState,
};
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

pub(super) async fn persist_card(
    state: AppState,
    settings: Settings,
    cards: Vec<MessageCard>,
    delivery: &str,
) -> Result<StatusCode, AppError> {
    let mut encoded = Vec::new();
    for card in &cards {
        encoded.push(
            serde_json::from_str::<serde_json::Value>(&card.encode()?)
                .map_err(|e| AppError::Internal(e.into()))?,
        );
    }
    if encoded.is_empty() {
        return Err(AppError::BadRequest("Empty webhook message".into()));
    }
    let content = message_content(&cards);
    let card_data = if encoded.len() == 1 {
        encoded.remove(0)
    } else {
        serde_json::json!({"cards": encoded})
    }
    .to_string();
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
            kind: "message".into(),
            broadcast: false,
            metadata: None,
        },
        false,
        Some(card_data),
    )
    .await?;
    Ok(StatusCode::OK)
}

/// Keep the notification readable in announcements, admin lists and older clients.
fn message_content(cards: &[MessageCard]) -> String {
    cards
        .iter()
        .flat_map(|card| {
            let title = if card.title == "Webhook" && !card.description.is_empty() {
                ""
            } else {
                &card.title
            };
            [title, card.description.as_str()]
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(crate::validation::MAX_MESSAGE_CONTENT_LENGTH)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn message_content_includes_slack_text_and_flux_attachment_bodies() {
        let cards = crate::cards::parse_webhook_cards(br#"{"text":"Deployment results","attachments":[{"text":"Reconciliation failed"},{"title":"Production","text":"Ready"},{"title":"Title only"}]}"#).unwrap();
        assert_eq!(
            message_content(&cards),
            "Deployment results\nReconciliation failed\nProduction\nReady\nTitle only"
        );
    }

    #[test]
    fn message_content_is_bounded_without_truncating_stored_cards_or_splitting_unicode() {
        let payload = serde_json::json!({"attachments": vec![serde_json::json!({"title": "a".repeat(500), "text": "通知😀".repeat(1000)}); 100]});
        let cards = crate::cards::parse_webhook_cards(payload.to_string().as_bytes()).unwrap();
        let content = message_content(&cards);
        assert_eq!(
            content.chars().count(),
            crate::validation::MAX_MESSAGE_CONTENT_LENGTH
        );
        assert!(crate::validation::validate_message_content(&content).is_ok());
        assert!(content.contains("通知😀"));
        assert_eq!(cards.len(), 100);
        assert_eq!(cards[99].description, "通知😀".repeat(1000));
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
