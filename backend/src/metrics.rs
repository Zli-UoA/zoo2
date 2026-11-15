//! Prometheus / OpenTelemetry メトリクス定義。
//!
//! カーディナリティ爆発を防ぐため、ラベルには閉じた列挙値・メソッド・
//! `MatchedPath`（正規化済みルート）など有界な値のみを使用する。

use std::sync::OnceLock;

use opentelemetry::{
    KeyValue, global,
    metrics::{Counter, Histogram, UpDownCounter},
};
use prometheus::{Encoder, Registry, TextEncoder};

static METRICS: OnceLock<Metrics> = OnceLock::new();
static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub struct Metrics {
    http_requests_total: Counter<u64>,
    http_request_duration_seconds: Histogram<f64>,
    http_in_flight_requests: UpDownCounter<i64>,
    sse_active_connections: UpDownCounter<i64>,
    ws_active_connections: UpDownCounter<i64>,
    electric_proxy_requests_total: Counter<u64>,
    auth_verifications_total: Counter<u64>,
    app_errors_total: Counter<u64>,
    slack_import_jobs_total: Counter<u64>,
    slack_import_duration_seconds: Histogram<f64>,
    frontend_errors_total: Counter<u64>,
    frontend_web_vitals_seconds: Histogram<f64>,
    frontend_electric_sync_events_total: Counter<u64>,
    frontend_webrtc_events_total: Counter<u64>,
}

impl Metrics {
    pub fn from_meter(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            http_requests_total: meter
                .u64_counter("zoo_http_requests")
                .with_description("Total HTTP requests handled by Zoo services.")
                .build(),
            http_request_duration_seconds: meter
                .f64_histogram("zoo_http_request_duration_seconds")
                .with_description("HTTP request latency in seconds (excluding long-lived streams).")
                .with_boundaries(vec![
                    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
                ])
                .build(),
            http_in_flight_requests: meter
                .i64_up_down_counter("zoo_http_in_flight_requests")
                .with_description("Number of HTTP requests currently in flight.")
                .build(),
            sse_active_connections: meter
                .i64_up_down_counter("zoo_sse_active_connections")
                .with_description("Number of active Electric Shape proxy connections.")
                .build(),
            ws_active_connections: meter
                .i64_up_down_counter("zoo_ws_active_connections")
                .with_description("Number of active WebSocket connections.")
                .build(),
            electric_proxy_requests_total: meter
                .u64_counter("zoo_electric_proxy_requests")
                .with_description("Total Electric Shape proxy upstream requests.")
                .build(),
            auth_verifications_total: meter
                .u64_counter("zoo_auth_verifications")
                .with_description("Authentication verification outcomes.")
                .build(),
            app_errors_total: meter
                .u64_counter("zoo_app_errors")
                .with_description("Application errors returned or recorded by kind.")
                .build(),
            slack_import_jobs_total: meter
                .u64_counter("zoo_slack_import_jobs")
                .with_description("Slack import job step outcomes.")
                .build(),
            slack_import_duration_seconds: meter
                .f64_histogram("zoo_slack_import_duration_seconds")
                .with_description("Duration of Slack import job steps in seconds.")
                .with_boundaries(vec![0.1, 0.5, 1.0, 5.0, 15.0, 60.0, 300.0, 1800.0, 3600.0])
                .build(),
            frontend_errors_total: meter
                .u64_counter("zoo_frontend_errors")
                .with_description("Frontend error events reported via /api/telemetry.")
                .build(),
            frontend_web_vitals_seconds: meter
                .f64_histogram("zoo_frontend_web_vitals_seconds")
                .with_description("Frontend Web Vitals timings in seconds.")
                .with_boundaries(vec![0.05, 0.1, 0.25, 0.5, 1.0, 1.8, 2.5, 4.0, 6.0, 10.0])
                .build(),
            frontend_electric_sync_events_total: meter
                .u64_counter("zoo_frontend_electric_sync_events")
                .with_description("Frontend Electric sync state/error events.")
                .build(),
            frontend_webrtc_events_total: meter
                .u64_counter("zoo_frontend_webrtc_events")
                .with_description("Frontend WebRTC connectivity/quality events.")
                .build(),
        }
    }
}

/// Prometheus レジストリとグローバルメトリクスを登録する（`telemetry::init` から呼ぶ）。
pub fn register(registry: Registry) {
    let _ = REGISTRY.set(registry);
    let _ = METRICS.get_or_init(|| Metrics::from_meter(&global::meter("zoo")));
}

/// 登録済み Prometheus メトリクスをテキスト形式で描画する。
pub fn render_prometheus() -> String {
    let Some(registry) = REGISTRY.get() else {
        return String::new();
    };
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    let mut buf = Vec::new();
    if encoder.encode(&metric_families, &mut buf).is_err() {
        return String::new();
    }
    String::from_utf8(buf).unwrap_or_default()
}

