//! Native launch driver: receive activation, select role, then initialize services.
use crate::{
    bootstrap::{Bootstrap, Selection},
    delivery::Delivery,
    native_events::Registration,
};
use acheflow_extension_api::forwarding::ForwardRecipient;
use acheflow_launcher::{ActivationError, HANDOFF_TIMEOUT, Request, Startup};
use acheflow_ui::native_window::NativeWindow;
use std::{ffi::OsString, sync::mpsc, thread::JoinHandle, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{StartCause, WindowEvent},
    event_loop::{
        ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy,
        run_on_demand::EventLoopExtRunOnDemand,
    },
    platform::macos::{ActivationPolicy, ApplicationHandlerExtMacOS, EventLoopBuilderExtMacOS},
    window::WindowId,
};

pub fn run(request: Request, client_only: bool, os_launch: bool) -> Result<(), String> {
    let mut builder = EventLoop::builder();
    builder.with_activation_policy(ActivationPolicy::Accessory);
    let mut event_loop = builder
        .build()
        .map_err(|_| "Cannot initialize native application events")?;
    // Winit must create its application before installing the custom delegate.
    let registration = Registration::install(event_loop.create_proxy())?;
    let mut bootstrap = Bootstrap::new(request, client_only);
    if !os_launch {
        bootstrap.launch_finished(true);
    }
    let mut application = Application {
        bootstrap,
        registration,
        os_launch,
        selected: false,
        handoff_only: false,
        quitting: false,
        surfaces_allowed: false,
        deadline: Instant::now() + HANDOFF_TIMEOUT,
        window: None,
        backend: None,
        backend_completed: false,
        delivery: Delivery::new(event_loop.create_proxy()),
        proxy: event_loop.create_proxy(),
        error: None,
    };
    let result = event_loop
        .run_app_on_demand(&mut application)
        .map_err(|_| "Native application loop failed".to_owned());
    let window_result = application
        .window
        .take()
        .map(NativeWindow::finish)
        .unwrap_or(Ok(()));
    let server_result = application
        .backend
        .take()
        .map(Backend::finish)
        .unwrap_or(Ok(()));
    result
        .and(window_result)
        .and(server_result)
        .and(application.error.map_or(Ok(()), Err))
}

/// Native link activation uses writable app data, including a recipient started
/// later by an already running application. Explicit standalone CLI stays separate.
pub fn activation_server_args() -> Result<Vec<OsString>, String> {
    let mut args = vec![OsString::from("--native-links")];
    if std::env::var_os("ACHEFLOW_CONFIG").is_none() {
        let directory = std::path::PathBuf::from(
            std::env::var_os("HOME").ok_or("Cannot resolve native application data")?,
        )
        .join("Library/Application Support/Acheflow/Server");
        std::fs::create_dir_all(&directory).map_err(|_| "Cannot prepare native server data")?;
        args.extend([
            "--config".into(),
            directory.join("acheflow-server.json").into_os_string(),
        ]);
    }
    Ok(args)
}

