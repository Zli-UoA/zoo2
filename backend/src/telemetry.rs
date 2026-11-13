//! 構造化ログ・OpenTelemetry (Trace / Metrics / Logs)・Prometheus エンドポイントの共通初期化。

use std::{
    fmt,
    future::Future,
    net::SocketAddr,
    sync::OnceLock,
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{MatchedPath, Request},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use opentelemetry::{
    global,
    trace::{TraceContextExt, TracerProvider as _},
};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_http::HeaderExtractor;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    logs::{SdkLoggerProvider, log_processor_with_async_runtime::BatchLogProcessor},
    metrics::{SdkMeterProvider, periodic_reader_with_async_runtime::PeriodicReader},
    propagation::TraceContextPropagator,
    runtime,
    trace::{SdkTracerProvider, span_processor_with_async_runtime::BatchSpanProcessor},
};
use prometheus::Registry;
use serde_json::{Map, Value};
use tracing::{Event, Instrument, Subscriber, field::Visit};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{
    EnvFilter, Layer,
    fmt::{
        FmtContext, FormatEvent, FormatFields,
        format::{JsonFields, Writer},
    },
    layer::SubscriberExt,
    registry::LookupSpan,
    util::SubscriberInitExt,
};

static SERVICE_NAME: OnceLock<&'static str> = OnceLock::new();

/// 初期化されたサービス名を返す（未初期化時は `"zoo-backend"`）。
pub fn service_name() -> &'static str {
    SERVICE_NAME.get().copied().unwrap_or("zoo-backend")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

/// `LOG_FORMAT` と `ENVIRONMENT` からログ出力形式を決める。
pub(crate) fn parse_log_format(log_format_env: Option<&str>, environment: &str) -> LogFormat {
    match log_format_env
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => LogFormat::Json,
        Some("pretty" | "text" | "compact") => LogFormat::Pretty,
        _ if environment == "production" => LogFormat::Json,
        _ => LogFormat::Pretty,
    }
}

/// 共通 OTLP またはシグナル固有のエンドポイントが設定されているか判定する。
fn otlp_signal_enabled(signal_env: &str) -> bool {
    let has_env = |name: &str| std::env::var(name).is_ok_and(|v| !v.trim().is_empty());
    has_env("OTEL_EXPORTER_OTLP_ENDPOINT") || has_env(signal_env)
}

fn build_resource(default_service: &'static str) -> Resource {
    let has_service_name = std::env::var("OTEL_SERVICE_NAME").is_ok_and(|v| !v.trim().is_empty());
    let mut builder = Resource::builder();
    if !has_service_name {
        builder = builder.with_service_name(default_service);
    }
    builder.build()
}

/// 起動中の OpenTelemetry プロバイダを保持し、終了時にフラッシュ・シャットダウンするガード。
pub struct TelemetryGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<SdkLoggerProvider>,
}

