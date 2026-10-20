use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "channels"]
pub struct Channel {
    #[key]
    pub id: String,
    pub tenant_id: String,
    pub channel_name: String,
    pub description: Option<String>,
    pub created_user_id: String,
    pub is_private: bool,
    pub created_at: jiff::civil::DateTime,
    pub updated_at: Option<jiff::civil::DateTime>,
    pub archive_at: Option<jiff::civil::DateTime>,
    pub deleted_at: Option<jiff::civil::DateTime>,
}
