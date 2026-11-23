use clap::Parser;
use std::{error::Error, time::Duration};

use axum::{Router, routing::get};
use tower_http::services::ServeDir;
mod access;
mod assets;
mod audio_input;
mod audio_mixer;
mod audio_packet;
#[cfg(test)]
mod audio_pipeline_tests;
mod av_sync;
#[cfg(test)]
mod av_timing_tests;
mod browser_access;
mod camera_input;
mod client_bridge;
mod compositor;
mod control;
mod desktop;
mod diagnostics;
mod encoded_preview;
mod extension_forwarding;
#[cfg(test)]
mod extension_forwarding_tests;
mod extension_host;
mod extension_registry;
mod ffmpeg;
mod ingest;
mod integration_host;
#[cfg(test)]
mod integration_tests;
mod live;
mod mcp;
mod media_monitor;
mod microphone_input;
mod native_activation;
mod output;
#[cfg(test)]
mod pipeline_tests;
mod providers;
mod rtmp;
mod server_workspace;
mod settings;
mod srt_input;
mod tls;
mod traffic;
#[cfg(test)]
mod visual_source_tests;
mod workspace_extensions;
mod workspace_store;

pub use tokio_util::sync::CancellationToken as Shutdown;

/// Shared server entrypoint for the unified executable and development binary.
/// The optional initial link was already bounded, normalized and recipient-checked.
pub fn run_from(
    args: impl IntoIterator<Item = std::ffi::OsString>,
    request: Option<acheflow_launcher::Request>,
) -> Result<(), Box<dyn Error>> {
    let args = settings::Args::parse_from(args);
    #[cfg(target_os = "macos")]
    {
        std::thread::scope(|scope| {
            let backend = scope.spawn(move || {
                run_parsed(args, request, Shutdown::new()).map_err(|error| error.to_string())
            });
            // AVFoundation delivers hardware changes through the main run loop, even
            // when discovery and capture are called from background threads.
            acheflow_camera::run_main_loop_until(|| backend.is_finished());
            backend.join().map_err(|_| "Server runtime failed")??;
            Ok(())
        })
    }
    #[cfg(not(target_os = "macos"))]
    run_parsed(args, request, Shutdown::new())
}

/// Runs the headless backend while an embedding application owns the OS event loop.
pub fn run_with_shutdown(
    args: impl IntoIterator<Item = std::ffi::OsString>,
    request: Option<acheflow_launcher::Request>,
    shutdown: Shutdown,
) -> Result<(), Box<dyn Error>> {
    let args = settings::Args::parse_from(args);
    run_parsed(args, request, shutdown)
}

fn run_parsed(
    args: settings::Args,
    request: Option<acheflow_launcher::Request>,
    shutdown: Shutdown,
) -> Result<(), Box<dyn Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(args, request, shutdown))
}