impl TelemetryGuard {
    /// 非同期コンテキストから安全に全プロバイダをフラッシュ・停止する。
    ///
    /// exporter の `shutdown()` が応答せずブロックした場合でも、Tokio ランタイムの
    /// blocking スレッドプール終了待ちに影響しないよう独立 OS スレッドで実行し、
    /// 最大 5 秒で待機を打ち切る。
    pub async fn shutdown(mut self) {
        let tracer = self.tracer_provider.take();
        let meter = self.meter_provider.take();
        let logger = self.logger_provider.take();
        if tracer.is_none() && meter.is_none() && logger.is_none() {
            return;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        if std::thread::Builder::new()
            .name("otel-shutdown".to_string())
            .spawn(move || {
                if let Some(p) = logger {
                    let _ = p.shutdown();
                }
                if let Some(p) = tracer {
                    let _ = p.shutdown();
                }
                if let Some(p) = meter {
                    let _ = p.shutdown();
                }
                let _ = tx.send(());
            })
            .is_ok()
        {
            let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if tokio::runtime::Handle::try_current().is_ok() {
            // 非同期コンテキスト内の Drop では `block_on` によるワーカー停止を避け、
            // 明示的な `shutdown().await` が呼ばれなかった場合のみ別スレッドへ逃がす。
            let tracer = self.tracer_provider.take();
            let meter = self.meter_provider.take();
            let logger = self.logger_provider.take();
            if tracer.is_some() || meter.is_some() || logger.is_some() {
                std::thread::spawn(move || {
                    if let Some(p) = logger {
                        let _ = p.shutdown();
                    }
                    if let Some(p) = tracer {
                        let _ = p.shutdown();
                    }
                    if let Some(p) = meter {
                        let _ = p.shutdown();
                    }
                });
            }
            return;
        }
        if let Some(p) = self.logger_provider.take() {
            let _ = p.shutdown();
        }
        if let Some(p) = self.tracer_provider.take() {
            let _ = p.shutdown();
        }
        if let Some(p) = self.meter_provider.take() {
            let _ = p.shutdown();
        }
    }
}

/// ロギング・OpenTelemetry・Prometheus メトリクスを初期化する。
pub fn init(service_name: &'static str, default_filter: &str) -> TelemetryGuard {
    let _ = SERVICE_NAME.set(service_name);
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();

    let environment = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());
    let log_format = parse_log_format(std::env::var("LOG_FORMAT").ok().as_deref(), &environment);
    let resource = build_resource(service_name);

    global::set_text_map_propagator(TraceContextPropagator::new());

    // TracerProvider: OTLP エンドポイント未設定でも W3C trace_id/span_id は生成し、
    // ログとスパンの相関を維持する。
    let mut tracer_builder = SdkTracerProvider::builder().with_resource(resource.clone());
    if otlp_signal_enabled("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
        match opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .build()
        {
            Ok(exporter) => {
                let processor = BatchSpanProcessor::builder(exporter, runtime::Tokio).build();
                tracer_builder = tracer_builder.with_span_processor(processor);
            }
            Err(err) => {
                eprintln!("Failed to initialize OTLP span exporter: {err}");
            }
        }
    }
    let tracer_provider = tracer_builder.build();
    let tracer = tracer_provider.tracer(service_name);
    global::set_tracer_provider(tracer_provider.clone());

    // MeterProvider: Prometheus レジストリは常に有効化し、OTLP メトリクスは設定時のみ追加。
    let registry = Registry::new();
    let mut meter_builder = SdkMeterProvider::builder().with_resource(resource.clone());
    match opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .scope_info_enabled(false)
        .without_units()
        .build()
    {
        Ok(prom_exporter) => {
            meter_builder = meter_builder.with_reader(prom_exporter);
        }
        Err(err) => {
            eprintln!("Failed to initialize Prometheus exporter: {err}");
        }
    }
    if otlp_signal_enabled("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT") {
        match opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .build()
        {
            Ok(exporter) => {
                let reader = PeriodicReader::builder(exporter, runtime::Tokio).build();
                meter_builder = meter_builder.with_reader(reader);
            }
            Err(err) => {
                eprintln!("Failed to initialize OTLP metric exporter: {err}");
            }
        }
    }
    let meter_provider = meter_builder.build();
    global::set_meter_provider(meter_provider.clone());
    crate::metrics::register(registry);

    // LoggerProvider: OTLP ログ出力が設定された場合のみ有効化する。
    let logger_provider = if otlp_signal_enabled("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT") {
        match opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .build()
        {
            Ok(exporter) => {
                let processor = BatchLogProcessor::builder(exporter, runtime::Tokio).build();
                Some(
                    SdkLoggerProvider::builder()
                        .with_resource(resource)
                        .with_log_processor(processor)
                        .build(),
                )
            }
            Err(err) => {
                eprintln!("Failed to initialize OTLP log exporter: {err}");
                None
            }
        }
    } else {
        None
    };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let otel_trace_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let otel_log_layer = logger_provider
        .as_ref()
        .map(OpenTelemetryTracingBridge::new);

    let fmt_layer = match log_format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .fmt_fields(JsonFields::new())
            .event_format(JsonLogFormat::new(service_name))
            .boxed(),
        LogFormat::Pretty => tracing_subscriber::fmt::layer().boxed(),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(otel_trace_layer)
        .with(otel_log_layer)
        .with(fmt_layer)
        .init();

    TelemetryGuard {
        tracer_provider: Some(tracer_provider),
        meter_provider: Some(meter_provider),
        logger_provider,
    }
}

/// 環境変数 `METRICS_HOST` / `METRICS_PORT` から内部メトリクスサーバーの待ち受けアドレスを解決する。
///
/// `METRICS_PORT` が `0` / `off` / `false` / `disabled` なら `Ok(None)` を返す。
pub async fn metrics_bind_from_env(default_port: u16) -> anyhow::Result<Option<SocketAddr>> {
    let host = std::env::var("METRICS_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let port: u16 = match std::env::var("METRICS_PORT") {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                default_port
            } else if matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "0" | "off" | "false" | "disabled"
            ) {
                return Ok(None);
            } else {
                trimmed
                    .parse()
                    .map_err(|e| anyhow::anyhow!("METRICS_PORT is not a valid port: {e}"))?
            }
        }
        Err(_) => default_port,
    };

    let bind_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let addr = tokio::net::lookup_host((bind_host, port))
        .await
        .map_err(|e| anyhow::anyhow!("METRICS_HOST {host:?} could not be resolved: {e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("METRICS_HOST {host:?} did not resolve"))?;
    Ok(Some(addr))
}

/// Prometheus `/metrics` と `/healthz` を返す内部 HTTP サーバーを起動する。
pub async fn spawn_metrics_server<F>(
    addr: SocketAddr,
    shutdown: F,
) -> anyhow::Result<tokio::task::JoinHandle<()>>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("could not listen metrics server on {addr}: {e}"))?;
    let bound = listener.local_addr()?;
    tracing::info!(addr = %bound, "Metrics server listening");

    let app = Router::new()
        .route("/metrics", get(prometheus_metrics_handler))
        .route("/healthz", get(|| async { "ok" }));

    Ok(tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::error!(error = %err, "Metrics server exited with error");
        }
    }))
}

