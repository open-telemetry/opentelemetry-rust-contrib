mod app;
mod config;
mod decode;
mod gcs;
mod ingest;
mod models;
mod sqlite;

use crate::app::build_router;
use crate::config::ServerConfig;
use crate::sqlite::spawn_worker;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = ServerConfig::from_env()?;
    let (state, worker) = spawn_worker(config.clone())?;
    let app = build_router(Arc::new(state));
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;

    info!(
        listen_addr = %config.listen_addr,
        public_base_url = %config.public_base_url,
        db_path = %config.db_path.display(),
        "geneva-test-server listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    drop(worker);
    Ok(())
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "geneva_test_server=info,tower_http=info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};

        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
