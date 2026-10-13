//! shapefile-converter: zip a shapefile in, GeoJSON or FlatGeobuf out.
//!
//! Coordinates pass through verbatim — no reprojection. The source CRS
//! (from the .prj WKT) is carried into the output metadata.

use shapefile_converter::{config, routes};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let app_config = config::AppConfig::from_env();
    let app = routes::router(app_config.limits);

    let listener = tokio::net::TcpListener::bind((app_config.hostname, app_config.port))
        .await
        .expect("failed to bind");
    tracing::info!(app_config.port, "listening");

    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("install SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        {
            let _ = ctrl_c.await;
        }
        tracing::info!("shutdown signal received");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("server error");
}
