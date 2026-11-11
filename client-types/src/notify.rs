//! OS の通知を出すかどうかの判定。
//!
//! Tauri 版は (a) Leptos と (b) React の両方で、この判定だけを使う。条件は次のとおり。
//!
//! - 通知の Shape の初回同期が終わってから届いた行だけを出す。初回同期や、409 などで
//!   取り直した (`reset`) バッチは出さない。起動や再接続のたびに過去の通知が大量に出ないようにする。
//! - すでに知っている通知 (既読への変更などの update) は出さない。未読のものだけを出す。
//! - 自分が起こした通知は出さない。
//! - DND (`dnd_until` が今より後) の間は出さない。backend の Web Push (`push.rs`) と同じく、
//!   止めた分を後でまとめて出すことはしない。
//! - ウィンドウが前面にあり、そのチャンネルを表示している間は出さない。
//!
//! 時刻はすべて、タイムゾーンなしの UTC (`jiff::civil::DateTime`) で比べる。Electric の
//! timestamp 列と同じ形にそろえるため。

use std::collections::HashMap;

use jiff::{Timestamp, civil::DateTime, tz::TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    rows::{FromRow, Notification},
    shape::{Batch, Change, ShapeId},
    store::Store,
};

/// 判定に使う、その時点の状態。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NotifyContext {
    /// 自分のユーザー id。
    pub me: String,
    /// 通知を出す設定になっているか。
    pub enabled: bool,
    /// 今の時刻 (UTC)。
    pub now: Option<DateTime>,
    /// `GET /api/users/me/profile` の `dnd_until` を UTC にしたもの。
    pub dnd_until: Option<DateTime>,
    /// ウィンドウが前面にあるときに表示しているチャンネル。前面にないときは None。
    pub focused_channel: Option<String>,
}

