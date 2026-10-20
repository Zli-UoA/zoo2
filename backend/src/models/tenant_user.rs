use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "tenant_users"]
pub struct TenantUser {
    #[key]
    pub tenant_id: String,
    #[key]
    pub user_id: String,
    pub joined_at: jiff::civil::DateTime,
}
