use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "role"]
pub struct Role {
    #[key]
    pub id: String,
    pub role_name: String,
}
