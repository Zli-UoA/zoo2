//! zoo-dispatcher: App の HTTP イベント配信だけを行う独立プロセス。
//!
//! API・認証・S3 の設定は要らない（`DATABASE_URL` だけで動く）。配信ロジックは
//! `backend::apps::delivery` を backend と共有する。複数台・backend のワーカーとの
//! 同時稼働は `FOR UPDATE SKIP LOCKED` とインストール単位の advisory lock で安全。
//!
//! 環境変数: `DATABASE_URL`（または `POSTGRES_*`）、`DISPATCHER_PORT`（既定 8081）、
//! `DISPATCHER_HOST`（既定 0.0.0.0）。
use axum::{Router, http::header, response::IntoResponse, routing::get};
use std::{net::SocketAddr, time::Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// SIGTERM 後に、送信中の配信を待つ最大時間。残ったリースは期限切れで他のインスタンスが拾う。
/// 1 回の送信は最大 3 秒なので、claim と ACK の分の余裕を持たせる（Pod の
/// terminationGracePeriodSeconds はこれより長くする）。
/// 通知が無くても配信を探す間隔。
const POLL: Duration = Duration::from_secs(3);

const DRAIN: Duration = Duration::from_secs(8);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=info,zoo_dispatcher=info,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = backend::config::database_url_from_env();
    let host = std::env::var("DISPATCHER_HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("DISPATCHER_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8081);

    let db = backend::db::init_db(&database_url).await?;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    // LISTEN で即時に起き、通知の取りこぼしやバックオフ明けは POLL ごとのポーリングで拾う。
    let wake = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify_listener =
        backend::apps::delivery::listen_for_deliveries(database_url.clone(), wake.clone());
    let http = tokio::spawn(backend::apps::delivery::http_loop_with(
        db.clone(),
        shutdown,
        wake,
        POLL,
    ));
    let gauges = tokio::spawn({
        let db = db.clone();
        async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(5));
            loop {
                ticks.tick().await;
                if let Err(e) = backend::apps::delivery::refresh_queue_metrics(&db).await {
                    tracing::warn!(error=%e,"Queue metrics refresh failed");
                }
            }
        }
    });

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(metrics));
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    tracing::info!(%addr, "zoo-dispatcher listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // 停止信号を受けたらすぐ新しいリースを取らなくし（HTTP サーバーの終了を待たない）、
    // 送信中の分を待つ。
    let stop_on_signal = stop.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = stop_on_signal.send(true);
        })
        .await?;
    let _ = stop.send(true);
    gauges.abort();
    notify_listener.abort();
    let mut http = http;
    if tokio::time::timeout(DRAIN, &mut http).await.is_err() {
        tracing::warn!("Drain timed out; unfinished leases will expire and be retried");
        http.abort();
    }
    tracing::info!("zoo-dispatcher stopped");
    Ok(())
}

async fn metrics() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        backend::apps::dispatch_metrics::global().render(),
    )
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}
