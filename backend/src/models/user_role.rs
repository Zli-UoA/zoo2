use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "UserRole"]
pub struct UserRole {
    #[key]
    pub user_id: String,
    #[key]
    pub role_id: String,
}
