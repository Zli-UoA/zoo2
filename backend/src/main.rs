use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use backend::{config::Config, db::init_db, routes::create_router, state::AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ロギング初期化 (RUST_LOG=info など)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=debug,tower_http=debug,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 設定読み込み
    let config = Config::from_env();
    if let Err(err) = config.validate() {
        tracing::error!("Configuration validation error: {err}");
        anyhow::bail!("Configuration error: {err}");
    }

    tracing::info!(
        host = %config.host,
        port = %config.port,
        "Starting backend service"
    );

    backend::push::PushConfig::from_env()?;
    if std::env::var("ACTIVITYPUB_BASE_URL").is_ok_and(|v| !v.is_empty()) {
        backend::activitypub::base_url()?;
    }

    // データベース接続初期化 (Toasty ORM)
    let db = init_db(&config.database_url).await?;

    // アプリケーション状態構築
    let state = AppState::new(db, config.clone());

    // 開発・ステージング環境の場合、S3 バケットの存在を確認し必要に応じて作成
    if config.environment != "production" {
        backend::storage::ensure_bucket_exists(&state.s3_client, &config.s3.bucket).await;
    }

    // ルーター構築
    let delivery_worker = tokio::spawn(backend::apps::delivery::worker(state.clone()));
    let federation_worker = tokio::spawn(backend::activitypub::worker(state.clone()));
    let app = create_router(state);

    // サーバーバインド
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    tracing::info!("Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    federation_worker.abort();
    delivery_worker.abort();
    tracing::info!("Server shut down gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, starting graceful shutdown...");
}
