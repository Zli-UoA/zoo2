use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "user_role"]
pub struct UserRole {
    #[key]
    pub tenant_id: String,
    #[key]
    pub user_id: String,
    #[key]
    pub role_id: String,
}
