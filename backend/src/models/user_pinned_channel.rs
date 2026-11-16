use serde::{Deserialize, Serialize};
use toasty::Model;

/// ユーザーごとにピン留めしたチャンネル・DM と、その並び順。
///
/// 並びは `user_id` ごとに独立しており、`position` の小さい順に上から表示する。
/// 書き込み経路は `routes/channel_pins.rs` の生 SQL に寄せており、この型は読み取りと
/// クライアントへ返す行の形をそろえるために置いている。
#[derive(Debug, Clone, Model, Serialize, Deserialize)]
#[table = "user_pinned_channels"]
pub struct UserPinnedChannel {
    #[key]
    pub user_id: String,
    #[key]
    pub channel_id: String,
    /// 0 から始まる、ユーザーごとの並び順。
    pub position: i64,
    pub created_at: jiff::civil::DateTime,
    pub updated_at: jiff::civil::DateTime,
}
