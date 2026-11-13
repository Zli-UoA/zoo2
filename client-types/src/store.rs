//! シェイプから届いた行を保持し、画面に出す形 (チャンネル一覧、タイムライン、スレッド、
//! リアクションの集計) を作る。frontend の `lib/electric/messageFeed.ts` や `threads.ts` と
//! 同じく、結合と絞り込みはクライアントで行う。

pub use crate::accounts::AccountProfiles;
use std::collections::{HashMap, HashSet};

use crate::{
    rows::{Attachment, Channel, ChannelUser, FromRow, Message, Notification, Reaction, User},
    shape::{Batch, Change, ShapeId},
};

#[derive(Debug, Default)]
pub struct Store {
    /// AuthZ directory; never filled from the application's users Shape.
    pub account_profiles: crate::accounts::AccountProfiles,
    channels: HashMap<String, Channel>,
    users: HashMap<String, User>,
    /// 行はシェイプのキーで持つので、id から引くための索引を別に持つ。
    channel_keys: HashMap<String, String>,
    user_keys: HashMap<String, String>,
    channel_users: HashMap<String, ChannelUser>,
    messages: HashMap<String, HashMap<String, Message>>,
    reactions: HashMap<String, HashMap<String, Reaction>>,
    attachments: HashMap<String, HashMap<String, Attachment>>,
    notifications: HashMap<String, Notification>,
    notification_keys: HashMap<String, String>,
    synced: HashSet<ShapeId>,
    /// 型に変換できなかった行の数。スキーマのずれに気づけるよう画面に出す。
    pub invalid_rows: usize,
}

/// 同じ絵文字のリアクションをまとめたもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionGroup {
    pub emoji: String,
    pub count: usize,
    pub mine: bool,
}

impl Store {
    pub fn apply(&mut self, id: &ShapeId, batch: Batch) {
        if batch.reset
            && let ShapeId::Messages(channel) = id
        {
            self.reactions.remove(channel);
            self.attachments.remove(channel);
        }
        let mut invalid = 0;
        match id {
            ShapeId::Channels => {
                apply_rows(&mut self.channels, batch.reset, batch.changes, &mut invalid)
            }
            ShapeId::Users => apply_rows(&mut self.users, batch.reset, batch.changes, &mut invalid),
            ShapeId::ChannelUsers => apply_rows(
                &mut self.channel_users,
                batch.reset,
                batch.changes,
                &mut invalid,
            ),
            ShapeId::Messages(channel_id) => apply_rows(
                self.messages.entry(channel_id.clone()).or_default(),
                batch.reset,
                batch.changes,
                &mut invalid,
            ),
            ShapeId::Reactions(channel_id) => apply_rows(
                self.reactions.entry(channel_id.clone()).or_default(),
                batch.reset,
                batch.changes,
                &mut invalid,
            ),
            ShapeId::Attachments(channel_id) => apply_rows(
                self.attachments.entry(channel_id.clone()).or_default(),
                batch.reset,
                batch.changes,
                &mut invalid,
            ),
            ShapeId::Notifications => apply_rows(
                &mut self.notifications,
                batch.reset,
                batch.changes,
                &mut invalid,
            ),
        }
        self.invalid_rows += invalid;
        match id {
            ShapeId::Channels => self.channel_keys = index(&self.channels, |c| &c.id),
            ShapeId::Users => self.user_keys = index(&self.users, |u| &u.id),
            ShapeId::Notifications => {
                self.notification_keys = index(&self.notifications, |n| &n.id)
            }
            _ => {}
        }
        if batch.reset {
            self.synced.remove(id);
        }
        if batch.up_to_date {
            self.synced.insert(id.clone());
        }
    }

    /// 初回同期が終わったか。終わるまでは「読み込み中」と表示する。
    pub fn is_synced(&self, id: &ShapeId) -> bool {
        self.synced.contains(id)
    }

    /// 購読をやめたチャンネルの行を捨てる。チャンネルを切り替えるときに使う。
    pub fn forget(&mut self, id: &ShapeId) {
        match id {
            ShapeId::Messages(channel_id) => {
                self.messages.remove(channel_id);
            }
            ShapeId::Reactions(channel_id) => {
                self.reactions.remove(channel_id);
            }
            ShapeId::Attachments(channel_id) => {
                self.attachments.remove(channel_id);
            }
            // チャンネル一覧などは購読をやめないので、捨てない。
            _ => return,
        }
        self.synced.remove(id);
    }

