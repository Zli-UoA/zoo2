//! OS 通知。判定は `zoo2_client_core::notify::notifications_to_show` に任せ、ここでは
//! 判定に渡す状態の組み立て、DND の取得、通知の表示を行う。
//!
//! 通知は notify-rust で出す (Tauri 版と同じ)。Linux ではクリックでウィンドウを前に出し、
//! そのチャンネルを開く。macOS は出すだけ。

use std::time::Duration;

use jiff::{Timestamp, civil::DateTime, tz::TimeZone};
use zoo2_client_core::{
    electric::Batch,
    notify::{self, NotifyContext, Toast},
};

use super::{ResidentEvent, send};
use crate::app::{AppEvent, AppState, DesktopEvent};

/// DND (`dnd_until`) を取り直す間隔。
pub const DND_REFRESH: Duration = Duration::from_secs(5 * 60);

pub fn now_utc() -> DateTime {
    Timestamp::now().to_zoned(TimeZone::UTC).datetime()
}

/// 起動時と 5 分ごとに `GET /api/users/me/profile` から `dnd_until` を取る。
///
/// 取れなかったときは前の値を保つ。最初に取れなかったときも「確認済み」にして、
/// DND なしとして通知を出し始める (通知を止め続けない)。
pub fn spawn_dnd_refresh(state: &mut AppState) {
    let Some(api) = state.api.clone() else {
        return;
    };
    let session_gen = state.session_gen;
    let tx = state.event_tx.clone();
    state.spawn_session_task(async move {
        loop {
            let result = api.my_profile().await.ok().map(|profile| {
                profile
                    .dnd_until
                    .as_deref()
                    .and_then(notify::parse_dnd_until)
            });
            if tx
                .send(DesktopEvent::App {
                    session_gen,
                    event: AppEvent::Dnd(result),
                })
                .is_err()
            {
                break;
            }
            tokio::time::sleep(DND_REFRESH).await;
        }
    });
}

impl AppState {
    /// 判定に渡す状態。`focused_channel` は、ウィンドウが前面かつそのチャンネルを表示中のときだけ Some。
    pub fn notify_context(&self) -> NotifyContext {
        NotifyContext {
            me: self.me.as_ref().map(|me| me.id.clone()).unwrap_or_default(),
            enabled: self.notifications_enabled && self.dnd_checked && self.me.is_some(),
            now: Some(now_utc()),
            dnd_until: self.dnd_until,
            focused_channel: self
                .window_active
                .then(|| self.current_channel.clone())
                .flatten(),
        }
    }

    /// Notifications の Shape のバッチから、出す通知を決めて出す。
    /// **バッチを Store に適用する前に**呼ぶ (既に知っている行かどうかを Store で見るため)。
    pub fn show_notifications(&self, batch: &Batch) {
        let toasts = notify::notifications_to_show(&self.store, batch, &self.notify_context());
        for toast in toasts {
            if let Some(sink) = &self.notification_sink {
                sink.lock().unwrap().push((toast.title, toast.body));
            } else {
                std::thread::spawn(move || show(&toast));
            }
        }
    }
}

