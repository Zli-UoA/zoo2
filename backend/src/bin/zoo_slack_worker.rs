//! zoo-slack-worker: Slack の取り込み（解析・取り込み・添付の取得）と差分同期だけを行う独立プロセス。
//!
//! backend と同じイメージの別バイナリ。backend のローリング更新や再起動に巻き込まれずに、長い取り込みを
//! 進められるようにする（`zoo_dispatcher` と同じ作り）。HTTP の API は持たず、`/healthz` だけを返す。
//! 設定は backend と同じ環境変数（`DATABASE_URL`（または `POSTGRES_*`）、`S3_*`、`SLACK_*`、`ENVIRONMENT` など）。
//! `Config::validate` は backend と同じ検査をする（本番では OIDC の設定も必要）。
//!
//! 環境変数: `SLACK_WORKER_PORT`（既定 8082）、`SLACK_WORKER_HOST`（既定 0.0.0.0）、`SLACK_IMPORT_TMP_DIR`
//! （ZIP を置く、書き込めるディレクトリ。Kubernetes では `emptyDir` を使う）。
//!
//! backend 側は `SLACK_IMPORT_WORKER=false` で、取り込みと差分同期のワーカーを止める。専用ワーカーと backend の
//! ワーカーが同時に動いても、ジョブは `FOR UPDATE SKIP LOCKED` とリースで 1 台だけが進める（配備の途中で
//! 両方が動く期間があっても安全）。
//!
//! SIGTERM を受けたら、処理中のジョブを手放し（`attempts` は消費しない）、`DRAIN` の間だけ待って終了する
//! （Pod の terminationGracePeriodSeconds はこれより長くする）。
use axum::{Router, routing::get};
use std::{net::SocketAddr, time::Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// SIGTERM 後に、ジョブを手放すのを待つ最大時間。残ったリースは、期限切れで他のワーカーが拾う
const DRAIN: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=info,zoo_slack_worker=info,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = backend::config::Config::from_env();
    if let Err(err) = config.validate() {
        tracing::error!("Configuration validation error: {err}");
        anyhow::bail!("Configuration error: {err}");
    }

    let host = std::env::var("SLACK_WORKER_HOST").unwrap_or_else(|_| "0.0.0.0".into());
    // 設定ミスは握りつぶさず起動時に失敗させる。ワーカーを動かす前に失敗しないと、ジョブを取ってから
    // プロセスが終わり、再起動のたびに試行回数を消費する
    let port: u16 = match std::env::var("SLACK_WORKER_PORT") {
        Ok(v) if !v.trim().is_empty() => v
            .trim()
            .parse()
            .map_err(|e| anyhow::anyhow!("SLACK_WORKER_PORT is not a valid port: {e}"))?,
        _ => 8082,
    };
    if port == 0 {
        anyhow::bail!("SLACK_WORKER_PORT must not be 0");
    }
    let bind_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let addr: SocketAddr = tokio::net::lookup_host((bind_host, port))
        .await
        .map_err(|e| anyhow::anyhow!("SLACK_WORKER_HOST {host:?} could not be resolved: {e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("SLACK_WORKER_HOST {host:?} did not resolve"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("could not listen on {addr}: {e}"))?;

    let db = backend::db::init_db(&config.database_url).await?;
    // 通話のレジストリの LISTEN 接続など、ワーカーが使わないものは起動しない
    let state = backend::state::AppState::new_for_worker(db, config);

    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    // 取り込みのワーカーが（panic などで）終わったら /healthz を 503 にして、liveness probe で再起動させる
    let import = tokio::spawn({
        let guard = AliveGuard(alive.clone());
        let state = state.clone();
        async move {
            let _guard = guard;
            backend::slack::worker::worker_until(state, stop_rx).await
        }
    });
    let sync = tokio::spawn({
        let guard = AliveGuard(alive.clone());
        let state = state.clone();
        async move {
            let _guard = guard;
            backend::slack::sync::worker(state).await
        }
    });

    let app = Router::new().route(
        "/healthz",
        get(move || {
            let alive = alive.clone();
            async move {
                if alive.load(std::sync::atomic::Ordering::Relaxed) {
                    (axum::http::StatusCode::OK, "ok")
                } else {
                    (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        "worker stopped",
                    )
                }
            }
        }),
    );
    tracing::info!(addr=%listener.local_addr()?, "zoo-slack-worker listening");
    // 合図を受けたら、HTTP サーバーの終了を待たずに、ジョブを手放し始める
    let stop_on_signal = stop.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = stop_on_signal.send(true);
        })
        .await?;
    let _ = stop.send(true);
    sync.abort();
    let mut import = import;
    if tokio::time::timeout(DRAIN, &mut import).await.is_err() {
        tracing::warn!("Drain timed out; the lease will expire and the job will be retried");
        import.abort();
    }
    tracing::info!("zoo-slack-worker stopped");
    Ok(())
}

/// ワーカーのタスクが終わったら（panic で巻き戻る場合も）生存フラグを下げる。
struct AliveGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }
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