    /// 通知を新しい順に返す。
    pub fn notifications(&self) -> Vec<&Notification> {
        let mut notifications: Vec<&Notification> = self.notifications.values().collect();
        notifications.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
        notifications
    }

    /// 通知を id で引く。通知の行のキーは id と同じではないため、索引を通す。
    pub fn notification(&self, notification_id: &str) -> Option<&Notification> {
        self.notifications
            .get(self.notification_keys.get(notification_id)?)
    }

    /// 削除されていないチャンネルを、通常のチャンネル (名前順)、DM の順に並べる。
    pub fn channels(&self) -> Vec<&Channel> {
        let mut channels: Vec<&Channel> = self
            .channels
            .values()
            .filter(|channel| channel.deleted_at.is_none())
            .collect();
        channels.sort_by(|a, b| {
            (a.is_dm, &a.channel_name, &a.id).cmp(&(b.is_dm, &b.channel_name, &b.id))
        });
        channels
    }

    pub fn channel(&self, channel_id: &str) -> Option<&Channel> {
        self.channels.get(self.channel_keys.get(channel_id)?)
    }

    pub fn user(&self, user_id: &str) -> Option<&User> {
        self.users.get(self.user_keys.get(user_id)?)
    }

    /// 表示名。`display_name`、`user_name`、id の順に使う (frontend の `messageFormat.ts` と同じ)。
    pub fn display_name(&self, user_id: &str) -> String {
        match self.user(user_id) {
            Some(user) if !user.display_name.is_empty() => user.display_name.clone(),
            Some(user) => user.user_name.clone(),
            None => user_id.to_string(),
        }
    }

    pub fn is_member(&self, channel_id: &str, user_id: &str) -> bool {
        self.channel_users
            .values()
            .any(|member| member.channel_id == channel_id && member.user_id == user_id)
    }

    /// チャンネルに参加しているユーザー一覧。
    pub fn channel_members(&self, channel_id: &str) -> Vec<&User> {
        let member_ids: HashSet<&str> = self
            .channel_users
            .values()
            .filter(|cu| cu.channel_id == channel_id)
            .map(|cu| cu.user_id.as_str())
            .collect();

        self.users
            .values()
            .filter(|u| member_ids.contains(u.id.as_str()))
            .collect()
    }

    /// チャンネル内の全添付ファイルを返す。
    pub fn channel_attachments(&self, channel_id: &str) -> Vec<&Attachment> {
        let mut result: Vec<&Attachment> = self
            .attachments
            .get(channel_id)
            .into_iter()
            .flat_map(|map| map.values())
            .filter(|attachment| {
                !attachment.message_id.as_ref().is_some_and(|id| {
                    self.message(channel_id, id)
                        .is_some_and(|message| message.display_attachments.is_some())
                })
            })
            .collect();
        if let Some(messages) = self.messages.get(channel_id) {
            for message in messages.values() {
                if message.deleted_at.is_none() {
                    result.extend(message.display_attachments.iter().flatten());
                }
            }
        }
        result
    }

    /// メッセージに紐づく添付ファイル一覧。
    pub fn attachments_for_message(&self, channel_id: &str, message_id: &str) -> Vec<&Attachment> {
        let mut result: Vec<_> = self
            .channel_attachments(channel_id)
            .into_iter()
            .filter(|att| att.message_id.as_deref() == Some(message_id))
            .collect();
        result.sort_by_key(|att| att.created_at);
        result
    }

    /// 同梱メタデータを優先し、旧データだけをShapeから補完する。
    pub fn attachments_by_message(&self, channel_id: &str) -> HashMap<String, Vec<Attachment>> {
        let mut by_message: HashMap<String, Vec<Attachment>> = HashMap::new();
        for att in self.channel_attachments(channel_id) {
            if let Some(msg_id) = &att.message_id {
                by_message
                    .entry(msg_id.clone())
                    .or_default()
                    .push(att.clone());
            }
        }
        for list in by_message.values_mut() {
            list.sort_by_key(|att| att.created_at);
        }
        by_message
    }

    pub fn attachment(&self, channel_id: &str, attachment_id: &str) -> Option<&Attachment> {
        self.channel_attachments(channel_id)
            .into_iter()
            .find(|att| att.id == attachment_id)
    }

