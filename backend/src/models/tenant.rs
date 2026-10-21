use serde::{Deserialize, Serialize};
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "tenants"]
pub struct Tenant {
    #[key]
    pub id: String,
    pub name: String,
    pub created_at: jiff::civil::DateTime,
}
