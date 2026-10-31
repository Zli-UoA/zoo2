//! シェイプから届いた行を保持し、画面に出す形 (チャンネル一覧、タイムライン、スレッド、
//! リアクションの集計) を作る。frontend の `lib/electric/messageFeed.ts` や `threads.ts` と
//! 同じく、結合と絞り込みはクライアントで行う。

use std::collections::{HashMap, HashSet};

use crate::electric::{
    Batch, Change, ShapeId,
    rows::{Channel, ChannelUser, FromRow, Message, Reaction, User},
};

#[derive(Debug, Default)]
pub struct Store {
    channels: HashMap<String, Channel>,
    users: HashMap<String, User>,
    channel_users: HashMap<String, ChannelUser>,
    messages: HashMap<String, HashMap<String, Message>>,
    reactions: HashMap<String, HashMap<String, Reaction>>,
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
        }
        self.invalid_rows += invalid;
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
        self.channels
            .values()
            .find(|channel| channel.id == channel_id)
    }

    pub fn user(&self, user_id: &str) -> Option<&User> {
        self.users.values().find(|user| user.id == user_id)
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

    fn visible_messages(&self, channel_id: &str) -> impl Iterator<Item = &Message> {
        self.messages
            .get(channel_id)
            .into_iter()
            .flat_map(|rows| rows.values())
            .filter(|message| message.deleted_at.is_none())
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
            .filter(|message| message.parent_id.as_deref() == Some(root_id))
            .collect();
        sort_messages(&mut messages);
        messages
    }

    /// ルートメッセージごとの返信数。
    pub fn reply_counts(&self, channel_id: &str) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for message in self.visible_messages(channel_id) {
            if let Some(parent_id) = &message.parent_id {
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
        let mut reactions: Vec<&Reaction> = self
            .reactions
            .get(channel_id)
            .into_iter()
            .flat_map(|rows| rows.values())
            .filter(|reaction| reaction.message_id == message_id)
            .collect();
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
    use crate::electric::Row;

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
    fn builds_timeline_threads_and_reply_counts() {
        let store = store_with_messages();
        let roots: Vec<&str> = store
            .root_messages("c1")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(roots, ["m1", "m2"]);
        let replies: Vec<&str> = store
            .replies("c1", "m1")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(replies, ["r1"]);
        assert_eq!(store.reply_counts("c1").get("m1"), Some(&1));
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
        assert_eq!(store.invalid_rows, 1);
    }
}
