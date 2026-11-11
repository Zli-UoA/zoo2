//! backend の REST API の応答のうち、Rust 側と WebView 側の両方で使うもの。

use serde::{Deserialize, Serialize};

/// `GET /api/users/me`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Me {
    pub id: String,
    pub user_name: String,
    pub display_name: String,
}
