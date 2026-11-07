use crate::error::AppError;
use serde::{Deserialize, Serialize};

/// Service-independent card data. Only trusted integration routes may persist it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageCard {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub badge: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<CardField>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardField {
    pub title: String,
    pub value: String,
    #[serde(default)]
    pub short: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub code: bool,
}
impl MessageCard {
    pub fn encode(&self) -> Result<String, AppError> {
        if self.fields.len() > 10
            || self.fields.iter().any(|field| {
                field.title.chars().count() > 100
                    || field.value.chars().count() > 1000
                    || field.url.chars().count() > 2000
                    || (!field.url.is_empty()
                        && !reqwest::Url::parse(&field.url)
                            .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some()))
            })
        {
            return Err(AppError::BadRequest("Invalid card fields".into()));
        }
        if !self.color.is_empty() && normalize_color(&self.color).is_none() {
            return Err(AppError::BadRequest("Invalid card color".into()));
        }
        if self.title.trim().is_empty()
            || self.title.chars().count() > 500
            || self.description.chars().count() > 4000
            || [
                &self.url,
                &self.source,
                &self.context,
                &self.author,
                &self.badge,
            ]
            .iter()
            .any(|s| s.chars().count() > 2000)
        {
            return Err(AppError::BadRequest("Invalid card fields or size".into()));
        }
        if !self.url.is_empty()
            && !reqwest::Url::parse(&self.url)
                .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
        {
            return Err(AppError::BadRequest("Card URL must use HTTPS".into()));
        }
        serde_json::to_string(self).map_err(|e| AppError::Internal(e.into()))
    }
}

pub fn normalize_color(value: &str) -> Option<String> {
    match value {
        "good" => Some("#2eb886".into()),
        "warning" => Some("#daa038".into()),
        "danger" => Some("#a30200".into()),
        _ => {
            let hex = value.strip_prefix('#').unwrap_or(value);
            (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| format!("#{hex}"))
        }
    }
}

