use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "notifications"]
pub struct Notification {
    #[key]
    pub id: String,
    pub user_id: String,
    pub actor_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub notification_type: String,
    pub is_read: bool,
    pub created_at: jiff::civil::DateTime,
}
