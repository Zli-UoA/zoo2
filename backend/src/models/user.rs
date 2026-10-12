use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "User"]
pub struct User {
    #[key]
    pub id: String,
    #[unique]
    pub user_name: String,
    pub display_name: String,
    pub created_at: jiff::civil::DateTime,
    pub updated_at: Option<jiff::civil::DateTime>,
}
