//! 本文のリンクが指すメッセージの、購読の共有と取得の結果。TUI とデスクトップ版で共有する。
//! 取得そのものは [`super::spawn_linked`] が行う。

use std::{
    cmp::Reverse,
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use tokio::{sync::mpsc, task::JoinHandle};
use zoo2_client_types::{
    links::LinkedFetch,
    rows::{FromRow, Message},
};

pub use zoo2_client_types::links::LOAD_TIMEOUT;

use super::LinkedEvent;

/// 画面の近くからリンクがなくなって、これだけたったら取得と購読をやめる。
pub const RELEASE_DELAY: Duration = Duration::from_secs(30);
/// 取得できた結果を覚える件数。
const CACHE_SIZE: usize = 256;
/// 同時に購読するチャンネルの数の上限。backend のアカウントごとのシェイプの上限は、ほかの画面や
/// 端末と分け合うので、少なくする。
pub const MAX_CHANNELS: usize = 8;

/// リンク先のメッセージ (チャンネル ID, メッセージ ID)。
pub type LinkedKey = (String, String);

/// [`LinkedCache::report`] で返すもの。
#[derive(Debug, Clone, PartialEq)]
pub struct LinkReport {
    /// 状態が変わったリンク先。分かっていたことがなくなったものは `None`。
    pub linked: Vec<(LinkedKey, Option<LinkedState>)>,
    /// 暗号文の行が届いたことのあるチャンネルのすべて。
    pub ciphertext: Vec<String>,
}

/// [`LinkedCache::report`] で返す、リンク先の状態。
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedState {
    pub fetch: LinkedFetch,
    /// この結果が、いまの [`LinkedCache::check`] のあとに始めた取得で届いたものなら、その確認の番号。
    pub checked: Option<u64>,
}

/// 移動先の確認。
struct Check {
    id: u64,
    /// 確認を始めたときの、直近の購読の番号。これより後の購読は、確認のあとに取得を始めている。
    since: u64,
    /// そうした購読で届いたリンク先。
    arrived: HashSet<LinkedKey>,
}

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
    /// 権限の失敗などで取得が止まった。
    stopped: bool,
}

impl Fetching {
    fn is_running(&self) -> bool {
        !self.stopped && !self.requests.is_closed()
    }
}

/// リンク先のメッセージの取得。チャンネルごとに 1 本の購読を、画面の近くのリンクで共有する。
#[derive(Default)]
pub struct LinkedCache {
    channels: HashMap<String, Fetching>,
    /// 取得できた結果。その ID のメッセージがなければ `None`。
    results: HashMap<LinkedKey, Option<Box<Message>>>,
    order: VecDeque<LinkedKey>,
    failed: HashSet<LinkedKey>,
    started: HashMap<LinkedKey, Instant>,
    /// 購読するチャンネルの上限を超えて、取得していないリンク先。
    over_limit: HashSet<LinkedKey>,
    /// 直近に始めた購読の番号。
    generation: u64,
    /// 画面の近くに出ている、取得が要るリンク先。
    pub visible: Vec<LinkedKey>,
    /// 暗号文の行が届いたことのあるチャンネル。行を捨てたあとも忘れない。
    ciphertext: BTreeSet<String>,
    /// [`Self::report`] で返した状態。
    reported: HashMap<LinkedKey, LinkedState>,
    check: Option<Check>,
    /// 前の [`Self::report`] のあとで、[`Self::ciphertext`] が増えたか。
    ciphertext_changed: bool,
    /// 次の [`Self::report`] で、画面の近くのリンク先は変わっていなくても返す。
    resend: bool,
}

impl Drop for LinkedCache {
    fn drop(&mut self) {
        for fetching in self.channels.values() {
            fetching.task.abort();
        }
    }
}