/// 通知を 1 件出す。Linux ではクリックされるまで待つので、専用のスレッドで呼ぶ。
pub fn show(toast: &Toast) {
    let mut notification = notify_rust::Notification::new();
    notification
        .appname("zoo2")
        .summary(&toast.title)
        .body(&toast.body);
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        notification.action("default", "開く");
        match notification.show() {
            Ok(handle) => {
                let channel_id = toast.channel_id.clone();
                handle.wait_for_action(|action| {
                    if action == "default" {
                        send(ResidentEvent::Open { channel_id });
                    }
                });
            }
            Err(err) => eprintln!("通知を出せませんでした: {err}"),
        }
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = send;
        if let Err(err) = notification.show() {
            eprintln!("通知を出せませんでした: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::connected_state;
    use zoo2_client_core::electric::{Change, ShapeEvent, ShapeId};

    fn notif(id: &str, channel: &str, actor: &str, read: bool) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "user_id": "u1",
            "channel_id": channel,
            "message_id": format!("m-{id}"),
            "actor_id": actor,
            "notification_type": "mention",
            "is_read": if read { "true" } else { "false" },
            "created_at": "2026-11-11 00:00:00",
        })
    }

    fn batch(rows: Vec<(&str, serde_json::Value)>, reset: bool) -> Batch {
        Batch {
            reset,
            changes: rows
                .into_iter()
                .map(|(key, row)| Change::Upsert {
                    key: key.into(),
                    row: serde_json::from_value(row).unwrap(),
                })
                .collect(),
            up_to_date: true,
        }
    }

    fn ready_state() -> (AppState, crate::app::NotificationSink) {
        let mut state = connected_state();
        state.notifications_enabled = true;
        state.dnd_checked = true;
        state.window_active = false;
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        state.notification_sink = Some(sink.clone());
        // 初回同期を済ませる。初回の行は出さない。
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Notifications,
            batch(vec![("n0", notif("n0", "c1", "u2", false))], true),
        ));
        assert!(sink.lock().unwrap().is_empty());
        (state, sink)
    }

    #[test]
    fn shows_new_unread_notification_once() {
        let (mut state, sink) = ready_state();
        let b = batch(vec![("n1", notif("n1", "c1", "u2", false))], false);
        state.on_shape(ShapeEvent::Batch(ShapeId::Notifications, b.clone()));
        assert_eq!(sink.lock().unwrap().len(), 1);
        // 既に知っている行の再送は出さない。
        state.on_shape(ShapeEvent::Batch(ShapeId::Notifications, b));
        assert_eq!(sink.lock().unwrap().len(), 1);
    }

    #[test]
    fn skips_own_read_and_focused_channel() {
        let (mut state, sink) = ready_state();
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Notifications,
            batch(
                vec![
                    ("n1", notif("n1", "c1", "u1", false)),
                    ("n2", notif("n2", "c1", "u2", true)),
                ],
                false,
            ),
        ));
        assert!(sink.lock().unwrap().is_empty());

        state.window_active = true;
        state.current_channel = Some("c1".into());
        assert_eq!(
            state.notify_context().focused_channel.as_deref(),
            Some("c1")
        );
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Notifications,
            batch(vec![("n3", notif("n3", "c1", "u2", false))], false),
        ));
        assert!(sink.lock().unwrap().is_empty());
        // 別のチャンネルなら出す。
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Notifications,
            batch(vec![("n4", notif("n4", "c2", "u2", false))], false),
        ));
        assert_eq!(sink.lock().unwrap().len(), 1);
    }

    #[test]
    fn dnd_and_unchecked_suppress() {
        let (mut state, sink) = ready_state();
        state.dnd_until = Some(
            now_utc()
                .checked_add(jiff::SignedDuration::from_hours(1))
                .unwrap(),
        );
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Notifications,
            batch(vec![("n1", notif("n1", "c1", "u2", false))], false),
        ));
        assert!(sink.lock().unwrap().is_empty());

        state.dnd_until = None;
        state.dnd_checked = false;
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Notifications,
            batch(vec![("n2", notif("n2", "c1", "u2", false))], false),
        ));
        assert!(sink.lock().unwrap().is_empty());
    }

    #[test]
    fn dnd_event_updates_state_and_failure_keeps_value() {
        let mut state = connected_state();
        let until = now_utc();
        state.on_app_event(AppEvent::Dnd(Some(Some(until))));
        assert_eq!(state.dnd_until, Some(until));
        assert!(state.dnd_checked);
        state.on_app_event(AppEvent::Dnd(None));
        assert_eq!(state.dnd_until, Some(until));
        state.on_app_event(AppEvent::Dnd(Some(None)));
        assert_eq!(state.dnd_until, None);
    }
}
