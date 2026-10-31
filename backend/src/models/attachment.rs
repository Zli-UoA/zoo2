use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "attachments"]
pub struct Attachment {
    #[key]
    pub id: String,
    pub message_id: Option<String>,
    pub uploader_id: String,
    pub channel_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub content_type: String,
    pub storage_key: String,
    pub created_at: jiff::civil::DateTime,
}
