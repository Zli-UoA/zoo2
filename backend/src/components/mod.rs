//! メッセージ部品（Discord Components V2 形式）の取り込み。
//!
//! 入力は Discord（`components`・`flags`・`embeds`）、Slack Block Kit（`blocks`・`attachments`・`text`）、
//! 旧 zoo の Block 一覧の 3 つ。どれも [`zoo_components::Document`] にする。
//! - [`webhook`]: 受信 Webhook。寛容に受け取り、操作部品は押せない状態にする。
//! - [`app`]: App API。知らない部品は 400 にする。

pub mod backfill;
pub mod blockkit;
pub mod discord;

use crate::error::AppError;
use crate::slack::convert::{Resolver, mrkdwn_to_mfm};
use blockkit::{Converter, Mode, NoResolver};
use serde_json::Value;
pub use zoo_components::*;

/// 入力の形式（メトリクスと Deprecation ヘッダーに使う）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Discord,
    Slack,
    Legacy,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Discord => "discord",
            Format::Slack => "slack",
            Format::Legacy => "legacy",
        }
    }
}

/// 部品から作る代わりの文字。部品に文字がない（線や操作部品だけなど）ときも、本文が空にならないようにする
/// （本文が空の投稿は保存できない）。
fn fallback_content(doc: &Document) -> String {
    let text = doc.fallback_text(crate::validation::MAX_MESSAGE_CONTENT_LENGTH);
    if text.trim().is_empty() {
        "（メッセージ部品）".to_string()
    } else {
        text
    }
}

/// 取り込んだ結果。`doc` が None なら部品はなく、本文だけの投稿にする。
#[derive(Debug)]
pub struct Ingested {
    pub doc: Option<Document>,
    pub content: String,
    pub format: Format,
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::BadRequest(msg.into())
}

fn errors_message(errors: &[ValidationError]) -> String {
    let list: Vec<String> = errors.iter().take(5).map(ToString::to_string).collect();
    format!("Invalid components: {}", list.join("; "))
}

fn arr<'v>(v: &'v Value, k: &str) -> &'v [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// 寛容に読む：壊れた部品は「未対応の部品」にして文字だけ残す。
fn lenient_components(list: &[Value]) -> Vec<Component> {
    list.iter()
        .map(|v| {
            Component::from_value(v.clone()).unwrap_or_else(|_| {
                Component::Unsupported(Unsupported {
                    id: None,
                    original_type: v.get("type").map(|t| t.to_string()).unwrap_or_default(),
                    text: salvage_text(v),
                })
            })
        })
        .collect()
}

/// 検証に通らない部品を、トップレベルの単位で「未対応の部品」に置き換える。
/// 大きさの上限を超えた場合は直せないので、エラーのまま返す。
fn repair(doc: &mut Document, cx: Context) -> Result<(), Vec<ValidationError>> {
    let Err(errors) = doc.validate(cx) else {
        return Ok(());
    };
    let mut indices: Vec<usize> = errors
        .iter()
        .filter_map(|e| {
            e.path
                .strip_prefix("components[")
                .and_then(|r| r.split(']').next())
                .and_then(|i| i.parse().ok())
        })
        .collect();
    indices.sort_unstable();
    indices.dedup();
    for i in indices {
        if let Some(c) = doc.components.get_mut(i) {
            let one = Document::new(0, vec![c.clone()]);
            *c = Component::Unsupported(Unsupported {
                id: None,
                original_type: kind::name(c.kind()).unwrap_or("component").to_string(),
                text: one.fallback_text(limits::MAX_TEXT),
            });
        }
    }
    doc.validate(cx)
}

/// 上限（部品の数・文字数・大きさ）に収まるまで末尾の部品を外し、省いたことを書き足す。
fn truncate(doc: &mut Document) {
    let over = |d: &Document| match d.validate(Context::Webhook) {
        Ok(()) => false,
        Err(e) => e.iter().any(|e| e.path.is_empty()),
    };
    if !over(doc) {
        return;
    }
    let total = doc.components.len();
    while doc.components.len() > 1 && over(doc) {
        doc.components.pop();
    }
    if doc.components.len() < total {
        // 省いたことを知らせる部品を足し、それで上限を超えるなら、その前の部品をもう 1 つ省く
        let notice = |omitted: usize| {
            Component::text(format!(
                "<small>（ほかに {omitted} 件の部品を省略しました）</small>"
            ))
        };
        doc.components.push(notice(total - doc.components.len()));
        while doc.components.len() > 2 && over(doc) {
            let at = doc.components.len() - 2;
            doc.components.remove(at);
            let omitted = total - (doc.components.len() - 1);
            *doc.components.last_mut().unwrap() = notice(omitted);
        }
    }
}

