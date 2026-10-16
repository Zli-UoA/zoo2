use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "user_identities"]
pub struct UserIdentity {
    #[key]
    pub id: String,
    pub user_id: String,
    pub iss: String,
    pub sub: String,
    pub created_at: jiff::civil::DateTime,
    pub updated_at: Option<jiff::civil::DateTime>,
}
