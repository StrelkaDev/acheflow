//! Local executor. Platform credentials and provider transports never enter the server.
pub mod catalog;
#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;
#[cfg(not(target_arch = "wasm32"))]
pub mod transport;

#[cfg(not(target_arch = "wasm32"))]
pub mod bridge;
