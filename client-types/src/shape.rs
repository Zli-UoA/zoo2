//! Electric のシェイプで届く行と、その変化のまとまり。
//!
//! 通信の部分 (`ShapeStream`) は `zoo2-client-core` にある。ここには、Tauri 版で
//! Rust 側から WebView 側へ IPC でそのまま渡せる、データだけの型を置く。

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type Row = Map<String, Value>;

/// 1回の取得で起きた変化。`reset` が true なら、それまでの行をすべて捨ててから適用する。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    pub reset: bool,
    pub changes: Vec<Change>,
    /// 初回同期が終わり、ローカルの行がサーバーと揃ったことを示す。
    pub up_to_date: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    Upsert { key: String, row: Row },
    Delete { key: String },
}

/// backend のシェイププロキシ (`/api/shapes/*`) のうち、クライアントが購読するもの。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "shape", content = "channel_id", rename_all = "snake_case")]
pub enum ShapeId {
    Channels,
    Users,
    ChannelUsers,
    Messages(String),
    Reactions(String),
    Attachments(String),
    /// 自分あての通知 (`notification_events`)。
    Notifications,
}

impl ShapeId {
    /// プロキシのパス (`/api/shapes/` の後ろ) と、追加のクエリ。
    pub fn path_and_params(&self) -> (&'static str, Vec<(String, String)>) {
        let channel = |id: &String| vec![("channel_id".to_string(), id.clone())];
        match self {
            ShapeId::Channels => ("channels", Vec::new()),
            ShapeId::Users => ("users", Vec::new()),
            ShapeId::ChannelUsers => ("channel_users", Vec::new()),
            ShapeId::Messages(id) => ("messages", channel(id)),
            ShapeId::Reactions(id) => ("message_reactions", channel(id)),
            ShapeId::Attachments(id) => ("attachments", channel(id)),
            ShapeId::Notifications => ("notifications", Vec::new()),
        }
    }
}

/// シェイプごとの生の行の写し。
///
/// Tauri 版では Rust 側が購読を持ち、WebView 側は `Store` の写しを持つ。WebView が
/// 読み込み直されたとき (開発中のリロードや、画面の切り替え) に、Rust 側がこれまでの行を
/// `snapshot` でまとめて送り直せるように、届いた行を型に変換する前の形で保持する。
#[derive(Debug, Default)]
pub struct Replica {
    shapes: BTreeMap<ShapeId, BTreeMap<String, Row>>,
    synced: HashSet<ShapeId>,
}

impl Replica {
    pub fn apply(&mut self, id: &ShapeId, batch: &Batch) {
        let rows = self.shapes.entry(id.clone()).or_default();
        if batch.reset {
            rows.clear();
            self.synced.remove(id);
        }
        for change in &batch.changes {
            match change {
                Change::Upsert { key, row } => {
                    rows.insert(key.clone(), row.clone());
                }
                Change::Delete { key } => {
                    rows.remove(key);
                }
            }
        }
        if batch.up_to_date {
            self.synced.insert(id.clone());
        }
    }

    /// 購読をやめたシェイプの行を捨てる。
    pub fn forget(&mut self, id: &ShapeId) {
        self.shapes.remove(id);
        self.synced.remove(id);
    }

    /// 持っている行を、`reset` 付きの 1 バッチずつにまとめる。適用する側は
    /// それまでの行を捨ててから入れ直すので、何度送っても同じ状態になる。
    pub fn snapshot(&self) -> Vec<(ShapeId, Batch)> {
        self.shapes
            .iter()
            .map(|(id, rows)| {
                let changes = rows
                    .iter()
                    .map(|(key, row)| Change::Upsert {
                        key: key.clone(),
                        row: row.clone(),
                    })
                    .collect();
                let batch = Batch {
                    reset: true,
                    changes,
                    up_to_date: self.synced.contains(id),
                };
                (id.clone(), batch)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn upsert(key: &str) -> Change {
        Change::Upsert {
            key: key.into(),
            row: json!({ "id": key }).as_object().unwrap().clone(),
        }
    }

    #[test]
    fn shape_id_and_change_have_stable_json() {
        assert_eq!(
            serde_json::to_value(ShapeId::Messages("c1".into())).unwrap(),
            json!({ "shape": "messages", "channel_id": "c1" })
        );
        assert_eq!(
            serde_json::to_value(ShapeId::Notifications).unwrap(),
            json!({ "shape": "notifications" })
        );
        assert_eq!(
            serde_json::to_value(Change::Delete { key: "k".into() }).unwrap(),
            json!({ "op": "delete", "key": "k" })
        );
        let id: ShapeId = serde_json::from_value(json!({ "shape": "channels" })).unwrap();
        assert_eq!(id, ShapeId::Channels);
    }

    #[test]
    fn replica_snapshot_replays_current_rows() {
        let mut replica = Replica::default();
        let id = ShapeId::Channels;
        replica.apply(
            &id,
            &Batch {
                reset: false,
                changes: vec![upsert("a"), upsert("b")],
                up_to_date: false,
            },
        );
        replica.apply(
            &id,
            &Batch {
                reset: false,
                changes: vec![Change::Delete { key: "a".into() }],
                up_to_date: true,
            },
        );
        let snapshot = replica.snapshot();
        assert_eq!(snapshot.len(), 1);
        let (snap_id, batch) = &snapshot[0];
        assert_eq!(snap_id, &id);
        assert!(batch.reset && batch.up_to_date);
        assert_eq!(batch.changes, vec![upsert("b")]);

        replica.apply(
            &id,
            &Batch {
                reset: true,
                changes: vec![upsert("c")],
                up_to_date: false,
            },
        );
        let (_, batch) = &replica.snapshot()[0];
        assert!(!batch.up_to_date);
        assert_eq!(batch.changes, vec![upsert("c")]);

        replica.forget(&id);
        assert!(replica.snapshot().is_empty());
    }
}