pub async fn prometheus_metrics_handler() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        crate::metrics::render_prometheus(),
    )
}

/// ルートパターンから高カーディナリティ・秘匿パスを除いたメトリクス/トレース用ルート名を返す。
pub(crate) fn sanitize_route(matched_path: Option<&str>, _raw_path: &str) -> String {
    match matched_path {
        Some(path) if path.starts_with("/api/app-responses/") => {
            "/api/app-responses/[redacted]".to_string()
        }
        Some(path) if path.starts_with("/api/webhooks/") => "/api/webhooks/[redacted]".to_string(),
        Some(path) => path.to_string(),
        None => "unmatched".to_string(),
    }
}

/// WebSocket アップグレードまたは SSE ストリーミング応答かどうかを判定する。
fn is_streaming_response(response: &Response) -> bool {
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        return true;
    }
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("text/event-stream"))
}

/// 標準 HTTP メソッドのみをそのまま返し、任意のカスタムメソッドは `"OTHER"` へ集約する。
///
/// 任意のメソッド名によるメトリクス系列数上限 (`otel_metric_overflow`) やカーディナリティ増加を防ぐ。
pub(crate) fn normalize_http_method(method: &axum::http::Method) -> &'static str {
    match *method {
        axum::http::Method::GET => "GET",
        axum::http::Method::POST => "POST",
        axum::http::Method::PUT => "PUT",
        axum::http::Method::PATCH => "PATCH",
        axum::http::Method::DELETE => "DELETE",
        axum::http::Method::HEAD => "HEAD",
        axum::http::Method::OPTIONS => "OPTIONS",
        axum::http::Method::CONNECT => "CONNECT",
        axum::http::Method::TRACE => "TRACE",
        _ => "OTHER",
    }
}

