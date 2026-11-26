//! 本文のリンクが指すメッセージの、購読の共有と取得の結果。TUI とデスクトップ版で共有する。
//! 取得そのものは [`super::spawn_linked`] が行う。

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use tokio::{sync::mpsc, task::JoinHandle};
use zoo2_client_types::{
    links::LinkedFetch,
    rows::{FromRow, Message},
};

use super::LinkedEvent;

/// 取得を始めてから、これだけたっても終わらないときは、読み込めなかったものとする。
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// 画面の近くからリンクがなくなって、これだけたったら購読を閉じる。
pub const RELEASE_DELAY: Duration = Duration::from_secs(30);
/// 取得できた結果を覚える件数。
const CACHE_SIZE: usize = 256;
/// 同時に購読するチャンネルの数の上限。backend のアカウントごとのシェイプの上限は、ほかの画面や
/// 端末と分け合うので、少なくする。
pub const MAX_CHANNELS: usize = 8;

/// リンク先のメッセージ (チャンネル ID, メッセージ ID)。
pub type LinkedKey = (String, String);

struct Fetching {
    ids: mpsc::UnboundedSender<String>,
    task: JoinHandle<()>,
    sent: HashSet<String>,
    last_wanted: Instant,
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
    /// 画面の近くに出ている、取得が要るリンク先。
    pub visible: Vec<LinkedKey>,
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

    /// `keys` のうち、何か分かったもの（取得できた、失敗した、時間切れ）。取得を別のプロセスの画面に
    /// 伝えるときに使う。まだ何も分からないものを送らないので、前に分かったことを読み込み中で
    /// 上書きしない。
    pub fn known(&self, keys: &[LinkedKey], now: Instant) -> Vec<(LinkedKey, LinkedFetch)> {
        keys.iter()
            .map(|key| (key.clone(), self.fetch(key, now)))
            .filter(|(_, fetch)| *fetch != LinkedFetch::Loading { stalled: false })
            .collect()
    }

    /// このチャンネルで取得した行に、暗号文の行があるか。
    pub fn has_ciphertext(&self, channel_id: &str) -> bool {
        self.results.iter().any(|((channel, _), message)| {
            channel == channel_id && message.as_ref().is_some_and(|m| m.ciphertext.is_some())
        })
    }

    /// 画面の近くのリンク先を入れ替えて [`Self::sync`] する。画面が変わったときだけ呼ぶ側のためのもので、
    /// 入れ替える前のリンク先は、いままで使っていたものとして扱う。
    pub fn watch(
        &mut self,
        now: Instant,
        visible: Vec<LinkedKey>,
        spawn: impl FnMut(String, mpsc::UnboundedReceiver<String>) -> JoinHandle<()>,
    ) {
        for (channel_id, _) in &self.visible {
            if let Some(fetching) = self.channels.get_mut(channel_id) {
                fetching.last_wanted = now;
            }
        }
        self.visible = visible;
        self.sync(now, spawn);
    }

