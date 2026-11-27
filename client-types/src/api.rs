//! backend の REST API の応答のうち、Rust 側と WebView 側の両方で使うもの。

use serde::{Deserialize, Serialize};

/// `GET /api/users/me`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Me {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
}

/// チャンネルごとの通知レベル（`channel_user.notification_level`）。
/// 値は Shape と REST の両方で同じ文字列を使う。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    /// 通常の投稿も通知する。
    All,
    /// 明示メンションだけを通知する（既定）。
    #[default]
    Mentions,
    /// 明示メンション以外を通知しない。
    Mute,
}

impl NotificationLevel {
    pub const ALL: [Self; 3] = [Self::All, Self::Mentions, Self::Mute];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
            Self::Mute => "mute",
        }
    }

    /// 未知の値は既定の `mentions` として扱う。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "mentions" => Some(Self::Mentions),
            "mute" => Some(Self::Mute),
            _ => None,
        }
    }

    /// このレベルで、この種類の通知を出してよいか。
    /// backend が通知行を作らないので、表示側の安全弁として使う。
    /// 不在着信はチャンネル投稿ではないため、レベルに関係なく残す。
    pub fn shows(self, notification_type: &str) -> bool {
        if notification_type == "missed_call" {
            return true;
        }
        match self {
            Self::All => true,
            Self::Mute => notification_type == "mention",
            Self::Mentions => notification_type != "channel_message",
        }
    }
}

/// `GET /api/channels/{id}/notification-settings`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelNotificationSettings {
    pub level: NotificationLevel,
}

/// `PATCH /api/channels/{id}/notification-settings` の本文。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChannelNotificationSettingsUpdate {
    pub level: NotificationLevel,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn notification_level_uses_the_wire_values() {
        assert_eq!(NotificationLevel::default(), NotificationLevel::Mentions);
        assert_eq!(NotificationLevel::All.as_str(), "all");
        assert_eq!(
            NotificationLevel::parse("mute"),
            Some(NotificationLevel::Mute)
        );
        assert_eq!(NotificationLevel::parse("unknown"), None);
        assert_eq!(
            serde_json::to_value(ChannelNotificationSettings {
                level: NotificationLevel::Mute
            })
            .unwrap(),
            json!({ "level": "mute" })
        );
        assert_eq!(
            serde_json::from_value::<ChannelNotificationSettings>(json!({ "level": "all" }))
                .unwrap()
                .level,
            NotificationLevel::All
        );
    }

    #[test]
    fn notification_level_filters_channel_messages_and_mutes() {
        assert!(NotificationLevel::Mentions.shows("mention"));
        assert!(!NotificationLevel::Mentions.shows("channel_message"));
        assert!(NotificationLevel::All.shows("channel_message"));
        assert!(NotificationLevel::Mute.shows("mention"));
        assert!(!NotificationLevel::Mute.shows("channel_message"));
        assert!(!NotificationLevel::Mute.shows("reaction"));
        assert!(NotificationLevel::Mute.shows("missed_call"));
    }
}
