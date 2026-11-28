//! 本文のアプリ内リンクの、リンク先の取得と表示（docs/mfm.md の「チャンネル・メッセージのリンクの
//! 表示」、docs/request-design.md の「メッセージリンクの埋め込み」）。

use std::{
    cell::RefCell,
    cmp::Reverse,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use tokio::{sync::mpsc, task::JoinHandle};
use zoo2_client_core::{
    electric::LinkedEvent,
    links::{self, LinkState, LinkTarget, LinkedFetch, MessagePreview, PreviewBody},
    store::Store,
    text::sanitize_display,
};

use super::timeline::{DIM, local};
use crate::electric::rows::{Channel, FromRow, Message};

/// 取得を始めてから、これだけたっても終わらないときは、読み込めなかったものとする。
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// 画面の近くからリンクがなくなって、これだけたったら取得と購読をやめる。
pub const RELEASE_DELAY: Duration = Duration::from_secs(30);
/// 取得できた結果を覚える件数。
const CACHE_SIZE: usize = 256;
/// 同時に購読するチャンネルの数の上限。backend のアカウントごとのシェイプの上限は、ほかの画面や
/// 端末と分け合うので、少なくする。
pub const MAX_CHANNELS: usize = 8;

/// リンク先のメッセージ (チャンネル ID, メッセージ ID)。
pub type Key = (String, String);

struct Fetching {
    requests: mpsc::UnboundedSender<Vec<String>>,
    task: JoinHandle<()>,
    /// この購読で追うメッセージの ID と、最後に画面の近くにあった時刻。
    wanted: HashMap<String, Instant>,
    /// タスクに最後に伝えた並び。
    sent: Vec<String>,
    last_wanted: Instant,
    /// 購読ごとの番号。閉じた購読から遅れて届いたものを捨てる。
    generation: u64,
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
    /// 購読するチャンネルの上限を超えて、取得していないリンク先。
    over_limit: HashSet<Key>,
    /// 直近に始めた購読の番号。
    generation: u64,
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
            None if self.failed.contains(key) || self.over_limit.contains(key) => {
                LinkedFetch::Failed
            }
            None => LinkedFetch::Loading {
                stalled: self
                    .started
                    .get(key)
                    .is_some_and(|at| now.duration_since(*at) >= LOAD_TIMEOUT),
            },
        }
    }

    /// いまの購読の番号。
    #[cfg(test)]
    pub fn generation(&self, channel_id: &str) -> Option<u64> {
        self.channels
            .get(channel_id)
            .map(|fetching| fetching.generation)
    }

    /// [`Self::visible`] のリンク先を取得し、しばらく使っていない取得と購読をやめる。`spawn` は
    /// チャンネルの取得のタスクを、購読の番号を付けて起動する。
    pub fn sync(
        &mut self,
        now: Instant,
        mut spawn: impl FnMut(String, u64, mpsc::UnboundedReceiver<Vec<String>>) -> JoinHandle<()>,
    ) {
        self.over_limit.clear();
        for (channel_id, _) in &self.visible {
            if let Some(fetching) = self.channels.get_mut(channel_id) {
                fetching.last_wanted = now;
            }
        }
        for key in self.visible.clone() {
            let (channel_id, message_id) = &key;
            if !self.channels.contains_key(channel_id)
                && self.running() >= MAX_CHANNELS
                && !self.release_idle()
            {
                self.over_limit.insert(key.clone());
                continue;
            }
            let generation = &mut self.generation;
            let fetching = self.channels.entry(channel_id.clone()).or_insert_with(|| {
                *generation += 1;
                let (requests, receiver) = mpsc::unbounded_channel();
                Fetching {
                    task: spawn(channel_id.clone(), *generation, receiver),
                    requests,
                    wanted: HashMap::new(),
                    sent: Vec::new(),
                    last_wanted: now,
                    generation: *generation,
                }
            });
            fetching.last_wanted = now;
            if fetching.wanted.insert(message_id.clone(), now).is_none()
                && !self.results.contains_key(&key)
            {
                self.started.entry(key.clone()).or_insert(now);
            }
        }
        let expired: Vec<String> = self
            .channels
            .iter()
            .filter(|(_, fetching)| now.duration_since(fetching.last_wanted) >= RELEASE_DELAY)
            .map(|(id, _)| id.clone())
            .collect();
        for channel_id in expired {
            self.release(&channel_id);
        }
        self.request(now);
        self.evict();
    }

    /// 画面の近くから外れて [`RELEASE_DELAY`] たったリンク先は追うのをやめ、追う ID が変わった購読に
    /// 伝える。取り直しで画面の近くのものが先になるように並べる。
    fn request(&mut self, now: Instant) {
        let Self {
            channels,
            visible,
            results,
            failed,
            started,
            ..
        } = self;
        for (channel_id, fetching) in channels {
            let mut seen = HashSet::new();
            let mut order: Vec<String> = visible
                .iter()
                .filter(|(channel, id)| channel == channel_id && seen.insert(id))
                .map(|(_, id)| id.clone())
                .collect();
            fetching.wanted.retain(|id, at| {
                let keep = now.duration_since(*at) < RELEASE_DELAY;
                if !keep {
                    let key = (channel_id.clone(), id.clone());
                    started.remove(&key);
                    failed.remove(&key);
                }
                keep
            });
            // 画面の近くにないものは、見ていた時刻が新しい順に後ろに置く。
            let mut away: Vec<(&String, &Instant)> = fetching
                .wanted
                .iter()
                .filter(|(id, _)| !seen.contains(id))
                .collect();
            away.sort_by_key(|(id, at)| (Reverse(**at), *id));
            order.extend(away.into_iter().map(|(id, _)| id.clone()));
            if order != fetching.sent {
                // 止まった購読には届かない。下で失敗にする。
                let _ = fetching.requests.send(order.clone());
                fetching.sent = order;
            }
            // 権限の失敗などで止まった購読は、閉じるまで作り直さない。
            if fetching.requests.is_closed() {
                for id in fetching.wanted.keys() {
                    let key = (channel_id.clone(), id.clone());
                    if !results.contains_key(&key) {
                        failed.insert(key);
                    }
                }
            }
        }
    }

    /// チャンネルの購読を閉じる。取得できた結果は残し、戻ったときに先に出す。
    pub fn release(&mut self, channel_id: &str) {
        if let Some(fetching) = self.channels.remove(channel_id) {
            fetching.task.abort();
        }
        // 失敗は覚えず、次に表示したときに取り直す。
        self.failed.retain(|(channel, _)| channel != channel_id);
        self.started.retain(|(channel, _), _| channel != channel_id);
    }

    /// このチャンネルで取得できた結果を捨てる。捨てたリンク先を返す。
    pub fn forget(&mut self, channel_id: &str) -> Vec<Key> {
        let keys: Vec<Key> = self
            .results
            .keys()
            .filter(|(channel, _)| channel == channel_id)
            .cloned()
            .collect();
        for key in &keys {
            self.results.remove(key);
        }
        self.order.retain(|(channel, _)| channel != channel_id);
        keys
    }

    /// このチャンネルの取得を拒否された。権限を失ったかもしれないので、前に取得できた内容も出さず、
    /// 取り直せるまで読み込めなかったものにする。
    pub fn reject(&mut self, channel_id: &str) {
        for key in self.forget(channel_id) {
            self.failed.insert(key);
        }
    }

    /// 動いている購読の数。権限の失敗などで止まった購読は、取り直さないよう残すが、数えない。
    fn running(&self) -> usize {
        self.channels
            .values()
            .filter(|fetching| !fetching.requests.is_closed())
            .count()
    }

    /// 画面の近くにリンクがないチャンネルのうち、いちばん前に使った動いている購読を閉じる。なければ `false`。
    fn release_idle(&mut self) -> bool {
        let idle = self
            .channels
            .iter()
            .filter(|(id, fetching)| {
                !fetching.requests.is_closed()
                    && !self.visible.iter().any(|(channel, _)| channel == *id)
            })
            .min_by_key(|(_, fetching)| fetching.last_wanted)
            .map(|(id, _)| id.clone());
        match idle {
            Some(id) => {
                self.release(&id);
                true
            }
            None => false,
        }
    }

    /// 覚える件数を超えた分を古い順に捨てる。購読で追っているリンク先の結果は残す。取得のタスクは
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
            let tracked = channels
                .get(&key.0)
                .is_some_and(|fetching| fetching.wanted.contains_key(&key.1));
            let drop = excess > 0 && !tracked;
            if drop {
                excess -= 1;
                results.remove(key);
            }
            !drop
        });
    }

    /// 番号が `generation` の購読から届いたものを反映する。
    pub fn apply(&mut self, generation: u64, event: LinkedEvent) {
        // 閉じた購読から遅れて届いたものは捨てる。止まった購読は残しているので、その失敗は反映する。
        if self
            .channels
            .get(event.channel_id())
            .is_none_or(|fetching| fetching.generation != generation)
        {
            return;
        }
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
            // 暗号化の扱いは、呼ぶ側がチャンネルごとに覚える。
            LinkedEvent::Ciphertext { .. } => {}
            LinkedEvent::Failed {
                channel_id,
                message_ids,
            } => {
                self.reject(&channel_id);
                for message_id in message_ids {
                    let key = (channel_id.clone(), message_id);
                    self.started.remove(&key);
                    self.failed.insert(key);
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

/// 1 回の描画の間だけ使う、リンクの表示の覚え書き（次の描画では作り直す）。
#[derive(Default)]
pub struct DrawCache {
    /// チャンネルごとの、読み込んでいる行に暗号文の行があるか。
    pub ciphertext: RefCell<HashMap<String, bool>>,
    /// リンク先ごとの表示。
    pub views: RefCell<HashMap<LinkTarget, LinkView>>,
}

/// 1 つの本文を調べて分かる、アプリ内リンク。
#[derive(Debug, Default)]
pub struct Parsed {
    /// 本文のアプリ内リンクのすべて。
    pub all: Vec<(String, LinkTarget)>,
    /// 投稿の下に埋め込むもの。
    pub embedded: Vec<(String, LinkTarget)>,
    /// 取得するもの。
    pub targets: Vec<LinkTarget>,
}

/// 本文ごとの [`Parsed`]。タイムラインは描画のたびに読み込んだ全部の投稿を組み立てるので、描画を
/// またいで覚え、同じ本文を調べ直さない。接続先は変わらないものとする。
///
/// 直前の描画で使った本文だけを次の描画へ引き継ぐので、覚える量は読み込んでいる本文の分で収まり、
/// 履歴がどれだけ長くても、調べるのは新しく出た本文だけになる。
#[derive(Default)]
pub struct ParsedLinks {
    /// いまの描画の番号。
    frame: u64,
    /// 本文ごとの、最後に使った描画の番号と結果。
    entries: RefCell<HashMap<String, (u64, Arc<Parsed>)>>,
}

impl ParsedLinks {
    /// 描画を始めるときに呼び、直前の描画で使わなかった本文を捨てる。直前の描画で本文を 1 つも
    /// 使っていなければ、そのまま残す。
    pub fn next_frame(&mut self) {
        let frame = self.frame;
        let entries = self.entries.get_mut();
        if entries.values().any(|(used, _)| *used == frame) {
            entries.retain(|_, (used, _)| *used == frame);
            self.frame += 1;
        }
    }

    pub fn get(&self, body: &str, app_base_url: &str) -> Arc<Parsed> {
        if let Some((used, parsed)) = self.entries.borrow_mut().get_mut(body) {
            *used = self.frame;
            return parsed.clone();
        }
        let parsed = Arc::new(parse(body, app_base_url));
        self.entries
            .borrow_mut()
            .insert(body.to_owned(), (self.frame, parsed.clone()));
        parsed
    }
}

fn parse(body: &str, app_base_url: &str) -> Parsed {
    let all = links::app_links(body, app_base_url);
    if all.is_empty() {
        return Parsed::default();
    }
    let embedded = links::embedded_links(body, app_base_url);
    Parsed {
        targets: links::fetch_targets(body, app_base_url, &embedded, &all),
        all,
        embedded,
    }
}

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
        format!("#{}", sanitize_display(&channel.channel_name))
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

    /// `ids` のリンクを画面に出して購読を始め、その番号を返す。受け取り側は持っておく。
    fn subscribe(links: &mut Links, now: Instant, ids: &[&str]) -> u64 {
        links.visible = ids.iter().map(|id| key(id)).collect();
        links.sync(now, |_, _, mut receiver| {
            tokio::spawn(async move { while receiver.recv().await.is_some() {} })
        });
        links.generation("c1").unwrap()
    }

    #[tokio::test]
    async fn shares_one_subscription_per_channel_and_releases_it() {
        let mut links = Links::default();
        let start = Instant::now();
        let mut received = Vec::new();
        let mut spawned = 0;
        links.visible = vec![key("m1"), key("m1"), key("m2")];
        links.sync(start, |_, _, receiver| {
            spawned += 1;
            received.push(receiver);
            tokio::spawn(std::future::pending())
        });
        links.sync(start, |_, _, _| {
            unreachable!("同じチャンネルは作り直さない")
        });
        assert_eq!(spawned, 1);
        assert_eq!(received[0].try_recv().unwrap(), ["m1", "m2"]);
        assert!(received[0].try_recv().is_err(), "変わらなければ送らない");
        let generation = links.generation("c1").unwrap();

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
        links.apply(generation, rows("m1", Some("本文")));
        links.apply(generation, rows("m2", None));
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
        assert_eq!(links.fetch(&key("m2"), late), LinkedFetch::Missing);

        // 使わなくなって 30 秒で閉じる。結果は残る。
        links.visible.clear();
        links.sync(start + RELEASE_DELAY, |_, _, _| unreachable!());
        assert!(links.channels.is_empty());
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
    }

    #[tokio::test]
    async fn limits_the_channels_subscribed_at_once() {
        let mut links = Links::default();
        let start = Instant::now();
        // 受信側を持っておく（捨てると、送れずに取得の失敗になる）。
        let mut receivers = Vec::new();
        let mut idle = |_: String, _, receiver| {
            receivers.push(receiver);
            tokio::spawn(std::future::pending())
        };
        let channel = |n: usize| (format!("c{n}"), "m".to_owned());
        links.visible = (0..MAX_CHANNELS + 2).map(channel).collect();
        links.sync(start, &mut idle);
        assert_eq!(links.channels.len(), MAX_CHANNELS);
        // 超えた分は取得せず、読み込めなかったものとする。
        assert_eq!(
            links.fetch(&channel(MAX_CHANNELS), start),
            LinkedFetch::Failed
        );
        assert_ne!(links.fetch(&channel(0), start), LinkedFetch::Failed);
        // 画面の近くから外れた購読があれば、閉じて空ける。
        links.visible = vec![channel(MAX_CHANNELS), channel(1)];
        links.sync(start + Duration::from_secs(1), &mut idle);
        assert_eq!(links.channels.len(), MAX_CHANNELS);
        assert!(links.channels.contains_key("c1"));
        assert!(links.channels.contains_key(&format!("c{MAX_CHANNELS}")));
        assert_ne!(
            links.fetch(&channel(MAX_CHANNELS), start),
            LinkedFetch::Failed
        );
    }

    #[tokio::test]
    async fn stopped_subscriptions_do_not_take_a_slot() {
        let mut links = Links::default();
        let start = Instant::now();
        let mut receivers = Vec::new();
        // c0 の購読は、権限の失敗などで止まる（受け取り側がなくなる）。
        let mut spawn = |channel: String, _, receiver| {
            if channel != "c0" {
                receivers.push(receiver);
            }
            tokio::spawn(std::future::pending())
        };
        let channel = |n: usize| (format!("c{n}"), "m".to_owned());
        links.visible = (0..=MAX_CHANNELS).map(channel).collect();
        links.sync(start, &mut spawn);
        assert_ne!(
            links.fetch(&channel(MAX_CHANNELS), start),
            LinkedFetch::Failed
        );
        // 止まった購読は残し、作り直さない。
        links.sync(start + Duration::from_secs(1), |_, _, _| unreachable!());
        assert_eq!(links.fetch(&channel(0), start), LinkedFetch::Failed);
    }

    #[tokio::test]
    async fn failures_are_forgotten_after_release() {
        let mut links = Links::default();
        let start = Instant::now();
        links.visible = vec![key("m1")];
        links.sync(start, |_, _, receiver| {
            drop(receiver);
            tokio::spawn(async {})
        });
        assert_eq!(links.fetch(&key("m1"), start), LinkedFetch::Failed);
        links.visible.clear();
        links.sync(start + RELEASE_DELAY, |_, _, _| unreachable!());
        assert_eq!(
            links.fetch(&key("m1"), start),
            LinkedFetch::Loading { stalled: false }
        );
    }

    #[tokio::test]
    async fn keeps_results_of_tracked_messages() {
        let mut links = Links::default();
        let start = Instant::now();
        let generation = subscribe(&mut links, start, &["m0"]);
        for index in 0..=CACHE_SIZE {
            links.apply(generation, rows(&format!("m{index}"), Some("本文")));
        }
        // 追っている結果は、上限を超えても捨てない。ほかは古い順に捨てる。
        assert!(matches!(
            links.fetch(&key("m0"), start),
            LinkedFetch::Found(_)
        ));
        assert_eq!(links.results.len(), CACHE_SIZE);
        assert!(!links.results.contains_key(&key("m1")));
        // 追うのをやめたら、上限を超えた分として捨てる。
        subscribe(&mut links, start + RELEASE_DELAY, &["m2"]);
        links.apply(generation, rows("new", Some("本文")));
        assert!(!links.results.contains_key(&key("m0")));
    }

    #[tokio::test]
    async fn messages_away_from_the_screen_are_released() {
        let mut links = Links::default();
        let start = Instant::now();
        let mut received = Vec::new();
        links.visible = vec![key("m1"), key("m2")];
        links.sync(start, |_, _, receiver| {
            received.push(receiver);
            tokio::spawn(std::future::pending())
        });
        let requested = &mut received[0];
        assert_eq!(requested.try_recv().unwrap(), ["m1", "m2"]);
        // 画面の近くにあるものを先にする。外れた直後は追い続ける。
        let soon = start + Duration::from_secs(1);
        links.visible = vec![key("m3"), key("m2")];
        links.sync(soon, |_, _, _| unreachable!());
        assert_eq!(requested.try_recv().unwrap(), ["m3", "m2", "m1"]);
        // 30 秒たったら追うのをやめ、もう一度出たら頼み直して待ち時間も数え直す。
        let later = start + RELEASE_DELAY;
        links.sync(later, |_, _, _| unreachable!());
        assert_eq!(requested.try_recv().unwrap(), ["m3", "m2"]);
        links.visible = vec![key("m1")];
        links.sync(later + Duration::from_secs(1), |_, _, _| unreachable!());
        assert_eq!(requested.try_recv().unwrap(), ["m1", "m2", "m3"]);
        assert_eq!(
            links.fetch(&key("m1"), later + Duration::from_secs(1)),
            LinkedFetch::Loading { stalled: false }
        );
    }

    #[tokio::test]
    async fn a_rejected_channel_hides_what_was_fetched() {
        let mut links = Links::default();
        let start = Instant::now();
        let generation = subscribe(&mut links, start, &["m1", "m2"]);
        links.apply(generation, rows("m1", Some("本文")));
        links.apply(
            generation,
            LinkedEvent::Failed {
                channel_id: "c1".into(),
                message_ids: vec!["m2".into()],
            },
        );
        for id in ["m1", "m2"] {
            assert_eq!(links.fetch(&key(id), start), LinkedFetch::Failed, "{id}");
        }
        // 閉じて取り直し、届くまでは前の内容を出さない。
        links.release("c1");
        let generation = subscribe(&mut links, start, &["m1"]);
        assert_eq!(
            links.fetch(&key("m1"), start),
            LinkedFetch::Loading { stalled: false }
        );
        links.apply(generation, rows("m1", Some("取り直した本文")));
        assert!(matches!(
            links.fetch(&key("m1"), start),
            LinkedFetch::Found(_)
        ));
    }

    #[tokio::test]
    async fn events_of_released_subscriptions_are_ignored() {
        let mut links = Links::default();
        let start = Instant::now();
        let old = subscribe(&mut links, start, &["m1"]);
        links.release("c1");
        let failed = || LinkedEvent::Failed {
            channel_id: "c1".into(),
            message_ids: vec!["m1".into()],
        };
        links.apply(old, failed());
        links.apply(old, rows("m1", Some("古い本文")));
        subscribe(&mut links, start, &["m1"]);
        links.apply(old, failed());
        assert_eq!(
            links.fetch(&key("m1"), start),
            LinkedFetch::Loading { stalled: false }
        );
    }

    #[tokio::test]
    async fn an_unreadable_update_replaces_the_old_result() {
        let mut links = Links::default();
        let generation = subscribe(&mut links, Instant::now(), &["m1"]);
        links.apply(generation, rows("m1", Some("本文")));
        let broken = json!({ "id": "m1" }).as_object().unwrap().clone();
        links.apply(
            generation,
            LinkedEvent::Rows {
                channel_id: "c1".into(),
                rows: vec![("m1".into(), Some(broken))],
            },
        );
        assert_eq!(links.fetch(&key("m1"), Instant::now()), LinkedFetch::Failed);
        assert!(links.order.is_empty());
        links.apply(generation, rows("m1", Some("直った本文")));
        assert!(matches!(
            links.fetch(&key("m1"), Instant::now()),
            LinkedFetch::Found(_)
        ));
    }

    #[test]
    fn parsed_bodies_are_kept_while_draws_use_them() {
        let mut cache = ParsedLinks::default();
        let get = |cache: &ParsedLinks, body: &str| cache.get(body, "https://zli.chat");
        let bodies: Vec<String> = (0..50_000).map(|n| n.to_string()).collect();
        let first: Vec<_> = bodies.iter().map(|body| get(&cache, body)).collect();
        // 毎回同じ本文を使う長い履歴は、件数によらず調べ直さない。
        for _ in 0..2 {
            cache.next_frame();
            for (body, parsed) in bodies.iter().zip(&first) {
                assert!(Arc::ptr_eq(parsed, &get(&cache, body)));
            }
        }
        cache.next_frame();
        get(&cache, "0");
        cache.next_frame();
        // 直前の描画で使わなかった本文は捨てる。
        assert_eq!(cache.entries.borrow().len(), 1);
        // 本文を使わない描画をはさんでも、残した本文は調べ直さない。
        cache.next_frame();
        cache.next_frame();
        assert!(Arc::ptr_eq(&first[0], &get(&cache, "0")));
        assert!(!Arc::ptr_eq(&first[1], &get(&cache, "1")));
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