/// Webhook と Slack の取り込みに共通の後処理。操作部品を押せない状態にし、検証する。
fn finish_lenient(
    mut doc: Document,
    source: &'static str,
    format: Format,
) -> Result<Document, AppError> {
    doc.disable_interactive();
    truncate(&mut doc);
    repair(&mut doc, Context::Webhook).map_err(|e| bad(errors_message(&e)))?;
    let unsupported = doc.unsupported_types();
    if !unsupported.is_empty() {
        tracing::info!(source, types = ?unsupported, "message components kept unsupported types as text");
    }
    crate::metrics::record_message_components(source, format.as_str(), !unsupported.is_empty());
    Ok(doc)
}

fn is_discord(v: &Value) -> bool {
    ["components", "flags", "embeds", "content"]
        .iter()
        .any(|k| v.get(*k).is_some())
        && ["blocks", "attachments", "text"]
            .iter()
            .all(|k| v.get(*k).is_none())
}

/// 受信 Webhook の本文を読む。Slack 形式と Discord 形式の両方を受け付ける。
pub fn webhook(body: &[u8]) -> Result<Ingested, AppError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| bad("Invalid webhook payload"))?;
    if !value.is_object() {
        return Err(bad("Invalid webhook payload"));
    }
    if body.len() > 256 * 1024 {
        return Err(bad("Webhook payload too large"));
    }
    if is_discord(&value) {
        discord_message(&value, "webhook")
    } else {
        slack_message(&value, &NoResolver, "webhook")
    }
}

/// Discord の Webhook（`content`・`embeds`・`components`・`flags`）を読む。
fn discord_message(v: &Value, source: &'static str) -> Result<Ingested, AppError> {
    let text = v
        .get("content")
        .and_then(Value::as_str)
        .map(discord::markdown_to_mfm)
        .unwrap_or_default();
    let mut components = lenient_components(arr(v, "components"));
    #[allow(unused_mut)]
    let mut doc = Document::new(0, std::mem::take(&mut components));
    discord::normalize(&mut doc);
    let mut components = std::mem::take(&mut doc.components);
    components.extend(discord::embeds(arr(v, "embeds")));
    if components.is_empty() {
        if text.trim().is_empty() {
            return Err(bad("Empty webhook message"));
        }
        return Ok(Ingested {
            doc: None,
            content: text,
            format: Format::Discord,
        });
    }
    // 本文も部品として先頭に置き、本文の列には代わりの文字を入れる
    if !text.trim().is_empty() {
        components.insert(0, Component::text(text));
    }
    doc.flags = IS_COMPONENTS_V2;
    doc.components = components;
    let doc = finish_lenient(doc, source, Format::Discord)?;
    let content = fallback_content(&doc);
    Ok(Ingested {
        doc: Some(doc),
        content,
        format: Format::Discord,
    })
}

/// Slack の投稿（`text`・`blocks`・`attachments`）を読む。Slack の取り込みも使う。
pub fn slack_message(
    v: &Value,
    resolver: &impl Resolver,
    source: &'static str,
) -> Result<Ingested, AppError> {
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    let blocks = arr(v, "blocks");
    let attachments = arr(v, "attachments");
    if blocks.len() > 100 || attachments.len() > 100 {
        return Err(bad("Invalid webhook payload"));
    }
    let mut conv = Converter::new(Mode::Lenient, resolver);
    let mut components = conv.blocks(blocks).map_err(bad)?;
    let atts = blockkit::attachments(&mut conv, attachments);
    let text_mfm = if text.trim().is_empty() {
        String::new()
    } else {
        mrkdwn_to_mfm(text, resolver)
    };
    // Slack では blocks があると text は通知用の代わりの文字になる。blocks がなければ text を表示する。
    if blocks.is_empty() && !text_mfm.is_empty() && !atts.is_empty() {
        components.push(Component::text(text_mfm.clone()));
    }
    components.extend(atts);
    if components.is_empty() {
        if text_mfm.is_empty() {
            return Err(bad("Empty webhook message"));
        }
        return Ok(Ingested {
            doc: None,
            content: text_mfm,
            format: Format::Slack,
        });
    }
    let doc = finish_lenient(
        Document::new(IS_COMPONENTS_V2, components),
        source,
        Format::Slack,
    )?;
    let content = if !blocks.is_empty() && !text_mfm.is_empty() {
        text_mfm
            .chars()
            .take(crate::validation::MAX_MESSAGE_CONTENT_LENGTH)
            .collect()
    } else {
        fallback_content(&doc)
    };
    Ok(Ingested {
        doc: Some(doc),
        content,
        format: Format::Slack,
    })
}

