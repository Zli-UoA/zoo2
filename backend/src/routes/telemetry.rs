//! フロントエンドからの構造化テレメトリ取り込み (`POST /api/telemetry`)。
//!
//! ブラウザ入力を信頼せず、ボディサイズ上限・バッチ件数上限・IP 単位のレート制限・
//! 閉じた列挙型/正規化ルート・メッセージサニタイズを通してから
//! 構造化ログ（Loki 向け）と Prometheus メトリクスへ記録する。

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Request},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::Deserialize;

use crate::{activitypub::client_ip, error::AppError};

pub const MAX_BODY_BYTES: usize = 32 * 1024;
pub const MAX_BATCH_EVENTS: usize = 20;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
const RATE_LIMIT_MAX_REQUESTS: u32 = 30;
const MAX_TRACKED_IPS: usize = 4096;
const MAX_MESSAGE_CHARS: usize = 200;

static RATE_LIMITER: OnceLock<TelemetryRateLimiter> = OnceLock::new();

pub fn router() -> Router<crate::state::AppState> {
    Router::new().route(
        "/api/telemetry",
        post(ingest_telemetry).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
    )
}

#[derive(Debug, Deserialize)]
pub struct TelemetryBatchRequest {
    pub events: Vec<FrontendTelemetryEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FrontendTelemetryEvent {
    Error {
        category: ErrorCategory,
        #[serde(default)]
        route: Option<String>,
        #[serde(default)]
        http_status: Option<u16>,
        #[serde(default)]
        duration_ms: Option<f64>,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        trace_id: Option<String>,
        #[serde(default)]
        span_id: Option<String>,
    },
    WebVital {
        metric: WebVitalMetric,
        value_ms: f64,
        #[serde(default)]
        rating: Option<WebVitalRating>,
        #[serde(default)]
        route: Option<String>,
        #[serde(default)]
        trace_id: Option<String>,
        #[serde(default)]
        span_id: Option<String>,
    },
    ElectricSync {
        shape: ElectricShape,
        state: ElectricSyncState,
        #[serde(default)]
        http_status: Option<u16>,
        #[serde(default)]
        route: Option<String>,
        #[serde(default)]
        trace_id: Option<String>,
        #[serde(default)]
        span_id: Option<String>,
    },
    Webrtc {
        state: WebrtcEventState,
        #[serde(default)]
        candidate_type: Option<IceCandidateType>,
        #[serde(default)]
        rtt_ms: Option<f64>,
        #[serde(default)]
        loss_ratio: Option<f64>,
        #[serde(default)]
        trace_id: Option<String>,
        #[serde(default)]
        span_id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Runtime,
    UnhandledRejection,
    ReactBoundary,
    Api,
    Electric,
    Webrtc,
}

impl ErrorCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::UnhandledRejection => "unhandled_rejection",
            Self::ReactBoundary => "react_boundary",
            Self::Api => "api",
            Self::Electric => "electric",
            Self::Webrtc => "webrtc",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebVitalMetric {
    Lcp,
    Inp,
    Ttfb,
    Fcp,
}

impl WebVitalMetric {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lcp => "lcp",
            Self::Inp => "inp",
            Self::Ttfb => "ttfb",
            Self::Fcp => "fcp",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebVitalRating {
    Good,
    NeedsImprovement,
    Poor,
}

impl WebVitalRating {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::NeedsImprovement => "needs_improvement",
            Self::Poor => "poor",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ElectricShape {
    Channels,
    Users,
    ChannelUsers,
    Messages,
    MessageReactions,
    Notifications,
    Other,
}

impl ElectricShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Channels => "channels",
            Self::Users => "users",
            Self::ChannelUsers => "channel_users",
            Self::Messages => "messages",
            Self::MessageReactions => "message_reactions",
            Self::Notifications => "notifications",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ElectricSyncState {
    Error,
    Stalled,
    Reconnected,
    OfflineFallback,
}

impl ElectricSyncState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Stalled => "stalled",
            Self::Reconnected => "reconnected",
            Self::OfflineFallback => "offline_fallback",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebrtcEventState {
    IceFailed,
    ConnectionFailed,
    HeartbeatTimeout,
    PoorQuality,
    Recovered,
}

impl WebrtcEventState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IceFailed => "ice_failed",
            Self::ConnectionFailed => "connection_failed",
            Self::HeartbeatTimeout => "heartbeat_timeout",
            Self::PoorQuality => "poor_quality",
            Self::Recovered => "recovered",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IceCandidateType {
    Host,
    Srflx,
    Prflx,
    Relay,
    Unknown,
}

impl IceCandidateType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Srflx => "srflx",
            Self::Prflx => "prflx",
            Self::Relay => "relay",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
enum IpBucket {
    Addr(IpAddr),
    Unknown,
}

struct WindowState {
    started_at: Instant,
    count: u32,
}

struct TelemetryRateLimiter {
    window: Duration,
    max_requests: u32,
    buckets: Mutex<HashMap<IpBucket, WindowState>>,
}

impl TelemetryRateLimiter {
    fn new(window: Duration, max_requests: u32) -> Self {
        Self {
            window,
            max_requests,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, key: IpBucket, now: Instant) -> bool {
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if map.len() >= MAX_TRACKED_IPS && !map.contains_key(&key) {
            map.retain(|_, v| now.duration_since(v.started_at) < self.window);
            if map.len() >= MAX_TRACKED_IPS {
                map.clear();
            }
        }
        let entry = map.entry(key).or_insert(WindowState {
            started_at: now,
            count: 0,
        });
        if now.duration_since(entry.started_at) >= self.window {
            entry.started_at = now;
            entry.count = 0;
        }
        if entry.count >= self.max_requests {
            return false;
        }
        entry.count += 1;
        true
    }
}

fn resolve_rate_limit_key(peer: Option<IpAddr>, headers: &HeaderMap) -> Result<IpBucket, AppError> {
    let tunnel = matches!(
        std::env::var("ACTIVITYPUB_CLOUDFLARE_TUNNEL").as_deref(),
        Ok("true" | "1")
    );
    if !tunnel && peer.is_none() {
        return Ok(IpBucket::Unknown);
    }
    let resolved = client_ip::resolve(peer, headers, tunnel)?;
    Ok(resolved.map_or(IpBucket::Unknown, IpBucket::Addr))
}

fn is_json_content_type(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|essence| essence.eq_ignore_ascii_case("application/json"))
}

async fn ingest_telemetry(request: Request) -> Result<Response, AppError> {
    if !is_json_content_type(request.headers()) {
        return Ok((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "Content-Type must be application/json" })),
        )
            .into_response());
    }

    let peer = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|v| v.0.ip());
    let key = resolve_rate_limit_key(peer, request.headers())?;

