//! 宛先ホストごとのサーキットブレーカーと同時接続上限。
//!
//! 落ちている宛先に配信ワーカーの枠と試行回数を使い切らせないための、プロセス内の仕組み。
//! 状態はレプリカごとで共有しない（共有しなくても、各レプリカが数回失敗すれば開く）。
//!
//! - 連続して `FAILURES` 回失敗（タイムアウト・接続失敗・名前解決失敗・5xx）で開く。
//! - 開いている間は配信を取らない（試行回数を消費しない）。呼び出し側が、そのホストの
//!   配信の `available_at` を開放時刻まで先送りする。
//! - 開放時刻を過ぎたら 1 件だけ試す（half-open）。成功で閉じ、失敗なら期間を倍にして開き直す。
//! - 4xx など応答があったものは「ホストは生きている」として成功扱い。
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const FAILURES: u32 = 5;
const OPEN_MIN: Duration = Duration::from_secs(30);
const OPEN_MAX: Duration = Duration::from_secs(300);
/// 試し送りが戻ってこないまま残った場合に、次の試し送りを許すまでの時間。
const PROBE_STALE: Duration = Duration::from_secs(10);
/// 1 ホストへの同時送信の上限（インストール単位の 32 並列とは別）。
pub const HOST_CONCURRENCY: usize = 16;
/// `/metrics` に出す開放中ホストの最大数（ラベルのカーディナリティ制限）。
const METRIC_HOSTS: usize = 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// 通常どおり送る。
    Closed,
    /// 試し送りを 1 件だけ行う。
    Probe,
    /// 開いている。この時間だけ配信を先送りする。
    Open(Duration),
}

/// 送信結果がブレーカーに与える影響。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure,
    /// 宛先の健康状態を表さない失敗（公開 IP でない等）。何も変えない。
    Ignore,
}

#[derive(Default)]
struct State {
    failures: u32,
    open_until: Option<Instant>,
    /// 直近に開いていた期間（再度失敗したときに倍にする）。
    last_open: Duration,
    probing_since: Option<Instant>,
}

#[derive(Default)]
pub struct Breakers {
    hosts: Mutex<HashMap<String, State>>,
    semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
}

pub fn global() -> &'static Breakers {
    static B: OnceLock<Breakers> = OnceLock::new();
    B.get_or_init(Breakers::default)
}

/// URL のホスト名（小文字）。解析できなければ空。
pub fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

impl Breakers {
    pub fn check(&self, host: &str) -> Decision {
        self.check_at(host, Instant::now())
    }

    pub fn check_at(&self, host: &str, now: Instant) -> Decision {
        let mut hosts = self.hosts.lock().unwrap();
        let Some(state) = hosts.get_mut(host) else {
            return Decision::Closed;
        };
        let Some(until) = state.open_until else {
            return Decision::Closed;
        };
        if now < until {
            return Decision::Open(until - now);
        }
        match state.probing_since {
            Some(since) if now.duration_since(since) < PROBE_STALE => Decision::Open(PROBE_STALE),
            _ => {
                state.probing_since = Some(now);
                Decision::Probe
            }
        }
    }

    pub fn record(&self, host: &str, outcome: Outcome) {
        self.record_at(host, outcome, Instant::now())
    }

    pub fn record_at(&self, host: &str, outcome: Outcome, now: Instant) {
        if outcome == Outcome::Ignore {
            return;
        }
        let mut hosts = self.hosts.lock().unwrap();
        if outcome == Outcome::Success {
            // 開いている間に届いた成功（開く前に送った分、別のパスへの 4xx など）では閉じない。
            // 閉じるのは通常時か、試し送りの成功だけ。
            let held_open = hosts
                .get(host)
                .is_some_and(|s| s.open_until.is_some_and(|u| now < u));
            if !held_open {
                hosts.remove(host);
            }
            return;
        }
        let state = hosts.entry(host.to_owned()).or_default();
        state.failures += 1;
        let half_open = state.probing_since.is_some();
        if state.open_until.is_none() && state.failures < FAILURES {
            return;
        }
        // 試し送りの失敗、または閾値到達。
        // 開いている最中に届いた古い失敗（開く前に送ったもの）では期間を延ばさない。
        if state.open_until.is_some_and(|u| now < u) && !half_open {
            return;
        }
        let next = if half_open {
            (state.last_open * 2).clamp(OPEN_MIN, OPEN_MAX)
        } else {
            OPEN_MIN
        };
        state.last_open = next;
        state.open_until = Some(now + next);
        state.probing_since = None;
    }

    /// 読み取りだけ。開放時刻より前なら true（試し送りの状態は進めない）。
    pub fn is_open(&self, host: &str) -> bool {
        let now = Instant::now();
        self.hosts
            .lock()
            .unwrap()
            .get(host)
            .is_some_and(|s| s.open_until.is_some_and(|u| now < u))
    }

