use super::messages::{CreateMessageRequest, create_message_impl};
use crate::cards::MessageCard;
use crate::{auth::AuthUser, error::AppError, models::Message, state::AppState};
use axum::{
    Extension, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use sha2::{Digest, Sha256};

// Loaded once at startup. Never expose the secret in Debug output or responses.
#[derive(Clone)]
pub(super) struct Settings {
    pub(super) secret: String,
    pub(super) channel: String,
    pub(super) user: String,
}

pub fn router() -> Router<AppState> {
    let setting = (|| {
        let get = |key| std::env::var(key).ok().filter(|s| !s.trim().is_empty());
        Some(Settings {
            secret: get("GITHUB_WEBHOOK_SECRET")?,
            channel: get("GITHUB_WEBHOOK_CHANNEL_ID")?,
            user: get("GITHUB_WEBHOOK_USER_ID")?,
        })
    })();
    Router::new()
        .route(
            "/api/webhooks/github",
            post(receive).layer(axum::extract::DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        .layer(Extension(setting))
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

fn card(event: &str, v: &Value) -> Option<MessageCard> {
    let text = |pointer| {
        v.pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(2000)
            .collect::<String>()
    };
    let (title, url, description) = match event {
        "push" => (
            format!("Push · {}", text("/ref").trim_start_matches("refs/heads/")),
            text("/compare"),
            v["commits"]
                .as_array()
                .map(|cs| {
                    cs.iter()
                        .take(10)
                        .map(|c| {
                            format!(
                                "{} {}",
                                c["id"]
                                    .as_str()
                                    .unwrap_or("")
                                    .chars()
                                    .take(7)
                                    .collect::<String>(),
                                c["message"]
                                    .as_str()
                                    .unwrap_or("")
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                                    .chars()
                                    .take(200)
                                    .collect::<String>()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
        ),
        "pull_request" | "issues" => {
            let key = if event == "issues" {
                "issue"
            } else {
                "pull_request"
            };
            let item = &v[key];
            (
                format!(
                    "{} #{} · {} · {}",
                    if event == "issues" { "Issue" } else { "PR" },
                    item["number"],
                    if item["merged"] == true {
                        "merged".into()
                    } else {
                        text("/action")
                    },
                    item["title"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(300)
                        .collect::<String>()
                ),
                item["html_url"].as_str().unwrap_or("").to_owned(),
                item["body"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(2000)
                    .collect(),
            )
        }
        "release" => (
            format!(
                "Release · {} · {}",
                text("/release/tag_name"),
                text("/action")
            ),
            text("/release/html_url"),
            text("/release/body"),
        ),
        "workflow_run" => (
            format!(
                "Actions · {} · {}",
                text("/workflow_run/name"),
                v.pointer("/workflow_run/conclusion")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| v
                        .pointer("/workflow_run/status")
                        .and_then(Value::as_str)
                        .unwrap_or(""))
            ),
            text("/workflow_run/html_url"),
            text("/workflow_run/head_branch"),
        ),
        _ => return None,
    };
    Some(MessageCard {
        title,
        url,
        description,
        source: "GitHub".into(),
        context: text("/repository/full_name"),
        author: text("/sender/login"),
        badge: event.into(),
    })
}

async fn receive(
    State(state): State<AppState>,
    Extension(settings): Extension<Option<Settings>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let settings =
        settings.ok_or_else(|| AppError::NotFound("GitHub webhook is not configured".into()))?;
    receive_delivery(state, settings, headers, body).await
}

pub(super) async fn receive_delivery(
    state: AppState,
    settings: Settings,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let header = |key| headers.get(key).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !verify(&settings.secret, &body, header("x-hub-signature-256")) {
        return Err(AppError::Unauthorized("Invalid webhook signature".into()));
    }
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Invalid JSON payload".into()))?;
    let event = header("x-github-event");
    if event == "ping" {
        return Ok(StatusCode::OK);
    }
    let Some(card) = card(event, &value) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    persist_card(state, settings, card, header("x-github-delivery")).await
}

pub(super) async fn persist_card(
    state: AppState,
    settings: Settings,
    card: MessageCard,
    delivery: &str,
) -> Result<StatusCode, AppError> {
    uuid::Uuid::parse_str(delivery)
        .map_err(|_| AppError::BadRequest("Missing or invalid webhook delivery ID".into()))?;
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
            user_name: "github".into(),
            display_name: "GitHub".into(),
        },
        CreateMessageRequest {
            id: Some(id),
            channel_id: settings.channel,
            content: card.title.clone(),
            parent_id: None,
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
    fn github_signature_vector() {
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(verify("It's a Secret to Everybody", b"Hello, World!", sig));
        assert!(!verify("wrong", b"Hello, World!", sig));
        assert!(!verify("It's a Secret to Everybody", b"changed", sig));
        assert!(!verify("secret", b"body", "sha256=bad"));
    }
    #[test]
    fn formats_events_and_ignores_unknown_events() {
        let v = json!({"action":"closed", "pull_request":{"number":42,"merged":true,"title":"Fix","html_url":"https://github.com/a/b/pull/42"}});
        assert!(card("pull_request", &v).unwrap().title.contains("merged"));
        assert!(card("unsupported", &v).is_none());
        assert!(
            card("push", &json!({"ref":"refs/heads/main","commits":[]}))
                .unwrap()
                .title
                .contains("Push · main")
        );
    }
}
