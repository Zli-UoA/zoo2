//! 部品の導入前に、Slack の bot 投稿を `messages.card` に取り込んでいた変換（main の `cards.rs`・`export.rs`・
//! `worker.rs` のまま）。
//!
//! 編集の反映（`hints::edit`）で、`card` で取り込んだ行の Slack 側が変わったかを、取り込みと同じ変換で作り直して
//! バイト単位で比べるためだけに使う。保存した部品（旧カードから作ったもの）と、attachments から作り直した部品は
//! 形式が違うので、部品どうしでは比べられない。旧列を消すマイグレーション B で、このモジュールも消す。

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{convert::Resolver, export::SlackMessage};
use crate::cards::{MessageCard, normalize_color};

const IMPORTED_TEXT_CARD_TITLE: &str = "Slack";
const MAX_CARD_DESCRIPTION: usize = 4000;

/// 旧形式で取り込んだときの (本文, card)。本文は、ファイル名で補う前のもの（空なら空のまま）
pub(super) fn render(message: &SlackMessage, resolver: &impl Resolver) -> (String, String) {
    let mut cards = cards(message);
    let mut content = super::convert::mrkdwn_to_mfm(&message.text, resolver);
    if !cards.is_empty() && !content.trim().is_empty() {
        let mut card = text_card(
            super::convert::mrkdwn_to_text(&message.text, resolver)
                .chars()
                .take(MAX_CARD_DESCRIPTION)
                .collect(),
        );
        card.title = IMPORTED_TEXT_CARD_TITLE.into();
        cards.insert(0, card);
    }
    let card = encode_cards(&cards).unwrap_or_default();
    if content.trim().is_empty() {
        content = cards_content(&cards);
    }
    if content.trim().is_empty() {
        content = super::convert::truncate_content(message.attachments_text());
    }
    (content, card)
}

fn cards(message: &SlackMessage) -> Vec<MessageCard> {
    message
        .attachments
        .iter()
        .filter(|a| a.get("from_url").is_none() && a.get("is_app_unfurl").is_none())
        .map(sanitize_attachment)
        .filter_map(|a| card(&a))
        .take(100)
        .collect()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Attachment {
    title: String,
    title_link: String,
    text: String,
    fallback: String,
    pretext: String,
    author_name: String,
    footer: String,
    color: String,
    fields: Vec<Field>,
}

#[derive(Deserialize)]
struct Field {
    #[serde(default)]
    title: String,
    value: String,
}

fn card(value: &Value) -> Option<MessageCard> {
    let attachment: Attachment = serde_json::from_value(value.clone()).ok()?;
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
        return None;
    }
    let card = MessageCard {
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
            normalize_color(&attachment.color)?
        },
        fields: Vec::new(),
    };
    card.encode().ok().map(|_| card)
}

fn text_card(text: String) -> MessageCard {
    MessageCard {
        title: "Webhook".into(),
        description: text,
        url: String::new(),
        source: String::new(),
        context: String::new(),
        author: String::new(),
        badge: String::new(),
        color: String::new(),
        fields: Vec::new(),
    }
}

fn encode_cards(cards: &[MessageCard]) -> Option<String> {
    let mut encoded = Vec::new();
    for card in cards {
        encoded.push(serde_json::from_str::<Value>(&card.encode().ok()?).ok()?);
    }
    match encoded.len() {
        0 => None,
        1 => Some(encoded.remove(0).to_string()),
        _ => Some(serde_json::json!({ "cards": encoded }).to_string()),
    }
}

fn cards_content(cards: &[MessageCard]) -> String {
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

fn clip(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn sanitize_attachment(attachment: &Value) -> Value {
    let mut value = attachment.clone();
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    let text = |object: &Map<String, Value>, key: &str| {
        object
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    if !text(object, "title_link").starts_with("https://") {
        object.remove("title_link");
    }
    if !text(object, "color").is_empty() && normalize_color(&text(object, "color")).is_none() {
        object.remove("color");
    }
    for (key, limit) in [
        ("title", 500),
        ("footer", 2000),
        ("author_name", 2000),
        ("fallback", 3000),
    ] {
        let clipped = clip(&text(object, key), limit);
        if object.contains_key(key) {
            object.insert(key.into(), clipped.into());
        }
    }
    let fields: Vec<Value> = object
        .get("fields")
        .and_then(|v| v.as_array())
        .map(|fields| {
            fields
                .iter()
                .take(10)
                .map(|f| {
                    let get = |k: &str| f.get(k).and_then(|v| v.as_str()).unwrap_or("");
                    serde_json::json!({ "title": clip(get("title"), 50), "value": clip(get("value"), 200) })
                })
                .collect()
        })
        .unwrap_or_default();
    let used: usize = fields
        .iter()
        .map(|f| {
            f["title"].as_str().unwrap_or("").chars().count()
                + f["value"].as_str().unwrap_or("").chars().count()
                + 3
        })
        .sum();
    let pretext = clip(&text(object, "pretext"), 500);
    let budget = 3900usize.saturating_sub(used + pretext.chars().count());
    let body = clip(&text(object, "text"), budget);
    object.insert("fields".into(), fields.into());
    object.insert("pretext".into(), pretext.into());
    object.insert("text".into(), body.into());
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// main の `parse_webhook_cards` のテストと同じ結果になる
    #[test]
    fn matches_the_previous_import() {
        let message: SlackMessage = serde_json::from_value(serde_json::json!({
            "ts": "1700000000.000100",
            "text": "",
            "bot_id": "B1",
            "attachments": [
                {"title":"Ready","title_link":"https://example.com","text":"Healthy","color":"good",
                 "fields":[{"title":"Cluster","value":"production","short":true}]},
                {"text":"Failure","color":"#ff0000"}
            ]
        }))
        .unwrap();
        let (content, card) = render(&message, &crate::components::blockkit::NoResolver);
        let card: Value = serde_json::from_str(&card).unwrap();
        assert_eq!(card["cards"][0]["color"], "#2eb886");
        assert_eq!(
            card["cards"][0]["description"],
            "Healthy\nCluster: production"
        );
        assert_eq!(card["cards"][1]["title"], "Webhook");
        assert_eq!(content, "Ready\nHealthy\nCluster: production\nFailure");
    }
}
