use std::{env, error::Error, path::PathBuf, sync::Arc};

use axum::{
    Router, middleware,
    routing::{get, post},
};
use dave_wang_6c2c_daily_video::{
    config,
    db,
    middleware::auth::{AdminApiKey, require_admin_api_key},
    pipeline::Pipeline,
    providers::{
        image_to_3d::{ImageTo3DProviderKind, MeshyImageTo3DProvider},
        video::{VeoVideoProvider, VideoProviderKind},
    },
    repo::Repository,
    routes,
    storage::ObjectStorage,
};
use tokio::{net::TcpListener, signal};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    init_tracing();

    let config = config::Config::from_env()?;
    let db_pool = db::connect_and_migrate(&config.database).await?;
    let repository = Repository::new(db_pool.clone());
    let object_storage = ObjectStorage::new(&config.object_storage);
    let video_provider = match VideoProviderKind::from_config(&config.providers)? {
        VideoProviderKind::GeminiVeo => Arc::new(VeoVideoProvider::new(
            config.providers.gemini_api_key.clone(),
        )),
    };
    let image_to_3d_provider = match ImageTo3DProviderKind::from_config(&config.providers)? {
        ImageTo3DProviderKind::Meshy => Arc::new(MeshyImageTo3DProvider::new(
            config.providers.meshy_api_key.clone(),
        )),
    };
    let pipeline = Pipeline::new(
        repository.clone(),
        object_storage.clone(),
        video_provider,
        image_to_3d_provider,
        pipeline_workspace_dir(),
    );
    let videos_state = routes::videos::VideosState::new(repository.clone(), object_storage);
    let admin_state = routes::admin::AdminState::new(repository, pipeline);
    let admin_api_key = AdminApiKey::new(config.admin.api_key)?;
    let videos_router = Router::new()
        .route("/videos", get(routes::videos::list_videos))
        .route("/videos/latest", get(routes::videos::latest_video))
        .with_state(videos_state);
    let admin_router = Router::new()
        .route("/admin/runs", post(routes::admin::trigger_run))
        .route("/admin/runs/{id}/retry", post(routes::admin::retry_run))
        .route_layer(middleware::from_fn_with_state(
            admin_api_key,
            require_admin_api_key,
        ))
        .with_state(admin_state);
    let app = Router::new()
        .route("/health", get(routes::health::health_check))
        .merge(videos_router)
        .merge(admin_router);
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

fn pipeline_workspace_dir() -> PathBuf {
    env::var("PIPELINE_WORKSPACE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/dave-wang-6c2c-daily-video"))
}
