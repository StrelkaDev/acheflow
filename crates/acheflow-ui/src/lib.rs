mod audio_mixer;
mod client_authorization;
mod code_editor;
mod control_client;
mod dashboard;
pub mod extension_panel;
#[cfg(test)]
mod extension_tests;
mod fps;
mod interaction;
mod layer_editor;
mod live;
mod live_client;
mod media;
mod project;
mod scene_canvas;
mod scene_geometry;
mod scene_preview;
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
    let font_ctx =
        dioxus_native::build_single_font_ctx(include_bytes!("../assets/DejaVuSans.woff2"));
    let config = dioxus_native::Config::new().with_font_ctx(font_ctx);
    #[cfg(not(target_arch = "wasm32"))]
    let config = config.with_window_attributes(
        winit::window::WindowAttributes::default()
            .with_title("Acheflow")
            .with_surface_size(winit::dpi::LogicalSize::new(1280.0, 860.0)),
    );
    dioxus_native::launch_cfg(dashboard::dashboard, vec![], vec![Box::new(config)]);
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