/// W3C `traceparent` 抽出・HTTP スパン生成・メトリクス計測・構造化完了ログ出力を行うミドルウェア。
pub async fn http_telemetry_middleware(request: Request, next: Next) -> Response {
    let parent_cx =
        global::get_text_map_propagator(|prop| prop.extract(&HeaderExtractor(request.headers())));

    let matched = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string());
    let route = sanitize_route(matched.as_deref(), request.uri().path());
    let method = normalize_http_method(request.method());
    let service = service_name();

    let span = tracing::info_span!(
        "http_request",
        method = %method,
        route = %route,
        otel.name = %format!("{method} {route}"),
        otel.kind = "server",
        http.request.method = %method,
        http.route = %route,
        http.response.status_code = tracing::field::Empty,
    );
    let _ = span.set_parent(parent_cx);

    let _in_flight = crate::metrics::InFlightGuard::enter(service, route.clone());
    let started = Instant::now();
    let response = next.run(request).instrument(span.clone()).await;
    let elapsed = started.elapsed();

    let status = response.status().as_u16();
    span.record("http.response.status_code", status);

    let duration_secs = (!is_streaming_response(&response)).then_some(elapsed.as_secs_f64());
    crate::metrics::record_http_request(service, method, &route, status, duration_secs);

    let duration_ms = (elapsed.as_secs_f64() * 1000.0 * 100.0).round() / 100.0;
    span.in_scope(|| {
        if status >= 500 {
            tracing::error!(
                http_status = status,
                duration_ms,
                error_class = "http_5xx",
                "Request failed"
            );
        } else if status >= 400 {
            tracing::warn!(
                http_status = status,
                duration_ms,
                error_class = "http_4xx",
                "Request client error"
            );
        } else if route == "/healthz" || route == "/metrics" {
            tracing::debug!(http_status = status, duration_ms, "Request completed");
        } else {
            tracing::info!(http_status = status, duration_ms, "Request completed");
        }
    });

    response
}

/// Loki / Grafana Alloy 向けの 1 行 JSON フォーマッタ。
///
/// `trace_id` / `span_id`、親スパンの属性、イベントフィールドをフラットに出力する。
pub struct JsonLogFormat {
    service: &'static str,
}

impl JsonLogFormat {
    pub fn new(service: &'static str) -> Self {
        Self { service }
    }
}

impl<S, N> FormatEvent<S, N> for JsonLogFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        // `Extensions` ロックと競合しないよう、先に現在の OTel コンテキストを取得する。
        // `tracing_opentelemetry::OpenTelemetryLayer::on_enter` が ThreadLocal に
        // アクティブな OTel Context をアタッチするため、`Context::current()` から取得できる。
        let otel_cx = opentelemetry::Context::current();
        let mut span_cx = otel_cx.span().span_context().clone();
        if !span_cx.is_valid() {
            let fallback_cx = tracing::Span::current().context();
            span_cx = fallback_cx.span().span_context().clone();
        }

        let mut obj = Map::new();
        obj.insert(
            "timestamp".into(),
            Value::String(jiff::Timestamp::now().to_string()),
        );
        obj.insert(
            "level".into(),
            Value::String(event.metadata().level().as_str().to_ascii_lowercase()),
        );
        obj.insert("service".into(), Value::String(self.service.to_string()));
        obj.insert(
            "target".into(),
            Value::String(event.metadata().target().to_string()),
        );

        if span_cx.is_valid() {
            obj.insert(
                "trace_id".into(),
                Value::String(span_cx.trace_id().to_string()),
            );
            obj.insert(
                "span_id".into(),
                Value::String(span_cx.span_id().to_string()),
            );
        }

        if let Some(scope) = ctx.event_scope() {
            let mut current_span_name: Option<&'static str> = None;
            let spans: Vec<_> = scope.from_root().collect();
            for span in spans {
                current_span_name = Some(span.name());
                let extensions = span.extensions();
                if let Some(fields) =
                    extensions.get::<tracing_subscriber::fmt::FormattedFields<N>>()
                    && let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&fields.fields)
                {
                    for (k, v) in map {
                        if !is_reserved_log_key(&k) {
                            obj.insert(k, v);
                        }
                    }
                }
            }
            if let Some(name) = current_span_name {
                obj.insert("span".into(), Value::String(name.to_string()));
            }
        }

        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        for (k, v) in visitor.fields {
            if !is_reserved_log_key(&k) {
                obj.insert(k, v);
            }
        }

        let line = serde_json::to_string(&Value::Object(obj)).map_err(|_| fmt::Error)?;
        writeln!(writer, "{line}")
    }
}