/// Slack incoming-webhook subset. Unknown presentation options are ignored.
#[derive(Deserialize, Default)]
#[serde(default)]
struct SlackMessage {
    text: String,
    attachments: Vec<SlackAttachment>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SlackAttachment {
    title: String,
    title_link: String,
    text: String,
    fallback: String,
    pretext: String,
    author_name: String,
    footer: String,
    color: String,
    fields: Vec<SlackField>,
}

#[derive(Deserialize)]
struct SlackField {
    #[serde(default)]
    title: String,
    value: String,
}

pub fn parse_webhook_cards(body: &[u8]) -> Result<Vec<MessageCard>, AppError> {
    let invalid = || AppError::BadRequest("Invalid webhook payload".into());
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    if !value.is_object() {
        return Err(invalid());
    }
    let message: SlackMessage = serde_json::from_value(value).map_err(|_| invalid())?;
    if message.attachments.len() > 100 {
        return Err(invalid());
    }
    let mut cards = Vec::new();
    if !message.text.trim().is_empty() {
        cards.push(MessageCard {
            title: "Webhook".into(),
            description: message.text,
            url: String::new(),
            source: String::new(),
            context: String::new(),
            author: String::new(),
            badge: String::new(),
            color: String::new(),
            fields: Vec::new(),
        });
    }
    for attachment in message.attachments {
        if attachment.fields.len() > 100 {
            return Err(invalid());
        }
        let mut parts = vec![attachment.pretext, attachment.text];
        for field in attachment.fields {
            parts.push(if field.title.is_empty() {
                field.value
            } else {
                format!("{}: {}", field.title, field.value)
            });
        }
        let mut description = parts
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if description.is_empty() && attachment.title.is_empty() {
            description = attachment.fallback;
        }
        if description.trim().is_empty() && attachment.title.trim().is_empty() {
            return Err(invalid());
        }
        cards.push(MessageCard {
            title: if attachment.title.trim().is_empty() {
                "Webhook".into()
            } else {
                attachment.title
            },
            description,
            url: attachment.title_link,
            source: attachment.footer,
            context: String::new(),
            author: attachment.author_name,
            badge: String::new(),
            color: if attachment.color.is_empty() {
                String::new()
            } else {
                normalize_color(&attachment.color).ok_or_else(invalid)?
            },
            fields: Vec::new(),
        });
    }
    if cards.is_empty() {
        return Err(invalid());
    }
    for card in &cards {
        card.encode()?;
    }
    Ok(cards)
}

/// カードを `messages.card` の形にする (1 枚ならそのまま、複数なら `{"cards":[...]}`)。カードがなければ None。
pub fn encode_cards(cards: &[MessageCard]) -> Result<Option<String>, AppError> {
    let mut encoded = Vec::new();
    for card in cards {
        encoded.push(
            serde_json::from_str::<serde_json::Value>(&card.encode()?)
                .map_err(|e| AppError::Internal(e.into()))?,
        );
    }
    Ok(match encoded.len() {
        0 => None,
        1 => Some(encoded.remove(0).to_string()),
        _ => Some(serde_json::json!({ "cards": encoded }).to_string()),
    })
}

/// Keep the notification readable in announcements, admin lists and older clients.
pub fn cards_content(cards: &[MessageCard]) -> String {
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
    fn converts_slack_attachments_and_preserves_colors() {
        let cards = parse_webhook_cards(br##"{"text":"Deploy results","username":"ignored","attachments":[{"title":"Ready","title_link":"https://example.com","text":"Healthy","color":"good","fields":[{"title":"Cluster","value":"production","short":true}]},{"text":"Failure","color":"#ff0000"}]}"##).unwrap();
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[0].description, "Deploy results");
        assert_eq!(cards[1].color, "#2eb886");
        assert_eq!(cards[1].description, "Healthy\nCluster: production");
        assert_eq!(cards[2].color, "#ff0000");
    }
    #[test]
    fn validates_slack_messages_before_persistence() {
        for body in [
            r#"{}"#,
            r#"{"blocks":[]}"#,
            r#"{"attachments":[{"text":"x","color":"url(evil)"}]}"#,
            r#"{"attachments":[{"title":"x","title_link":"javascript:alert(1)"}]}"#,
            r#"{"attachments":[{"text":42}]}"#,
        ] {
            assert!(parse_webhook_cards(body.as_bytes()).is_err(), "{body}");
        }
        assert!(parse_webhook_cards(br#"{"text":"Hello"}"#).is_ok());
        assert!(parse_webhook_cards(br#"{"title":"Legacy"}"#).is_err());
        let oversized =
            serde_json::json!({"attachments": vec![serde_json::json!({"text":"x"}); 101]});
        assert!(parse_webhook_cards(oversized.to_string().as_bytes()).is_err());
    }
    #[test]
    fn accepts_minimal_service_independent_cards() {
        let card: MessageCard = serde_json::from_str(r#"{"title":"Deployment complete"}"#).unwrap();
        assert!(card.encode().unwrap().contains("Deployment complete"));
        assert!(card.source.is_empty());
    }
    #[test]
    fn rejects_invalid_cards_and_unsafe_links() {
        for payload in [
            r#"{"title":""}"#,
            r#"{"title":"test","url":"javascript:alert(1)"}"#,
            r#"{"title":"test","url":"http://example.com"}"#,
        ] {
            assert!(
                serde_json::from_str::<MessageCard>(payload)
                    .unwrap()
                    .encode()
                    .is_err()
            );
        }
        assert!(
            serde_json::from_str::<MessageCard>(r#"{"title":"test","description":42}"#).is_err()
        );
        let card: MessageCard =
            serde_json::from_value(serde_json::json!({"title":"x".repeat(501)})).unwrap();
        assert!(card.encode().is_err());
    }
}
