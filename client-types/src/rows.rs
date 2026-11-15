//! シェイプの行を型に変換する。Electric は値を Postgres のテキスト表現で送るため、
//! 真偽値は `"true"`/`"t"`、時刻はタイムゾーンなしの UTC 文字列で届く。

use anyhow::{Context, Result, anyhow};
use jiff::civil::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::shape::Row;

pub trait FromRow: Sized {
    fn from_row(row: &Row) -> Result<Self>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub channel_name: String,
    pub description: Option<String>,
    pub is_private: bool,
    #[serde(default)]
    pub is_nsfw: bool,
    pub is_dm: bool,
    /// 暗号化チャンネル（作成時だけ指定でき、変わらない。docs/e2ee.md）。
    #[serde(default)]
    pub encrypted: bool,
    /// 暗号化チャンネルの `membership_epoch`（docs/e2ee.md の「membership_epoch と送信の順序」）。
    /// Electric は int8 を文字列で配る。平文のチャンネルや古いスキーマでは `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership_epoch: Option<u64>,
    pub archive_at: Option<DateTime>,
    pub deleted_at: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
    pub is_bot: bool,
    pub bot_mentionable: bool,
    pub is_guest: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelUser {
    pub user_id: String,
    pub channel_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub user_id: String,
    pub channel_id: String,
    pub content: String,
    pub card: Option<String>,
    pub created_at: DateTime,
    pub updated_at: Option<DateTime>,
    pub deleted_at: Option<DateTime>,
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_reactions: Option<Vec<Reaction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_attachments: Option<Vec<Attachment>>,
    #[serde(default)]
    pub thread_archive_at: Option<DateTime>,
    /// 暗号化チャンネルのメッセージの Megolm の暗号文（`messages.ciphertext`。docs/e2ee.md の
    /// 「messages.ciphertext の形式」）。平文のメッセージと、削除した行では `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ciphertext: Option<Value>,
    /// 暗号化して送ったときの `membership_epoch`。平文のメッセージ（暗号化に切り替える前の行を含む）では `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    /// 版の番号（編集のたびにサーバーが 1 増やす）。
    #[serde(default)]
    pub revision: u64,
    /// `message` か `emote`（`messages.message_kind`）。古いスキーマでは `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub id: String,
    pub message_id: String,
    pub user_id: String,
    pub emoji: String,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub message_id: Option<String>,
    pub uploader_id: String,
    pub channel_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub content_type: String,
    pub storage_key: String,
    #[serde(default)]
    pub is_sensitive: bool,
    pub created_at: DateTime,
}

/// 自分あての通知。プロキシが `notification_events` の `source = 'channel'` だけを返すので、
/// チャンネルは必ずある。メッセージは、不在着信 (`missed_call`) だけない。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub user_id: String,
    pub actor_id: String,
    pub channel_id: String,
    pub message_id: Option<String>,
    /// `mention`、`reply`、`dm`、`reaction` など。backend の `notifications.rs` を参照。
    pub notification_type: String,
    pub is_read: bool,
    pub created_at: DateTime,
    /// 投稿者名や本文の抜粋など、表示用に backend が詰めた JSON。
    pub display_data: Option<Value>,
}

/// 通話の招待（呼び出し）。呼び出し中 (`pending`) の行だけが Shape に届く（docs/calls.md）。
///
/// 時刻は Electric の `timestamptz`（`2026-11-14 10:00:00.123+00`）のまま持つ。
/// 読むときは [`crate::notify::parse_timestamptz`] を使う。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInvite {
    pub id: String,
    pub channel_id: String,
    pub inviter_id: String,
    pub invitee_id: String,
    pub status: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardField {
    pub title: String,
    pub value: String,
    #[serde(default)]
    pub short: Option<bool>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub code: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardData {
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
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub fields: Option<Vec<CardField>>,
}

/// JSON 文字列から 1 個または複数の CardData をパースする。
pub fn parse_message_cards(value: &str) -> Vec<CardData> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    // {"cards": [...]} 形式のパース
    if let Ok(wrapper) = serde_json::from_str::<CardsWrapper>(value) {
        let cards = wrapper.cards;
        if !cards.is_empty() {
            return cards;
        }
    }
    // 単一 CardData 形式のパース
    if let Ok(card) = serde_json::from_str::<CardData>(value) {
        let title = card.title.trim();
        if !title.is_empty() {
            return vec![card];
        }
    }
    Vec::new()
}