fn is_reserved_log_key(key: &str) -> bool {
    matches!(
        key,
        "timestamp" | "level" | "service" | "target" | "trace_id" | "span_id" | "span"
    )
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl Visit for JsonFieldVisitor {
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if let Some(num) = serde_json::Number::from_f64(value) {
            self.fields
                .insert(field.name().to_string(), Value::Number(num));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .insert(field.name().to_string(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), Value::Number(value.into()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), Value::Bool(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.fields
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.fields.insert(
            field.name().to_string(),
            Value::String(format!("{value:?}")),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn parse_log_format_defaults_to_json_only_in_production() {
        assert_eq!(parse_log_format(None, "development"), LogFormat::Pretty);
        assert_eq!(parse_log_format(None, "production"), LogFormat::Json);
        assert_eq!(
            parse_log_format(Some("json"), "development"),
            LogFormat::Json
        );
        assert_eq!(
            parse_log_format(Some("pretty"), "production"),
            LogFormat::Pretty
        );
    }

    #[test]
    fn json_log_format_emits_trace_id_span_id_and_flat_fields() {
        let tracer_provider = SdkTracerProvider::builder().build();
        let tracer = tracer_provider.tracer("test");
        let buf = BufferWriter::default();

        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(
                tracing_subscriber::fmt::layer()
                    .fmt_fields(JsonFields::new())
                    .event_format(JsonLogFormat::new("zoo-test"))
                    .with_writer(buf.clone()),
            );

        tracing::subscriber::with_default(subscriber, || {
            let span =
                tracing::info_span!("http_request", method = "GET", route = "/api/channels/{id}");
            let _guard = span.enter();
            tracing::info!(
                http_status = 200u16,
                duration_ms = 12.5f64,
                "Request completed"
            );
        });

        let output = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).expect("valid json line");

        assert_eq!(parsed["level"], "info");
        assert_eq!(parsed["service"], "zoo-test");
        assert_eq!(parsed["span"], "http_request");
        assert_eq!(parsed["method"], "GET");
        assert_eq!(parsed["route"], "/api/channels/{id}");
        assert_eq!(parsed["http_status"], 200);
        assert_eq!(parsed["duration_ms"], 12.5);
        assert_eq!(parsed["message"], "Request completed");

        let trace_id = parsed["trace_id"].as_str().expect("trace_id string");
        let span_id = parsed["span_id"].as_str().expect("span_id string");
        assert_eq!(trace_id.len(), 32);
        assert_eq!(span_id.len(), 16);
        assert!(trace_id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(span_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sanitize_route_redacts_secrets_and_collapses_unmatched() {
        assert_eq!(
            sanitize_route(Some("/api/channels/{id}"), "/api/channels/12345"),
            "/api/channels/{id}"
        );
        assert_eq!(
            sanitize_route(
                Some("/api/app-responses/{token}"),
                "/api/app-responses/secret-token"
            ),
            "/api/app-responses/[redacted]"
        );
        assert_eq!(
            sanitize_route(Some("/api/webhooks/{token}"), "/api/webhooks/secret-token"),
            "/api/webhooks/[redacted]"
        );
        assert_eq!(
            sanitize_route(None, "/api/webhooks/secret-token"),
            "unmatched"
        );
        assert_eq!(sanitize_route(None, "/nonexistent/404/path"), "unmatched");
    }

    #[test]
    fn is_streaming_response_detects_ws_and_sse() {
        let ws = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(is_streaming_response(&ws));

        let sse = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(is_streaming_response(&sse));

        let json = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::empty())
            .unwrap();
        assert!(!is_streaming_response(&json));
    }

    #[test]
    fn w3c_traceparent_propagates_parent_trace_id_to_child_span() {
        use opentelemetry::propagation::TextMapPropagator;

        let propagator = TraceContextPropagator::new();
        let mut headers = axum::http::HeaderMap::new();
        let expected_trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
        headers.insert(
            "traceparent",
            format!("00-{expected_trace_id}-00f067aa0ba902b7-01")
                .parse()
                .unwrap(),
        );
        let parent_cx = propagator.extract(&HeaderExtractor(&headers));

        let tracer_provider = SdkTracerProvider::builder().build();
        let tracer = tracer_provider.tracer("test");
        let buf = BufferWriter::default();

        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(
                tracing_subscriber::fmt::layer()
                    .fmt_fields(JsonFields::new())
                    .event_format(JsonLogFormat::new("zoo-test"))
                    .with_writer(buf.clone()),
            );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("http_request", route = "/healthz");
            let _ = span.set_parent(parent_cx);
            let _guard = span.enter();
            tracing::info!("Child event");
        });

        let output = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).expect("valid json line");
        assert_eq!(parsed["trace_id"], expected_trace_id);
        assert_ne!(parsed["span_id"], "00f067aa0ba902b7");
    }

    #[test]
    fn normalize_http_method_allows_standard_methods_and_collapses_unknown() {
        use axum::http::Method;

        for (raw, expected) in [
            ("GET", "GET"),
            ("POST", "POST"),
            ("PUT", "PUT"),
            ("PATCH", "PATCH"),
            ("DELETE", "DELETE"),
            ("HEAD", "HEAD"),
            ("OPTIONS", "OPTIONS"),
            ("CONNECT", "CONNECT"),
            ("TRACE", "TRACE"),
            ("PROPFIND", "OTHER"),
            ("CUSTOM_METHOD", "OTHER"),
        ] {
            let method = Method::from_bytes(raw.as_bytes()).unwrap();
            assert_eq!(normalize_http_method(&method), expected);
        }
    }

    #[tokio::test]
    async fn custom_http_methods_do_not_overflow_metrics_or_drop_get() {
        use axum::http::Method;
        use tower::ServiceExt;

        let registry = Registry::new();
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .scope_info_enabled(false)
            .without_units()
            .build()
            .unwrap();
        let meter_provider = SdkMeterProvider::builder().with_reader(exporter).build();
        global::set_meter_provider(meter_provider.clone());
        crate::metrics::register(registry);

        let app = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(http_telemetry_middleware));

        for i in 0..2005 {
            let method = Method::from_bytes(format!("M{i:04}").as_bytes()).unwrap();
            let req = axum::http::Request::builder()
                .method(method)
                .uri("/healthz")
                .body(axum::body::Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        }

        let get_req = axum::http::Request::builder()
            .method(Method::GET)
            .uri("/healthz")
            .body(axum::body::Body::empty())
            .unwrap();
        let get_resp = app.oneshot(get_req).await.unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);

        let prom = crate::metrics::render_prometheus();
        assert!(
            !prom.contains("otel_metric_overflow"),
            "unexpected metric overflow in prometheus output:\n{prom}"
        );
        assert!(
            prom.contains(
                r#"zoo_http_requests_total{method="OTHER",route="/healthz",service="zoo-backend",status_class="4xx"} 2005"#
            ),
            "expected unknown methods to be aggregated under OTHER:\n{prom}"
        );
        assert!(
            prom.contains(
                r#"zoo_http_requests_total{method="GET",route="/healthz",service="zoo-backend",status_class="2xx"} 1"#
            ),
            "expected normal GET /healthz series to remain recorded:\n{prom}"
        );

        let _ = meter_provider.shutdown();
    }
}