/// HTTP ステータスコードを低カーディナリティのクラス文字列へ変換する。
pub fn status_class(status: u16) -> &'static str {
    match status {
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

/// 進行中 HTTP リクエスト数を RAII で増減するガード。
pub struct InFlightGuard {
    service: &'static str,
    route: String,
    active: bool,
}

impl InFlightGuard {
    pub fn enter(service: &'static str, route: impl Into<String>) -> Self {
        let route = route.into();
        let active = if let Some(m) = METRICS.get() {
            m.http_in_flight_requests.add(
                1,
                &[
                    KeyValue::new("service", service),
                    KeyValue::new("route", route.clone()),
                ],
            );
            true
        } else {
            false
        };
        Self {
            service,
            route,
            active,
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.active
            && let Some(m) = METRICS.get()
        {
            m.http_in_flight_requests.add(
                -1,
                &[
                    KeyValue::new("service", self.service),
                    KeyValue::new("route", self.route.clone()),
                ],
            );
        }
    }
}

/// Electric Shape プロキシのアクティブ接続数を RAII で増減するガード。
pub struct SseConnectionGuard {
    shape: &'static str,
    active: bool,
}

impl SseConnectionGuard {
    pub fn enter(shape: &'static str) -> Self {
        let active = if let Some(m) = METRICS.get() {
            m.sse_active_connections
                .add(1, &[KeyValue::new("shape", shape)]);
            true
        } else {
            false
        };
        Self { shape, active }
    }
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        if self.active
            && let Some(m) = METRICS.get()
        {
            m.sse_active_connections
                .add(-1, &[KeyValue::new("shape", self.shape)]);
        }
    }
}

/// WebSocket アクティブ接続数を RAII で増減するガード。
pub struct WsConnectionGuard {
    kind: &'static str,
    active: bool,
}

impl WsConnectionGuard {
    pub fn enter(kind: &'static str) -> Self {
        let active = if let Some(m) = METRICS.get() {
            m.ws_active_connections
                .add(1, &[KeyValue::new("kind", kind)]);
            true
        } else {
            false
        };
        Self { kind, active }
    }
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        if self.active
            && let Some(m) = METRICS.get()
        {
            m.ws_active_connections
                .add(-1, &[KeyValue::new("kind", self.kind)]);
        }
    }
}

pub fn record_http_request(
    service: &'static str,
    method: &str,
    route: &str,
    status: u16,
    duration_secs: Option<f64>,
) {
    let Some(m) = METRICS.get() else {
        return;
    };
    let class = status_class(status);
    let method = method.to_owned();
    let route = route.to_owned();
    m.http_requests_total.add(
        1,
        &[
            KeyValue::new("service", service),
            KeyValue::new("method", method.clone()),
            KeyValue::new("route", route.clone()),
            KeyValue::new("status_class", class),
        ],
    );
    if let Some(secs) = duration_secs {
        m.http_request_duration_seconds.record(
            secs.max(0.0),
            &[
                KeyValue::new("service", service),
                KeyValue::new("method", method),
                KeyValue::new("route", route),
            ],
        );
    }
}

pub fn record_electric_proxy_request(shape: &'static str, status: u16) {
    let Some(m) = METRICS.get() else {
        return;
    };
    m.electric_proxy_requests_total.add(
        1,
        &[
            KeyValue::new("shape", shape),
            KeyValue::new("status_class", status_class(status)),
        ],
    );
}

pub fn record_auth_verification(mode: &'static str, result: &'static str) {
    let Some(m) = METRICS.get() else {
        return;
    };
    m.auth_verifications_total.add(
        1,
        &[KeyValue::new("mode", mode), KeyValue::new("result", result)],
    );
}

pub fn record_app_error(service: &'static str, kind: &'static str) {
    let Some(m) = METRICS.get() else {
        return;
    };
    m.app_errors_total.add(
        1,
        &[
            KeyValue::new("service", service),
            KeyValue::new("kind", kind),
        ],
    );
}

pub fn record_slack_import_step(step: &str, status: &'static str, duration_secs: f64) {
    let Some(m) = METRICS.get() else {
        return;
    };
    m.slack_import_jobs_total.add(
        1,
        &[
            KeyValue::new("step", step.to_owned()),
            KeyValue::new("status", status),
        ],
    );
    m.slack_import_duration_seconds.record(
        duration_secs.max(0.0),
        &[KeyValue::new("step", step.to_owned())],
    );
}

pub fn record_frontend_error(category: &'static str, route: &'static str) {
    let Some(m) = METRICS.get() else {
        return;
    };
    m.frontend_errors_total.add(
        1,
        &[
            KeyValue::new("category", category),
            KeyValue::new("route", route),
        ],
    );
}

pub fn record_frontend_web_vital(metric: &'static str, route: &'static str, seconds: f64) {
    let Some(m) = METRICS.get() else {
        return;
    };
    if seconds.is_finite() && seconds >= 0.0 {
        m.frontend_web_vitals_seconds.record(
            seconds,
            &[
                KeyValue::new("metric", metric),
                KeyValue::new("route", route),
            ],
        );
    }
}