    let limiter = RATE_LIMITER
        .get_or_init(|| TelemetryRateLimiter::new(RATE_LIMIT_WINDOW, RATE_LIMIT_MAX_REQUESTS));
    if !limiter.check(key, Instant::now()) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            [
                (header::RETRY_AFTER, "60"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            Json(serde_json::json!({ "error": "Rate limit exceeded" })),
        )
            .into_response());
    }

    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES).await else {
        return Ok((
            StatusCode::PAYLOAD_TOO_LARGE,
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "Telemetry payload too large" })),
        )
            .into_response());
    };

    let payload: TelemetryBatchRequest = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Invalid telemetry payload".to_string()))?;

    record_batch(&payload)?;

    Ok((StatusCode::ACCEPTED, [(header::CACHE_CONTROL, "no-store")]).into_response())
}

pub(crate) fn record_batch(payload: &TelemetryBatchRequest) -> Result<(), AppError> {
    if payload.events.len() > MAX_BATCH_EVENTS {
        return Err(AppError::BadRequest(format!(
            "Telemetry batch exceeds limit of {MAX_BATCH_EVENTS} events"
        )));
    }

    for event in &payload.events {
        record_single_event(event);
    }

    Ok(())
}

fn record_single_event(event: &FrontendTelemetryEvent) {
    match event {
        FrontendTelemetryEvent::Error {
            category,
            route,
            http_status,
            duration_ms,
            message,
            trace_id,
            span_id,
        } => {
            let route_group = normalize_route_group(route.as_deref());
            let category_str = category.as_str();
            crate::metrics::record_frontend_error(category_str, route_group);

            let sanitized_msg = sanitize_message(message.as_deref());
            let valid_trace_id = sanitize_trace_id(trace_id.as_deref());
            let valid_span_id = sanitize_span_id(span_id.as_deref());
            let bounded_duration =
                duration_ms.filter(|d| d.is_finite() && *d >= 0.0 && *d <= 3_600_000.0);

            tracing::warn!(
                target: "zoo_frontend",
                event = "frontend_error",
                category = category_str,
                route = route_group,
                http_status = http_status.unwrap_or(0),
                duration_ms = bounded_duration.unwrap_or(0.0),
                error_class = category_str,
                frontend_trace_id = valid_trace_id.unwrap_or(""),
                frontend_span_id = valid_span_id.unwrap_or(""),
                error_summary = sanitized_msg.as_deref().unwrap_or(""),
                "Frontend error reported"
            );
        }
        FrontendTelemetryEvent::WebVital {
            metric,
            value_ms,
            rating,
            route,
            trace_id,
            span_id,
        } => {
            if !value_ms.is_finite() || *value_ms < 0.0 || *value_ms > 600_000.0 {
                return;
            }
            let route_group = normalize_route_group(route.as_deref());
            let metric_str = metric.as_str();
            crate::metrics::record_frontend_web_vital(metric_str, route_group, *value_ms / 1000.0);

            let valid_trace_id = sanitize_trace_id(trace_id.as_deref());
            let valid_span_id = sanitize_span_id(span_id.as_deref());
            tracing::info!(
                target: "zoo_frontend",
                event = "frontend_web_vital",
                metric = metric_str,
                rating = rating.map_or("unknown", WebVitalRating::as_str),
                route = route_group,
                duration_ms = *value_ms,
                frontend_trace_id = valid_trace_id.unwrap_or(""),
                frontend_span_id = valid_span_id.unwrap_or(""),
                "Frontend Web Vital recorded"
            );
        }
        FrontendTelemetryEvent::ElectricSync {
            shape,
            state,
            http_status,
            route,
            trace_id,
            span_id,
        } => {
            let shape_str = shape.as_str();
            let state_str = state.as_str();
            let route_group = normalize_route_group(route.as_deref());
            crate::metrics::record_frontend_electric_sync(shape_str, state_str);

            let valid_trace_id = sanitize_trace_id(trace_id.as_deref());
            let valid_span_id = sanitize_span_id(span_id.as_deref());
            tracing::warn!(
                target: "zoo_frontend",
                event = "frontend_electric_sync",
                shape = shape_str,
                state = state_str,
                route = route_group,
                http_status = http_status.unwrap_or(0),
                error_class = "electric_sync",
                frontend_trace_id = valid_trace_id.unwrap_or(""),
                frontend_span_id = valid_span_id.unwrap_or(""),
                "Frontend Electric sync event"
            );
        }
        FrontendTelemetryEvent::Webrtc {
            state,
            candidate_type,
            rtt_ms,
            loss_ratio,
            trace_id,
            span_id,
        } => {
            let state_str = state.as_str();
            crate::metrics::record_frontend_webrtc(state_str);

            let bounded_rtt = rtt_ms.filter(|v| v.is_finite() && *v >= 0.0 && *v <= 60_000.0);
            let bounded_loss = loss_ratio.filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0);
            let valid_trace_id = sanitize_trace_id(trace_id.as_deref());
            let valid_span_id = sanitize_span_id(span_id.as_deref());

            tracing::warn!(
                target: "zoo_frontend",
                event = "frontend_webrtc",
                state = state_str,
                candidate_type = candidate_type.map_or("unknown", IceCandidateType::as_str),
                rtt_ms = bounded_rtt.unwrap_or(0.0),
                loss_ratio = bounded_loss.unwrap_or(0.0),
                error_class = "webrtc",
                frontend_trace_id = valid_trace_id.unwrap_or(""),
                frontend_span_id = valid_span_id.unwrap_or(""),
                "Frontend WebRTC event"
            );
        }
    }
}

