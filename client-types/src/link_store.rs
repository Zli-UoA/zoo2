//! 同期した行（[`Store`]）と、別に取ったリンク先の結果から、本文のアプリ内リンクの状態を決める。
//! 取得を別のプロセスが持つデスクトップ版の画面で使う。

use std::collections::{HashMap, HashSet};

use crate::{
    ipc::LinkedMessage,
    links::{self, EncryptionCheck, LinkState, LinkTarget, LinkedFetch},
    rows::Message,
    shape::ShapeId,
    store::Store,
};

/// 開いているチャンネル以外の、取得した結果。
pub type Fetched = HashMap<LinkedMessage, LinkedFetch>;

/// このチャンネルで読み込んでいる行に、暗号文の行があるか。
pub fn has_ciphertext(store: &Store, channel_id: &str) -> bool {
    store
        .visible_messages(channel_id)
        .any(|message| message.ciphertext.is_some())
}

/// 画面の近くにある本文 (`contents`) の、取得が要るリンク先。本文ごとに
/// [`links::MAX_FETCHED_PER_BODY`] 件までのうち、一覧にあるチャンネルのメッセージで、開いている
/// チャンネル（同期した行を使う）以外のものを、重複なく返す。
pub fn wanted<'a>(
    store: &Store,
    open: Option<&str>,
    contents: impl IntoIterator<Item = &'a str>,
    app_base_url: &str,
) -> Vec<LinkedMessage> {
    let mut result: Vec<LinkedMessage> = contents
        .into_iter()
        .flat_map(|content| {
            links::fetch_targets(
                content,
                app_base_url,
                &links::embedded_links(content, app_base_url),
                &links::app_links(content, app_base_url),
            )
        })
        .filter_map(|target| {
            let message_id = target.linked_message_id()?.to_owned();
            let channel = store.channel(&target.channel_id)?;
            (channel.deleted_at.is_none() && open != Some(channel.id.as_str())).then_some(
                LinkedMessage {
                    channel_id: target.channel_id,
                    message_id,
                },
            )
        })
        .collect();
    result.sort();
    result.dedup();
    result
}

/// 開いているチャンネルのこと。
#[derive(Debug, Clone, Copy, Default)]
pub struct Open<'a> {
    pub channel_id: Option<&'a str>,
    /// 同期した行に暗号文の行があるか（[`has_ciphertext`]）。リンクごとに行を調べないよう、
    /// 呼ぶ側が行の変わったときに調べておく。
    pub has_ciphertext: bool,
    /// 同期を待ち始めて [`links::LOAD_TIMEOUT`] がたった。
    pub stalled: bool,
    /// 同期が停止した。権限を失ったかもしれないので、届いていた行も出さない。
    pub stopped: bool,
}

/// リンク先の送る先の行と、返信かどうか。この画面はスレッドを開けないので、返信は親の行にする。
/// 行か、返信の親の行を表示できなければ `None`。
pub fn anchor(store: &Store, channel_id: &str, message_id: &str) -> Option<(String, bool)> {
    let message = store.message(channel_id, message_id)?;
    match &message.parent_id {
        Some(parent) => store
            .message(channel_id, parent)
            .filter(|parent| parent.parent_id.is_none())
            .map(|parent| (parent.id.clone(), true)),
        None => Some((message.id.clone(), false)),
    }
}

/// 別のチャンネルのメッセージへ移る前に確かめた、送る先の行。
#[derive(Debug, Clone, PartialEq)]
pub enum Destination {
    /// 表示できる。
    Ready,
    /// 取得を待つ。そのあいだ取り続けるリンク先（返信は親も）。
    Waiting(Vec<LinkedMessage>),
    Unavailable,
    /// 取得できなかった。
    Failed,
}

enum Looked<'a> {
    Shown(&'a Message),
    Waiting,
    Unavailable,
    Failed,
}

fn look<'a>(
    fetched: &'a Fetched,
    checked: &HashSet<LinkedMessage>,
    key: &LinkedMessage,
) -> Looked<'a> {
    match fetched.get(key) {
        None | Some(LinkedFetch::Loading { stalled: false }) => Looked::Waiting,
        Some(LinkedFetch::Loading { stalled: true } | LinkedFetch::Failed) => Looked::Failed,
        // 覚えていた結果は、削除などを見落としているかもしれないので、取り直しを待つ。
        Some(_) if !checked.contains(key) => Looked::Waiting,
        Some(LinkedFetch::Found(message))
            if message.id == key.message_id
                && message.channel_id == key.channel_id
                && message.deleted_at.is_none() =>
        {
            Looked::Shown(message)
        }
        Some(LinkedFetch::Missing | LinkedFetch::Found(_)) => Looked::Unavailable,
    }
}

