//! Local executor. Platform credentials and provider transports never enter the server.
#[cfg(not(target_arch = "wasm32"))]
pub mod browser;
pub mod catalog;
pub mod local;
#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(not(target_arch = "wasm32"))]
mod stream;
pub mod transport;
#[cfg(not(target_arch = "wasm32"))]
mod vault;

#[cfg(not(target_arch = "wasm32"))]
pub mod bridge;
