//! 本文のアプリ内リンクの、リンク先の取得と表示（docs/mfm.md の「チャンネル・メッセージのリンクの
//! 表示」、docs/request-design.md の「メッセージリンクの埋め込み」）。

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tokio::{sync::mpsc, task::JoinHandle};
use zoo2_client_core::{
    electric::LinkedEvent,
    links::{LinkState, LinkTarget, LinkedFetch, MessagePreview, PreviewBody},
    store::Store,
};

use super::timeline::{DIM, local};
use crate::electric::rows::{Channel, FromRow, Message};

/// 取得を始めてから、これだけたっても終わらないときは、読み込めなかったものとする。
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// 画面の近くからリンクがなくなって、これだけたったら購読を閉じる。
pub const RELEASE_DELAY: Duration = Duration::from_secs(30);
/// 取得できた結果を覚える件数。
const CACHE_SIZE: usize = 256;

/// リンク先のメッセージ (チャンネル ID, メッセージ ID)。
pub type Key = (String, String);

struct Fetching {
    ids: mpsc::UnboundedSender<String>,
    task: JoinHandle<()>,
    sent: HashSet<String>,
    last_wanted: Instant,
}

/// リンク先のメッセージの取得。チャンネルごとに 1 本の購読を、画面の近くのリンクで共有する。
#[derive(Default)]
pub struct Links {
    channels: HashMap<String, Fetching>,
    /// 取得できた結果。その ID のメッセージがなければ `None`。
    results: HashMap<Key, Option<Box<Message>>>,
    order: VecDeque<Key>,
    failed: HashSet<Key>,
    started: HashMap<Key, Instant>,
    /// 直近の描画で画面の近くに出ていた、取得が要るリンク先。
    pub visible: Vec<Key>,
}

impl Drop for Links {
    fn drop(&mut self) {
        for fetching in self.channels.values() {
            fetching.task.abort();
        }
    }
}

impl Links {
    pub fn fetch(&self, key: &Key, now: Instant) -> LinkedFetch {
        match self.results.get(key) {
            Some(Some(message)) => LinkedFetch::Found(message.clone()),
            Some(None) => LinkedFetch::Missing,
            None if self.failed.contains(key) => LinkedFetch::Failed,
            None => LinkedFetch::Loading {
                stalled: self
                    .started
                    .get(key)
                    .is_some_and(|at| now.duration_since(*at) >= LOAD_TIMEOUT),
            },
        }
    }

    /// このチャンネルで取得した行に、暗号文の行があるか。
    pub fn has_ciphertext(&self, channel_id: &str) -> bool {
        self.results.iter().any(|((channel, _), message)| {
            channel == channel_id && message.as_ref().is_some_and(|m| m.ciphertext.is_some())
        })
    }

    /// [`Self::visible`] のリンク先を取得し、しばらく使っていない購読を閉じる。`spawn` はチャンネルの
    /// 取得のタスクを起動する。
    pub fn sync(
        &mut self,
        now: Instant,
        mut spawn: impl FnMut(String, mpsc::UnboundedReceiver<String>) -> JoinHandle<()>,
    ) {
        for key in self.visible.clone() {
            let (channel_id, message_id) = &key;
            let fetching = self.channels.entry(channel_id.clone()).or_insert_with(|| {
                let (ids, receiver) = mpsc::unbounded_channel();
                Fetching {
                    task: spawn(channel_id.clone(), receiver),
                    ids,
                    sent: HashSet::new(),
                    last_wanted: now,
                }
            });
            fetching.last_wanted = now;
            if fetching.sent.insert(message_id.clone()) {
                if !self.results.contains_key(&key) {
                    self.started.entry(key.clone()).or_insert(now);
                }
                // 権限の失敗などで止まった購読は、閉じるまで作り直さない。
                if fetching.ids.send(message_id.clone()).is_err() {
                    self.failed.insert(key.clone());
                }
            }
        }
        let expired: Vec<String> = self
            .channels
            .iter()
            .filter(|(_, fetching)| now.duration_since(fetching.last_wanted) >= RELEASE_DELAY)
            .map(|(id, _)| id.clone())
            .collect();
        for channel_id in expired {
            if let Some(fetching) = self.channels.remove(&channel_id) {
                fetching.task.abort();
            }
            // 失敗は覚えず、次に表示したときに取り直す。取得できた結果は残し、戻ったときに先に出す。
            self.failed.retain(|(channel, _)| *channel != channel_id);
            self.started
                .retain(|(channel, _), _| *channel != channel_id);
        }
        self.evict();
    }

    /// 覚える件数を超えた分を古い順に捨てる。購読中のチャンネルの結果は残す。取得のタスクは
    /// 取った ID をもう一度は取らないので、捨てると読み込み中のままになる。
    fn evict(&mut self) {
        let mut excess = self.order.len().saturating_sub(CACHE_SIZE);
        let Self {
            order,
            results,
            channels,
            ..
        } = self;
        order.retain(|key| {
            let drop = excess > 0 && !channels.contains_key(&key.0);
            if drop {
                excess -= 1;
                results.remove(key);
            }
            !drop
        });
    }

