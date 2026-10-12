pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod routes;
pub mod state;

use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{config::Config, db::init_db, routes::create_router, state::AppState};

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
    tracing::info!(
        host = %config.host,
        port = %config.port,
        "Starting backend service"
    );

    // データベース接続初期化 (Toasty ORM)
    let db = init_db(&config.database_url).await?;

    // アプリケーション状態構築
    let state = AppState::new(db, config.clone());

    // ルーター構築
    let app = create_router(state);

    // サーバーバインド
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    tracing::info!("Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