async fn serve(
    args: settings::Args,
    request: Option<acheflow_launcher::Request>,
    shutdown: Shutdown,
) -> Result<(), Box<dyn Error>> {
    let desktop = desktop::Desktop::new(args.desktop_managed)?;
    let receiver = match native_activation::prepare(&args, request).await? {
        native_activation::Launch::Serve(receiver) => receiver,
        #[cfg(unix)]
        native_activation::Launch::HandedOff => return Ok(()),
    };
    let cors = browser_access::from_env()?;
    let access_path = args.config.with_extension("access.json");
    let workspace_path = args.config.with_extension("workspace.json");
    let settings = settings::SettingsState::from_args(args)?;
    let config = &settings.active;
    let http_tls = tls::load(&config.http_tls)?;
    let rtmp_tls = tls::load(&config.rtmp_tls)?;
    let listener =
        tokio::net::TcpListener::bind((config.http_host.as_str(), config.http_port)).await?;
    let rtmp_listener =
        tokio::net::TcpListener::bind((config.rtmp_host.as_str(), config.rtmp_port)).await?;
    crate::desktop::log(format_args!(
        "Acheflow listening on {}://{}",
        if http_tls.is_some() { "https" } else { "http" },
        listener.local_addr()?
    ));
    crate::desktop::log(format_args!(
        "{} ingest listening on {}",
        if rtmp_tls.is_some() { "RTMPS" } else { "RTMP" },
        rtmp_listener.local_addr()?
    ));
    let web_root = assets::web_root();
    let capabilities = tokio::select! {
        _ = shutdown.cancelled() => return Ok(()),
        _ = desktop.shutdown.cancelled() => return Ok(()),
        capabilities = ffmpeg::discover() => capabilities,
    };
    crate::desktop::log(format_args!(
        "FFmpeg: {}",
        capabilities
            .version
            .as_deref()
            .or(capabilities.error.as_deref())
            .unwrap_or("unavailable")
    ));
    let media_state: ffmpeg::CapabilityState =
        std::sync::Arc::new(tokio::sync::Mutex::new(capabilities));
    let runtime = live::Runtime::new(media_state.clone(), rtmp_listener.local_addr()?.to_string());
    let control_host = control::ControlHost::new(runtime.clone());
    let access = access::Access::open(access_path, control_host.clone())?;
    control_host.providers.set_server(&access.server_id())?;
    runtime.attach_store(workspace_store::Store::open(
        workspace_path,
        &access.server_id(),
    )?)?;
    // Restoring the saved studio resumes its collectors, never media output.
    control_host.providers.reconcile(&control_host).await;
    let native_task = receiver.map(|receiver| tokio::spawn(receiver.run(control_host.clone())));
    let mcp_host = control_host.clone();
    let mcp_task = tokio::spawn(async move {
        if let Err(error) = mcp::serve(mcp_host).await {
            crate::desktop::log(format_args!("MCP: {error}"));
        }
    });
    let workspace_routes = workspace_extensions::routes(control_host.clone());
    let control_routes = Router::new()
        .route("/api/control/state", get(control::snapshot))
        .route(
            "/api/control/executor",
            axum::routing::post(client_bridge::studio),
        )
        .route(
            "/api/control/commands",
            axum::routing::post(control::command),
        )
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .with_state(control_host.clone());
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
        .route("/api/live/mixer", axum::routing::post(live::edit_mixer))
        .route("/api/live/sources", axum::routing::post(live::save_source))
        .route(
            "/api/live/output/profile",
            axum::routing::post(live::save_output_profile),
        )
        .route("/api/live/scenes", axum::routing::post(live::save_scene))
        .route("/api/live/take", axum::routing::post(live::take_scene))
        .route("/api/live/stop", axum::routing::post(live::stop))
        .route(
            "/api/live/output/restart",
            axum::routing::post(live::restart_output),
        )
        .route(
            "/api/live/outputs",
            axum::routing::post(live::manage_outputs),
        )
        .route("/api/live/preview/{format}", get(live::preview))
        .route(
            "/api/live/source-preview",
            axum::routing::post(live::renew_source_preview),
        )
        .route("/api/live/source/{id}", get(live::source_preview))
        .with_state(runtime.clone());
    let app = Router::new()
        .route("/api/media/cameras", get(camera_input::list))
        .route("/api/media/microphones", get(microphone_input::list))
        .route("/api/server/logs", get(diagnostics::get))
        .route("/healthz", get(|| async { "ok\n" }))
        .route(
            "/api/server/info",
            get({
                let info = acheflow_media_api::server::ServerInfo {
                    protocol_version: acheflow_media_api::server::SERVER_PROTOCOL_VERSION,
                    version: env!("CARGO_PKG_VERSION").into(),
                    process_id: std::process::id(),
                    managed_instance: desktop.instance,
                    desktop_attached: false,
                    http_address: listener.local_addr()?,
                    rtmp_address: rtmp_listener.local_addr()?,
                };
                let attached = desktop.attached;
                move || {
                    let mut info = info.clone();
                    info.desktop_attached = attached.load(std::sync::atomic::Ordering::Acquire);
                    async move { axum::Json(info) }
                }
            }),
        )
        .route(
            "/api/media/capabilities",
            get(ffmpeg::get).post(ffmpeg::refresh),
        )
        .with_state(media_state)
        .merge(live_routes)
        .merge(control_routes)
        .merge(workspace_routes)
        .merge(server_workspace::routes(control_host.clone()))
        .merge(
            access
                .routes()
                .layer(axum::extract::DefaultBodyLimit::max(16 * 1024)),
        )
        .layer(cors)
        .merge(settings_routes)
        .fallback_service(ServeDir::new(web_root).append_index_html_on_directories(true))
        .layer(axum::middleware::from_fn(diagnostics::requests));
    let app = access.protect(app);
    let rtmp_runtime = runtime.clone();
    let rtmp_task = tokio::spawn(async move {
        if let Err(error) = rtmp::serve(rtmp_listener, rtmp_runtime, rtmp_tls).await {
            crate::desktop::log(format_args!("RTMP listener stopped: {error}"));
        }
    });
    let handle = axum_server::Handle::new();
    let shutdown_handle = handle.clone();
    let shutdown_runtime = runtime.clone();
    let shutdown_task = tokio::spawn(async move {
        tokio::select! {
            _ = shutdown_signal() => {},
            _ = shutdown.cancelled() => {},
            _ = desktop.shutdown.cancelled() => {},
        }
        shutdown_runtime.shutdown.cancel();
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(10)));
    });
    let listener = listener.into_std()?;
    let result = if let Some(tls) = http_tls {
        axum_server::from_tcp(listener)?
            .acceptor(axum_server::tls_openssl::OpenSSLAcceptor::new(tls))
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
    } else {
        axum_server::from_tcp(listener)?
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
    };
    shutdown_task.abort();
    control_host.providers.suspend(&control_host).await;
    runtime.close().await;
    if let Some(task) = native_task {
        let _ = task.await;
    }
    let _ = mcp_task.await;
    let _ = rtmp_task.await;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        crate::desktop::log(format_args!("Failed to install Ctrl-C handler: {error}"));
    }
}
