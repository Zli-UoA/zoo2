//! 同期した行（[`Store`]）と、別に取ったリンク先の結果から、本文のアプリ内リンクの状態を決める。
//! 取得を別のプロセスが持つデスクトップ版の画面で使う。

use std::collections::HashMap;

use crate::{
    ipc::LinkedMessage,
    links::{self, EncryptionCheck, LinkState, LinkTarget, LinkedFetch},
    rows::Message,
    shape::ShapeId,
    store::Store,
};

/// 開いているチャンネル以外の、取得した結果。
pub type Fetched = HashMap<LinkedMessage, LinkedFetch>;

/// 画面の近くにある本文 (`contents`) の、取得が要るリンク先。一覧にあるチャンネルのメッセージのうち、
/// 開いているチャンネル（同期した行を使う）以外のものを、重複なく返す。
pub fn wanted<'a>(
    store: &Store,
    open: Option<&str>,
    contents: impl IntoIterator<Item = &'a str>,
    app_base_url: &str,
) -> Vec<LinkedMessage> {
    let mut result: Vec<LinkedMessage> = contents
        .into_iter()
        .flat_map(|content| {
            let mut found = links::embedded_links(content, app_base_url);
            found.extend(links::bare_links(content, app_base_url));
            found
        })
        .filter_map(|(_, target)| {
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

/// リンク先の状態。`recorded` は端末がそのチャンネルを暗号化チャンネルと記録しているかで、
/// 記録を読めなければ `None`（平文の本文を出さない）。
pub fn state<'a>(
    store: &'a Store,
    fetched: &Fetched,
    open: Option<&str>,
    recorded: Option<bool>,
    target: &LinkTarget,
) -> LinkState<'a> {
    let channel_id = target.channel_id.as_str();
    let channel = store.channel(channel_id);
    let open_here = open == Some(channel_id);
    let message_id = target.linked_message_id();
    let fetch = match message_id {
        // チャンネルだけのリンクは、メッセージを取らない。
        None => LinkedFetch::Missing,
        Some(id) if open_here => match store.message(channel_id, id) {
            Some(message) => LinkedFetch::Found(Box::new(message.clone())),
            None if store.is_synced(&ShapeId::Messages(channel_id.to_owned())) => {
                LinkedFetch::Missing
            }
            None => LinkedFetch::Loading { stalled: false },
        },
        Some(id) => fetched
            .get(&LinkedMessage {
                channel_id: channel_id.to_owned(),
                message_id: id.to_owned(),
            })
            .cloned()
            .unwrap_or(LinkedFetch::Loading { stalled: false }),
    };
    // 暗号文の行は、対象の行と、読み込んでいる同じチャンネルの行を見る。チャンネルだけのリンクは見ない。
    let encrypted_row = |message: &Message| message.ciphertext.is_some();
    let has_ciphertext = message_id.is_some()
        && ((open_here && store.visible_messages(channel_id).any(encrypted_row))
            || fetched.iter().any(|(key, fetch)| {
                key.channel_id == channel_id
                    && matches!(fetch, LinkedFetch::Found(message) if encrypted_row(message))
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
            // 4 件目より後でも、URL の文字そのものを書いたリンクは短くするので取る。
            "https://example.com/1 https://example.com/2 https://example.com/3 \
             https://zli.chat/channels/there?thread=t [x](https://zli.chat/channels/there?message=z)",
            "https://zli.chat/channels/there?message=a",
        ];
        assert_eq!(
            wanted(&store, Some("here"), contents, APP),
            [key("there", "a"), key("there", "t")]
        );
    }

    #[test]
    fn the_open_channel_uses_synced_rows() {
        let store = store();
        let fetched = Fetched::new();
        let look = |id| {
            name(state(
                &store,
                &fetched,
                Some("here"),
                Some(false),
                &target("here", Some(id)),
            ))
        };
        assert_eq!(look("m1"), "text");
        assert_eq!(look("gone"), "unavailable");
    }

    #[test]
    fn other_channels_use_fetched_results() {
        let store = store();
        let mut fetched = Fetched::new();
        let linked = target("there", Some("a"));
        let look = |fetched: &Fetched, recorded| {
            name(state(&store, fetched, Some("here"), recorded, &linked))
        };
        assert_eq!(look(&fetched, Some(false)), "loading");
        fetched.insert(key("there", "a"), found("a", "there", json!({})));
        assert_eq!(look(&fetched, Some(false)), "text");
        // 端末の記録と、読み込んでいる同じチャンネルの暗号文の行でも、暗号化として扱う。
        assert_eq!(look(&fetched, Some(true)), "encrypted");
        assert_eq!(look(&fetched, None), "failed");
        fetched.insert(
            key("there", "b"),
            found("b", "there", json!({ "ciphertext": "{}" })),
        );
        assert_eq!(look(&fetched, Some(false)), "encrypted");
        let channel_only = target("there", None);
        assert_eq!(
            name(state(&store, &fetched, None, Some(false), &channel_only)),
            "channel"
        );
        assert_eq!(
            name(state(&store, &fetched, None, Some(true), &channel_only)),
            "encrypted channel"
        );
        let hidden = target("hidden", Some("a"));
        assert_eq!(
            name(state(&store, &fetched, None, Some(false), &hidden)),
            "unavailable"
        );
    }
}
