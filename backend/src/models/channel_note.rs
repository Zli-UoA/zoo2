use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "channel_notes"]
pub struct ChannelNote {
    #[key]
    pub id: String,
    pub channel_id: String,
    pub title: String,
    pub content: String,
    pub position: i32,
    pub deleted_at: Option<jiff::civil::DateTime>,
    pub version: i32,
    pub created_user_id: String,
    pub created_at: jiff::civil::DateTime,
    pub updated_at: jiff::civil::DateTime,
}
