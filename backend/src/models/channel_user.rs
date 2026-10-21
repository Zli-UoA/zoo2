use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "channel_user"]
pub struct ChannelUser {
    pub tenant_id: String,
    #[key]
    pub user_id: String,
    #[key]
    pub channel_id: String,
    pub joined_at: jiff::civil::DateTime,
}
