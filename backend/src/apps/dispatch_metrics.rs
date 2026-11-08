//! zoo-dispatcher の Prometheus 形式メトリクス。依存を増やさないため手書きの小さな実装にしている。
//! ホスト名などカーディナリティの高いラベルは、開放中のサーキットの上位数件だけに限る。
use std::{
    collections::BTreeMap,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

/// 作成から ACK までの秒数のバケット（上限）。
const BUCKETS: [f64; 12] = [
    0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 120.0, 600.0, 3600.0,
];

#[derive(Default)]
pub struct Metrics {
    attempts: Mutex<BTreeMap<(&'static str, &'static str), u64>>,
    buckets: [AtomicU64; BUCKETS.len()],
    latency_count: AtomicU64,
    latency_sum_micros: AtomicU64,
    queue_depth: AtomicU64,
    /// 最古の滞留秒数（ミリ秒で保持）
    oldest_pending_millis: AtomicU64,
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
        for (i, upper) in BUCKETS.iter().enumerate() {
            if seconds <= *upper {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.latency_count.fetch_add(1, Ordering::Relaxed);
        self.latency_sum_micros
            .fetch_add((seconds * 1e6) as u64, Ordering::Relaxed);
    }

    pub fn set_queue(&self, depth: u64, oldest_pending_seconds: f64) {
        self.queue_depth.store(depth, Ordering::Relaxed);
        self.oldest_pending_millis.store(
            (oldest_pending_seconds.max(0.0) * 1000.0) as u64,
            Ordering::Relaxed,
        );
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
        for (i, upper) in BUCKETS.iter().enumerate() {
            out.push_str(&format!(
                "zoo_dispatch_latency_seconds_bucket{{le=\"{upper}\"}} {}\n",
                self.buckets[i].load(Ordering::Relaxed)
            ));
        }
        let count = self.latency_count.load(Ordering::Relaxed);
        out.push_str(&format!(
            "zoo_dispatch_latency_seconds_bucket{{le=\"+Inf\"}} {count}\nzoo_dispatch_latency_seconds_sum {}\nzoo_dispatch_latency_seconds_count {count}\n",
            self.latency_sum_micros.load(Ordering::Relaxed) as f64 / 1e6
        ));
        out.push_str(&format!(
            "# HELP zoo_dispatch_queue_depth Undelivered, unexpired deliveries.\n# TYPE zoo_dispatch_queue_depth gauge\nzoo_dispatch_queue_depth {}\n# HELP zoo_dispatch_oldest_pending_seconds Age of the oldest undelivered, unexpired delivery.\n# TYPE zoo_dispatch_oldest_pending_seconds gauge\nzoo_dispatch_oldest_pending_seconds {}\n",
            self.queue_depth.load(Ordering::Relaxed),
            self.oldest_pending_millis.load(Ordering::Relaxed) as f64 / 1000.0
        ));
        let (open_total, open_hosts) = super::circuit::global().open_hosts();
        out.push_str("# HELP zoo_dispatch_circuit_open Destination hosts whose circuit is open (top hosts only).\n# TYPE zoo_dispatch_circuit_open gauge\n");
        for host in open_hosts {
            out.push_str(&format!(
                "zoo_dispatch_circuit_open{{host=\"{}\"}} 1\n",
                host.replace('\\', "\\\\").replace('"', "\\\"")
            ));
        }
        out.push_str(&format!(
            "# HELP zoo_dispatch_circuit_open_hosts Number of destination hosts whose circuit is open.\n# TYPE zoo_dispatch_circuit_open_hosts gauge\nzoo_dispatch_circuit_open_hosts {open_total}\n"
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