    pub fn apply(&mut self, event: LinkedEvent) {
        match event {
            LinkedEvent::Rows { channel_id, rows } => {
                for (message_id, row) in rows {
                    let key = (channel_id.clone(), message_id);
                    let message = match row.as_ref().map(Message::from_row) {
                        Some(Ok(message)) => Some(Box::new(message)),
                        // 読めない行は、取得できなかったものとする。
                        Some(Err(_)) => {
                            self.started.remove(&key);
                            if self.results.remove(&key).is_some() {
                                self.order.retain(|old| *old != key);
                            }
                            self.failed.insert(key);
                            continue;
                        }
                        None => None,
                    };
                    self.started.remove(&key);
                    self.failed.remove(&key);
                    if self.results.insert(key.clone(), message).is_none() {
                        self.order.push_back(key);
                    }
                }
                self.evict();
            }
            LinkedEvent::Failed {
                channel_id,
                message_ids,
            } => {
                for message_id in message_ids {
                    let key = (channel_id.clone(), message_id);
                    // 取り直しに失敗したときは、前に取得できた結果を残す。
                    if !self.results.contains_key(&key) {
                        self.failed.insert(key);
                    }
                }
            }
        }
    }
}

/// 本文中の短い表示と、投稿の下の埋め込みの内容。
#[derive(Debug, Clone, Default)]
pub struct LinkView {
    /// URL の文字そのものを書いたリンクの代わりに出す短い表示。行き先が分かったときだけ。
    pub inline: Option<String>,
    pub header: Vec<Line<'static>>,
    /// 本文の要約や説明。表示は `body_lines` 行まで。
    pub body: Option<Line<'static>>,
    pub body_lines: usize,
}

/// チャンネルごとの、読み込んでいる行に暗号文の行があるかの結果（1 回の描画の間だけ使う）。
pub type CiphertextRows = RefCell<HashMap<String, bool>>;

/// 1 つの本文のアプリ内リンクの表示と、取得の対象。
#[derive(Debug, Clone, Default)]
pub struct BodyLinks {
    /// URL ごとの短い表示。
    pub inline: HashMap<String, String>,
    /// 投稿の下の埋め込み（本文の URL の順）。
    pub embeds: Vec<LinkView>,
    pub targets: Vec<LinkTarget>,
}

/// チャンネルの呼び名。DM は名前が内部の識別子なので、総称を出す。
pub fn channel_name(channel: &Channel) -> String {
    if channel.is_dm {
        "ダイレクトメッセージ".to_owned()
    } else {
        format!("#{}", channel.channel_name)
    }
}

/// 埋め込みに添える日時。「11月23日 16:30」の形で、今年でなければ年も付ける。
fn timestamp(preview: &MessagePreview) -> String {
    let Some(time) = local(preview.created_at) else {
        return String::new();
    };
    let format = if time.year() == jiff::Zoned::now().year() {
        "%-m月%-d日 %H:%M"
    } else {
        "%Y年%-m月%-d日 %H:%M"
    };
    time.strftime(format).to_string()
}

