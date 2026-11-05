//! Private parent pipe, never an unauthenticated HTTP shutdown endpoint.
use std::io::Read;
use tokio_util::sync::CancellationToken;

pub struct Desktop {
    pub instance: Option<String>,
    pub shutdown: CancellationToken,
}
impl Desktop {
    pub fn new(managed: bool) -> Result<Self, &'static str> {
        let shutdown = CancellationToken::new();
        let instance = if managed {
            let value = std::env::var("ACHEFLOW_DESKTOP_INSTANCE")
                .map_err(|_| "Missing desktop launch identifier")?;
            if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid desktop launch identifier");
            }
            let cancel = shutdown.clone();
            // Tokio stdin uses a non-cancellable blocking read which can keep
            // runtime shutdown waiting. A dedicated detached thread has no such
            // runtime dependency and owns no application resources.
            std::thread::Builder::new()
                .name("desktop-owner".into())
                .spawn(move || {
                    let _ = std::io::stdin().lock().read(&mut [0]);
                    cancel.cancel();
                })
                .map_err(|_| "Cannot monitor desktop parent")?;
            Some(value)
        } else {
            None
        };
        Ok(Self { instance, shutdown })
    }
}