/// 旧 zoo の Block 一覧か（Slack の blocks とは type が重ならない）。
fn is_legacy_blocks(list: &[Value]) -> bool {
    !list.is_empty()
        && list
            .iter()
            .all(|b| match b.get("type").and_then(Value::as_str) {
                // 旧形式の card は中身を `card` キーに持つ（Slack の card ブロックはトップレベルに title などを持つ）
                Some("card") => b.get("card").is_some_and(Value::is_object),
                Some("text" | "button" | "select") => true,
                Some("input") => b.get("label").is_some_and(Value::is_string),
                _ => false,
            })
}

/// App API の入力を読む。受け付けるのは次の 4 つ。
/// - Discord：`{"flags":..,"components":[..]}`
/// - Slack：`{"blocks":[..]}`
/// - 旧形式：`[..]` か `{"blocks":[..]}`（旧 Block の一覧）
/// - `"format"` で形式を明示したもの（`discord`・`slack`・`legacy`）
pub fn app(
    value: &Value,
    cx: Context,
    source: &'static str,
) -> Result<(Document, Format), AppError> {
    let modal = cx == Context::Modal;
    let explicit = value.get("format").and_then(Value::as_str);
    let (mut doc, format) = match (value, explicit) {
        (Value::Array(list), None) => (
            legacy::blocks(&Value::Array(list.clone()), modal),
            Format::Legacy,
        ),
        (Value::Object(_), Some("legacy")) => (
            legacy::blocks(value.get("blocks").unwrap_or(&Value::Null), modal),
            Format::Legacy,
        ),
        (Value::Object(_), Some("slack")) => (slack_strict(arr(value, "blocks"))?, Format::Slack),
        (Value::Object(_), Some("discord")) => (discord_strict(value)?, Format::Discord),
        (Value::Object(_), None)
            if value.get("components").is_some() || value.get("flags").is_some() =>
        {
            (discord_strict(value)?, Format::Discord)
        }
        (Value::Object(_), None) if is_legacy_blocks(arr(value, "blocks")) => (
            legacy::blocks(value.get("blocks").unwrap(), modal),
            Format::Legacy,
        ),
        (Value::Object(_), None) if value.get("blocks").is_some() => {
            (slack_strict(arr(value, "blocks"))?, Format::Slack)
        }
        _ => return Err(bad("Invalid component document")),
    };
    if format == Format::Legacy {
        let blocks = match value {
            Value::Array(_) => value,
            _ => value.get("blocks").unwrap_or(&Value::Null),
        };
        legacy::lossless_blocks(blocks).map_err(|e| {
            bad(format!(
                "Legacy blocks cannot be converted without losing data: {e}"
            ))
        })?;
        // 旧形式の検証（action_id の長さなど）も通す。旧形式の文書は新しい検証にも通る
        crate::apps::ui::validate_blocks(match value {
            Value::Array(_) => value,
            _ => value.get("blocks").unwrap_or(&Value::Null),
        })?;
    }
    if cx == Context::Unfurl {
        doc.flags = 0;
    }
    if let Err(e) = doc.validate(cx) {
        return Err(bad(errors_message(&e)));
    }
    if !doc.unsupported_types().is_empty() {
        return Err(bad(format!(
            "Unsupported component types: {:?}",
            doc.unsupported_types()
        )));
    }
    crate::metrics::record_message_components(source, format.as_str(), false);
    Ok((doc, format))
}