pub fn record_frontend_electric_sync(shape: &'static str, state: &'static str) {
    let Some(m) = METRICS.get() else {
        return;
    };
    m.frontend_electric_sync_events_total.add(
        1,
        &[KeyValue::new("shape", shape), KeyValue::new("state", state)],
    );
}

pub fn record_frontend_webrtc(state: &'static str) {
    let Some(m) = METRICS.get() else {
        return;
    };
    m.frontend_webrtc_events_total
        .add(1, &[KeyValue::new("state", state)]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::SdkMeterProvider;

    #[test]
    fn status_class_buckets_correctly() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(304), "3xx");
        assert_eq!(status_class(401), "4xx");
        assert_eq!(status_class(429), "4xx");
        assert_eq!(status_class(503), "5xx");
        assert_eq!(status_class(101), "other");
    }

    #[test]
    fn prometheus_exporter_renders_expected_metric_names() {
        let registry = Registry::new();
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .scope_info_enabled(false)
            .without_units()
            .build()
            .unwrap();
        let provider = SdkMeterProvider::builder().with_reader(exporter).build();
        let metrics = Metrics::from_meter(&provider.meter("zoo"));

        metrics.http_requests_total.add(
            1,
            &[
                KeyValue::new("service", "zoo-backend"),
                KeyValue::new("method", "GET"),
                KeyValue::new("route", "/api/channels/{id}"),
                KeyValue::new("status_class", "2xx"),
            ],
        );
        metrics.http_request_duration_seconds.record(
            0.042,
            &[
                KeyValue::new("service", "zoo-backend"),
                KeyValue::new("method", "GET"),
                KeyValue::new("route", "/api/channels/{id}"),
            ],
        );
        metrics.http_in_flight_requests.add(
            1,
            &[
                KeyValue::new("service", "zoo-backend"),
                KeyValue::new("route", "/api/channels/{id}"),
            ],
        );
        metrics
            .sse_active_connections
            .add(1, &[KeyValue::new("shape", "messages")]);
        metrics
            .ws_active_connections
            .add(1, &[KeyValue::new("kind", "call_signaling")]);
        metrics.electric_proxy_requests_total.add(
            1,
            &[
                KeyValue::new("shape", "messages"),
                KeyValue::new("status_class", "2xx"),
            ],
        );
        metrics.auth_verifications_total.add(
            1,
            &[KeyValue::new("mode", "oidc"), KeyValue::new("result", "ok")],
        );
        metrics.app_errors_total.add(
            1,
            &[
                KeyValue::new("service", "zoo-backend"),
                KeyValue::new("kind", "internal"),
            ],
        );
        metrics.slack_import_jobs_total.add(
            1,
            &[
                KeyValue::new("step", "archive_import"),
                KeyValue::new("status", "ok"),
            ],
        );
        metrics
            .slack_import_duration_seconds
            .record(1.25, &[KeyValue::new("step", "archive_import")]);
        metrics.frontend_errors_total.add(
            1,
            &[
                KeyValue::new("category", "api"),
                KeyValue::new("route", "/channels/:channelId"),
            ],
        );
        metrics.frontend_web_vitals_seconds.record(
            0.85,
            &[
                KeyValue::new("metric", "LCP"),
                KeyValue::new("route", "/channels/:channelId"),
            ],
        );
        metrics.frontend_electric_sync_events_total.add(
            1,
            &[
                KeyValue::new("shape", "messages"),
                KeyValue::new("state", "fetch_error"),
            ],
        );
        metrics
            .frontend_webrtc_events_total
            .add(1, &[KeyValue::new("state", "ice_failed")]);

        let encoder = TextEncoder::new();
        let mut out = Vec::new();
        encoder.encode(&registry.gather(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(text.contains("zoo_http_requests_total{"));
        assert!(text.contains(r#"route="/api/channels/{id}""#));
        assert!(text.contains(r#"status_class="2xx""#));
        assert!(text.contains("zoo_http_request_duration_seconds_bucket{"));
        assert!(text.contains("zoo_http_in_flight_requests{"));
        assert!(text.contains("zoo_sse_active_connections{"));
        assert!(text.contains("zoo_ws_active_connections{"));
        assert!(text.contains("zoo_electric_proxy_requests_total{"));
        assert!(text.contains("zoo_auth_verifications_total{"));
        assert!(text.contains("zoo_app_errors_total{"));
        assert!(text.contains("zoo_slack_import_jobs_total{"));
        assert!(text.contains("zoo_slack_import_duration_seconds_bucket{"));
        assert!(text.contains("zoo_frontend_errors_total{"));
        assert!(text.contains("zoo_frontend_web_vitals_seconds_bucket{"));
        assert!(text.contains("zoo_frontend_electric_sync_events_total{"));
        assert!(text.contains("zoo_frontend_webrtc_events_total{"));
        assert!(!text.contains("otel_scope_name"));
    }
}