struct Backend {
    shutdown: acheflow_server::Shutdown,
    completed: mpsc::Receiver<Result<(), String>>,
    worker: Option<JoinHandle<()>>,
}
impl Backend {
    fn start(request: Request, proxy: EventLoopProxy) -> Result<Self, String> {
        let shutdown = acheflow_server::Shutdown::new();
        let cancel = shutdown.clone();
        let (sender, completed) = mpsc::sync_channel(1);
        let mut args = vec![OsString::from("acheflow-server")];
        args.extend(activation_server_args()?);
        let worker = std::thread::Builder::new()
            .name("acheflow-server".into())
            .spawn(move || {
                let result = acheflow_server::run_with_shutdown(args, Some(request), cancel)
                    .map_err(|error| error.to_string());
                let _ = sender.send(result);
                proxy.wake_up();
            })
            .map_err(|_| "Cannot start the server runtime")?;
        Ok(Self {
            shutdown,
            completed,
            worker: Some(worker),
        })
    }
    fn finish(mut self) -> Result<(), String> {
        self.shutdown.cancel();
        self.worker
            .take()
            .ok_or("Server runtime is missing")?
            .join()
            .map_err(|_| "Server runtime failed")?;
        Ok(())
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

struct Application {
    bootstrap: Bootstrap,
    registration: Registration,
    os_launch: bool,
    selected: bool,
    handoff_only: bool,
    quitting: bool,
    surfaces_allowed: bool,
    deadline: Instant,
    window: Option<NativeWindow>,
    backend: Option<Backend>,
    backend_completed: bool,
    delivery: Delivery,
    proxy: EventLoopProxy,
    error: Option<String>,
}
impl Application {
    fn fail(&mut self, event_loop: &dyn ActiveEventLoop, error: impl ToString) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }
    fn select(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        selection: Selection,
    ) -> Result<(), String> {
        event_loop.set_control_flow(ControlFlow::Wait);
        match selection.recipient {
            ForwardRecipient::Client => {
                let directory =
                    acheflow_launcher::default_directory().map_err(|error| error.to_string())?;
                match acheflow_launcher::start(
                    ForwardRecipient::Client,
                    selection.request,
                    &directory,
                )
                .map_err(|error| error.to_string())?
                {
                    Startup::HandedOff => self.handoff_only = true,
                    Startup::Primary(primary) => {
                        let (proxy, events) = blitz_shell::BlitzShellProxy::new(self.proxy.clone());
                        let mut window = NativeWindow::new(primary, proxy, events)?;
                        self.registration.show_dock();
                        window.can_create_surfaces(event_loop);
                        self.window = Some(window);
                    }
                }
            }
            ForwardRecipient::Server => {
                self.backend = Some(Backend::start(selection.request, self.proxy.clone())?)
            }
        }
        self.selected = true;
        Ok(())
    }
    fn receive(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        request: Request,
    ) -> Result<(), String> {
        if self.selected {
            self.bootstrap
                .route(request)
                .and_then(|(_, request)| self.delivery.submit(request))
                .map_err(|error| error.to_string())
        } else {
            let selection = self
                .bootstrap
                .activate(request)
                .map_err(|error| error.to_string())?;
            self.select(event_loop, selection)
        }
    }
    fn select_default(&mut self, event_loop: &dyn ActiveEventLoop) -> Result<(), String> {
        if self.selected {
            return Ok(());
        }
        if let Some(selection) = self
            .bootstrap
            .default_selection()
            .map_err(|error| error.to_string())?
        {
            return self.select(event_loop, selection);
        }
        if Instant::now() >= self.deadline {
            return Err(ActivationError::Unavailable.to_string());
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
        Ok(())
    }
    fn poll_backend(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !self.backend_completed
            && let Some(backend) = &self.backend
        {
            match backend.completed.try_recv() {
                Ok(result) => {
                    self.error = result.err();
                    self.backend_completed = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("Server runtime stopped unexpectedly".into());
                    self.backend_completed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        // A server-mode transient process can hand off its first URL while
        // further URLs from the same native batch are still being delivered.
        if self.backend_completed && self.delivery.is_idle() {
            event_loop.exit();
        }
    }
    fn advance(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !self.surfaces_allowed {
            return;
        }
        let events = self.registration.take();
        if events.quit {
            self.quitting = true;
            if let Some(backend) = &self.backend {
                backend.shutdown.cancel();
            } else {
                event_loop.exit();
            }
        }
        if self.quitting {
            self.poll_backend(event_loop);
            return;
        }
        if let Some(default) = events.default_launch
            && self.os_launch
        {
            self.bootstrap.launch_finished(default);
        }
        for request in events.requests {
            if let Err(error) = self.receive(event_loop, request) {
                if !self.selected {
                    self.fail(event_loop, error);
                    return;
                }
                eprintln!("Native activation: {error}");
            }
        }
        if let Err(error) = self.select_default(event_loop) {
            self.fail(event_loop, error);
            return;
        }
        if self.handoff_only && self.delivery.is_idle() {
            event_loop.exit();
        }
        self.poll_backend(event_loop);
    }
}
impl ApplicationHandler for Application {
    fn new_events(&mut self, event_loop: &dyn ActiveEventLoop, cause: StartCause) {
        if let Some(window) = &mut self.window {
            window.new_events(event_loop, cause);
        }
    }
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.surfaces_allowed = true;
        if let Some(window) = &mut self.window {
            window.can_create_surfaces(event_loop);
        }
    }
    fn destroy_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if let Some(window) = &mut self.window {
            window.destroy_surfaces(event_loop);
        }
    }
    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(window) = &mut self.window {
            window.window_event(event_loop, id, event);
        }
    }
    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.advance(event_loop);
        if let Some(window) = &mut self.window {
            window.proxy_wake_up(event_loop);
        }
    }
    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.advance(event_loop);
        if let Some(window) = &mut self.window {
            window.about_to_wait(event_loop);
        }
    }
    fn macos_handler(&mut self) -> Option<&mut dyn ApplicationHandlerExtMacOS> {
        Some(self)
    }
}
impl ApplicationHandlerExtMacOS for Application {
    fn standard_key_binding(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        action: &str,
    ) {
        if let Some(window) = &mut self.window {
            window.standard_key_binding(event_loop, id, action);
        }
    }
}