    pub fn semaphore(&self, host: &str) -> Arc<Semaphore> {
        self.semaphores
            .lock()
            .unwrap()
            .entry(host.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(HOST_CONCURRENCY)))
            .clone()
    }

    /// 開いているホストを残り時間の長い順に。（総数, 上位 `METRIC_HOSTS` 件）
    pub fn open_hosts(&self) -> (usize, Vec<String>) {
        let now = Instant::now();
        let hosts = self.hosts.lock().unwrap();
        let mut open: Vec<_> = hosts
            .iter()
            .filter(|(_, s)| {
                s.open_until
                    .is_some_and(|u| now < u || s.probing_since.is_some())
            })
            .map(|(h, s)| (h.clone(), s.open_until))
            .collect();
        open.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let total = open.len();
        (
            total,
            open.into_iter()
                .take(METRIC_HOSTS)
                .map(|(h, _)| h)
                .collect(),
        )
    }

    /// テスト用: ホストを指定期間だけ開く。
    pub fn force_open(&self, host: &str, duration: Duration) {
        let mut hosts = self.hosts.lock().unwrap();
        let state = hosts.entry(host.to_owned()).or_default();
        state.failures = FAILURES;
        state.last_open = duration;
        state.open_until = Some(Instant::now() + duration);
        state.probing_since = None;
    }

    /// テスト用: 状態を消す。
    pub fn reset(&self, host: &str) {
        self.hosts.lock().unwrap().remove(host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trip(b: &Breakers, host: &str, now: Instant) {
        for _ in 0..FAILURES {
            b.record_at(host, Outcome::Failure, now);
        }
    }

    #[test]
    fn opens_after_consecutive_failures_and_success_resets_the_count() {
        let b = Breakers::default();
        let t0 = Instant::now();
        for _ in 0..FAILURES - 1 {
            b.record_at("h", Outcome::Failure, t0);
        }
        assert_eq!(b.check_at("h", t0), Decision::Closed);
        b.record_at("h", Outcome::Success, t0);
        for _ in 0..FAILURES - 1 {
            b.record_at("h", Outcome::Failure, t0);
        }
        assert_eq!(b.check_at("h", t0), Decision::Closed, "not consecutive");
        b.record_at("h", Outcome::Failure, t0);
        assert_eq!(b.check_at("h", t0), Decision::Open(OPEN_MIN));
        // 他のホストには影響しない。
        assert_eq!(b.check_at("other", t0), Decision::Closed);
    }

    #[test]
    fn ignored_outcomes_change_nothing() {
        let b = Breakers::default();
        let t0 = Instant::now();
        for _ in 0..FAILURES * 2 {
            b.record_at("h", Outcome::Ignore, t0);
        }
        assert_eq!(b.check_at("h", t0), Decision::Closed);
    }

    #[test]
    fn half_open_allows_one_probe_then_closes_or_reopens_longer() {
        let b = Breakers::default();
        let t0 = Instant::now();
        trip(&b, "h", t0);
        let t1 = t0 + OPEN_MIN;
        assert_eq!(b.check_at("h", t1), Decision::Probe);
        // 試し送りが戻るまで、他の呼び出しは待たされる。
        assert!(matches!(b.check_at("h", t1), Decision::Open(_)));
        // 失敗すると期間が倍になる。
        b.record_at("h", Outcome::Failure, t1);
        assert_eq!(b.check_at("h", t1), Decision::Open(OPEN_MIN * 2));
        // 上限を超えない。
        let mut t = t1 + OPEN_MIN * 2;
        for _ in 0..10 {
            assert_eq!(b.check_at("h", t), Decision::Probe);
            b.record_at("h", Outcome::Failure, t);
            t += OPEN_MAX;
        }
        assert!(matches!(b.check_at("h", t - OPEN_MAX), Decision::Open(d) if d <= OPEN_MAX));
        // 成功で閉じる。
        assert_eq!(b.check_at("h", t), Decision::Probe);
        b.record_at("h", Outcome::Success, t);
        assert_eq!(b.check_at("h", t), Decision::Closed);
    }

    #[test]
    fn a_lost_probe_does_not_block_the_host_forever() {
        let b = Breakers::default();
        let t0 = Instant::now();
        trip(&b, "h", t0);
        let t1 = t0 + OPEN_MIN;
        assert_eq!(b.check_at("h", t1), Decision::Probe);
        assert_eq!(b.check_at("h", t1 + PROBE_STALE), Decision::Probe);
    }

    #[test]
    fn late_failures_from_before_opening_do_not_extend_the_open_period() {
        let b = Breakers::default();
        let t0 = Instant::now();
        trip(&b, "h", t0);
        // 開いた後に、開く前に送った分の失敗が届く。
        b.record_at("h", Outcome::Failure, t0 + Duration::from_secs(1));
        assert_eq!(
            b.check_at("h", t0 + Duration::from_secs(1)),
            Decision::Open(OPEN_MIN - Duration::from_secs(1))
        );
    }

    #[test]
    fn a_stray_success_does_not_close_an_open_circuit() {
        let b = Breakers::default();
        let t0 = Instant::now();
        trip(&b, "h", t0);
        b.record_at("h", Outcome::Success, t0 + Duration::from_secs(1));
        assert!(matches!(
            b.check_at("h", t0 + Duration::from_secs(1)),
            Decision::Open(_)
        ));
        // 試し送りの成功では閉じる。
        let t1 = t0 + OPEN_MIN;
        assert_eq!(b.check_at("h", t1), Decision::Probe);
        b.record_at("h", Outcome::Success, t1);
        assert_eq!(b.check_at("h", t1), Decision::Closed);
    }

    #[test]
    fn open_hosts_lists_only_open_ones_and_caps_the_count() {
        let b = Breakers::default();
        for i in 0..METRIC_HOSTS + 5 {
            b.force_open(&format!("host{i}.example"), Duration::from_secs(60));
        }
        b.record("closed.example", Outcome::Success);
        let (total, shown) = b.open_hosts();
        assert_eq!(total, METRIC_HOSTS + 5);
        assert_eq!(shown.len(), METRIC_HOSTS);
        assert!(!shown.iter().any(|h| h == "closed.example"));
    }

    #[test]
    fn host_of_normalises_case_and_rejects_garbage() {
        assert_eq!(host_of("https://Example.COM:8443/x"), "example.com");
        assert_eq!(host_of("not a url"), "");
    }
}