    /// 一覧に出す名前。DM は `dm:<id>:<id>` という内部名なので、自分以外の参加者の名前にする。
    pub fn channel_label(&self, channel: &Channel, me: &str) -> String {
        if !channel.is_dm {
            return format!("#{}", channel.channel_name);
        }
        let mut names: Vec<String> = self
            .channel_users
            .values()
            .filter(|member| member.channel_id == channel.id && member.user_id != me)
            .map(|member| self.display_name(&member.user_id))
            .collect();
        names.sort();
        if names.is_empty() {
            "@(自分)".to_string()
        } else {
            format!("@{}", names.join(", "))
        }
    }

    pub fn visible_messages(&self, channel_id: &str) -> impl Iterator<Item = &Message> {
        self.messages
            .get(channel_id)
            .into_iter()
            .flat_map(|rows| rows.values())
            .filter(|message| message.deleted_at.is_none())
    }

    /// チャンネル内の全メッセージ (ルートおよび全スレッド返信を含む) を古い順に返す。
    pub fn channel_messages(&self, channel_id: &str) -> Vec<&Message> {
        let mut messages: Vec<&Message> = self.visible_messages(channel_id).collect();
        sort_messages(&mut messages);
        messages
    }

    /// チャンネルのタイムライン (スレッド返信を除く) を古い順に返す。
    pub fn root_messages(&self, channel_id: &str) -> Vec<&Message> {
        let mut messages: Vec<&Message> = self
            .visible_messages(channel_id)
            .filter(|message| message.parent_id.is_none())
            .collect();
        sort_messages(&mut messages);
        messages
    }

    /// スレッドの返信を古い順に返す。
    pub fn replies(&self, channel_id: &str, root_id: &str) -> Vec<&Message> {
        let mut messages: Vec<&Message> = self
            .visible_messages(channel_id)
            .filter(|message| {
                message.reply_to_message_id.as_deref() == Some(root_id)
                    || message.parent_id.as_deref() == Some(root_id)
            })
            .collect();
        sort_messages(&mut messages);
        messages
    }

    /// ルートメッセージごとの返信数。
    pub fn reply_counts(&self, channel_id: &str) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for message in self.visible_messages(channel_id) {
            if let Some(parent_id) = message
                .reply_to_message_id
                .as_ref()
                .or(message.parent_id.as_ref())
            {
                *counts.entry(parent_id.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    pub fn message(&self, channel_id: &str, message_id: &str) -> Option<&Message> {
        self.visible_messages(channel_id)
            .find(|message| message.id == message_id)
    }

    /// 既読位置に使う、最新のルートメッセージ。
    pub fn latest_root(&self, channel_id: &str) -> Option<&Message> {
        self.root_messages(channel_id).into_iter().last()
    }

    /// メッセージのリアクションを絵文字ごとにまとめ、最初に付いた順に並べる。
    pub fn reaction_groups(
        &self,
        channel_id: &str,
        message_id: &str,
        me: &str,
    ) -> Vec<ReactionGroup> {
        let reactions = self.message_reactions(channel_id, message_id);
        group_reactions(reactions, me)
    }

    /// チャンネル内の全メッセージのリアクションを、メッセージ ID ごとに1回でまとめる。
    /// 描画のたびにメッセージごとに全リアクションを走査しないためのもの。
    pub fn reaction_groups_by_message(
        &self,
        channel_id: &str,
        me: &str,
    ) -> HashMap<String, Vec<ReactionGroup>> {
        let mut by_message: HashMap<&str, Vec<&Reaction>> = HashMap::new();
        for row in self
            .reactions
            .get(channel_id)
            .into_iter()
            .flat_map(|rows| rows.values())
        {
            by_message.entry(&row.message_id).or_default().push(row);
        }
        for message in self.visible_messages(channel_id) {
            if let Some(rows) = &message.display_reactions {
                by_message.insert(&message.id, rows.iter().collect());
            }
        }
        by_message
            .into_iter()
            .filter(|(_, rows)| !rows.is_empty())
            .map(|(id, rows)| (id.to_string(), group_reactions(rows, me)))
            .collect()
    }

    fn message_reactions(&self, channel_id: &str, message_id: &str) -> Vec<&Reaction> {
        if let Some(rows) = self
            .message(channel_id, message_id)
            .and_then(|message| message.display_reactions.as_ref())
        {
            return rows.iter().collect();
        }
        self.reactions
            .get(channel_id)
            .into_iter()
            .flat_map(|rows| rows.values())
            .filter(|reaction| reaction.message_id == message_id)
            .collect()
    }
}

/// 絵文字ごとにまとめ、最初に付いた順に並べる。
fn group_reactions<'a>(
    reactions: impl IntoIterator<Item = &'a Reaction>,
    me: &str,
) -> Vec<ReactionGroup> {
    let mut reactions: Vec<&Reaction> = reactions.into_iter().collect();
    reactions.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));

