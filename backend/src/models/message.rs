use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "Message"]
pub struct Message {
    #[key]
    pub id: String,
    pub user_id: String,
    pub channel_id: String,
    pub content: String,
    pub created_at: jiff::civil::DateTime,
    pub updated_at: Option<jiff::civil::DateTime>,
    pub deleted_at: Option<jiff::civil::DateTime>,
}
