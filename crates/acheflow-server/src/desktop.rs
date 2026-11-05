//! Private parent pipe, never an unauthenticated HTTP shutdown endpoint.
use std::{
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

pub struct Desktop {
    pub instance: Option<String>,
    pub shutdown: CancellationToken,
    pub attached: Arc<AtomicBool>,
}
impl Desktop {
    pub fn new(managed: bool) -> Result<Self, &'static str> {
        let shutdown = CancellationToken::new();
        let attached = Arc::new(AtomicBool::new(managed));
        let instance = if managed {
            let value = std::env::var("ACHEFLOW_DESKTOP_INSTANCE")
                .map_err(|_| "Missing desktop launch identifier")?;
            if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid desktop launch identifier");
            }
            let cancel = shutdown.clone();
            let state = attached.clone();
            // Tokio stdin uses a non-cancellable blocking read which can keep
            // runtime shutdown waiting. A dedicated detached thread has no such
            // runtime dependency and owns no application resources.
            std::thread::Builder::new()
                .name("desktop-owner".into())
                .spawn(move || {
                    let mut byte = [0];
                    let result = std::io::stdin().lock().read(&mut byte);
                    state.store(false, Ordering::Release);
                    if !matches!(result, Ok(1)) || byte[0] != b'D' {
                        cancel.cancel();
                    }
                })
                .map_err(|_| "Cannot monitor desktop parent")?;
            Some(value)
        } else {
            None
        };
        Ok(Self {
            instance,
            shutdown,
            attached,
        })
    }
}

/// A detached server may outlive the reader of its diagnostic pipe. Unlike
/// eprintln!, failed writes must not panic and kill the running server.
pub(crate) fn log(message: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr().lock(), "{message}");
}