    let mut groups: Vec<ReactionGroup> = Vec::new();
    for reaction in reactions {
        let group = match groups
            .iter_mut()
            .find(|group| group.emoji == reaction.emoji)
        {
            Some(group) => group,
            None => {
                groups.push(ReactionGroup {
                    emoji: reaction.emoji.clone(),
                    count: 0,
                    mine: false,
                });
                groups.last_mut().expect("直前に追加した")
            }
        };
        group.count += 1;
        group.mine |= reaction.user_id == me;
    }
    groups
}

/// 行の id からシェイプのキーへの索引。
fn index<T>(rows: &HashMap<String, T>, id: impl Fn(&T) -> &String) -> HashMap<String, String> {
    rows.iter()
        .map(|(key, row)| (id(row).clone(), key.clone()))
        .collect()
}

fn apply_rows<T: FromRow>(
    rows: &mut HashMap<String, T>,
    reset: bool,
    changes: Vec<Change>,
    invalid: &mut usize,
) {
    if reset {
        rows.clear();
    }
    for change in changes {
        match change {
            Change::Upsert { key, row } => match T::from_row(&row) {
                Ok(value) => {
                    rows.insert(key, value);
                }
                Err(_) => *invalid += 1,
            },
            Change::Delete { key } => {
                rows.remove(&key);
            }
        }
    }
}

