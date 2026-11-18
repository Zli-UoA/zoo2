use toasty::Model;

// 端末トークンや解除情報をログ・差分配信に出さない。
#[derive(Clone, Model)]
#[table = "apns_subscriptions"]
pub struct ApnsSubscription {
    #[key]
    pub id: String,
    pub user_id: String,
    pub device_token: String,
    pub environment: String,
    pub revocation_hash: String,
    pub created_at: jiff::civil::DateTime,
}
