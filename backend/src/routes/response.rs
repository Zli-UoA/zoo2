use serde::{Deserialize, Serialize};

/// 書き込み系 REST API の共通レスポンスエンベロープ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub data: T,
    pub txid: String,
}

impl<T> ApiResponse<T> {
    pub fn new(data: T, txid: String) -> Self {
        Self { data, txid }
    }
}