fn sort_messages(messages: &mut [&Message]) {
    messages.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::shape::Row;

    fn upsert(key: &str, value: Value) -> Change {
        Change::Upsert {
            key: key.into(),
            row: value.as_object().unwrap().clone() as Row,
        }
    }

    fn batch(changes: Vec<Change>) -> Batch {
        Batch {
            reset: false,
            changes,
            up_to_date: true,
        }
    }

    fn message(id: &str, at: &str, parent: Option<&str>, deleted: bool) -> Change {
        upsert(
            id,
            json!({
                "id": id, "user_id": "u1", "channel_id": "c1", "content": id,
                "created_at": at, "parent_id": parent,
                "deleted_at": if deleted { Some("2026-11-01 00:00:00") } else { None },
            }),
        )
    }

    fn store_with_messages() -> Store {
        let mut store = Store::default();
        store.apply(
            &ShapeId::Messages("c1".into()),
            batch(vec![
                message("m2", "2026-11-01 00:00:02", None, false),
                message("m1", "2026-11-01 00:00:01", None, false),
                message("r1", "2026-11-01 00:00:03", Some("m1"), false),
                message("r2", "2026-11-01 00:00:04", Some("m1"), true),
                message("m3", "2026-11-01 00:00:05", None, true),
            ]),
        );
        store
    }

    #[test]
    fn joined_attachments_override_legacy_rows_and_empty_lists_remove_stale_files() {
        let mut store = Store::default();
        let attachment = json!({"id":"a", "message_id":"m", "uploader_id":"u", "channel_id":"c", "file_name":"pic.png", "file_size":100, "content_type":"image/png", "storage_key":"key", "created_at":"2026-11-01 03:00:00"});
        store.apply(
            &ShapeId::Attachments("c".into()),
            batch(vec![upsert("a", attachment.clone())]),
        );
        let mut row = json!({"id":"m", "user_id":"u", "channel_id":"c", "content":"", "created_at":"2026-11-01 03:00:00", "display_data": {"attachments":[attachment]}});
        store.apply(
            &ShapeId::Messages("c".into()),
            batch(vec![upsert("electric-message-key", row.clone())]),
        );
        assert_eq!(store.channel_attachments("c").len(), 1);
        row["display_data"] = json!({"attachments":[]});
        store.apply(
            &ShapeId::Messages("c".into()),
            batch(vec![upsert("electric-message-key", row)]),
        );
        assert!(store.channel_attachments("c").is_empty());
        assert!(store.attachments_by_message("c").is_empty());
        assert!(store.attachment("c", "a").is_none());
        assert_eq!(store.invalid_rows, 0);
    }

    #[test]
    fn builds_timeline_threads_and_reply_counts() {
        let mut store = store_with_messages();
        store.apply(
            &ShapeId::Messages("c1".into()),
            batch(vec![upsert(
                "r3",
                json!({
                    "id":"r3", "user_id":"u1", "channel_id":"c1", "content":"direct reply",
                    "created_at":"2026-11-01 00:00:06", "parent_id":null,
                    "reply_to_message_id":"m2"
                }),
            )]),
        );
        let roots: Vec<&str> = store
            .root_messages("c1")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(roots, ["m1", "m2", "r3"]);
        let replies: Vec<&str> = store
            .replies("c1", "m1")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(replies, ["r1"]);
        assert_eq!(store.reply_counts("c1").get("m1"), Some(&1));
        assert_eq!(store.reply_counts("c1").get("m2"), Some(&1));
        assert_eq!(store.replies("c1", "m2")[0].id, "r3");
        assert_eq!(store.latest_root("c1").unwrap().id, "m2");
        assert!(store.is_synced(&ShapeId::Messages("c1".into())));
    }

    #[test]
    fn reset_and_delete_remove_rows() {
        let mut store = store_with_messages();
        let id = ShapeId::Messages("c1".into());
        store.apply(
            &id,
            Batch {
                changes: vec![Change::Delete { key: "m1".into() }],
                ..Default::default()
            },
        );
        assert_eq!(store.root_messages("c1").len(), 1);
        store.apply(
            &id,
            Batch {
                reset: true,
                ..Default::default()
            },
        );
        assert!(store.root_messages("c1").is_empty());
        assert!(!store.is_synced(&id));
    }

    #[test]
    fn looks_up_rows_by_id_not_shape_key() {
        let mut store = Store::default();
        let key = "\"public\".\"users\"/\"u1\"";
        store.apply(
            &ShapeId::Users,
            batch(vec![upsert(
                key,
                json!({ "id": "u1", "user_name": "alice", "display_name": "" }),
            )]),
        );
        assert_eq!(store.display_name("u1"), "alice");
        store.apply(
            &ShapeId::Users,
            batch(vec![Change::Delete { key: key.into() }]),
        );
        assert_eq!(store.display_name("u1"), "u1");
    }

    #[test]
    fn sorts_channels_and_labels_dms_by_other_members() {
        let mut store = Store::default();
        store.apply(
            &ShapeId::Channels,
            batch(vec![
                upsert("c3", json!({ "id": "c3", "channel_name": "dm:u1:u2", "is_private": "t", "is_dm": "t" })),
                upsert("c2", json!({ "id": "c2", "channel_name": "random", "is_private": "f" })),
                upsert("c1", json!({ "id": "c1", "channel_name": "general", "is_private": "f" })),
                upsert(
                    "c4",
                    json!({ "id": "c4", "channel_name": "old", "is_private": "f",
                            "deleted_at": "2026-11-01 00:00:00" }),
                ),
            ]),
        );
        store.apply(
            &ShapeId::Users,
            batch(vec![
                upsert(
                    "u1",
                    json!({ "id": "u1", "user_name": "alice", "display_name": "Alice" }),
                ),
                upsert(
                    "u2",
                    json!({ "id": "u2", "user_name": "bob", "display_name": "" }),
                ),
            ]),
        );
        store.apply(
            &ShapeId::ChannelUsers,
            batch(vec![
                upsert("a", json!({ "user_id": "u1", "channel_id": "c3" })),
                upsert("b", json!({ "user_id": "u2", "channel_id": "c3" })),
            ]),
        );

        let labels: Vec<String> = store
            .channels()
            .iter()
            .map(|channel| store.channel_label(channel, "u1"))
            .collect();
        assert_eq!(labels, ["#general", "#random", "@bob"]);
        assert!(store.is_member("c3", "u2"));
        assert!(!store.is_member("c1", "u2"));
        assert_eq!(store.display_name("u1"), "Alice");
        assert_eq!(store.display_name("u9"), "u9");

        let members = store.channel_members("c3");
        assert_eq!(members.len(), 2);

        store.apply(
            &ShapeId::Attachments("c1".into()),
            batch(vec![upsert(
                "att1",
                json!({
                    "id": "att1", "message_id": "m1", "uploader_id": "u1",
                    "channel_id": "c1", "file_name": "pic.png", "file_size": 100,
                    "content_type": "image/png", "storage_key": "k1",
                    "created_at": "2026-11-01 10:00:00"
                }),
            )]),
        );

        let atts = store.attachments_for_message("c1", "m1");
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0].file_name, "pic.png");
        assert_eq!(store.attachment("c1", "att1").unwrap().file_name, "pic.png");
        assert!(store.attachments_for_message("c1", "m2").is_empty());

        let by_msg = store.attachments_by_message("c1");
        assert_eq!(by_msg.get("m1").unwrap().len(), 1);
        assert_eq!(by_msg.get("m1").unwrap()[0].file_name, "pic.png");
        assert!(!by_msg.contains_key("m2"));
    }

    #[test]
    fn bundled_reactions_include_remote_actors_and_empty_updates_remove_them() {
        let mut store = Store::default();
        let mut row = json!({ "id":"m1", "user_id":"me", "channel_id":"c1", "created_at":"2026-11-01 00:00:00", "display_data": { "reactions":[
            {"id":"local", "message_id":"m1", "user_id":"me", "emoji":"👍", "created_at":"2026-11-01 00:00:01"},
            {"id":"remote", "message_id":"m1", "user_id":"https://remote/user", "emoji":"👍", "created_at":"2026-11-01 00:00:02"}
        ] }});
        let id = ShapeId::Messages("c1".into());
        store.apply(&id, batch(vec![upsert("m1", row.clone())]));
        assert_eq!(store.reaction_groups("c1", "m1", "me")[0].count, 2);
        assert!(store.reaction_groups_by_message("c1", "me")["m1"][0].mine);
        row["display_data"] = Value::String(json!({"reactions":[]}).to_string());
        store.apply(&id, batch(vec![upsert("m1", row)]));
        assert!(store.reaction_groups("c1", "m1", "me").is_empty());
        assert!(store.reaction_groups_by_message("c1", "me").is_empty());
        assert_eq!(store.invalid_rows, 0);
    }

    #[test]
    fn groups_reactions_and_counts_invalid_rows() {
        let mut store = Store::default();
        let reaction = |id: &str, user: &str, emoji: &str, at: &str| {
            upsert(
                id,
                json!({ "id": id, "message_id": "m1", "user_id": user, "emoji": emoji, "created_at": at }),
            )
        };
        store.apply(
            &ShapeId::Reactions("c1".into()),
            batch(vec![
                reaction("r2", "u2", "🎉", "2026-11-01 00:00:02"),
                reaction("r1", "u1", "👍", "2026-11-01 00:00:01"),
                reaction("r3", "u2", "👍", "2026-11-01 00:00:03"),
                upsert("bad", json!({ "id": "bad" })),
            ]),
        );
        assert_eq!(
            store.reaction_groups("c1", "m1", "u1"),
            vec![
                ReactionGroup {
                    emoji: "👍".into(),
                    count: 2,
                    mine: true
                },
                ReactionGroup {
                    emoji: "🎉".into(),
                    count: 1,
                    mine: false
                },
            ]
        );
        let by_message = store.reaction_groups_by_message("c1", "u1");
        assert_eq!(by_message.len(), 1);
        assert_eq!(by_message["m1"], store.reaction_groups("c1", "m1", "u1"));
        assert_eq!(store.invalid_rows, 1);
    }
    #[test]
    fn looks_up_notifications_by_id_through_the_index() {
        let notification = |id: &str| {
            upsert(
                &format!("\"public\".\"notification_events\"/\"{id}\""),
                json!({
                    "id": id, "user_id": "me", "actor_id": "alice", "channel_id": "c1",
                    "message_id": "m1", "notification_type": "mention", "is_read": "false",
                    "created_at": "2026-11-11 00:00:00",
                }),
            )
        };
        let mut store = Store::default();
        store.apply(
            &ShapeId::Notifications,
            batch(vec![notification("n1"), notification("n2")]),
        );
        assert_eq!(store.notification("n1").map(|n| n.id.as_str()), Some("n1"));

        store.apply(
            &ShapeId::Notifications,
            batch(vec![Change::Delete {
                key: "\"public\".\"notification_events\"/\"n1\"".into(),
            }]),
        );
        assert!(store.notification("n1").is_none());
        assert!(store.notification("n2").is_some());
    }
}
