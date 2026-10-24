use toasty::Model;

// Endpoint is a capability URL: never serialize, log, or replicate it.
#[derive(Clone, Model)]
#[table = "push_subscriptions"]
pub struct PushSubscription {
    #[key]
    pub id: String,
    pub user_id: String,
    pub endpoint: String,
    pub p256dh: Option<String>,
    pub auth_secret: Option<String>,
    pub created_at: jiff::civil::DateTime,
}
