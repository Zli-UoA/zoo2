use serde::{Deserialize, Serialize};
use toasty::Model;

fn default_notification_level() -> String {
    "mentions".to_owned()
}

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "channel_user"]
pub struct ChannelUser {
    #[key]
    pub user_id: String,
    #[key]
    pub channel_id: String,
    pub joined_at: jiff::civil::DateTime,
    /// 旧アプリ互換列。実際の設定は `user_channel_notification_settings` が正本で、
    /// この列には実際の設定を入れず常に `mentions`。旧アプリがここへ書いた値は
    /// トリガーで正本へ移送され、この列は既定値へ戻る。
    #[serde(default = "default_notification_level")]
    #[default("mentions".to_owned())]
    pub notification_level: String,
}
