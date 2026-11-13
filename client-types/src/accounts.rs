use serde::{Deserialize, Serialize};

/// Only AuthZ profiles supplied through the emoji service belong in this map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}
pub type AccountProfiles = std::collections::HashMap<String, AccountProfile>;
