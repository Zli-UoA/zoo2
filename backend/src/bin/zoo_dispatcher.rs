//! zoo-dispatcher: App の HTTP イベント配信だけを行う独立プロセス。
//!
//! API・認証・S3 の設定は要らない（`DATABASE_URL` だけで動く）。配信ロジックは
//! `backend::apps::delivery` を backend と共有する。複数台・backend のワーカーとの
//! 同時稼働は `FOR UPDATE SKIP LOCKED` とインストール単位の advisory lock で安全。
//!
//! 環境変数: `DATABASE_URL`（または `POSTGRES_*`）、`DISPATCHER_PORT`（既定 8081）、
//! `DISPATCHER_HOST`（既定 0.0.0.0）。
//!
//! `DATABASE_URL` は直接接続にすること。LISTEN（#378 以降）はトランザクションプールのプーラー
//! （PgBouncer のトランザクションモードなど）を経由すると動かない（配信はポーリングで続く）。
//! DB プールの上限は `DB_MAX_POOL_SIZE`（未設定ならドライバの既定: 論理 CPU 数 × 2）。
use axum::{Router, http::header, response::IntoResponse, routing::get};
use std::{net::SocketAddr, time::Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// 通知が無くても配信を探す間隔。
const POLL: Duration = Duration::from_secs(3);

/// SIGTERM 後に、送信中の配信を待つ最大時間。残ったリースは期限切れで他のインスタンスが拾う。
/// 1 回の送信は最大 3 秒なので、claim と ACK の分の余裕を持たせる（Pod の
/// terminationGracePeriodSeconds はこれより長くする）。
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
    // 設定ミスは握りつぶさず起動時に失敗させる（別のポートで待ち受けたまま probe が失敗しない）。
    let port: u16 = match std::env::var("DISPATCHER_PORT") {
        Ok(v) if !v.trim().is_empty() => v
            .trim()
            .parse()
            .map_err(|e| anyhow::anyhow!("DISPATCHER_PORT is not a valid port: {e}"))?,
        _ => 8081,
    };
    if port == 0 {
        anyhow::bail!("DISPATCHER_PORT must not be 0");
    }
    // 待ち受けの設定ミス（解決できないホスト、使用中のポート）は、配信ループを動かす前に失敗させる。
    // 先に配信を取ってからプロセスが終わると、再起動のたびに試行回数を消費して配信を遅らせるため。
    // ホスト名（localhost など）と IPv6（`::`）も受ける。`[::]` のように角括弧付きでもよい。
    let bind_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let addr: SocketAddr = tokio::net::lookup_host((bind_host, port))
        .await
        .map_err(|e| anyhow::anyhow!("DISPATCHER_HOST {host:?} could not be resolved: {e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("DISPATCHER_HOST {host:?} did not resolve"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("could not listen on {addr}: {e}"))?;

    let db = backend::db::init_db(&database_url).await?;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    // LISTEN で即時に起き、通知の取りこぼしやバックオフ明けは POLL ごとのポーリングで拾う。
    let wake = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify_listener =
        backend::apps::delivery::listen_for_deliveries(database_url.clone(), wake.clone());
    // 配信ループが（panic などで）終わったら /healthz を 503 にして、liveness probe で再起動させる。
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let http = tokio::spawn({
        let guard = AliveGuard(alive.clone());
        let db = db.clone();
        async move {
            let _guard = guard;
            backend::apps::delivery::http_loop_with(db, shutdown, wake, POLL).await
        }
    });
    let gauges = tokio::spawn({
        let db = db.clone();
        async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(15));
            loop {
                ticks.tick().await;
                if let Err(e) = backend::apps::delivery::refresh_queue_metrics(&db).await {
                    tracing::warn!(error=%e,"Queue metrics refresh failed");
                }
            }
        }
    });

    let app = Router::new()
        .route(
            "/healthz",
            get(move || {
                let alive = alive.clone();
                async move {
                    if alive.load(std::sync::atomic::Ordering::Relaxed) {
                        (axum::http::StatusCode::OK, "ok")
                    } else {
                        (
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            "delivery loop stopped",
                        )
                    }
                }
            }),
        )
        .route("/metrics", get(metrics));
    tracing::info!(addr=%listener.local_addr()?, "zoo-dispatcher listening");
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

/// 配信ループのタスクが終わったら（panic で巻き戻る場合も）生存フラグを下げる。
struct AliveGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }
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
