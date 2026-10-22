use toasty::Model;

// Credentials deliberately have no Serialize or Debug implementation and are never replicated.
#[derive(Clone, Model)]
#[table = "channel_webhooks"]
pub struct ChannelWebhook {
    #[key]
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub secret: String,
    pub user_id: String,
    pub created_at: jiff::civil::DateTime,
}