impl LinkedCache {
    pub fn fetch(&self, key: &LinkedKey, now: Instant) -> LinkedFetch {
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

    /// いまの購読の番号。購読していなければ `None`。
    pub fn generation(&self, channel_id: &str) -> Option<u64> {
        self.channels
            .get(channel_id)
            .map(|fetching| fetching.generation)
    }

    /// このチャンネルで暗号文の行が届いたことを覚える。
    pub fn saw_ciphertext(&mut self, channel_id: &str) {
        if !self.ciphertext.contains(channel_id) {
            self.ciphertext.insert(channel_id.to_owned());
            self.ciphertext_changed = true;
        }
    }

    /// このチャンネルで、暗号文の行が届いたことがあるか。
    pub fn has_ciphertext(&self, channel_id: &str) -> bool {
        self.ciphertext.contains(channel_id)
    }

    /// 前に返してから変わった、画面の近くのリンク先と返したことのあるリンク先の状態。取得を別の
    /// プロセスの画面に伝えるときに使う。何も分かっていないリンク先は、分かっていたことがなくなった
    /// ときだけ返すので、読み込み中は伝えない。変わったことがなければ `None`。
    pub fn report(&mut self, now: Instant) -> Option<LinkReport> {
        let keys: BTreeSet<LinkedKey> = self
            .reported
            .keys()
            .chain(self.visible.iter())
            .cloned()
            .collect();
        let again: HashSet<LinkedKey> = if std::mem::take(&mut self.resend) {
            self.visible.iter().cloned().collect()
        } else {
            HashSet::new()
        };
        let mut linked = Vec::new();
        for key in keys {
            let again = again.contains(&key);
            let checked = self
                .check
                .as_ref()
                .filter(|check| check.arrived.contains(&key))
                .map(|check| check.id);
            // 変わっていない行は、写さずに比べる。
            if !again
                && let (Some(Some(message)), Some(sent)) =
                    (self.results.get(&key), self.reported.get(&key))
                && sent.checked == checked
                && matches!(&sent.fetch, LinkedFetch::Found(sent) if sent == message)
            {
                continue;
            }
            let fetch = self.fetch(&key, now);
            if fetch == (LinkedFetch::Loading { stalled: false }) {
                if self.reported.remove(&key).is_some() {
                    linked.push((key, None));
                }
                continue;
            }
            let state = LinkedState { fetch, checked };
            if again || self.reported.get(&key) != Some(&state) {
                self.reported.insert(key.clone(), state.clone());
                linked.push((key, Some(state)));
            }
        }
        if linked.is_empty() && !self.ciphertext_changed {
            return None;
        }
        self.ciphertext_changed = false;
        Some(LinkReport {
            linked,
            ciphertext: self.ciphertext.iter().cloned().collect(),
        })
    }

    /// 画面を読み込み直したかもしれないので、画面の近くのリンク先の分かっていることと、暗号文の
    /// チャンネルを、次の [`Self::report`] でもう一度返す。
    pub fn report_again(&mut self) {
        self.resend = true;
        self.ciphertext_changed = !self.ciphertext.is_empty();
    }

    /// 番号が `id` の確認として、移動先の `keys` を取り直させる。覚えている結果や、確認の前に始めた
    /// 取得の応答を使わないよう、確認より前からある購読は閉じる（止まった購読も作り直される）。覚えて
    /// いる結果は残す。このあとに [`Self::watch`] して、新しい購読で取得を始める。作り直した購読で
    /// 届いた結果に [`LinkedState::checked`] を付ける。
    pub fn check(&mut self, id: u64, keys: &[LinkedKey]) {
        if self.check.as_ref().is_none_or(|check| check.id != id) {
            self.check = Some(Check {
                id,
                since: self.generation,
                arrived: HashSet::new(),
            });
        }
        let since = self.check.as_ref().map_or(0, |check| check.since);
        for (channel_id, _) in keys {
            if self
                .generation(channel_id)
                .is_some_and(|generation| generation <= since)
            {
                self.release(channel_id);
            }
        }
    }

    /// 移動先の確認をやめる。
    pub fn end_check(&mut self) {
        self.check = None;
    }

    /// 画面の近くのリンク先を入れ替えて [`Self::sync`] する。画面が変わったときだけ呼ぶ側のためのもので、
    /// 入れ替える前のリンク先は、いままで使っていたものとして扱う。
    pub fn watch(
        &mut self,
        now: Instant,
        visible: Vec<LinkedKey>,
        spawn: impl FnMut(String, u64, mpsc::UnboundedReceiver<Vec<String>>) -> JoinHandle<()>,
    ) {
        for (channel_id, message_id) in &self.visible {
            if let Some(fetching) = self.channels.get_mut(channel_id) {
                fetching.last_wanted = now;
                if let Some(seen) = fetching.wanted.get_mut(message_id) {
                    *seen = now;
                }
            }
        }
        self.visible = visible;
        self.sync(now, spawn);
    }

    /// 次に状態が変わる時刻（取得の時間切れ、取得をやめる時刻、購読を閉じる時刻）。そのときに
    /// [`Self::sync`] する。
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let timeouts = self
            .visible
            .iter()
            .filter(|key| !self.results.contains_key(*key) && !self.failed.contains(*key))
            .filter_map(|key| self.started.get(key))
            .map(|at| *at + LOAD_TIMEOUT)
            .filter(|at| *at > now);
        let visible: HashSet<(&str, &str)> = self
            .visible
            .iter()
            .map(|(channel, id)| (channel.as_str(), id.as_str()))
            .collect();
        let releases = self.channels.iter().flat_map(|(channel_id, fetching)| {
            let idle = !self
                .visible
                .iter()
                .any(|(channel, _)| channel == channel_id);
            let channel = idle.then(|| fetching.last_wanted + RELEASE_DELAY);
            let visible = &visible;
            fetching
                .wanted
                .iter()
                .filter(move |(id, _)| !visible.contains(&(channel_id.as_str(), id.as_str())))
                .map(|(_, seen)| *seen + RELEASE_DELAY)
                .chain(channel)
        });
        timeouts.chain(releases).min()
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
                    stopped: false,
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
            if !fetching.is_running() {
                for id in fetching.wanted.keys() {
                    let key = (channel_id.clone(), id.clone());
                    if !results.contains_key(&key) {
                        failed.insert(key);
                    }
                }
            }
        }
    }

    /// このチャンネルのリンク先を追うのをやめ、購読を閉じる。チャンネルを開いて、本同期が同じ行を
    /// 持つときに使う。
    pub fn close(&mut self, channel_id: &str) {
        self.release(channel_id);
        self.visible.retain(|(channel, _)| channel != channel_id);
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
    pub fn forget(&mut self, channel_id: &str) -> Vec<LinkedKey> {
        let keys: Vec<LinkedKey> = self
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

    /// 上限で待っているリンク先があり、取得を始める空きができたか。そのときは [`Self::sync`] する。
    pub fn can_start_waiting(&self) -> bool {
        !self.over_limit.is_empty() && self.running() < MAX_CHANNELS
    }

    /// 動いている購読の数。権限の失敗などで止まった購読は、取り直さないよう残すが、数えない。
    fn running(&self) -> usize {
        self.channels
            .values()
            .filter(|fetching| fetching.is_running())
            .count()
    }

    /// 画面の近くにリンクがないチャンネルのうち、いちばん前に使った動いている購読を閉じる。なければ `false`。
    fn release_idle(&mut self) -> bool {
        let idle = self
            .channels
            .iter()
            .filter(|(id, fetching)| {
                fetching.is_running() && !self.visible.iter().any(|(channel, _)| channel == *id)
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
        // 暗号文が届いたことは、購読を閉じたあとに知っても覚える。
        if let LinkedEvent::Ciphertext { channel_id } = &event {
            self.saw_ciphertext(channel_id);
        }
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
                    if let Some(check) = &mut self.check
                        && generation > check.since
                    {
                        check.arrived.insert(key.clone());
                    }
                    if self.results.insert(key.clone(), message).is_none() {
                        self.order.push_back(key);
                    }
                }
                self.evict();
            }
            LinkedEvent::Ciphertext { .. } => {}
            LinkedEvent::Failed {
                channel_id,
                message_ids,
            } => {
                if let Some(fetching) = self.channels.get_mut(&channel_id) {
                    fetching.stopped = true;
                }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key(id: &str) -> LinkedKey {
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
    fn subscribe(links: &mut LinkedCache, now: Instant, ids: &[&str]) -> u64 {
        links.visible = ids.iter().map(|id| key(id)).collect();
        links.sync(now, |_, _, mut receiver| {
            tokio::spawn(async move { while receiver.recv().await.is_some() {} })
        });
        links.generation("c1").unwrap()
    }

    #[tokio::test]
    async fn shares_one_subscription_per_channel_and_releases_it() {
        let mut links = LinkedCache::default();
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
        assert_eq!(links.next_deadline(start), Some(start + LOAD_TIMEOUT));
        let late = start + LOAD_TIMEOUT;
        assert_eq!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Loading { stalled: true }
        );
        // 時間切れを知らせたあとは、同じ時刻を返し続けない。
        assert_eq!(links.next_deadline(late), None);
        links.apply(generation, rows("m1", Some("本文")));
        links.apply(generation, rows("m2", None));
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
        assert_eq!(links.fetch(&key("m2"), late), LinkedFetch::Missing);

        // 使わなくなって 30 秒で閉じる。結果は残る。
        links.visible.clear();
        assert_eq!(links.next_deadline(late), Some(start + RELEASE_DELAY));
        links.sync(start + RELEASE_DELAY, |_, _, _| unreachable!());
        assert!(links.channels.is_empty());
        assert_eq!(links.next_deadline(start + RELEASE_DELAY), None);
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
    }

    #[tokio::test]
    async fn limits_the_channels_subscribed_at_once() {
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
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

    fn changes(report: Option<LinkReport>) -> Vec<(LinkedKey, Option<&'static str>)> {
        report
            .map(|report| report.linked)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, fetch)| {
                let name = fetch.map(|state| match state.fetch {
                    LinkedFetch::Found(_) => "found",
                    LinkedFetch::Missing => "missing",
                    LinkedFetch::Failed => "failed",
                    LinkedFetch::Loading { stalled: true } => "stalled",
                    LinkedFetch::Loading { stalled: false } => "loading",
                });
                (key, name)
            })
            .collect()
    }

    #[tokio::test]
    async fn reports_every_change_of_the_states_once() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        // 受信側を持っておく（捨てると、送れずに取得の失敗になる）。
        let mut receivers = Vec::new();
        links.watch(start, vec![key("m1"), key("m2")], |_, _, receiver| {
            receivers.push(receiver);
            idle()
        });
        assert_eq!(links.report(start), None);
        let generation = links.generation("c1").unwrap();
        links.apply(generation, rows("m2", None));
        // ほかの結果と重なっても、時間切れになったリンクを返す。
        assert_eq!(
            changes(links.report(start + LOAD_TIMEOUT)),
            [(key("m1"), Some("stalled")), (key("m2"), Some("missing")),]
        );
        assert_eq!(links.report(start + LOAD_TIMEOUT), None);
        links.apply(generation, rows("m1", Some("本文")));
        assert_eq!(changes(links.report(start)), [(key("m1"), Some("found"))]);
        // 読み込み直した画面には、画面の近くのものをもう一度返す。
        links.report_again();
        assert_eq!(changes(links.report(start)).len(), 2);
        assert_eq!(links.report(start), None);
        // 拒否されたら内容を捨て、購読を閉じたら失敗も忘れる。
        links.apply(
            generation,
            LinkedEvent::Failed {
                channel_id: "c1".into(),
                message_ids: Vec::new(),
            },
        );
        assert_eq!(
            changes(links.report(start)),
            [(key("m1"), Some("failed")), (key("m2"), Some("failed"))]
        );
        links.close("c1");
        assert_eq!(
            changes(links.report(start)),
            [(key("m1"), None), (key("m2"), None)]
        );
        assert!(links.visible.is_empty());
    }

    fn checked(report: Option<LinkReport>) -> Vec<Option<u64>> {
        let linked = report.map(|report| report.linked).unwrap_or_default();
        linked
            .into_iter()
            .filter_map(|(_, state)| state.map(|state| state.checked))
            .collect()
    }

    #[tokio::test]
    async fn a_check_ignores_fetches_started_before_it() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        let before = subscribe(&mut links, start, &["m1"]);
        links.apply(before, rows("m1", Some("本文")));
        assert_eq!(checked(links.report(start)), [None]);
        // 確認より前の購読は閉じる。覚えている内容は残す。
        links.check(1, &[key("m1")]);
        assert_eq!(links.generation("c1"), None);
        let after = subscribe(&mut links, start, &["m1"]);
        assert_ne!(after, before);
        // 押す前に始めた取得の応答は、内容も使わない。
        links.apply(before, rows("m1", Some("古い応答")));
        let LinkedFetch::Found(message) = links.fetch(&key("m1"), start) else {
            panic!("覚えている内容を残す");
        };
        assert_eq!(message.content, "本文");
        assert_eq!(checked(links.report(start)), Vec::<Option<u64>>::new());
        // 内容が同じでも、新しい購読で届いたら伝える。
        links.apply(after, rows("m1", Some("本文")));
        assert_eq!(checked(links.report(start)), [Some(1)]);
        // 同じ確認で親を足しても、作り直した購読は閉じない。
        links.check(1, &[key("m1"), key("p")]);
        assert_eq!(links.generation("c1"), Some(after));
        links.sync(start, |_, _, _| idle());
        // 次の確認では、また作り直す。
        links.check(2, &[key("m1")]);
        assert_eq!(links.generation("c1"), None);
        let next = subscribe(&mut links, start, &["m1"]);
        links.report_again();
        assert_eq!(checked(links.report(start)), [None]);
        links.apply(next, rows("m1", None));
        assert_eq!(checked(links.report(start)), [Some(2)]);
        links.end_check();
        assert_eq!(checked(links.report(start)), [None]);
    }

    #[tokio::test]
    async fn a_check_does_not_trust_results_kept_after_unsubscribing() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        let generation = subscribe(&mut links, start, &["m1"]);
        links.apply(generation, rows("m1", None));
        links.watch(start, Vec::new(), |_, _, _| idle());
        let later = start + RELEASE_DELAY;
        links.sync(later, |_, _, _| idle());
        links.check(1, &[key("m1")]);
        let generation = subscribe(&mut links, later, &["m1"]);
        assert_eq!(links.fetch(&key("m1"), later), LinkedFetch::Missing);
        assert_eq!(checked(links.report(later)), [None]);
        links.apply(generation, rows("m1", Some("復活")));
        assert_eq!(checked(links.report(later)), [Some(1)]);
    }

    #[tokio::test]
    async fn a_check_restarts_a_stopped_subscription() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        let generation = subscribe(&mut links, start, &["m1"]);
        links.apply(
            generation,
            LinkedEvent::Failed {
                channel_id: "c1".into(),
                message_ids: vec!["m1".into()],
            },
        );
        assert_eq!(links.fetch(&key("m1"), start), LinkedFetch::Failed);
        links.check(1, &[key("m1")]);
        let restarted = subscribe(&mut links, start, &["m1"]);
        assert_ne!(restarted, generation);
        assert_eq!(
            links.fetch(&key("m1"), start),
            LinkedFetch::Loading { stalled: false }
        );
    }

    #[tokio::test]
    async fn waiting_links_start_when_a_subscription_stops() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        links.visible = (0..=MAX_CHANNELS)
            .map(|n| (format!("c{n}"), "m".to_owned()))
            .collect();
        let mut receivers = Vec::new();
        let mut spawn = |_: String, _, receiver| {
            receivers.push(receiver);
            idle()
        };
        links.sync(start, &mut spawn);
        let over = (format!("c{MAX_CHANNELS}"), "m".to_owned());
        assert_eq!(links.fetch(&over, start), LinkedFetch::Failed);
        assert!(!links.can_start_waiting());
        // 受け取り側がまだ残っていても、止まったと分かったら空きにする。
        let generation = links.generation("c0").unwrap();
        links.apply(
            generation,
            LinkedEvent::Failed {
                channel_id: "c0".into(),
                message_ids: vec!["m".into()],
            },
        );
        assert!(links.can_start_waiting());
        links.sync(start, &mut spawn);
        assert!(!links.can_start_waiting());
        assert_eq!(
            links.fetch(&over, start),
            LinkedFetch::Loading { stalled: false }
        );
        let stopped = ("c0".to_owned(), "m".to_owned());
        assert_eq!(links.fetch(&stopped, start), LinkedFetch::Failed);
    }

    #[tokio::test]
    async fn resending_still_reports_what_is_no_longer_known() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        links.visible = (0..=MAX_CHANNELS)
            .map(|n| (format!("c{n}"), "m".to_owned()))
            .collect();
        let mut receivers = Vec::new();
        let mut spawn = |_: String, _, receiver| {
            receivers.push(receiver);
            idle()
        };
        links.sync(start, &mut spawn);
        // 上限を超えて読み込めなかったリンク先。
        let over = (format!("c{MAX_CHANNELS}"), "m".to_owned());
        assert_eq!(
            changes(links.report(start)),
            [(over.clone(), Some("failed"))]
        );
        // 空きができて取得を始めたら、取り直しと重なっても、画面の失敗を取り消す。
        links.watch(start, vec![over.clone()], &mut spawn);
        links.report_again();
        assert_eq!(changes(links.report(start)), [(over, None)]);
    }

    #[tokio::test]
    async fn forgotten_results_are_reported_as_gone() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        let generation = subscribe(&mut links, start, &["m1"]);
        links.apply(generation, rows("m1", Some("本文")));
        links.report(start);
        // 画面から外れて、上限を超えて捨てた結果は、画面にも捨ててもらう。
        links.watch(start, Vec::new(), |_, _, _| unreachable!());
        for index in 0..=CACHE_SIZE {
            links.results.insert(key(&format!("n{index}")), None);
            links.order.push_back(key(&format!("n{index}")));
        }
        links.sync(start + RELEASE_DELAY, |_, _, _| unreachable!());
        assert_eq!(changes(links.report(start)), [(key("m1"), None)]);
    }

    #[test]
    fn ciphertext_is_remembered_and_reported() {
        let mut links = LinkedCache::default();
        let now = Instant::now();
        // 閉じた購読から届いた知らせも覚える。
        links.apply(
            7,
            LinkedEvent::Ciphertext {
                channel_id: "c1".into(),
            },
        );
        assert!(links.has_ciphertext("c1"));
        links.saw_ciphertext("c2");
        let report = links.report(now).unwrap();
        assert_eq!(report.ciphertext, ["c1", "c2"]);
        assert!(report.linked.is_empty());
        links.saw_ciphertext("c1");
        assert_eq!(links.report(now), None);
        links.report_again();
        assert_eq!(links.report(now).unwrap().ciphertext, ["c1", "c2"]);
    }

    #[tokio::test]
    async fn release_counts_from_when_the_link_left_the_screen() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        let mut receivers = Vec::new();
        links.watch(start, vec![key("m1"), key("m2")], |_, _, receiver| {
            receivers.push(receiver);
            idle()
        });
        // 長く表示したあとで外れても、そこから 30 秒は追い続ける。
        let left = start + RELEASE_DELAY * 3;
        links.watch(left, vec![key("m2")], |_, _, _| unreachable!());
        assert_eq!(links.next_deadline(left), Some(left + RELEASE_DELAY));
        links.sync(left + RELEASE_DELAY, |_, _, _| unreachable!());
        assert_eq!(links.channels["c1"].sent, ["m2"]);
        let gone = left + RELEASE_DELAY * 2;
        links.watch(gone, Vec::new(), |_, _, _| unreachable!());
        assert_eq!(links.channels.len(), 1);
        assert_eq!(links.next_deadline(gone), Some(gone + RELEASE_DELAY));
        links.sync(gone + RELEASE_DELAY, |_, _, _| unreachable!());
        assert!(links.channels.is_empty());
    }

    fn idle() -> JoinHandle<()> {
        tokio::spawn(std::future::pending())
    }
}
