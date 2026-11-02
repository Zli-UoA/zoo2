pub mod attachment;
pub mod channel_webhook;
pub use channel_webhook::ChannelWebhook;
pub mod channel;
pub mod channel_user;
pub mod message;
pub mod message_reaction;
pub mod notification;
pub mod push_subscription;
pub use push_subscription::PushSubscription;
pub mod role;
pub mod user;
pub mod user_identity;
pub mod user_role;

pub use attachment::Attachment;
pub use channel::Channel;
pub use channel_user::ChannelUser;
pub use message::Message;
pub use message_reaction::MessageReaction;
pub use notification::Notification;
pub use role::Role;
pub use user::User;
pub use user_identity::UserIdentity;
pub use user_role::UserRole;

/// 常に UTC 基準かつミリ秒精度（PostgreSQL timestamp(3) 適合）の現在日時を返す
pub fn now_utc_millis() -> jiff::civil::DateTime {
    let now = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .datetime();
    now.round(jiff::Unit::Millisecond)
        .expect("rounding to millisecond on civil::DateTime is guaranteed to succeed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_utc_millis_precision() {
        let dt = now_utc_millis();
        // サブミリ秒（マイクロ秒・ナノ秒）が 0 に切り捨てられているか確認
        assert_eq!(dt.subsec_nanosecond() % 1_000_000, 0);
    }
}
