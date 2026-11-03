use clap::Parser;
use std::{error::Error, time::Duration};

use axum::{Router, routing::get};
use tower_http::services::ServeDir;
mod audio_input;
mod audio_mixer;
mod audio_packet;
#[cfg(test)]
mod audio_pipeline_tests;
mod browser_access;
mod client_bridge;
mod compositor;
mod control;
mod extension_host;
mod extension_registry;
mod ffmpeg;
mod ingest;
mod integration_host;
#[cfg(test)]
mod integration_tests;
mod live;
mod mcp;
mod output;
#[cfg(test)]
mod pipeline_tests;
mod rtmp;
mod settings;
mod tls;
mod traffic;
mod workspace_extensions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cors = browser_access::from_env()?;
    let settings = settings::SettingsState::from_args(settings::Args::parse())?;
    let config = &settings.active;
    let http_tls = tls::load(&config.http_tls)?;
    let rtmp_tls = tls::load(&config.rtmp_tls)?;
    let listener =
        tokio::net::TcpListener::bind((config.http_host.as_str(), config.http_port)).await?;
    let rtmp_listener =
        tokio::net::TcpListener::bind((config.rtmp_host.as_str(), config.rtmp_port)).await?;
    eprintln!(
        "Acheflow listening on {}://{}",
        if http_tls.is_some() { "https" } else { "http" },
        listener.local_addr()?
    );
    eprintln!(
        "{} ingest listening on {}",
        if rtmp_tls.is_some() { "RTMPS" } else { "RTMP" },
        rtmp_listener.local_addr()?
    );
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
    let runtime = live::Runtime::new(media_state.clone(), rtmp_listener.local_addr()?.to_string());
    let control_host = control::ControlHost::new(runtime.clone());
    let mcp_host = control_host.clone();
    let mcp_task = tokio::spawn(async move {
        if let Err(error) = mcp::serve(mcp_host).await {
            eprintln!("MCP: {error}");
        }
    });
    let workspace_routes = workspace_extensions::routes(control_host.clone());
    let control_routes = Router::new()
        .route("/api/control/state", get(control::snapshot))
        .route(
            "/api/control/commands",
            axum::routing::post(control::command),
        )
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .with_state(control_host);
    runtime
        .rtmp_tls
        .store(rtmp_tls.is_some(), std::sync::atomic::Ordering::Relaxed);
    let settings_routes = Router::new()
        .route(
            "/api/server/settings",
            get(settings::get).put(settings::save),
        )
        .with_state(settings);
    let live_routes = Router::new()
        .route("/api/live/status", get(live::status))
        .route("/api/live/stop", axum::routing::post(live::stop))
        .route(
            "/api/live/output/restart",
            axum::routing::post(live::restart_output),
        )
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
        .merge(control_routes)
        .merge(workspace_routes)
        .layer(cors)
        .merge(settings_routes)
        .fallback_service(ServeDir::new(web_root).append_index_html_on_directories(true));
    let rtmp_runtime = runtime.clone();
    let rtmp_task = tokio::spawn(async move {
        if let Err(error) = rtmp::serve(rtmp_listener, rtmp_runtime, rtmp_tls).await {
            eprintln!("RTMP listener stopped: {error}");
        }
    });
    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    let shutdown_runtime = runtime.clone();
    let shutdown_task = tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_runtime.shutdown.cancel();
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(10)));
    });
    let listener = listener.into_std()?;
    let result = if let Some(tls) = http_tls {
        axum_server::from_tcp(listener)?
            .acceptor(axum_server::tls_openssl::OpenSSLAcceptor::new(tls))
            .handle(handle)
            .serve(app.into_make_service())
            .await
    } else {
        axum_server::from_tcp(listener)?
            .handle(handle)
            .serve(app.into_make_service())
            .await
    };
    shutdown_task.abort();
    runtime.close().await;
    let _ = mcp_task.await;
    let _ = rtmp_task.await;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("Failed to install Ctrl-C handler: {error}");
    }
}
