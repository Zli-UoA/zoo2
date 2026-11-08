//! zoo-dispatcher の Prometheus 形式メトリクス。依存を増やさないため手書きの小さな実装にしている。
//! ホスト名などカーディナリティの高いラベルは持たない（サーキットブレーカーの上位ホストは後続 PR）。
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

/// 作成から ACK までの秒数のバケット（上限）。
/// 配信の有効期限（24 時間）まで見えるよう、6・12・24 時間のバケットを持つ。
const BUCKETS: [f64; 15] = [
    0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 120.0, 600.0, 3600.0, 21600.0, 43200.0,
    86400.0,
];

/// ヒストグラム。スクレイプと観測が重なっても累積性（下位 ≤ 上位 ≤ `+Inf`）を保つため、
/// 1 つのロックの下でまとめて更新・読み出しする。
#[derive(Default)]
struct Histogram {
    buckets: [u64; BUCKETS.len()],
    count: u64,
    sum_seconds: f64,
}

#[derive(Default)]
pub struct Metrics {
    attempts: Mutex<BTreeMap<(&'static str, &'static str), u64>>,
    latency: Mutex<Histogram>,
    /// （滞留数, 最古の滞留秒数）。2 つの値が別々の更新を読まれないよう 1 つのロックで持つ。
    queue: Mutex<(u64, f64)>,
}

pub fn global() -> &'static Metrics {
    static METRICS: OnceLock<Metrics> = OnceLock::new();
    METRICS.get_or_init(Metrics::default)
}

impl Metrics {
    pub fn record_attempt(&self, ok: bool, status_class: &'static str) {
        let result = if ok { "ok" } else { "error" };
        *self
            .attempts
            .lock()
            .unwrap()
            .entry((result, status_class))
            .or_default() += 1;
    }

    /// 作成から ACK までの秒数。
    pub fn observe_latency(&self, seconds: f64) {
        let seconds = seconds.max(0.0);
        let mut h = self.latency.lock().unwrap();
        for (i, upper) in BUCKETS.iter().enumerate() {
            if seconds <= *upper {
                h.buckets[i] += 1;
            }
        }
        h.count += 1;
        h.sum_seconds += seconds;
    }

    pub fn set_queue(&self, depth: u64, oldest_pending_seconds: f64) {
        *self.queue.lock().unwrap() = (depth, oldest_pending_seconds.max(0.0));
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP zoo_dispatch_attempts_total HTTP delivery attempts.\n# TYPE zoo_dispatch_attempts_total counter\n");
        for ((result, class), n) in self.attempts.lock().unwrap().iter() {
            out.push_str(&format!(
                "zoo_dispatch_attempts_total{{result=\"{result}\",status_class=\"{class}\"}} {n}\n"
            ));
        }
        out.push_str("# HELP zoo_dispatch_latency_seconds Seconds from event creation to ACK.\n# TYPE zoo_dispatch_latency_seconds histogram\n");
        // observe_latency は該当する全バケットに加算するので、そのまま累積値になる。
        {
            let h = self.latency.lock().unwrap();
            for (i, upper) in BUCKETS.iter().enumerate() {
                out.push_str(&format!(
                    "zoo_dispatch_latency_seconds_bucket{{le=\"{upper}\"}} {}\n",
                    h.buckets[i]
                ));
            }
            out.push_str(&format!(
                "zoo_dispatch_latency_seconds_bucket{{le=\"+Inf\"}} {c}\nzoo_dispatch_latency_seconds_sum {}\nzoo_dispatch_latency_seconds_count {c}\n",
                h.sum_seconds,
                c = h.count
            ));
        }
        let (depth, oldest) = *self.queue.lock().unwrap();
        out.push_str(&format!(
            "# HELP zoo_dispatch_queue_depth Undelivered, unexpired deliveries (cluster-wide, every replica reports the same value; aggregate with max or avg, not sum).\n# TYPE zoo_dispatch_queue_depth gauge\nzoo_dispatch_queue_depth {depth}\n# HELP zoo_dispatch_oldest_pending_seconds Age of the oldest undelivered, unexpired delivery (cluster-wide; aggregate with max).\n# TYPE zoo_dispatch_oldest_pending_seconds gauge\nzoo_dispatch_oldest_pending_seconds {oldest}\n"
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_cumulative_histogram_and_labelled_counters() {
        let m = Metrics::default();
        m.record_attempt(true, "2xx");
        m.record_attempt(true, "2xx");
        m.record_attempt(false, "5xx");
        m.observe_latency(0.2);
        m.observe_latency(3.0);
        m.set_queue(7, 12.5);
        let text = m.render();
        assert!(text.contains("zoo_dispatch_attempts_total{result=\"ok\",status_class=\"2xx\"} 2"));
        assert!(
            text.contains("zoo_dispatch_attempts_total{result=\"error\",status_class=\"5xx\"} 1")
        );
        assert!(text.contains("zoo_dispatch_latency_seconds_bucket{le=\"0.25\"} 1"));
        assert!(text.contains("zoo_dispatch_latency_seconds_bucket{le=\"5\"} 2"));
        assert!(text.contains("zoo_dispatch_latency_seconds_bucket{le=\"+Inf\"} 2"));
        assert!(text.contains("zoo_dispatch_latency_seconds_count 2"));
        assert!(text.contains("zoo_dispatch_queue_depth 7"));
        assert!(text.contains("zoo_dispatch_oldest_pending_seconds 12.5"));
    }
}
