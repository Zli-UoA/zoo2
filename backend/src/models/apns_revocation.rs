use toasty::Model;

#[derive(Model)]
#[table = "apns_revocations"]
pub struct ApnsRevocation {
    #[key]
    pub id: String,
    pub revoked_at: jiff::civil::DateTime,
}