#[derive(Deserialize)]
struct CardsWrapper {
    cards: Vec<CardData>,
}

impl FromRow for Channel {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            channel_name: text(row, "channel_name")?,
            description: opt_text(row, "description"),
            is_private: boolean(row, "is_private")?,
            is_nsfw: opt_boolean(row, "is_nsfw")?.unwrap_or(false),
            // 古いスキーマには is_dm がないため、無ければ通常のチャンネルとして扱う。
            is_dm: opt_boolean(row, "is_dm")?.unwrap_or(false),
            // 古いスキーマには encrypted がないため、無ければ平文のチャンネルとして扱う。
            encrypted: opt_boolean(row, "encrypted")?.unwrap_or(false),
            membership_epoch: opt_unsigned(row, "membership_epoch")?,
            archive_at: opt_time(row, "archive_at")?,
            deleted_at: opt_time(row, "deleted_at")?,
        })
    }
}

impl FromRow for User {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            user_name: text(row, "user_name")?,
            display_name: opt_text(row, "display_name").unwrap_or_default(),
            is_bot: opt_boolean(row, "is_bot")?.unwrap_or(false),
            bot_mentionable: opt_boolean(row, "bot_mentionable")?.unwrap_or(false),
            is_guest: opt_boolean(row, "is_guest")?.unwrap_or(false),
        })
    }
}

impl FromRow for ChannelUser {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            user_id: text(row, "user_id")?,
            channel_id: text(row, "channel_id")?,
        })
    }
}

impl FromRow for Message {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            user_id: text(row, "user_id")?,
            channel_id: text(row, "channel_id")?,
            content: opt_text(row, "content").unwrap_or_default(),
            card: opt_text(row, "card"),
            created_at: time(row, "created_at")?,
            updated_at: opt_time(row, "updated_at")?,
            deleted_at: opt_time(row, "deleted_at")?,
            parent_id: opt_text(row, "parent_id"),
            display_reactions: display_reactions(row)?,
            display_attachments: display_attachments(row)?,
            thread_archive_at: opt_time(row, "thread_archive_at")?,
            ciphertext: opt_json(row, "ciphertext")?,
            epoch: opt_unsigned(row, "epoch")?,
            revision: opt_unsigned(row, "revision")?.unwrap_or(0),
            message_kind: opt_text(row, "message_kind"),
        })
    }
}

fn display_reactions(row: &Row) -> Result<Option<Vec<Reaction>>> {
    let display = match row.get("display_data") {
        Some(Value::String(raw)) => serde_json::from_str::<Value>(raw)?,
        Some(value) => value.clone(),
        None => return Ok(None),
    };
    let Some(reactions) = display.get("reactions").and_then(Value::as_array) else {
        return Ok(None);
    };
    reactions
        .iter()
        .map(|value| Reaction::from_row(value.as_object().context("Invalid display reaction")?))
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn display_attachments(row: &Row) -> Result<Option<Vec<Attachment>>> {
    let display = match row.get("display_data") {
        Some(Value::String(raw)) => serde_json::from_str::<Value>(raw)?,
        Some(value) => value.clone(),
        None => return Ok(None),
    };
    let Some(attachments) = display.get("attachments").and_then(Value::as_array) else {
        return Ok(None);
    };
    attachments
        .iter()
        .map(|value| Attachment::from_row(value.as_object().context("Invalid display attachment")?))
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

impl FromRow for Reaction {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            message_id: text(row, "message_id")?,
            user_id: text(row, "user_id")?,
            emoji: text(row, "emoji")?,
            created_at: time(row, "created_at")?,
        })
    }
}

impl FromRow for Attachment {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            message_id: opt_text(row, "message_id"),
            uploader_id: text(row, "uploader_id")?,
            channel_id: text(row, "channel_id")?,
            file_name: text(row, "file_name")?,
            file_size: integer(row, "file_size")?,
            content_type: text(row, "content_type")?,
            storage_key: text(row, "storage_key")?,
            is_sensitive: opt_boolean(row, "is_sensitive")?.unwrap_or(false),
            created_at: time(row, "created_at")?,
        })
    }
}

