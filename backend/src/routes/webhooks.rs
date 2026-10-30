//! Shared signature validation and card persistence for generic incoming webhooks.
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

pub(super) fn verify(secret: &str, body: &[u8], signature: &str) -> bool {
    let Some(hex) = signature.strip_prefix("sha256=") else {
        return false;
    };
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
    mac.update(body);
    mac.verify_slice(&bytes).is_ok()
}

pub(super) async fn persist_card(
    state: AppState,
    settings: Settings,
    card: MessageCard,
    delivery: &str,
) -> Result<StatusCode, AppError> {
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
            content: card.title.clone(),
            parent_id: None,
            kind: "message".into(),
            broadcast: false,
            metadata: None,
        },
        false,
        Some(card.encode()?),
    )
    .await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn webhook_signature_vector() {
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(verify("It's a Secret to Everybody", b"Hello, World!", sig));
        assert!(!verify("wrong", b"Hello, World!", sig));
        assert!(!verify("It's a Secret to Everybody", b"changed", sig));
        assert!(!verify("secret", b"body", "sha256=bad"));
    }
}