/// URL パスから低カーディナリティの画面グループ名を返す。ID やクエリは一切保持しない。
pub(crate) fn normalize_route_group(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw else {
        return "other";
    };
    let path = raw.split(['?', '#']).next().unwrap_or_default().trim();
    if path.is_empty() || path == "/" {
        return "/";
    }
    if path.starts_with("/channels") {
        "/channels"
    } else if path.starts_with("/home") {
        "/home"
    } else if path.starts_with("/dms") {
        "/dms"
    } else if path.starts_with("/notifications") {
        "/notifications"
    } else if path.starts_with("/settings") {
        "/settings"
    } else if path.starts_with("/public") {
        "/public"
    } else if path.starts_with("/auth") {
        "/auth"
    } else if path.starts_with("/apps") {
        "/apps"
    } else {
        "other"
    }
}

fn is_valid_hex_id(raw: &str, expected_len: usize) -> bool {
    raw.len() == expected_len
        && raw.bytes().all(|b| b.is_ascii_hexdigit())
        && raw.bytes().any(|b| b != b'0')
}

pub(crate) fn sanitize_trace_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|v| is_valid_hex_id(v, 32))
}

pub(crate) fn sanitize_span_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|v| is_valid_hex_id(v, 16))
}

/// 自由記述のエラー要約から制御文字・Bearer トークン・URL クエリを除去し長さを制限する。
pub(crate) fn sanitize_message(raw: Option<&str>) -> Option<String> {
    let text = raw?;
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut tokens: Vec<String> = Vec::new();
    let mut redact_next = false;
    for part in trimmed.split_whitespace() {
        if redact_next {
            tokens.push("[redacted]".to_string());
            redact_next = false;
            continue;
        }
        if part.eq_ignore_ascii_case("bearer") {
            tokens.push("Bearer".to_string());
            redact_next = true;
            continue;
        }
        if let Some((base, _query)) = part.split_once('?')
            && (base.starts_with("http://")
                || base.starts_with("https://")
                || base.starts_with('/'))
        {
            tokens.push(format!("{base}?[redacted]"));
            continue;
        }
        tokens.push(part.to_string());
    }

    let joined = tokens.join(" ");
    let truncated: String = joined.chars().take(MAX_MESSAGE_CHARS).collect();
    Some(truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    #[test]
    fn normalize_route_group_collapses_ids_and_queries() {
        assert_eq!(normalize_route_group(None), "other");
        assert_eq!(normalize_route_group(Some("/")), "/");
        assert_eq!(
            normalize_route_group(Some("/channels/0193a2f1-secret?msg=1#frag")),
            "/channels"
        );
        assert_eq!(
            normalize_route_group(Some("/public/users/alice-123")),
            "/public"
        );
        assert_eq!(
            normalize_route_group(Some("/auth/callback?code=xyz")),
            "/auth"
        );
        assert_eq!(normalize_route_group(Some("/unknown/page")), "other");
    }

    #[test]
    fn sanitize_trace_and_span_ids_reject_invalid_or_zero_values() {
        assert_eq!(
            sanitize_trace_id(Some("4bf92f3577b34da6a3ce929d0e0e4736")),
            Some("4bf92f3577b34da6a3ce929d0e0e4736")
        );
        assert_eq!(
            sanitize_trace_id(Some("00000000000000000000000000000000")),
            None
        );
        assert_eq!(sanitize_trace_id(Some("not-hex")), None);
        assert_eq!(
            sanitize_span_id(Some("00f067aa0ba902b7")),
            Some("00f067aa0ba902b7")
        );
        assert_eq!(sanitize_span_id(Some("0000000000000000")), None);
    }

    #[test]
    fn sanitize_message_masks_bearer_tokens_queries_and_truncates() {
        let msg = sanitize_message(Some(
            "Request failed\nBearer super-secret-jwt-token https://example.com/api/shapes/messages?secret=abc",
        ))
        .unwrap();
        assert!(!msg.contains("super-secret-jwt-token"));
        assert!(!msg.contains("secret=abc"));
        assert!(!msg.contains('\n'));
        assert!(msg.contains("Bearer [redacted]"));
        assert!(msg.contains("https://example.com/api/shapes/messages?[redacted]"));

        let long = "a".repeat(300);
        assert_eq!(
            sanitize_message(Some(&long)).unwrap().chars().count(),
            MAX_MESSAGE_CHARS
        );
    }

    #[test]
    fn record_batch_rejects_oversized_batches() {
        let events = (0..=MAX_BATCH_EVENTS)
            .map(|_| FrontendTelemetryEvent::Webrtc {
                state: WebrtcEventState::Recovered,
                candidate_type: None,
                rtt_ms: None,
                loss_ratio: None,
                trace_id: None,
                span_id: None,
            })
            .collect();
        let err = record_batch(&TelemetryBatchRequest { events }).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn rate_limiter_enforces_window_and_resets_after_expiry() {
        let limiter = TelemetryRateLimiter::new(Duration::from_secs(60), 2);
        let ip = IpBucket::Addr("203.0.113.1".parse().unwrap());
        let start = Instant::now();

        assert!(limiter.check(ip, start));
        assert!(limiter.check(ip, start + Duration::from_secs(10)));
        assert!(!limiter.check(ip, start + Duration::from_secs(20)));
        assert!(limiter.check(ip, start + Duration::from_secs(61)));
    }

    #[tokio::test]
    async fn ingest_telemetry_accepts_valid_payload_and_rejects_oversized_body() {
        let app = Router::new().route(
            "/api/telemetry",
            post(ingest_telemetry).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        );

        let valid_body = serde_json::json!({
            "events": [
                {
                    "kind": "error",
                    "category": "api",
                    "route": "/channels/123",
                    "http_status": 502,
                    "duration_ms": 42.5,
                    "message": "Bad gateway at /api/messages?token=abc",
                    "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
                    "span_id": "00f067aa0ba902b7"
                },
                {
                    "kind": "web_vital",
                    "metric": "lcp",
                    "value_ms": 1200.0,
                    "rating": "good",
                    "route": "/home"
                }
            ]
        });

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/telemetry")
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .body(Body::from(valid_body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let req_plain = axum::http::Request::builder()
            .method("POST")
            .uri("/api/telemetry")
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(valid_body.to_string()))
            .unwrap();
        let resp_plain = app.clone().oneshot(req_plain).await.unwrap();
        assert_eq!(resp_plain.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let req_missing_ct = axum::http::Request::builder()
            .method("POST")
            .uri("/api/telemetry")
            .body(Body::from(valid_body.to_string()))
            .unwrap();
        let resp_missing_ct = app.clone().oneshot(req_missing_ct).await.unwrap();
        assert_eq!(resp_missing_ct.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let oversized = "x".repeat(MAX_BODY_BYTES + 1024);
        let req_big = axum::http::Request::builder()
            .method("POST")
            .uri("/api/telemetry")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(oversized))
            .unwrap();
        let resp_big = app.oneshot(req_big).await.unwrap();
        assert_eq!(resp_big.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