pub fn view(state: &LinkState<'_>, store: &Store) -> LinkView {
    let note = |text: String| Line::styled(text, DIM);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    match state {
        LinkState::Loading => LinkView {
            header: vec![note("リンク先を読み込み中…".into())],
            ..Default::default()
        },
        LinkState::Failed => LinkView {
            header: vec![note("リンク先を読み込めませんでした".into())],
            ..Default::default()
        },
        // 理由は区別せず、チャンネル名と本文を出さない。
        LinkState::Unavailable { message } => LinkView {
            header: vec![note(if *message {
                "メッセージを表示できません".into()
            } else {
                "チャンネルを表示できません".into()
            })],
            ..Default::default()
        },
        LinkState::Channel { channel, encrypted } => {
            let name = channel_name(channel);
            let mut header = vec![Line::styled(name.clone(), bold)];
            if *encrypted {
                header.push(note("🔒 暗号化チャンネル".into()));
            } else if channel.is_private && !channel.is_dm {
                header.push(note("🔒 非公開チャンネル".into()));
            }
            LinkView {
                inline: Some(name),
                header,
                body: channel
                    .description
                    .as_deref()
                    .map(zoo2_client_core::links::excerpt)
                    .filter(|text| !text.is_empty())
                    .map(note),
                body_lines: 2,
            }
        }
        LinkState::Message { channel, preview } => {
            let author = store.display_name(&preview.author_id);
            let name = channel_name(channel);
            let place = if preview.parent_id.is_some() {
                format!("{name} 内のスレッド")
            } else {
                name.clone()
            };
            let body = match &preview.body {
                PreviewBody::Text(text) => Some(Line::from(text.clone())),
                PreviewBody::Encrypted => {
                    Some(note("🔒 このメッセージは暗号化されています".into()))
                }
                PreviewBody::App => Some(note("Appのメッセージ".into())),
                PreviewBody::Forwarded => Some(note("メッセージを転送しました".into())),
                PreviewBody::Attachments(count) => {
                    Some(note(format!("📎 添付ファイル {count} 件")))
                }
                PreviewBody::Empty => None,
            };
            LinkView {
                inline: Some(format!("💬 {author}さん: {name}")),
                header: vec![
                    Line::from(vec![
                        Span::styled(author, bold),
                        Span::raw("  "),
                        Span::styled(timestamp(preview), DIM),
                    ]),
                    note(place),
                ],
                body,
                body_lines: 3,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key(id: &str) -> Key {
        ("c1".into(), id.into())
    }

    fn rows(id: &str, content: Option<&str>) -> LinkedEvent {
        let row = content.map(|content| {
            json!({ "id": id, "user_id": "u1", "channel_id": "c1", "content": content,
                    "created_at": "2026-11-01 00:00:00" })
            .as_object()
            .unwrap()
            .clone()
        });
        LinkedEvent::Rows {
            channel_id: "c1".into(),
            rows: vec![(id.into(), row)],
        }
    }

    #[tokio::test]
    async fn shares_one_subscription_per_channel_and_releases_it() {
        let mut links = Links::default();
        let start = Instant::now();
        let mut received = Vec::new();
        let mut spawned = 0;
        links.visible = vec![key("m1"), key("m1"), key("m2")];
        links.sync(start, |_, receiver| {
            spawned += 1;
            received.push(receiver);
            tokio::spawn(std::future::pending())
        });
        links.sync(start, |_, _| unreachable!("同じチャンネルは作り直さない"));
        assert_eq!(spawned, 1);
        let mut sent = Vec::new();
        while let Ok(id) = received[0].try_recv() {
            sent.push(id);
        }
        assert_eq!(sent, ["m1", "m2"]);

        // 10 秒たっても届かなければ読み込めなかったものとし、届けば内容に変わる。
        assert_eq!(
            links.fetch(&key("m1"), start),
            LinkedFetch::Loading { stalled: false }
        );
        let late = start + LOAD_TIMEOUT;
        assert_eq!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Loading { stalled: true }
        );
        links.apply(rows("m1", Some("本文")));
        links.apply(rows("m2", None));
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
        assert_eq!(links.fetch(&key("m2"), late), LinkedFetch::Missing);

        // 使わなくなって 30 秒で閉じる。結果は残る。
        links.visible.clear();
        links.sync(start + RELEASE_DELAY, |_, _| unreachable!());
        assert!(links.channels.is_empty());
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
    }

    #[tokio::test]
    async fn failures_are_forgotten_after_release() {
        let mut links = Links::default();
        let start = Instant::now();
        links.visible = vec![key("m1")];
        links.sync(start, |_, receiver| {
            drop(receiver);
            tokio::spawn(async {})
        });
        assert_eq!(links.fetch(&key("m1"), start), LinkedFetch::Failed);
        links.visible.clear();
        links.sync(start + RELEASE_DELAY, |_, _| unreachable!());
        assert_eq!(
            links.fetch(&key("m1"), start),
            LinkedFetch::Loading { stalled: false }
        );
    }

    #[tokio::test]
    async fn keeps_results_of_subscribed_channels() {
        let mut links = Links::default();
        let start = Instant::now();
        links.visible = vec![key("m0")];
        links.sync(start, |_, _| tokio::spawn(std::future::pending()));
        for index in 0..=CACHE_SIZE {
            links.apply(rows(&format!("m{index}"), Some("本文")));
        }
        // 購読中は、上限を超えても捨てない。
        assert!(matches!(
            links.fetch(&key("m0"), start),
            LinkedFetch::Found(_)
        ));
        // 購読を閉じたら、その場で上限まで捨てる。
        links.visible.clear();
        links.sync(start + RELEASE_DELAY, |_, _| unreachable!());
        assert_eq!(links.results.len(), CACHE_SIZE);
        assert!(!links.results.contains_key(&key("m0")));
    }

    #[test]
    fn an_unreadable_update_replaces_the_old_result() {
        let mut links = Links::default();
        links.apply(rows("m1", Some("本文")));
        let broken = json!({ "id": "m1" }).as_object().unwrap().clone();
        links.apply(LinkedEvent::Rows {
            channel_id: "c1".into(),
            rows: vec![("m1".into(), Some(broken))],
        });
        assert_eq!(links.fetch(&key("m1"), Instant::now()), LinkedFetch::Failed);
        assert!(links.order.is_empty());
        links.apply(rows("m1", Some("直った本文")));
        assert!(matches!(
            links.fetch(&key("m1"), Instant::now()),
            LinkedFetch::Found(_)
        ));
    }

    #[test]
    fn views_do_not_reveal_unavailable_destinations() {
        let store = Store::default();
        let view = view(&LinkState::Unavailable { message: true }, &store);
        assert_eq!(view.inline, None);
        assert_eq!(view.header[0].to_string(), "メッセージを表示できません");
        assert!(view.body.is_none());
    }
}
