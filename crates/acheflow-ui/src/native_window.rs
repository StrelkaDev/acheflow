//! Construct the studio only after the application bootstrap selects client mode.
//! Uses the pinned dioxus-native launch sequence with a caller-owned event loop.
use blitz_shell::{BlitzShellEvent, BlitzShellProxy, WindowConfig};
use blitz_traits::{
    navigation::{NavigationOptions, NavigationProvider},
    net::{Method, NetProvider},
};
use dioxus_native::{
    DioxusDocument, DioxusNativeApplication, DioxusNativeWindowRenderer, DocumentConfig, prelude::*,
};
use std::{
    sync::{Arc, mpsc::Receiver},
    time::Duration,
};
use winit::{
    application::ApplicationHandler,
    event::{StartCause, WindowEvent},
    event_loop::ActiveEventLoop,
    platform::macos::ApplicationHandlerExtMacOS,
    window::WindowId,
};

pub struct NativeWindow {
    application: DioxusNativeApplication,
    server: crate::server_settings::native::Service,
    runtime: tokio::runtime::Runtime,
    _primary: acheflow_launcher::Primary,
}

struct Navigation;
impl NavigationProvider for Navigation {
    fn navigate_to(&self, options: NavigationOptions) {
        if options.method == Method::GET
            && matches!(options.url.scheme(), "http" | "https" | "mailto")
        {
            let _ = webbrowser::open(options.url.as_str());
        }
    }
}

impl NativeWindow {
    pub fn new(
        primary: acheflow_launcher::Primary,
        proxy: BlitzShellProxy,
        events: Receiver<BlitzShellEvent>,
    ) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| "Cannot start the desktop network runtime")?;
        let application = {
            let _entered = runtime.enter();
            let server = crate::server_settings::native::Service::new()?;
            let vdom = VirtualDom::new(crate::dashboard::dashboard);
            vdom.provide_root_context(primary.inbox());
            vdom.provide_root_context(server.manager.clone());
            let provider = Arc::new(blitz_net::Provider::new(Some(Arc::new(proxy.clone()))));
            vdom.provide_root_context(provider.clone());
            let provider: Arc<dyn NetProvider> = provider;
            vdom.provide_root_context(provider.clone());
            let document = DioxusDocument::new(
                vdom,
                DocumentConfig {
                    net_provider: Some(provider),
                    navigation_provider: Some(Arc::new(Navigation)),
                    font_ctx: Some(crate::font_context()),
                    style_threading: blitz_dom::StyleThreading::Parallel,
                    ..Default::default()
                },
            );
            let attributes = winit::window::WindowAttributes::default()
                .with_title("Acheflow")
                .with_surface_size(winit::dpi::LogicalSize::new(1280.0, 860.0));
            let config = WindowConfig::with_attributes(
                Box::new(document),
                DioxusNativeWindowRenderer::new(),
                attributes,
            );
            (DioxusNativeApplication::new(proxy, events, config), server)
        };
        Ok(Self {
            application: application.0,
            server: application.1,
            runtime,
            _primary: primary,
        })
    }

    pub fn finish(self) -> Result<(), String> {
        drop(self.application);
        let result = self.server.finish();
        self.runtime.shutdown_timeout(Duration::from_secs(2));
        result
    }
}

impl ApplicationHandler for NativeWindow {
    fn new_events(&mut self, event_loop: &dyn ActiveEventLoop, cause: StartCause) {
        let _entered = self.runtime.enter();
        self.application.new_events(event_loop, cause);
    }
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let _entered = self.runtime.enter();
        self.application.can_create_surfaces(event_loop);
    }
    fn destroy_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let _entered = self.runtime.enter();
        self.application.destroy_surfaces(event_loop);
    }
    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let _entered = self.runtime.enter();
        self.application.window_event(event_loop, window_id, event);
    }
    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        let _entered = self.runtime.enter();
        self.application.proxy_wake_up(event_loop);
    }
    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let _entered = self.runtime.enter();
        self.application.about_to_wait(event_loop);
    }
    fn macos_handler(&mut self) -> Option<&mut dyn ApplicationHandlerExtMacOS> {
        Some(self)
    }
}
impl ApplicationHandlerExtMacOS for NativeWindow {
    fn standard_key_binding(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        action: &str,
    ) {
        let _entered = self.runtime.enter();
        if let Some(handler) = self.application.macos_handler() {
            handler.standard_key_binding(event_loop, window_id, action);
        }
    }
}
