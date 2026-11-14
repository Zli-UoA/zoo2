// build.rs と lib.rs の両方から読む。名前は `zoo2_client_types::ipc::command` と同じにする
// (lib.rs のテストで確かめる)。
pub const COMMANDS: &[&str] = &[
    "zoo_session",
    "zoo_start_login",
    "zoo_cancel_login",
    "zoo_logout",
    "zoo_set_focus",
    "zoo_notification_settings",
    "zoo_set_notifications",
    "zoo_open_channel",
    "zoo_resync",
    "zoo_account_profiles",
    "zoo_send_message",
    "zoo_access_token",
    "zoo_notifications_observed",
];
