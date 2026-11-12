mod audio_mixer;
mod client_authorization;
mod code_editor;
mod control_client;
mod dashboard;
mod diagnostics;
pub mod extension_panel;
#[cfg(test)]
mod extension_tests;
mod fps;
mod interaction;
#[cfg(test)]
mod interaction_tests;
mod layer_editor;
mod live;
mod live_client;
mod media;
#[cfg(not(target_arch = "wasm32"))]
mod native_activation;
#[cfg(target_os = "macos")]
pub mod native_window;
mod project;
mod scene_canvas;
mod scene_geometry;
mod scene_preview;
mod server_settings;
mod slider;
#[cfg(test)]
mod slider_tests;
mod sparks;
mod spatial_editor;
mod spatial_overlay;
mod spatial_view;
mod spatial_viewport;
#[cfg(test)]
mod spatial_viewport_tests;
mod studio_mixer;
mod traffic;
mod video_preview;
mod workspaces;

/// The same component tree, stylesheet, font and shader are used on both targets.
pub fn launch() {
    if let Err(error) = launch_with_contexts(vec![]) {
        eprintln!("{error}");
    }
}
fn launch_with_contexts(
    mut contexts: Vec<Box<dyn Fn() -> Box<dyn std::any::Any> + Send + Sync>>,
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    let server = {
        let service = server_settings::native::Service::new()?;
        let manager = service.manager.clone();
        contexts.push(Box::new(move || Box::new(manager.clone())));
        service
    };
    #[cfg(target_arch = "wasm32")]
    let _ = &mut contexts;
    let font_ctx = font_context();
    let config = dioxus_native::Config::new().with_font_ctx(font_ctx);
    #[cfg(not(target_arch = "wasm32"))]
    let config = config.with_window_attributes(
        winit::window::WindowAttributes::default()
            .with_title("Acheflow")
            .with_surface_size(winit::dpi::LogicalSize::new(1280.0, 860.0)),
    );
    dioxus_native::launch_cfg(dashboard::dashboard, contexts, vec![Box::new(config)]);
    #[cfg(not(target_arch = "wasm32"))]
    server.finish()?;
    Ok(())
}

fn font_context() -> dioxus_native::FontContext {
    let mut context =
        dioxus_native::build_single_font_ctx(include_bytes!("../assets/DejaVuSans.woff2"));
    // Blitz's password editor currently paints ordinary glyphs. This font has
    // exactly one visible glyph ('*') for every code point; editing stays native.
    let families = context.collection.register_fonts(
        peniko::Blob::new(std::sync::Arc::new(
            include_bytes!("../assets/SecretMask.ttf").to_vec(),
        )),
        None,
    );
    assert!(
        !families.is_empty(),
        "credential mask font must load before rendering inputs"
    );
    context
}

#[cfg(not(target_arch = "wasm32"))]
pub fn launch_native() -> Result<(), String> {
    use acheflow_launcher::Command;
    let command =
        acheflow_launcher::parse_args(std::env::args_os().skip(1)).map_err(|e| e.to_string())?;
    let request = match command {
        Command::Help => {
            println!(
                "Acheflow desktop\n  --show\n  --forward <HTTPS URL or application URI>\n  --forward-stdin\n  --version"
            );
            return Ok(());
        }
        Command::Version => {
            println!("Acheflow {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Command::Run(request) => request,
        Command::ReadStdin => {
            acheflow_launcher::read_link(std::io::stdin().lock()).map_err(|e| e.to_string())?
        }
    };
    launch_native_request(request)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn launch_native_request(request: acheflow_launcher::Request) -> Result<(), String> {
    #[cfg(not(unix))]
    use acheflow_launcher::{ActivationError, Request};
    #[cfg(unix)]
    {
        use acheflow_extension_api::forwarding::ForwardRecipient;
        use acheflow_launcher::{Startup, default_directory, start};
        let request = request
            .normalize(ForwardRecipient::Client)
            .map_err(|e| e.to_string())?;
        let directory = default_directory().map_err(|e| e.to_string())?;
        match start(ForwardRecipient::Client, request, &directory).map_err(|e| e.to_string())? {
            Startup::HandedOff => Ok(()),
            Startup::Primary(primary) => {
                let inbox = primary.inbox();
                let result = launch_with_contexts(vec![Box::new(move || Box::new(inbox.clone()))]);
                drop(primary);
                result
            }
        }
    }
    #[cfg(not(unix))]
    match request {
        Request::Show => {
            launch();
            Ok(())
        }
        Request::Forward { .. } => Err(ActivationError::Unsupported.to_string()),
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub async fn start() {
    console_error_panic_hook::set_once();
    // Per-frame TRACE spans are expensive in browser WASM and fill the timeline.
    let config = tracing_wasm::WASMLayerConfigBuilder::new()
        .set_max_level(tracing::Level::WARN)
        .set_report_logs_in_timings(false)
        .build();
    tracing_wasm::set_as_global_default_with_config(config);
    media::detect_hosted_server().await;
    launch();
}

#[cfg(test)]
mod layer_editor_tests;