/// 取得した結果で、`message_id` へ移れるか。返信は、送る先になる親の行も確かめる。`checked` は、
/// 移る前に取り直した結果のリンク先で、ほかの結果は使わない。
pub fn destination(
    fetched: &Fetched,
    checked: &HashSet<LinkedMessage>,
    channel_id: &str,
    message_id: &str,
) -> Destination {
    let key = |message_id: &str| LinkedMessage {
        channel_id: channel_id.to_owned(),
        message_id: message_id.to_owned(),
    };
    let message = match look(fetched, checked, &key(message_id)) {
        Looked::Shown(message) => message,
        Looked::Waiting => return Destination::Waiting(vec![key(message_id)]),
        Looked::Unavailable => return Destination::Unavailable,
        Looked::Failed => return Destination::Failed,
    };
    let Some(parent_id) = &message.parent_id else {
        return Destination::Ready;
    };
    match look(fetched, checked, &key(parent_id)) {
        Looked::Shown(parent) if parent.parent_id.is_none() => Destination::Ready,
        Looked::Shown(_) | Looked::Unavailable => Destination::Unavailable,
        Looked::Waiting => Destination::Waiting(vec![key(message_id), key(parent_id)]),
        Looked::Failed => Destination::Failed,
    }
}

/// リンク先の状態。`ciphertext` は暗号文の行が届いたことのあるチャンネル。`recorded` は端末が
/// そのチャンネルを暗号化チャンネルと記録しているかで、記録を読めなければ `None`（平文の本文を
/// 出さない）。
pub fn state<'a>(
    store: &'a Store,
    fetched: &Fetched,
    open: Open<'_>,
    ciphertext: &HashSet<String>,
    recorded: Option<bool>,
    target: &LinkTarget,
) -> LinkState<'a> {
    let channel_id = target.channel_id.as_str();
    let channel = store.channel(channel_id);
    let open_here = open.channel_id == Some(channel_id);
    let message_id = target.linked_message_id();
    let fetch = match message_id {
        // チャンネルだけのリンクは、メッセージを取らない。
        None => LinkedFetch::Missing,
        Some(_) if open_here && open.stopped => LinkedFetch::Failed,
        Some(id) if open_here => match store.message(channel_id, id) {
            Some(message) => LinkedFetch::Found(Box::new(message.clone())),
            None if store.is_synced(&ShapeId::Messages(channel_id.to_owned())) => {
                LinkedFetch::Missing
            }
            None => LinkedFetch::Loading {
                stalled: open.stalled,
            },
        },
        Some(id) => fetched
            .get(&LinkedMessage {
                channel_id: channel_id.to_owned(),
                message_id: id.to_owned(),
            })
            .cloned()
            .unwrap_or(LinkedFetch::Loading { stalled: false }),
    };
    // 暗号文の行は、対象の行と、読み込んでいる同じチャンネルの行、届いたことのあるチャンネルを
    // 見る。チャンネルだけのリンクは見ない。
    let has_ciphertext = message_id.is_some()
        && ((open_here && open.has_ciphertext)
            || ciphertext.contains(channel_id)
            || fetched.iter().any(|(key, fetch)| {
                key.channel_id == channel_id
                    && matches!(fetch, LinkedFetch::Found(message) if message.ciphertext.is_some())
            }));
    let encrypted = channel.is_some_and(|channel| {
        links::treats_as_encrypted(channel, recorded == Some(true), has_ciphertext)
    });
    let check = match recorded {
        Some(_) => EncryptionCheck::Done,
        None => EncryptionCheck::Stalled,
    };
    links::resolve(
        target,
        channel,
        store.is_synced(&ShapeId::Channels),
        &fetch,
        encrypted,
        check,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        links::PreviewBody,
        shape::{Batch, Change},
    };

    const APP: &str = "https://zli.chat";

    fn apply(store: &mut Store, shape: ShapeId, rows: Vec<Value>) {
        let changes = rows
            .into_iter()
            .map(|row| Change::Upsert {
                key: row["id"].as_str().unwrap().to_owned(),
                row: row.as_object().unwrap().clone(),
            })
            .collect();
        store.apply(
            &shape,
            Batch {
                reset: false,
                up_to_date: true,
                changes,
            },
        );
    }

    fn channel(id: &str) -> Value {
        json!({ "id": id, "channel_name": id, "is_private": false, "is_dm": false })
    }

    fn message(id: &str, channel: &str, extra: Value) -> Value {
        let mut row = json!({ "id": id, "user_id": "alice", "channel_id": channel,
            "content": "本文", "created_at": "2026-11-23 07:30:00" });
        row.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        row
    }

    fn key(channel: &str, id: &str) -> LinkedMessage {
        LinkedMessage {
            channel_id: channel.into(),
            message_id: id.into(),
        }
    }

    fn found(id: &str, channel: &str, extra: Value) -> LinkedFetch {
        let row = message(id, channel, extra);
        LinkedFetch::Found(Box::new(
            crate::rows::FromRow::from_row(row.as_object().unwrap()).unwrap(),
        ))
    }

    fn target(channel: &str, id: Option<&str>) -> LinkTarget {
        LinkTarget {
            channel_id: channel.into(),
            message_id: id.map(str::to_owned),
            thread_id: None,
        }
    }

    fn name(state: LinkState<'_>) -> &'static str {
        match state {
            LinkState::Loading => "loading",
            LinkState::Failed => "failed",
            LinkState::Unavailable { .. } => "unavailable",
            LinkState::Channel {
                encrypted: true, ..
            } => "encrypted channel",
            LinkState::Channel { .. } => "channel",
            LinkState::Message { preview, .. } => match preview.body {
                PreviewBody::Text(_) => "text",
                PreviewBody::Encrypted => "encrypted",
                _ => "other",
            },
        }
    }

    fn store() -> Store {
        let mut store = Store::default();
        apply(
            &mut store,
            ShapeId::Channels,
            vec![channel("here"), channel("there")],
        );
        apply(
            &mut store,
            ShapeId::Messages("here".into()),
            vec![message("m1", "here", json!({}))],
        );
        store
    }

    #[test]
    fn asks_only_for_listed_channels_other_than_the_open_one() {
        let store = store();
        let contents = [
            "https://zli.chat/channels/there?message=a https://zli.chat/channels/here?message=m1",
            "https://zli.chat/channels/hidden?message=b https://zli.chat/channels/there",
            // 4 件目より後の URL でも、URL の文字そのものを書いたリンクは短くするので取る。
            "https://example.com/1 https://example.com/2 https://example.com/3 \
             https://zli.chat/channels/there?thread=t [x](https://zli.chat/channels/there?message=z)",
            "https://zli.chat/channels/there?message=a",
            // 1 つの本文からは 3 件まで。
            "https://zli.chat/channels/there?message=b https://zli.chat/channels/there?message=c \
             https://zli.chat/channels/there?message=d https://zli.chat/channels/there?message=e",
        ];
        assert_eq!(
            wanted(&store, Some("here"), contents, APP),
            [
                key("there", "a"),
                key("there", "b"),
                key("there", "c"),
                key("there", "d"),
                key("there", "t")
            ]
        );
    }

    #[test]
    fn the_open_channel_uses_synced_rows() {
        let store = store();
        let fetched = Fetched::new();
        let look = |store: &Store, id, open: Open<'_>| {
            let target = target("here", Some(id));
            let none = HashSet::new();
            name(state(store, &fetched, open, &none, Some(false), &target))
        };
        let open = Open {
            channel_id: Some("here"),
            ..Default::default()
        };
        assert!(!has_ciphertext(&store, "here"));
        assert_eq!(look(&store, "m1", open), "text");
        let secret = Open {
            has_ciphertext: true,
            ..open
        };
        assert_eq!(look(&store, "m1", secret), "encrypted");
        assert_eq!(look(&store, "gone", open), "unavailable");
        // 同期が停止したら、届いていた行も出さない。
        let stopped = Open {
            stopped: true,
            ..open
        };
        assert_eq!(look(&store, "m1", stopped), "failed");
        assert_eq!(look(&store, "gone", stopped), "failed");
        // 同期がそろわないうちは読み込み中で、時間がたったら読み込めなかったものとする。
        let mut loading = Store::default();
        apply(&mut loading, ShapeId::Channels, vec![channel("here")]);
        assert_eq!(look(&loading, "m1", open), "loading");
        let stalled = Open {
            stalled: true,
            ..open
        };
        assert_eq!(look(&loading, "m1", stalled), "failed");
    }

    #[test]
    fn replies_anchor_to_a_parent_that_is_shown() {
        let mut store = store();
        apply(
            &mut store,
            ShapeId::Messages("here".into()),
            vec![
                message("r1", "here", json!({ "parent_id": "m1" })),
                message(
                    "gone",
                    "here",
                    json!({ "deleted_at": "2026-11-24 00:00:00" }),
                ),
                message("r2", "here", json!({ "parent_id": "gone" })),
            ],
        );
        let anchor = |id| anchor(&store, "here", id);
        assert_eq!(anchor("m1"), Some(("m1".into(), false)));
        assert_eq!(anchor("r1"), Some(("m1".into(), true)));
        assert_eq!(anchor("r2"), None, "削除された親へは移動しない");
        assert_eq!(anchor("gone"), None);
    }

    #[test]
    fn another_channel_is_opened_only_when_its_row_is_shown() {
        let mut fetched = Fetched::new();
        let checked: HashSet<LinkedMessage> = ["a", "r", "p"].map(|id| key("there", id)).into();
        let check = |fetched: &Fetched, id| destination(fetched, &checked, "there", id);
        assert_eq!(
            check(&fetched, "a"),
            Destination::Waiting(vec![key("there", "a")])
        );
        fetched.insert(key("there", "a"), found("a", "there", json!({})));
        assert_eq!(check(&fetched, "a"), Destination::Ready);
        // 返信は、親の行も表示できるときだけ移る。
        let reply = found("r", "there", json!({ "parent_id": "p" }));
        fetched.insert(key("there", "r"), reply);
        assert_eq!(
            check(&fetched, "r"),
            Destination::Waiting(vec![key("there", "r"), key("there", "p")])
        );
        fetched.insert(key("there", "p"), LinkedFetch::Missing);
        assert_eq!(check(&fetched, "r"), Destination::Unavailable);
        let deleted = json!({ "deleted_at": "2026-11-24 00:00:00" });
        fetched.insert(key("there", "p"), found("p", "there", deleted.clone()));
        assert_eq!(check(&fetched, "r"), Destination::Unavailable);
        fetched.insert(key("there", "p"), found("p", "there", json!({})));
        assert_eq!(check(&fetched, "r"), Destination::Ready);
        fetched.insert(key("there", "a"), found("a", "there", deleted));
        assert_eq!(check(&fetched, "a"), Destination::Unavailable);
        for failed in [LinkedFetch::Failed, LinkedFetch::Loading { stalled: true }] {
            fetched.insert(key("there", "p"), failed);
            assert_eq!(check(&fetched, "r"), Destination::Failed);
        }
        // 覚えていた結果は、表示できてもできなくても、取り直しを待つ。
        fetched.insert(key("there", "a"), found("a", "there", json!({})));
        fetched.insert(key("there", "p"), found("p", "there", json!({})));
        let stale = |checked: &[&str], id| {
            let checked = checked.iter().map(|id| key("there", id)).collect();
            destination(&fetched, &checked, "there", id)
        };
        assert_eq!(
            stale(&[], "a"),
            Destination::Waiting(vec![key("there", "a")])
        );
        assert_eq!(
            stale(&["r"], "r"),
            Destination::Waiting(vec![key("there", "r"), key("there", "p")])
        );
        assert_eq!(stale(&["r", "p"], "r"), Destination::Ready);
    }

    #[test]
    fn other_channels_use_fetched_results() {
        let store = store();
        let mut fetched = Fetched::new();
        let linked = target("there", Some("a"));
        let open = Open {
            channel_id: Some("here"),
            ..Default::default()
        };
        let none = HashSet::new();
        let look = |fetched: &Fetched, recorded| {
            name(state(&store, fetched, open, &none, recorded, &linked))
        };
        assert_eq!(look(&fetched, Some(false)), "loading");
        fetched.insert(key("there", "a"), found("a", "there", json!({})));
        assert_eq!(look(&fetched, Some(false)), "text");
        // 端末の記録と、読み込んでいる同じチャンネルの暗号文の行でも、暗号化として扱う。
        assert_eq!(look(&fetched, Some(true)), "encrypted");
        assert_eq!(look(&fetched, None), "failed");
        // 以前に暗号文の行が届いたチャンネル（読み込み直す前の取得を含む）も同じ。
        let seen = HashSet::from(["there".to_owned()]);
        assert_eq!(
            name(state(&store, &fetched, open, &seen, Some(false), &linked)),
            "encrypted"
        );
        fetched.insert(
            key("there", "b"),
            found("b", "there", json!({ "ciphertext": "{}" })),
        );
        assert_eq!(look(&fetched, Some(false)), "encrypted");
        let channel_only = target("there", None);
        assert_eq!(
            name(state(
                &store,
                &fetched,
                Open::default(),
                &seen,
                Some(false),
                &channel_only
            )),
            "channel"
        );
        assert_eq!(
            name(state(
                &store,
                &fetched,
                Open::default(),
                &none,
                Some(true),
                &channel_only
            )),
            "encrypted channel"
        );
        let hidden = target("hidden", Some("a"));
        assert_eq!(
            name(state(
                &store,
                &fetched,
                Open::default(),
                &none,
                Some(false),
                &hidden
            )),
            "unavailable"
        );
    }
}