impl NotifyContext {
    pub fn in_dnd(&self) -> bool {
        match (self.dnd_until, self.now) {
            (Some(until), Some(now)) => until > now,
            // 今の時刻が分からないときは、止めている側に倒す。
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

/// `dnd_until` (timestamptz の JSON 文字列) を UTC の時刻にする。読めなければ None。
pub fn parse_dnd_until(value: &str) -> Option<DateTime> {
    let timestamp: Timestamp = value.parse().ok()?;
    Some(timestamp.to_zoned(TimeZone::UTC).datetime())
}

/// 出す通知の中身。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toast {
    pub notification_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub notification_type: String,
    pub title: String,
    pub body: String,
}

/// 通知の Shape のバッチのうち、OS の通知として出すものを返す。
///
/// `store.apply` で**適用する前の** `store` を渡す。適用した後では、新しく届いた行と
/// 前から知っている行を区別できないため。
pub fn notifications_to_show(store: &Store, batch: &Batch, ctx: &NotifyContext) -> Vec<Toast> {
    if !ctx.enabled || batch.reset || !store.is_synced(&ShapeId::Notifications) || ctx.in_dnd() {
        return Vec::new();
    }

    // 同じバッチの中で、後から既読になったり消えたりしたものは取り消す。
    let mut by_key: HashMap<&str, Option<Notification>> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for change in &batch.changes {
        match change {
            Change::Upsert { key, row } => {
                let Ok(notification) = Notification::from_row(row) else {
                    continue;
                };
                let first_seen_in_batch = !by_key.contains_key(key.as_str());
                let known_before = store.notification(&notification.id).is_some();
                if known_before {
                    continue;
                }
                let show = !notification.is_read
                    && notification.actor_id != ctx.me
                    && ctx.focused_channel.as_deref() != Some(notification.channel_id.as_str());
                by_key.insert(key, show.then_some(notification));
                if first_seen_in_batch {
                    order.push(key);
                }
            }
            Change::Delete { key } => {
                if let Some(slot) = by_key.get_mut(key.as_str()) {
                    *slot = None;
                }
            }
        }
    }

    order
        .into_iter()
        .filter_map(|key| by_key.remove(key).flatten())
        .map(|notification| toast(store, notification))
        .collect()
}

fn toast(store: &Store, notification: Notification) -> Toast {
    let display = notification.display_data.as_ref();
    let actor = display
        .and_then(|data| data.get("actor"))
        .and_then(display_name)
        .unwrap_or_else(|| store.display_name(&notification.actor_id));
    let channel = display
        .and_then(|data| data.get("channel"))
        .and_then(|channel| text(channel, "channel_name"))
        .or_else(|| {
            store
                .channel(&notification.channel_id)
                .map(|channel| channel.channel_name.clone())
        });
    let title = match (notification.notification_type.as_str(), channel) {
        ("dm", _) | (_, None) => actor,
        (_, Some(channel)) => format!("{actor} (#{channel})"),
    };
    let body = match notification.notification_type.as_str() {
        "reaction" => "リアクションしました".to_string(),
        _ => display
            .and_then(|data| data.get("message"))
            .and_then(|message| text(message, "content"))
            .map(|content| excerpt(&content))
            .unwrap_or_default(),
    };
    Toast {
        notification_id: notification.id,
        channel_id: notification.channel_id,
        message_id: notification.message_id,
        notification_type: notification.notification_type,
        title,
        body,
    }
}

fn display_name(user: &Value) -> Option<String> {
    text(user, "display_name")
        .filter(|name| !name.is_empty())
        .or_else(|| text(user, "user_name"))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

/// backend の Web Push と同じく、本文は先頭 240 文字までにする。
fn excerpt(content: &str) -> String {
    const LIMIT: usize = 240;
    match content.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}…", &content[..end]),
        None => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(id: &str, actor: &str, channel: &str, is_read: bool) -> Change {
        Change::Upsert {
            key: format!("\"public\".\"notification_events\"/\"{id}\""),
            row: json!({
                "id": id, "user_id": "me", "actor_id": actor, "channel_id": channel,
                "message_id": format!("m-{id}"), "notification_type": "mention",
                "is_read": if is_read { "true" } else { "false" },
                "created_at": "2026-11-11 00:00:00",
                "display_data": json!({
                    "actor": { "display_name": "Alice", "user_name": "alice" },
                    "channel": { "channel_name": "general" },
                    "message": { "content": "hi @me" },
                }).to_string(),
            })
            .as_object()
            .unwrap()
            .clone(),
        }
    }

    fn batch(reset: bool, changes: Vec<Change>) -> Batch {
        Batch {
            reset,
            changes,
            up_to_date: true,
        }
    }

    fn ctx() -> NotifyContext {
        NotifyContext {
            me: "me".into(),
            enabled: true,
            now: Some("2026-11-11T00:00:00".parse().unwrap()),
            dnd_until: None,
            focused_channel: None,
        }
    }

    /// 初回同期を終えた store。
    fn synced_store(changes: Vec<Change>) -> Store {
        let mut store = Store::default();
        store.apply(&ShapeId::Notifications, batch(false, changes));
        store
    }

    fn ids(toasts: &[Toast]) -> Vec<&str> {
        toasts.iter().map(|t| t.notification_id.as_str()).collect()
    }

    #[test]
    fn skips_initial_sync_and_refetch() {
        let store = Store::default();
        let incoming = batch(false, vec![row("n1", "alice", "c1", false)]);
        assert!(notifications_to_show(&store, &incoming, &ctx()).is_empty());

        let store = synced_store(vec![]);
        let refetch = batch(true, vec![row("n1", "alice", "c1", false)]);
        assert!(notifications_to_show(&store, &refetch, &ctx()).is_empty());
    }

    #[test]
    fn shows_only_new_unread_from_others() {
        let store = synced_store(vec![row("old", "alice", "c1", false)]);
        let incoming = batch(
            false,
            vec![
                row("old", "alice", "c1", true),
                row("mine", "me", "c1", false),
                row("read", "alice", "c1", true),
                row("n1", "alice", "c1", false),
            ],
        );
        let toasts = notifications_to_show(&store, &incoming, &ctx());
        assert_eq!(ids(&toasts), vec!["n1"]);
        assert_eq!(toasts[0].title, "Alice (#general)");
        assert_eq!(toasts[0].body, "hi @me");
        assert_eq!(toasts[0].channel_id, "c1");
        assert_eq!(toasts[0].message_id, "m-n1");
    }

    #[test]
    fn later_changes_in_same_batch_cancel() {
        let store = synced_store(vec![]);
        let n1 = row("n1", "alice", "c1", false);
        let Change::Upsert { key, .. } = &n1 else {
            unreachable!()
        };
        let incoming = batch(
            false,
            vec![
                n1.clone(),
                row("n2", "alice", "c1", false),
                row("n2", "alice", "c1", true),
                Change::Delete { key: key.clone() },
                row("n3", "alice", "c1", false),
            ],
        );
        assert_eq!(
            ids(&notifications_to_show(&store, &incoming, &ctx())),
            vec!["n3"]
        );
    }

    #[test]
    fn respects_dnd_focus_and_setting() {
        let store = synced_store(vec![]);
        let incoming = batch(
            false,
            vec![
                row("n1", "alice", "c1", false),
                row("n2", "alice", "c2", false),
            ],
        );

        let mut dnd = ctx();
        dnd.dnd_until = parse_dnd_until("2026-11-11T09:00:00+09:00");
        // 09:00+09:00 は 00:00 UTC と同じなので、もう明けている。
        assert_eq!(notifications_to_show(&store, &incoming, &dnd).len(), 2);
        dnd.dnd_until = parse_dnd_until("2026-11-11T10:00:00+09:00");
        assert!(notifications_to_show(&store, &incoming, &dnd).is_empty());

        let mut focused = ctx();
        focused.focused_channel = Some("c1".into());
        assert_eq!(
            ids(&notifications_to_show(&store, &incoming, &focused)),
            vec!["n2"]
        );

        let mut disabled = ctx();
        disabled.enabled = false;
        assert!(notifications_to_show(&store, &incoming, &disabled).is_empty());
    }

    #[test]
    fn excerpt_keeps_240_chars() {
        let long = "あ".repeat(300);
        let short = excerpt(&long);
        assert_eq!(short.chars().count(), 241);
        assert!(short.ends_with('…'));
        assert_eq!(excerpt("短い"), "短い");
    }
}