    /// [`Self::visible`] のリンク先を取得し、しばらく使っていない購読を閉じる。`spawn` はチャンネルの
    /// 取得のタスクを起動する。
    pub fn sync(
        &mut self,
        now: Instant,
        mut spawn: impl FnMut(String, mpsc::UnboundedReceiver<String>) -> JoinHandle<()>,
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
                && self.channels.len() >= MAX_CHANNELS
                && !self.release_idle()
            {
                self.over_limit.insert(key.clone());
                continue;
            }
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
            self.release(&channel_id);
        }
        self.evict();
    }

    fn release(&mut self, channel_id: &str) {
        if let Some(fetching) = self.channels.remove(channel_id) {
            fetching.task.abort();
        }
        // 失敗は覚えず、次に表示したときに取り直す。取得できた結果は残し、戻ったときに先に出す。
        self.failed.retain(|(channel, _)| channel != channel_id);
        self.started.retain(|(channel, _), _| channel != channel_id);
    }

    /// 画面の近くにリンクがないチャンネルのうち、いちばん前に使った購読を閉じる。なければ `false`。
    fn release_idle(&mut self) -> bool {
        let idle = self
            .channels
            .iter()
            .filter(|(id, _)| !self.visible.iter().any(|(channel, _)| channel == *id))
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

    /// 次に状態が変わる時刻（取得の時間切れか、購読を閉じる時刻）。そのときに [`Self::sync`] する。
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let timeouts = self
            .visible
            .iter()
            .filter(|key| !self.results.contains_key(*key) && !self.failed.contains(*key))
            .filter_map(|key| self.started.get(key))
            .map(|at| *at + LOAD_TIMEOUT)
            .filter(|at| *at > now);
        let releases = self
            .channels
            .iter()
            .filter(|(id, _)| !self.visible.iter().any(|(channel, _)| channel == *id))
            .map(|(_, fetching)| fetching.last_wanted + RELEASE_DELAY);
        timeouts.chain(releases).min()
    }

    /// 取得のタスクから届いたことを反映し、状態が変わったリンク先を返す。
    pub fn apply(&mut self, event: LinkedEvent) -> Vec<LinkedKey> {
        let mut changed = Vec::new();
        match event {
            LinkedEvent::Rows { channel_id, rows } => {
                for (message_id, row) in rows {
                    let key = (channel_id.clone(), message_id);
                    changed.push(key.clone());
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
                        changed.push(key.clone());
                        self.failed.insert(key);
                    }
                }
            }
        }
        changed
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

    fn idle() -> JoinHandle<()> {
        tokio::spawn(std::future::pending())
    }

    #[tokio::test]
    async fn shares_one_subscription_per_channel_and_releases_it() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        let mut received = Vec::new();
        let mut spawned = 0;
        links.visible = vec![key("m1"), key("m1"), key("m2")];
        links.sync(start, |_, receiver| {
            spawned += 1;
            received.push(receiver);
            idle()
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
        assert_eq!(links.next_deadline(start), Some(start + LOAD_TIMEOUT));
        let late = start + LOAD_TIMEOUT;
        assert_eq!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Loading { stalled: true }
        );
        // 時間切れを知らせたあとは、同じ時刻を返し続けない。
        assert_eq!(links.next_deadline(late), None);
        assert_eq!(links.apply(rows("m1", Some("本文"))), [key("m1")]);
        links.apply(rows("m2", None));
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
        assert_eq!(links.fetch(&key("m2"), late), LinkedFetch::Missing);

        // 使わなくなって 30 秒で閉じる。結果は残る。
        links.visible.clear();
        assert_eq!(links.next_deadline(late), Some(start + RELEASE_DELAY));
        links.sync(start + RELEASE_DELAY, |_, _| unreachable!());
        assert!(links.channels.is_empty());
        assert_eq!(links.next_deadline(start + RELEASE_DELAY), None);
        assert!(matches!(
            links.fetch(&key("m1"), late),
            LinkedFetch::Found(_)
        ));
    }

    #[tokio::test]
    async fn only_known_states_are_reported() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        // 受信側を持っておく（捨てると、送れずに取得の失敗になる）。
        let mut receivers = Vec::new();
        links.watch(start, vec![key("m1"), key("m2")], |_, receiver| {
            receivers.push(receiver);
            idle()
        });
        assert!(links.known(&links.visible, start).is_empty());
        links.apply(rows("m2", None));
        assert_eq!(
            links.known(&links.visible, start + LOAD_TIMEOUT),
            [
                (key("m1"), LinkedFetch::Loading { stalled: true }),
                (key("m2"), LinkedFetch::Missing),
            ]
        );
    }

    #[tokio::test]
    async fn limits_the_channels_subscribed_at_once() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        let mut receivers = Vec::new();
        let mut spawn = |_: String, receiver| {
            receivers.push(receiver);
            idle()
        };
        let channel = |n: usize| (format!("c{n}"), "m".to_owned());
        links.visible = (0..MAX_CHANNELS + 2).map(channel).collect();
        links.sync(start, &mut spawn);
        assert_eq!(links.channels.len(), MAX_CHANNELS);
        // 超えた分は取得せず、読み込めなかったものとする。
        assert_eq!(
            links.fetch(&channel(MAX_CHANNELS), start),
            LinkedFetch::Failed
        );
        assert_ne!(links.fetch(&channel(0), start), LinkedFetch::Failed);
        // 画面の近くから外れた購読があれば、閉じて空ける。
        links.watch(
            start + Duration::from_secs(1),
            vec![channel(MAX_CHANNELS), channel(1)],
            &mut spawn,
        );
        assert_eq!(links.channels.len(), MAX_CHANNELS);
        assert!(links.channels.contains_key("c1"));
        assert!(links.channels.contains_key(&format!("c{MAX_CHANNELS}")));
        assert_ne!(
            links.fetch(&channel(MAX_CHANNELS), start),
            LinkedFetch::Failed
        );
    }

    #[tokio::test]
    async fn release_counts_from_when_the_link_left_the_screen() {
        let mut links = LinkedCache::default();
        let start = Instant::now();
        links.watch(start, vec![key("m1")], |_, _| idle());
        // 長く表示したあとで外れても、そこから 30 秒は閉じない。
        let left = start + RELEASE_DELAY * 3;
        links.watch(left, Vec::new(), |_, _| unreachable!());
        assert_eq!(links.channels.len(), 1);
        assert_eq!(links.next_deadline(left), Some(left + RELEASE_DELAY));
        links.sync(left + RELEASE_DELAY, |_, _| unreachable!());
        assert!(links.channels.is_empty());
    }

    #[tokio::test]
    async fn failures_are_forgotten_after_release() {
        let mut links = LinkedCache::default();
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
        let mut links = LinkedCache::default();
        let start = Instant::now();
        links.visible = vec![key("m0")];
        links.sync(start, |_, _| idle());
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
        let mut links = LinkedCache::default();
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
}
