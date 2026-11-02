//! シェイプの行を型に変換する。Electric は値を Postgres のテキスト表現で送るため、
//! 真偽値は `"true"`/`"t"`、時刻はタイムゾーンなしの UTC 文字列で届く。

use anyhow::{Context, Result, anyhow};
use jiff::civil::DateTime;
use serde_json::Value;

use super::shape::Row;

pub trait FromRow: Sized {
    fn from_row(row: &Row) -> Result<Self>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub id: String,
    pub channel_name: String,
    pub description: Option<String>,
    pub is_private: bool,
    pub is_dm: bool,
    pub archive_at: Option<DateTime>,
    pub deleted_at: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
    pub is_bot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelUser {
    pub user_id: String,
    pub channel_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: String,
    pub user_id: String,
    pub channel_id: String,
    pub content: String,
    pub created_at: DateTime,
    pub updated_at: Option<DateTime>,
    pub deleted_at: Option<DateTime>,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reaction {
    pub id: String,
    pub message_id: String,
    pub user_id: String,
    pub emoji: String,
    pub created_at: DateTime,
}

impl FromRow for Channel {
    fn from_row(row: &Row) -> Result<Self> {
        Ok(Self {
            id: text(row, "id")?,
            channel_name: text(row, "channel_name")?,
            description: opt_text(row, "description"),
            is_private: boolean(row, "is_private")?,
            // 古いスキーマには is_dm がないため、無ければ通常のチャンネルとして扱う。
            is_dm: opt_boolean(row, "is_dm")?.unwrap_or(false),
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
            created_at: time(row, "created_at")?,
            updated_at: opt_time(row, "updated_at")?,
            deleted_at: opt_time(row, "deleted_at")?,
            parent_id: opt_text(row, "parent_id"),
        })
    }
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
            "is_private": "f", "is_dm": "true", "archive_at": "2026-11-01 01:02:03.456",
            "deleted_at": null
        })))
        .unwrap();
        assert!(!channel.is_private);
        assert!(channel.is_dm);
        assert_eq!(
            channel.archive_at.unwrap().to_string(),
            "2026-11-01T01:02:03.456"
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
        assert_eq!(message.updated_at, None);

        let channel = Channel::from_row(&row(json!({
            "id": "c1", "channel_name": "general", "is_private": false
        })))
        .unwrap();
        assert!(!channel.is_dm);
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
}