impl FromRow for CallInvite {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            channel_id: text(row, "channel_id")?,
            inviter_id: text(row, "inviter_id")?,
            invitee_id: text(row, "invitee_id")?,
            status: text(row, "status")?,
            created_at: text(row, "created_at")?,
            expires_at: text(row, "expires_at")?,
        })
    }
}

impl FromRow for Notification {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            user_id: text(row, "user_id")?,
            actor_id: text(row, "actor_id")?,
            channel_id: text(row, "channel_id")?,
            message_id: opt_text(row, "message_id"),
            notification_type: text(row, "notification_type")?,
            is_read: opt_boolean(row, "is_read")?.unwrap_or(false),
            created_at: time(row, "created_at")?,
            display_data: opt_json(row, "display_data")?,
        })
    }
}

/// jsonb の列。Electric は JSON の文字列で送るため、文字列ならパースする。
fn opt_json(row: &Row, column: &str) -> Result<Option<Value>> {
    match row.get(column) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => serde_json::from_str(raw)
            .map(Some)
            .with_context(|| format!("列 {column} が JSON ではありません")),
        Some(other) => Ok(Some(other.clone())),
    }
}

fn opt_integer(row: &Row, column: &str) -> Result<Option<i64>> {
    match row.get(column) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(num)) => num
            .as_i64()
            .ok_or_else(|| anyhow!("列 {column} が整数ではありません: {num}"))
            .map(Some),
        Some(Value::String(s)) => s
            .parse::<i64>()
            .map(Some)
            .map_err(|e| anyhow!("列 {column} が整数ではありません: {s} ({e})")),
        Some(other) => Err(anyhow!("列 {column} が整数ではありません: {other}")),
    }
}

/// 0 以上の int8 の列（`membership_epoch`、`epoch`、`revision`）。
fn opt_unsigned(row: &Row, column: &str) -> Result<Option<u64>> {
    opt_integer(row, column)?
        .map(|value| u64::try_from(value).map_err(|_| anyhow!("列 {column} が負の数です: {value}")))
        .transpose()
}

fn integer(row: &Row, column: &str) -> Result<i64> {
    opt_integer(row, column)?.ok_or_else(|| anyhow!("列 {column} がありません"))
}

