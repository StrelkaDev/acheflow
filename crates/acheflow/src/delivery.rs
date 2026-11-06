//! Warm OS activations use bounded background IPC, never the GUI thread.
use acheflow_launcher::{ActivationError, INBOX_CAPACITY, Request, Startup};
use std::{
    io::Write,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub struct Delivery {
    active: Arc<AtomicUsize>,
    proxy: winit::event_loop::EventLoopProxy,
}
struct Permit(Arc<AtomicUsize>, winit::event_loop::EventLoopProxy);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
        self.1.wake_up();
    }
}
impl Delivery {
    pub fn new(proxy: winit::event_loop::EventLoopProxy) -> Self {
        Self {
            active: Arc::default(),
            proxy,
        }
    }
    pub fn is_idle(&self) -> bool {
        self.active.load(Ordering::Acquire) == 0
    }

    pub fn submit(&self, request: Request) -> Result<(), ActivationError> {
        let (recipient, request) = request.route()?;
        self.active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < INBOX_CAPACITY).then_some(active + 1)
            })
            .map_err(|_| ActivationError::Busy)?;
        let permit = Permit(self.active.clone(), self.proxy.clone());
        std::thread::Builder::new()
            .name("native-link-delivery".into())
            .spawn(move || {
                let result = acheflow_launcher::default_directory().and_then(|directory| {
                    match acheflow_launcher::start(recipient, request.clone(), &directory)? {
                        Startup::HandedOff => Ok(None),
                        Startup::Primary(primary) => {
                            drop(primary);
                            cold_start(request).map(Some)
                        }
                    }
                });
                let child = match result {
                    Ok(child) => child,
                    Err(error) => {
                        eprintln!("Native activation: {error}");
                        None
                    }
                };
                // Submission is complete before waiting for an independent
                // recipient's lifetime. A transient forwarder can now exit.
                drop(permit);
                if let Some(mut child) = child
                    && !child.wait().is_ok_and(|status| status.success())
                {
                    eprintln!("Native activation: {}", ActivationError::Rejected);
                }
            })
            .map_err(|_| ActivationError::Unavailable)?;
        Ok(())
    }
}

fn cold_start(request: Request) -> Result<Child, ActivationError> {
    let executable = std::env::current_exe().map_err(|_| ActivationError::Unavailable)?;
    let mut command = Command::new(executable);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    match &request {
        Request::Show => {
            command.arg("--show");
        }
        Request::Forward { .. } => {
            command.arg("--forward-stdin").stdin(Stdio::piped());
        }
    }
    let mut child = command.spawn().map_err(|_| ActivationError::Unavailable)?;
    if let Request::Forward { url } = request {
        let result = child
            .stdin
            .take()
            .ok_or(ActivationError::Unavailable)
            .and_then(|mut input| {
                input
                    .write_all(url.as_bytes())
                    .map_err(|_| ActivationError::Unavailable)
            });
        if let Err(error) = result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    }
    Ok(child)
}
