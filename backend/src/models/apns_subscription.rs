use toasty::Model;

// Tokens and revocation capabilities must never enter logs or replicated shapes.
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
