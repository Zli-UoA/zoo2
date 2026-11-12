use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "channels"]
pub struct Channel {
    #[key]
    pub id: String,
    #[unique]
    pub channel_name: String,
    pub description: Option<String>,
    pub created_user_id: String,
    pub is_private: bool,
    pub is_nsfw: bool,
    pub is_dm: bool,
    pub created_at: jiff::civil::DateTime,
    pub updated_at: Option<jiff::civil::DateTime>,
    pub archive_at: Option<jiff::civil::DateTime>,
    pub deleted_at: Option<jiff::civil::DateTime>,
    /// 作成時だけ指定できる（DB のトリガーで変更を拒否する。docs/e2ee.md）。
    #[serde(default)]
    pub encrypted: bool,
    /// 暗号化チャンネルのメンバーと端末の世代（#357）。サーバーだけが増やす。
    #[serde(default)]
    pub membership_epoch: i64,
}