fn discord_strict(value: &Value) -> Result<Document, AppError> {
    let mut doc: Document = serde_json::from_value(serde_json::json!({
        "flags": value.get("flags").cloned().unwrap_or(Value::from(0)),
        "components": value.get("components").cloned().unwrap_or(Value::Array(vec![])),
    }))
    .map_err(|e| bad(format!("Invalid component document: {e}")))?;
    discord::normalize(&mut doc);
    Ok(doc)
}

fn slack_strict(blocks: &[Value]) -> Result<Document, AppError> {
    let components = Converter::new(Mode::Strict, &NoResolver)
        .blocks(blocks)
        .map_err(bad)?;
    // Slack のメッセージは text が通知用なので、blocks だけで本文を置き換える
    Ok(Document::new(IS_COMPONENTS_V2, components))
}

/// 保存した文書（`messages.components`）を読む。壊れていれば None。
pub fn stored(json: &str) -> Option<Document> {
    Document::parse(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hook(v: Value) -> Ingested {
        webhook(v.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn slack_text_only_is_plain_content() {
        let i = hook(json!({"text":"*deploy* ok"}));
        assert!(i.doc.is_none());
        assert_eq!(i.content, "**deploy** ok");
    }

    #[test]
    fn slack_blocks_use_text_as_fallback_and_disable_buttons() {
        let i = hook(
            json!({"text":"fallback","blocks":[{"type":"section","text":{"type":"mrkdwn","text":"hi"}},
            {"type":"actions","elements":[{"type":"button","text":{"type":"plain_text","text":"Approve"},"action_id":"ok"},
             {"type":"button","text":{"type":"plain_text","text":"Open"},"url":"https://e.x"}]},
            {"type":"brand_new_block","text":{"type":"plain_text","text":"future"}}]}),
        );
        let doc = i.doc.unwrap();
        assert_eq!(i.content, "fallback");
        assert!(doc.hides_content());
        let Some(Component::Button(b)) = doc.find("ok") else {
            panic!()
        };
        assert!(b.disabled);
        assert_eq!(doc.unsupported_types(), vec!["slack:brand_new_block"]);
    }

    #[test]
    fn slack_attachments_keep_text_first() {
        let i = hook(
            json!({"text":"Deployment results","attachments":[{"text":"Reconciliation failed","color":"danger"}]}),
        );
        let doc = i.doc.unwrap();
        assert_eq!(doc.components[0], Component::text("Deployment results"));
        assert_eq!(i.content, "Deployment results\nReconciliation failed");
    }

    #[test]
    fn discord_v2_and_embeds_are_accepted() {
        let i = hook(
            json!({"username":"bot","flags":32768,"components":[{"type":17,"components":[{"type":10,"content":"-# hi"}]},
            {"type":1,"components":[{"type":2,"style":1,"label":"x","custom_id":"x"}]}]}),
        );
        let doc = i.doc.unwrap();
        assert_eq!(i.content, "<small>hi</small>");
        assert!(matches!(doc.find("x"), Some(Component::Button(b)) if b.disabled));
        let e = hook(json!({"content":"Build","embeds":[{"title":"ok","color":65280}]}));
        assert_eq!(e.doc.unwrap().components.len(), 2);
    }

    #[test]
    fn invalid_discord_parts_are_kept_as_text() {
        let i = hook(
            json!({"components":[{"type":17,"components":[{"type":17,"components":[{"type":10,"content":"inner"}]}]}]}),
        );
        let doc = i.doc.unwrap();
        assert!(matches!(&doc.components[0], Component::Unsupported(u) if u.text == "inner"));
    }

    #[test]
    fn truncation_leaves_room_for_the_notice() {
        let many: Vec<Value> = (0..501)
            .map(|i| json!({"type":10,"content":format!("t{i}")}))
            .collect();
        let i = hook(json!({"components": many}));
        let doc = i.doc.unwrap();
        assert!(doc.validate(Context::Webhook).is_ok());
        assert!(
            matches!(doc.components.last(), Some(Component::TextDisplay(t)) if t.content.contains("件の部品を省略しました"))
        );
    }

    #[test]
    fn legacy_blocks_that_would_lose_options_are_rejected() {
        let options: Vec<String> = (0..26).map(|i| format!("o{i}")).collect();
        let err = app(
            &json!([{"type":"select","action_id":"s","label":"S","options":options}]),
            Context::AppMessage,
            "test",
        );
        assert!(err.is_err());
        // Slack の card ブロック（トップレベルに title）は旧形式として扱わない
        let (_, f) = app(&json!({"blocks":[{"type":"card","title":{"type":"plain_text","text":"T"},"body":{"type":"plain_text","text":"B"}}]}), Context::AppMessage, "test").unwrap();
        assert_eq!(f, Format::Slack);
    }

    #[test]
    fn media_only_documents_still_have_content() {
        let i = hook(
            json!({"components":[{"type":12,"items":[{"media":{"url":"https://example.com/photo.png"}}]}]}),
        );
        assert_eq!(i.content, "https://example.com/photo.png");
        let i = hook(json!({"components":[{"type":14}]}));
        assert!(!i.content.trim().is_empty());
    }

    #[test]
    fn truncation_keeps_text_and_size_within_limits() {
        let long = "a".repeat(10_000);
        let parts: Vec<Value> = (0..11).map(|_| json!({"type":10,"content":long})).collect();
        let doc = hook(json!({"components": parts})).doc.unwrap();
        assert!(doc.validate(Context::Webhook).is_ok());
        assert!(
            matches!(doc.components.last(), Some(Component::TextDisplay(t)) if t.content.contains("省略"))
        );
    }

    #[test]
    fn webhook_rejects_broken_or_empty_payloads() {
        assert!(webhook(b"not json").is_err());
        assert!(webhook(b"[]").is_err());
        assert!(webhook(br#"{"text":""}"#).is_err());
        assert!(webhook(br#"{"blocks":[]}"#).is_err());
    }

    #[test]
    fn app_accepts_three_formats() {
        let (d, f) = app(
            &json!([{"type":"button","action_id":"a","label":"A"}]),
            Context::AppMessage,
            "test",
        )
        .unwrap();
        assert_eq!(f, Format::Legacy);
        assert!(d.find("a").is_some());
        let (_, f) = app(&json!({"blocks":[{"type":"actions","elements":[{"type":"button","text":{"type":"plain_text","text":"A"},"action_id":"a"}]}]}), Context::AppMessage, "test").unwrap();
        assert_eq!(f, Format::Slack);
        let (_, f) = app(&json!({"components":[{"type":"action_row","components":[{"type":"button","label":"A","custom_id":"a"}]}]}), Context::AppMessage, "test").unwrap();
        assert_eq!(f, Format::Discord);
        assert!(
            app(
                &json!({"blocks":[{"type":"unknown_block"}]}),
                Context::AppMessage,
                "test"
            )
            .is_err()
        );
        assert!(
            app(
                &json!({"components":[{"type":99}]}),
                Context::AppMessage,
                "test"
            )
            .is_err()
        );
        assert!(app(&json!({"components":[{"type":1,"components":[{"type":2,"label":"A","custom_id":"a"}]}]}), Context::Unfurl, "test").is_err());
    }
}

#[cfg(test)]
mod manual_examples {
    //! docs/webhooks.md の記法の表と、実際の変換が合っていることを確かめる。
    use super::blockkit::NoResolver;
    use crate::slack::convert::mrkdwn_to_mfm;

    #[test]
    fn slack_mrkdwn_table() {
        let c = |s: &str| mrkdwn_to_mfm(s, &NoResolver);
        assert_eq!(c("*b* _i_ ~s~"), "**b** <i>i</i> ~~s~~");
        assert_eq!(c("<https://e.x|label>"), "[label](https://e.x)");
        assert_eq!(c("&lt;tag&gt; &amp;"), "<tag> &");
        assert_eq!(
            [
                c("<@U123>"),
                c("<@U123|name>"),
                c("<#C1|general>"),
                c("<!here>"),
                c("<!subteam^S1|@team>"),
                c("<!date^1700000000^{date_short}|fallback>"),
                c("<mailto:a@e.x|mail>"),
                c("$[x2 big]"),
            ],
            [
                "@U123",
                "@name",
                "#general",
                "@here",
                "@team",
                "fallback",
                "mail",
                "$\u{200b}[x2 big]"
            ]
        );
    }
}