fn opt_text(row: &Row, column: &str) -> Option<String> {
    match row.get(column)? {
        Value::String(value) => Some(value.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn text(row: &Row, column: &str) -> Result<String> {
    opt_text(row, column).ok_or_else(|| anyhow!("列 {column} がありません"))
}

fn opt_boolean(row: &Row, column: &str) -> Result<Option<bool>> {
    match row.get(column) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(Value::String(value)) => match value.as_str() {
            "true" | "t" => Ok(Some(true)),
            "false" | "f" => Ok(Some(false)),
            other => Err(anyhow!("列 {column} が真偽値ではありません: {other}")),
        },
        Some(other) => Err(anyhow!("列 {column} が真偽値ではありません: {other}")),
    }
}

fn boolean(row: &Row, column: &str) -> Result<bool> {
    opt_boolean(row, column)?.ok_or_else(|| anyhow!("列 {column} がありません"))
}

fn opt_time(row: &Row, column: &str) -> Result<Option<DateTime>> {
    opt_text(row, column)
        .map(|value| {
            parse_time(&value).with_context(|| format!("列 {column} が時刻ではありません: {value}"))
        })
        .transpose()
}

fn time(row: &Row, column: &str) -> Result<DateTime> {
    opt_time(row, column)?.ok_or_else(|| anyhow!("列 {column} がありません"))
}

/// `2026-11-01 12:34:56.789` と `2026-11-01T12:34:56.789` の両方を受け付ける。
fn parse_time(value: &str) -> Result<DateTime> {
    Ok(value.replacen(' ', "T", 1).parse()?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(value: Value) -> Row {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn parses_electric_text_values() {
        let channel = Channel::from_row(&row(json!({
            "id": "c1", "channel_name": "general", "description": null,
            "is_private": "f", "is_nsfw": "t", "is_dm": "true", "archive_at": "2026-11-01 01:02:03.456",
            "deleted_at": null, "encrypted": "t", "membership_epoch": "3"
        })))
        .unwrap();
        assert!(channel.encrypted);
        assert_eq!(channel.membership_epoch, Some(3));
        assert!(!channel.is_private);
        assert!(channel.is_nsfw);
        assert!(channel.is_dm);
        assert_eq!(
            channel.archive_at.unwrap().to_string(),
            "2026-11-01T01:02:03.456"
        );
    }

    #[test]
    fn parses_encrypted_message_columns() {
        let message = Message::from_row(&row(json!({
            "id": "m1", "user_id": "u1", "channel_id": "c1", "content": "",
            "created_at": "2026-11-01 01:02:03", "ciphertext": "{\"algorithm\":\"m.megolm.v1.aes-sha2\"}",
            "epoch": "4", "revision": "2", "message_kind": "emote"
        })))
        .unwrap();
        assert_eq!(
            message.ciphertext,
            Some(json!({ "algorithm": "m.megolm.v1.aes-sha2" }))
        );
        assert_eq!(message.epoch, Some(4));
        assert_eq!(message.revision, 2);
        assert_eq!(message.message_kind.as_deref(), Some("emote"));

        let plain = Message::from_row(&row(json!({
            "id": "m2", "user_id": "u1", "channel_id": "c1", "content": "hi",
            "created_at": "2026-11-01 01:02:03", "ciphertext": null, "epoch": null
        })))
        .unwrap();
        assert_eq!(plain.ciphertext, None);
        assert_eq!(plain.epoch, None);
        assert_eq!(plain.revision, 0);
        assert!(
            Message::from_row(&row(json!({
                "id": "m3", "user_id": "u1", "channel_id": "c1",
                "created_at": "2026-11-01 01:02:03", "revision": "-1"
            })))
            .is_err()
        );
    }

    #[test]
    fn ignores_unknown_columns_and_defaults_optional_ones() {
        let message = Message::from_row(&row(json!({
            "id": "m1", "user_id": "u1", "channel_id": "c1", "content": "hi",
            "created_at": "2026-11-01T00:00:00", "app_blocks": "[]", "broadcast": "false"
        })))
        .unwrap();
        assert_eq!(message.parent_id, None);
        assert_eq!(message.thread_archive_at, None);
        assert_eq!(message.updated_at, None);

        let channel = Channel::from_row(&row(json!({
            "id": "c1", "channel_name": "general", "is_private": false
        })))
        .unwrap();
        assert!(!channel.is_dm);
        assert!(!channel.is_nsfw);
        assert!(!channel.encrypted);
    }

    #[test]
    fn rejects_missing_or_malformed_values() {
        assert!(User::from_row(&row(json!({ "id": "u1" }))).is_err());
        assert!(
            Channel::from_row(&row(
                json!({ "id": "c", "channel_name": "n", "is_private": "maybe" })
            ))
            .is_err()
        );
        assert!(
            Reaction::from_row(&row(json!({
                "id": "r", "message_id": "m", "user_id": "u", "emoji": "👍", "created_at": "yesterday"
            })))
            .is_err()
        );
    }

    #[test]
    fn parses_attachment_and_message_card() {
        let attachment = Attachment::from_row(&row(json!({
            "id": "att1",
            "message_id": "m1",
            "uploader_id": "u1",
            "channel_id": "c1",
            "file_name": "photo.png",
            "file_size": 12345,
            "content_type": "image/png",
            "storage_key": "keys/photo.png",
            "is_sensitive": "t",
            "created_at": "2026-11-01 12:00:00"
        })))
        .unwrap();
        assert_eq!(attachment.id, "att1");
        assert!(attachment.is_sensitive);
        assert_eq!(attachment.file_size, 12345);
        assert_eq!(attachment.content_type, "image/png");

        let message = Message::from_row(&row(json!({
            "id": "m1", "user_id": "u1", "channel_id": "c1", "content": "hello",
            "card": "{\"title\":\"zoo2\",\"url\":\"https://zoo2.app\"}",
            "created_at": "2026-11-01T00:00:00"
        })))
        .unwrap();
        assert_eq!(
            message.card.as_deref(),
            Some("{\"title\":\"zoo2\",\"url\":\"https://zoo2.app\"}")
        );

        let cards = parse_message_cards(message.card.as_deref().unwrap());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].title, "zoo2");
        assert_eq!(cards[0].url, "https://zoo2.app");
    }
}
