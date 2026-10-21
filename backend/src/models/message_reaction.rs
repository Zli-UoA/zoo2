use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "message_reactions"]
pub struct MessageReaction {
    pub tenant_id: String,
    #[key]
    pub id: String,
    pub message_id: String,
    pub user_id: String,
    pub emoji: String,
    pub created_at: jiff::civil::DateTime,
}
