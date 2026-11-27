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
    /// このチャンネルの通知レベル。新規参加行は `mentions`（既存行も DEFAULT で移行済み）。
    #[serde(default = "default_notification_level")]
    #[default("mentions".to_owned())]
    pub notification_level: String,
}
