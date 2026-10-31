use std::{error::Error, net::SocketAddr};

use axum::{Router, routing::get};
use tower_http::services::ServeDir;
mod compositor;
mod ffmpeg;
mod ingest;
mod live;
mod output;
#[cfg(test)]
mod pipeline_tests;
mod rtmp;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let address: SocketAddr = std::env::var("ACHEFLOW_HTTP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()?;
    let web_root = std::env::var("ACHEFLOW_WEB_ROOT").unwrap_or_else(|_| "dist".to_owned());
    let capabilities = ffmpeg::discover().await;
    eprintln!(
        "FFmpeg: {}",
        capabilities
            .version
            .as_deref()
            .or(capabilities.error.as_deref())
            .unwrap_or("unavailable")
    );
    let media_state: ffmpeg::CapabilityState =
        std::sync::Arc::new(tokio::sync::Mutex::new(capabilities));
    let rtmp_address =
        std::env::var("ACHEFLOW_RTMP_ADDR").unwrap_or_else(|_| "127.0.0.1:1935".into());
    let rtmp_listener = tokio::net::TcpListener::bind(&rtmp_address).await?;
    let runtime = live::Runtime::new(media_state.clone(), rtmp_listener.local_addr()?.to_string());
    let live_routes = Router::new()
        .route("/api/live/status", get(live::status))
        .route(
            "/api/live/config",
            axum::routing::put(live::configure).delete(live::deactivate),
        )
        .route("/api/live/start", axum::routing::post(live::start))
        .route("/api/live/stop", axum::routing::post(live::stop))
        .route("/api/live/preview/{format}", get(live::preview))
        .route("/api/live/source/{id}", get(live::source_preview))
        .with_state(runtime.clone());
    let app = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .route(
            "/api/media/capabilities",
            get(ffmpeg::get).post(ffmpeg::refresh),
        )
        .with_state(media_state)
        .merge(live_routes)
        .fallback_service(ServeDir::new(web_root).append_index_html_on_directories(true));
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!(
        "Acheflow HTTP listening on http://{}",
        listener.local_addr()?
    );

    eprintln!("RTMP ingest listening on {}", runtime.rtmp_address);
    let rtmp_runtime = runtime.clone();
    let rtmp_task = tokio::spawn(async move {
        if let Err(error) = rtmp::serve(rtmp_listener, rtmp_runtime).await {
            eprintln!("RTMP listener stopped: {error}");
        }
    });
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    runtime.close().await;
    let _ = rtmp_task.await;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("Failed to install Ctrl-C handler: {error}");
    }
}
