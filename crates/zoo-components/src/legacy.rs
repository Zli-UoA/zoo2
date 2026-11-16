//! 旧形式（`messages.card` の MessageCard と、`messages.app_blocks` の Block 一覧）を部品の木にする。
//! バックフィル、旧形式の API 入力、マイグレーション B までの読み取りで使う。

use crate::model::*;
use serde_json::Value;

/// 旧 webhook の本文カードの見出し。container に入れず、直下の text_display にする。
pub const WEBHOOK_TEXT_TITLE: &str = "Webhook";
/// Slack の取り込みの本文カードの見出し。
pub const SLACK_TEXT_TITLE: &str = "Slack";

/// MFM として解釈されないよう、記号の前に `\` を置く代わりに `<plain>` で囲む。
/// 旧カードは本文を文字のまま表示していたので、見た目を変えない。
pub fn plain(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let special = |c: char| "*_~`[]()<>#$@:\\|>-!?".contains(c) || c == '\n';
    if !text.chars().any(special) {
        return text.to_string();
    }
    // </plain> を含むと閉じてしまうので、その部分だけ分ける
    format!("<plain>{}</plain>", text.replace("</plain>", "</plain><plain></</plain><plain>plain>"))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// 1 枚の MessageCard を container にする。見出しが `Webhook` か `Slack` で本文があれば、text_display 1 つにする。
pub fn card(v: &Value) -> Vec<Component> {
    let title = s(v, "title");
    let description = s(v, "description");
    let fields = v.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
    if (title == WEBHOOK_TEXT_TITLE || title == SLACK_TEXT_TITLE)
        && !description.is_empty()
        && fields.is_empty()
        && s(v, "url").is_empty()
        && s(v, "color").is_empty()
        && s(v, "source").is_empty()
        && s(v, "author").is_empty()
    {
        // Slack の取り込みの本文はもともと MFM（mrkdwn から変換済み）。webhook の本文は文字のまま。
        let content = if title == SLACK_TEXT_TITLE {
            description.to_string()
        } else {
            plain(description)
        };
        return vec![Component::text(content)];
    }
    let mut parts = Vec::new();
    if !s(v, "author").is_empty() {
        parts.push(Component::text(format!("<small>{}</small>", plain(s(v, "author")))));
    }
    let mut heading = if s(v, "url").is_empty() {
        format!("**{}**", plain(title))
    } else {
        format!("**[{}]({})**", plain(title), s(v, "url"))
    };
    if !s(v, "badge").is_empty() {
        heading.push_str(&format!(" `{}`", s(v, "badge").replace('`', "'")));
    }
    parts.push(Component::text(heading));
    if !description.is_empty() {
        parts.push(Component::text(plain(description)));
    }
    let lines: Vec<String> = fields
        .iter()
        .map(|f| {
            let value = s(f, "value");
            let value = if f.get("code").and_then(Value::as_bool).unwrap_or(false) {
                format!("`{}`", value.replace('`', "'"))
            } else {
                plain(value)
            };
            let value = if s(f, "url").is_empty() {
                value
            } else {
                format!("[{value}]({})", s(f, "url"))
            };
            format!("**{}**: {value}", plain(s(f, "title")))
        })
        .collect();
    if !lines.is_empty() {
        parts.push(Component::text(lines.join("\n")));
    }
    let footer: Vec<&str> = [s(v, "source"), s(v, "context")]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect();
    if !footer.is_empty() {
        parts.push(Component::Separator(Separator {
            divider: false,
            ..Default::default()
        }));
        parts.push(Component::text(format!(
            "<small>{}</small>",
            footer.iter().map(|x| plain(x)).collect::<Vec<_>>().join(" · ")
        )));
    }
    // 見出しと説明を 1 つにまとめて、部品の数を減らす
    vec![Component::Container(Container {
        accent_color: parse_color(s(v, "color")),
        components: merge_texts(parts),
        ..Default::default()
    })]
}

/// 続く text_display を 1 つにまとめる（上限の 40 部品に収めるため）。
pub fn merge_texts(items: Vec<Component>) -> Vec<Component> {
    let mut out: Vec<Component> = Vec::new();
    for c in items {
        if let (Component::TextDisplay(next), Some(Component::TextDisplay(prev))) = (&c, out.last_mut()) {
            prev.content.push('\n');
            prev.content.push_str(&next.content);
            continue;
        }
        out.push(c);
    }
    out
}

/// `messages.card` の値（1 枚、または `{"cards":[...]}`）を文書にする。
pub fn cards(stored: &Value) -> Document {
    let list: Vec<Value> = match stored.get("cards").and_then(Value::as_array) {
        Some(cards) => cards.clone(),
        None => vec![stored.clone()],
    };
    Document::new(
        IS_COMPONENTS_V2,
        list.iter().flat_map(card).collect(),
    )
}

/// `messages.app_blocks`（旧 Block の配列）を文書にする。`modal` なら select を label に入れる。
pub fn blocks(list: &Value, modal: bool) -> Document {
    let empty = Vec::new();
    let list = list.as_array().unwrap_or(&empty);
    let mut out: Vec<Component> = Vec::new();
    let mut flags = 0;
    for b in list {
        match s(b, "type") {
            "card" => {
                flags = IS_COMPONENTS_V2;
                out.extend(card(b.get("card").unwrap_or(&Value::Null)));
            }
            "text" => out.push(Component::text(plain(s(b, "text")))),
            "button" => {
                let button = Component::Button(Button {
                    style: button_style::SECONDARY,
                    label: Some(s(b, "label").to_string()),
                    custom_id: Some(s(b, "action_id").to_string()),
                    value: Some(s(b, "value").to_string()).filter(|v| !v.is_empty()),
                    ..Default::default()
                });
                match out.last_mut() {
                    Some(Component::ActionRow(row))
                        if row.components.len() < 5
                            && row.components.iter().all(|c| matches!(c, Component::Button(_))) =>
                    {
                        row.components.push(button)
                    }
                    _ => out.push(Component::ActionRow(ActionRow {
                        id: None,
                        components: vec![button],
                    })),
                }
            }
            "input" => out.push(Component::Label(Label {
                id: None,
                label: s(b, "label").chars().take(45).collect(),
                description: None,
                component: Box::new(Component::TextInput(TextInput {
                    custom_id: s(b, "action_id").to_string(),
                    style: 1,
                    required: Some(b.get("required").and_then(Value::as_bool).unwrap_or(false)),
                    ..Default::default()
                })),
            })),
            "select" => {
                let options = b
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|o| {
                        o.iter()
                            .filter_map(Value::as_str)
                            .take(25)
                            .map(|o| SelectOption {
                                label: o.chars().take(100).collect(),
                                value: o.chars().take(100).collect(),
                                ..Default::default()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let select = StringSelect {
                    custom_id: s(b, "action_id").to_string(),
                    options,
                    placeholder: Some(s(b, "label").chars().take(150).collect()),
                    ..Default::default()
                };
                if modal {
                    out.push(Component::Label(Label {
                        id: None,
                        label: s(b, "label").chars().take(45).collect(),
                        description: None,
                        component: Box::new(Component::StringSelect(select)),
                    }));
                } else {
                    out.push(Component::ActionRow(ActionRow {
                        id: None,
                        components: vec![Component::StringSelect(select)],
                    }));
                }
            }
            _ => {}
        }
    }
    Document::new(flags, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::Context;
    use serde_json::json;

    #[test]
    fn webhook_text_card_becomes_plain_text_display() {
        let doc = cards(&json!({"title":"Webhook","description":"*not bold*"}));
        assert_eq!(doc.components, vec![Component::text("<plain>*not bold*</plain>")]);
        assert!(doc.hides_content());
    }

    #[test]
    fn rich_card_becomes_container() {
        let doc = cards(&json!({"cards":[{"title":"Deploy","url":"https://example.com","color":"good","badge":"ok",
            "fields":[{"title":"Commit","value":"abc","code":true,"url":"https://example.com/c"}],
            "source":"GitHub","context":"now"}, {"title":"Webhook","description":"hi"}]}));
        assert_eq!(doc.components.len(), 2);
        let Component::Container(c) = &doc.components[0] else { panic!() };
        assert_eq!(c.accent_color, Some(0x2eb886));
        assert!(doc.validate(Context::Webhook).is_ok(), "{:?}", doc.validate(Context::Webhook));
        let text = doc.fallback_text(4000);
        assert!(text.contains("**[Deploy](https://example.com)** `ok`"), "{text}");
        assert!(text.contains("**Commit**: [`abc`](https://example.com/c)"), "{text}");
    }

    #[test]
    fn legacy_blocks_group_buttons_and_inputs() {
        let doc = blocks(
            &json!([{"type":"text","text":"Choose"},{"type":"button","action_id":"a","label":"A","value":"1"},
                {"type":"button","action_id":"b","label":"B"},{"type":"input","action_id":"c","label":"Name","required":true},
                {"type":"select","action_id":"d","label":"Pick","options":["x","y"]}]),
            true,
        );
        assert_eq!(doc.flags, 0);
        assert_eq!(doc.components.len(), 4);
        assert!(doc.validate(Context::Modal).is_ok(), "{:?}", doc.validate(Context::Modal));
        let Some(Component::Button(b)) = doc.find("a") else { panic!() };
        assert_eq!(b.value.as_deref(), Some("1"));
    }
}
