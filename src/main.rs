use std::error::Error;

use axum::{Router, routing::get};
use tokio::{net::TcpListener, signal};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod db;
mod routes {
    pub mod health;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    init_tracing();

    let config = config::Config::from_env()?;
    let _db_pool = db::connect_and_migrate(&config.database).await?;
    let app = Router::new().route("/health", get(routes::health::health_check));
    let addr = config.server.socket_addr();
    let listener = TcpListener::bind(addr).await?;

    info!(%addr, "starting HTTP server");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("HTTP server stopped");
    Ok(())
}

fn init_tracing() {
    let env_filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => EnvFilter::new("info,tower_http=info"),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer())
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = signal::ctrl_c().await {
            error!(error = %err, "failed to install Ctrl+C shutdown handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                error!(error = %err, "failed to install terminate shutdown handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received");
}
